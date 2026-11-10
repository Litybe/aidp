//! `/api/memory` and `/api/rag`: the project memory and the RAG search over HTTP.
//!
//! Writes are refused in read-only mode by the central guard of `http.rs`.

use crate::http::App;
use crate::memory_ingest::{Enqueued, MemoryHub, ReindexJob};
use crate::models::enums::ApiStatus;
use crate::unity_routes::resolve_project;
use aidp_memory::models::constants::limits::DEFAULT_K;
use aidp_memory::models::memory_filter::MemoryFilter;
use aidp_memory::models::memory_patch::MemoryPatch;
use aidp_memory::models::memory_ref::MemoryRef;
use aidp_memory::models::memory_scope::MemoryScope;
use aidp_memory::{
    pack, Bm25Retriever, Hit, Memory, MemoryError, MemoryKind, MemoryManager, MemoryStatus,
    NewMemory, ProjectRef, Retriever, Route, SearchQuery, Source,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tiny_http::Method;

const MEMORY_SEGMENT: &str = "memory";
const RAG_SEGMENT: &str = "rag";
const QUERY_PROJECT: &str = "project";
const DEFAULT_BUDGET: usize = 1500;
const STATUS_OK: u16 = 200;
const STATUS_CREATED: u16 = 201;
const STATUS_ACCEPTED: u16 = 202;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_CONFLICT: u16 = 409;
const STATUS_INTERNAL: u16 = 500;
const TRIGGER_API: &str = "api";

type Reply = (u16, Value);

fn failure(code: u16, message: impl ToString) -> Reply {
    let status = ApiStatus::from_status(code);
    (
        code,
        json!({ "error": message.to_string(), "code": status.code() }),
    )
}

fn memory_failure(error: MemoryError) -> Reply {
    let code = match &error {
        MemoryError::NotFound(_) => STATUS_NOT_FOUND,
        MemoryError::Conflict(_) => STATUS_CONFLICT,
        MemoryError::Invalid(_) | MemoryError::Json(_) => STATUS_BAD_REQUEST,
        MemoryError::Io(_) | MemoryError::Database(_) => STATUS_INTERNAL,
    };
    failure(code, error)
}

/// Body of `POST /api/memory`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBody {
    kind: MemoryKind,
    title: String,
    text: String,
    #[serde(default)]
    scope: MemoryScope,
    #[serde(default)]
    refs: Vec<MemoryRef>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    status: Option<MemoryStatus>,
    #[serde(default)]
    supersedes: Option<String>,
}

/// Body of `PATCH /api/memory/:id`; `scope` fields may also sit at the top level.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PatchBody {
    kind: Option<MemoryKind>,
    title: Option<String>,
    text: Option<String>,
    confidence: Option<f64>,
    route: Option<Route>,
    paths: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    scope: Option<PatchScope>,
}

#[derive(Deserialize, Default)]
struct PatchScope {
    route: Option<Route>,
    paths: Option<Vec<String>>,
    tags: Option<Vec<String>>,
}

impl From<PatchBody> for MemoryPatch {
    fn from(body: PatchBody) -> Self {
        let scope = body.scope.unwrap_or_default();
        MemoryPatch {
            kind: body.kind,
            title: body.title,
            text: body.text,
            route: scope.route.or(body.route),
            paths: scope.paths.or(body.paths),
            tags: scope.tags.or(body.tags),
            confidence: body.confidence,
        }
    }
}

fn parse_body<T: for<'de> Deserialize<'de>>(body: &Value) -> Result<T, Reply> {
    serde_json::from_value(body.clone())
        .map_err(|error| failure(STATUS_BAD_REQUEST, format!("invalid body: {error}")))
}

fn parse_filter(query: &HashMap<String, String>) -> Result<MemoryFilter, Reply> {
    let non_empty = |name: &str| {
        query
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    };
    Ok(MemoryFilter {
        status: non_empty("status")
            .map(MemoryStatus::parse)
            .transpose()
            .map_err(memory_failure)?,
        kind: non_empty("kind")
            .map(MemoryKind::parse)
            .transpose()
            .map_err(memory_failure)?,
        query: non_empty("q").map(str::to_string),
    })
}

/// Everything a handler needs: the hub, the manager's store and the project.
struct Scope {
    hub: std::sync::Arc<MemoryHub>,
    store: std::sync::Arc<aidp_memory::MemoryStore>,
    project: ProjectRef,
}

