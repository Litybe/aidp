//! `memory context`: the Markdown block of active memories and excerpts for a stage.

use crate::error::Result;
use crate::manager::{load, parse_id, row_to_memory, save, MemoryManager, MEMORY_COLUMNS};
use crate::models::constants::limits::CHARS_PER_TOKEN;
use crate::models::constants::limits::{MAX_CONTEXT_MEMORIES, MEMORY_BUDGET_SHARE};
use crate::models::constants::schema::{GLOBAL_PROJECT, MEMORY_WEIGHTS};
use crate::models::context_block::ContextBlock;
use crate::models::context_request::ContextRequest;
use crate::models::enums::{MemoryStatus, Route, Source};
use crate::models::memory::Memory;
use crate::models::search_query::SearchQuery;
use crate::pack::{pack, render_markdown};
use crate::retriever::Retriever;
use crate::text::{estimate_tokens, match_expression, truncate_chars};
use regex::Regex;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::LazyLock;

const HEADING_MEMORIES: &str = "## Memoire du projet";
const HEADING_EXCERPTS: &str = "## Extraits pertinents";
const EXCERPT_K: usize = 12;
const FRONTMATTER_OPEN: &str = "---\n";
const FRONTMATTER_CLOSE: &str = "\n---\n";
const SUMMARY_KEY: &str = "summary:";
const ROUTE_KEY: &str = "route:";
const DEFAULT_SOURCES: [Source; 5] = [
    Source::Bible,
    Source::Doc,
    Source::Code,
    Source::Knowledge,
    Source::Run,
];

static PATH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:Assets|Docs|\.claude)/[\w./-]+").expect("path pattern"));

impl MemoryManager<'_> {
    /// Builds the context block and records the injected memories as used.
    pub fn context(
        &self,
        request: &ContextRequest,
        retriever: &dyn Retriever,
    ) -> Result<ContextBlock> {
        let query_text = [
            request.issue_text.as_deref(),
            request.stage.as_deref(),
            request.route.map(|route| route.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        let memory_budget = (request.budget_tokens as f64 * MEMORY_BUDGET_SHARE) as usize;
        let mut memories = self.relevant_memories(request, &query_text)?;
        let mut used = estimate_tokens(HEADING_MEMORIES);
        let mut kept: Vec<Memory> = Vec::new();
        for mut memory in memories.drain(..) {
            let mut cost = estimate_tokens(&memory_line(&memory));
            if used + cost > memory_budget {
                if !kept.is_empty() {
                    continue;
                }
                let room = memory_budget.saturating_sub(
                    used + estimate_tokens(&memory_line(&Memory {
                        text: String::new(),
                        ..memory.clone()
                    })),
                );
                memory.text = truncate_chars(&memory.text, room * CHARS_PER_TOKEN).to_string();
                cost = estimate_tokens(&memory_line(&memory));
            }
            used += cost;
            kept.push(memory);
        }
        if request.record_use {
            self.mark_used(&kept)?;
        }
        let mut markdown = String::new();
        if !kept.is_empty() {
            markdown.push_str(&format!("{HEADING_MEMORIES} {}\n\n", request.project));
            for memory in &kept {
                markdown.push_str(&memory_line(memory));
            }
        } else {
            used = 0;
        }
        let mut excerpts = Vec::new();
        let remaining = request
            .budget_tokens
            .saturating_sub(used + estimate_tokens(HEADING_EXCERPTS));
        if request
            .issue_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
            || request.stage.is_some()
        {
            let mut query = SearchQuery::new(request.project.clone(), query_text.clone());
            query.k = EXCERPT_K;
            query.domains = request.domains.clone();
            query.agent_type = request.agent_type.clone();
            query.sources = match request.route {
                Some(route) => route.sources().to_vec(),
                None => DEFAULT_SOURCES.to_vec(),
            };
            let packed = pack(retriever.search(&query)?, remaining);
            if !packed.hits.is_empty() {
                if !markdown.is_empty() {
                    markdown.push('\n');
                }
                markdown.push_str(HEADING_EXCERPTS);
                markdown.push_str("\n\n");
                markdown.push_str(&render_markdown(&packed));
                used += packed.used_tokens + estimate_tokens(HEADING_EXCERPTS);
                excerpts = packed.hits;
            }
        }
        Ok(ContextBlock {
            markdown,
            memories: kept,
            excerpts,
            used_tokens: used,
            budget_tokens: request.budget_tokens,
        })
    }

    fn relevant_memories(&self, request: &ContextRequest, query_text: &str) -> Result<Vec<Memory>> {
        let mut scores = self.relevance(&request.project, query_text)?;
        scores.extend(self.relevance(GLOBAL_PROJECT, query_text)?);
        let paths: Vec<String> = request
            .issue_text
            .as_deref()
            .map(extract_paths)
            .unwrap_or_default();
        let filter = crate::models::memory_filter::MemoryFilter {
            status: Some(MemoryStatus::Active),
            ..Default::default()
        };
        let mut active = self.list(&request.project, &filter)?;
        if request.project != GLOBAL_PROJECT {
            active.extend(self.list(GLOBAL_PROJECT, &filter)?);
        }
        let has_query = request
            .issue_text
            .as_deref()
            .is_some_and(|text| match_expression(text).is_some());
        let mut scored: Vec<(f64, Memory)> = active
            .into_iter()
            .filter_map(|memory| {
                let score = scores.get(&memory.id).copied().unwrap_or(0.0);
                let mut path_matched = false;
                if let (Some(wanted), Some(offered)) = (memory.scope.route, request.route) {
                    if wanted != offered && wanted != Route::Tooling {
                        return None;
                    }
                }
                if !memory.scope.paths.is_empty() {
                    let applies = if paths.is_empty() {
                        score > 0.0
                    } else {
                        paths.iter().any(|path| {
                            memory.scope.paths.iter().any(|pattern| {
                                glob::Pattern::new(pattern).is_ok_and(|glob| glob.matches(path))
                            })
                        })
                    };
                    if !applies {
                        return None;
                    }
                    path_matched = !paths.is_empty();
                }
                if has_query && score <= 0.0 && !path_matched {
                    return None;
                }
                Some((score, memory))
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then(right.1.confidence.total_cmp(&left.1.confidence))
                .then(right.1.evidence.cmp(&left.1.evidence))
        });
        Ok(scored
            .into_iter()
            .take(MAX_CONTEXT_MEMORIES)
            .map(|(_, memory)| memory)
            .collect())
    }

    fn relevance(&self, project: &str, query_text: &str) -> Result<HashMap<String, f64>> {
        let Some(expression) = match_expression(query_text) else {
            return Ok(HashMap::new());
        };
        self.store.read(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {MEMORY_COLUMNS}, bm25(memories_fts, {MEMORY_WEIGHTS}) FROM memories_fts \
                 JOIN memories m ON m.num = memories_fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.project = ?2 AND m.status = 'active'"
            ))?;
            let rows = statement.query_map(params![expression, project], |row| {
                Ok((row_to_memory(row)?.id, -row.get::<_, f64>(15)?))
            })?;
            Ok(rows.collect::<std::result::Result<HashMap<_, _>, _>>()?)
        })
    }

    pub(crate) fn mark_used(&self, memories: &[Memory]) -> Result<()> {
        if memories.is_empty() {
            return Ok(());
        }
        let now = self.now();
        self.store.write(|connection| {
            for memory in memories {
                let mut current = load(connection, &memory.project, parse_id(&memory.id)?)?;
                current.uses += 1;
                current.last_used_at = Some(now.clone());
                save(connection, &current)?;
            }
            Ok(())
        })
    }
}