fn scope(app: &App, requested: Option<&str>) -> Result<Scope, Reply> {
    let project = resolve_project(app, requested)?;
    let store = app.memory.store().map_err(memory_failure)?;
    Ok(Scope {
        hub: app.memory.clone(),
        store,
        project: MemoryHub::project_ref(&project),
    })
}

fn memory_json(memory: &Memory) -> Value {
    serde_json::to_value(memory).unwrap_or(Value::Null)
}

fn list(scope: &Scope, query: &HashMap<String, String>) -> Reply {
    let filter = match parse_filter(query) {
        Ok(filter) => filter,
        Err(reply) => return reply,
    };
    match MemoryManager::new(&scope.store).list(&scope.project.key, &filter) {
        Ok(memories) => (
            STATUS_OK,
            Value::Array(memories.iter().map(memory_json).collect()),
        ),
        Err(error) => memory_failure(error),
    }
}

fn create(scope: &Scope, body: &Value) -> Reply {
    let parsed: CreateBody = match parse_body(body) {
        Ok(parsed) => parsed,
        Err(reply) => return reply,
    };
    let mut new = NewMemory::proposal(
        scope.project.key.clone(),
        parsed.kind,
        parsed.title,
        parsed.text,
    );
    new.scope = parsed.scope;
    new.refs = parsed.refs;
    new.supersedes = parsed.supersedes;
    if let Some(confidence) = parsed.confidence {
        new.confidence = confidence;
    }
    if let Some(status) = parsed.status {
        new.status = status;
    }
    match MemoryManager::new(&scope.store).add(new) {
        Ok(outcome) => {
            scope.hub.sync_notes(&scope.project);
            if outcome.memory.status == MemoryStatus::Proposed && !outcome.deduplicated {
                scope.hub.emit_proposed(&outcome.memory);
            } else {
                scope.hub.emit_changed(&outcome.memory, "added");
            }
            let code = if outcome.deduplicated {
                STATUS_OK
            } else {
                STATUS_CREATED
            };
            (
                code,
                json!({
                    "memory": memory_json(&outcome.memory),
                    "deduplicated": outcome.deduplicated,
                    "activated": outcome.activated,
                }),
            )
        }
        Err(error) => memory_failure(error),
    }
}

fn changed(scope: &Scope, action: &str, result: aidp_memory::Result<Memory>) -> Reply {
    match result {
        Ok(memory) => {
            scope.hub.sync_notes(&scope.project);
            scope.hub.emit_changed(&memory, action);
            (STATUS_OK, memory_json(&memory))
        }
        Err(error) => memory_failure(error),
    }
}

fn memory_route(
    scope: &Scope,
    method: &Method,
    rest: &[&str],
    query: &HashMap<String, String>,
    body: &Value,
) -> Option<Reply> {
    let manager = MemoryManager::new(&scope.store);
    let project = scope.project.key.as_str();
    Some(match (method, rest) {
        (Method::Get, []) => list(scope, query),
        (Method::Post, []) => create(scope, body),
        (Method::Get, [id]) => match manager.get(project, id) {
            Ok(memory) => (STATUS_OK, memory_json(&memory)),
            Err(error) => memory_failure(error),
        },
        (Method::Patch, [id]) => {
            let patch: PatchBody = match parse_body(body) {
                Ok(parsed) => parsed,
                Err(reply) => return Some(reply),
            };
            changed(scope, "edited", manager.edit(project, id, &patch.into()))
        }
        (Method::Post, [id, "approve"]) => changed(scope, "approved", manager.approve(project, id)),
        (Method::Post, [id, "archive"]) => changed(scope, "archived", manager.archive(project, id)),
        (Method::Post, [id, "merge"]) => match body.get("into").and_then(Value::as_str) {
            Some(into) => changed(scope, "merged", manager.merge(project, id, into)),
            None => failure(
                STATUS_BAD_REQUEST,
                "`into` (the target memory id) is required",
            ),
        },
        _ => return None,
    })
}

fn hit_json(hit: &Hit) -> Value {
    json!({
        "id": hit.id,
        "source": hit.source.as_str(),
        "path": hit.path,
        "lines": format!("L{}-L{}", hit.line_start, hit.line_end),
        "citation": hit.citation(),
        "title": hit.title,
        "score": (hit.score * 1000.0).round() / 1000.0,
        "via": hit.via,
        "text": hit.text,
    })
}

fn search(scope: &Scope, query: &HashMap<String, String>) -> Reply {
    let text = query.get("q").map(String::as_str).unwrap_or_default();
    if text.trim().is_empty() {
        return failure(STATUS_BAD_REQUEST, "`q` (the search text) is required");
    }
    let number = |name: &str, default: usize| {
        query
            .get(name)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default)
    };
    let mut request = SearchQuery::new(scope.project.key.clone(), text);
    request.k = number("k", DEFAULT_K);
    request.path_glob = query.get("path").filter(|v| !v.is_empty()).cloned();
    if let Some(raw) = query.get("source").filter(|v| !v.is_empty()) {
        match Source::parse_list(raw) {
            Ok(sources) => request.sources = sources,
            Err(error) => return memory_failure(error),
        }
    }
    let retriever = Bm25Retriever::new(
        &scope.store,
        scope
            .project
            .root
            .is_dir()
            .then(|| scope.project.root.clone()),
    );
    match retriever.search(&request) {
        Ok(hits) => {
            let packed = pack(hits, number("budget", DEFAULT_BUDGET));
            (
                STATUS_OK,
                json!({
                    "project": scope.project.key,
                    "query": text,
                    "usedTokens": packed.used_tokens,
                    "budgetTokens": packed.budget_tokens,
                    "dropped": packed.dropped,
                    "hits": packed.hits.iter().map(hit_json).collect::<Vec<_>>(),
                }),
            )
        }
        Err(error) => memory_failure(error),
    }
}

fn reindex(scope: &Scope, body: &Value) -> Reply {
    if !scope.project.root.is_dir() {
        return failure(STATUS_NOT_FOUND, "project folder not found");
    }
    let sources = match body.get("source").and_then(Value::as_str) {
        Some(raw) => match Source::parse_list(raw) {
            Ok(sources) => sources,
            Err(error) => return memory_failure(error),
        },
        None => Vec::new(),
    };
    if scope.hub.reindexing() {
        return failure(STATUS_CONFLICT, "a re-index is already running");
    }
    let job = ReindexJob {
        project: scope.project.clone(),
        sources,
        full: body.get("full").and_then(Value::as_bool).unwrap_or(false),
        trigger: TRIGGER_API.to_string(),
    };
    match scope.hub.enqueue_reindex(job) {
        Enqueued::Started => (
            STATUS_ACCEPTED,
            json!({ "ok": true, "started": true, "project": scope.project.key }),
        ),
        Enqueued::Queued | Enqueued::Duplicate => {
            failure(STATUS_CONFLICT, "a re-index is already running")
        }
    }
}

/// Answers the `/api/memory` and `/api/rag` routes; `None` for any other path.
pub fn handle(
    app: &App,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    body: &Value,
) -> Option<Reply> {
    let segments: Vec<&str> = path
        .trim_start_matches("/api")
        .trim_matches('/')
        .split('/')
        .collect();
    let (head, rest) = segments.split_first()?;
    let known = match *head {
        MEMORY_SEGMENT => true,
        RAG_SEGMENT => matches!(
            (method, rest),
            (Method::Get, ["search"]) | (Method::Post, ["reindex"])
        ),
        _ => false,
    };
    if !known {
        return None;
    }
    let requested = query
        .get(QUERY_PROJECT)
        .map(String::as_str)
        .or_else(|| body.get(QUERY_PROJECT).and_then(Value::as_str))
        .filter(|name| !name.is_empty());
    let scope = match scope(app, requested) {
        Ok(scope) => scope,
        Err(reply) => return Some(reply),
    };
    if *head == RAG_SEGMENT {
        return Some(match rest {
            ["search"] => search(&scope, query),
            _ => reindex(&scope, body),
        });
    }
    memory_route(&scope, method, rest, query, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_body_prefers_the_scope_object() {
        let body: PatchBody = serde_json::from_value(json!({
            "title": "t", "paths": ["a/**"], "scope": { "paths": ["b/**"], "route": "code" }
        }))
        .unwrap();
        let patch: MemoryPatch = body.into();
        assert_eq!(patch.paths, Some(vec!["b/**".to_string()]));
        assert_eq!(patch.route, Some(Route::Code));
        assert_eq!(patch.title.as_deref(), Some("t"));
    }

    #[test]
    fn filter_rejects_an_unknown_status() {
        let query = HashMap::from([("status".to_string(), "bogus".to_string())]);
        assert_eq!(parse_filter(&query).unwrap_err().0, STATUS_BAD_REQUEST);
    }

    #[test]
    fn memory_errors_map_to_http_codes() {
        assert_eq!(memory_failure(MemoryError::NotFound("x".into())).0, 404);
        assert_eq!(memory_failure(MemoryError::Conflict("x".into())).0, 409);
        assert_eq!(memory_failure(MemoryError::Invalid("x".into())).0, 400);
    }
}