fn memory_line(memory: &Memory) -> String {
    format!(
        "- **{}** ({}) {}: {}\n",
        memory.id, memory.kind, memory.title, memory.text
    )
}

/// The searchable text of a mirrored ticket: its summary first, then its body, without frontmatter.
pub fn issue_query_text(mirror: &str) -> String {
    let normalized = mirror.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix(FRONTMATTER_OPEN) else {
        return normalized;
    };
    let Some((front, body)) = rest.split_once(FRONTMATTER_CLOSE) else {
        return normalized;
    };
    let summary = front
        .lines()
        .find_map(|line| line.strip_prefix(SUMMARY_KEY))
        .map(|value| value.trim().trim_matches(['\'', '"']).to_string())
        .unwrap_or_default();
    format!("{summary}\n{}", body.trim())
}

/// The `route` of a mirrored ticket, read from its frontmatter.
pub fn ticket_route(mirror: &str) -> Option<Route> {
    let normalized = mirror.replace("\r\n", "\n");
    let (front, _) = normalized
        .strip_prefix(FRONTMATTER_OPEN)?
        .split_once(FRONTMATTER_CLOSE)?;
    front
        .lines()
        .find_map(|line| line.strip_prefix(ROUTE_KEY))
        .and_then(|value| Route::parse(value.trim().trim_matches(['\'', '"'])).ok())
}

/// The issue key when it is one path segment of `[A-Za-z0-9._-]` without `..`, else `None`.
pub fn safe_issue(issue: &str) -> Option<&str> {
    let valid = !issue.is_empty()
        && !issue.contains("..")
        && issue
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character));
    valid.then_some(issue)
}

/// The first `route:` line of a text, such as a ticket or a stage prompt; `None` when it names none.
pub fn route_in_text(text: &str) -> Option<Route> {
    text.lines()
        .filter_map(|line| line.trim_start().strip_prefix(ROUTE_KEY))
        .find_map(|value| Route::parse(value.trim().trim_matches(['\'', '"'])).ok())
}

/// Project paths (`Assets/...`, `Docs/...`) mentioned in a text.
pub fn extract_paths(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = PATH_PATTERN
        .find_iter(text)
        .map(|found| found.as_str().trim_end_matches(['.', ',']).to_string())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
#[path = "../tests/unit/context_tests.rs"]
mod tests;
