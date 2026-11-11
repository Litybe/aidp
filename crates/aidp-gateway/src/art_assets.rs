//! Read-only gallery of the sprites of every art run, finished or not:
//! `GET /api/art/assets?limit&before&issue&status&q`.
//! Runs come from the native run index; images come from the run directory listing
//! (the files themselves are served by `GET /api/runs/:id/files/*path`).
//! The scan of a finished run never changes, so it is cached by run id.

use crate::models::native_run_view::NativeRunView;
use crate::native_runs::NativeRunManager;
use crate::run_files::{list_files, FileKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tiny_http::Method;

const ROUTE: &str = "/api/art/assets";
const DEFAULT_LIMIT: usize = 24;
const MAX_LIMIT: usize = 100;
const ART_PREFIX: &str = "art";
const PIXELART_TEMPLATE: &str = "pixelart";
const ROUTE_INPUT: &str = "route";
const ART_ROUTE: &str = "art";
const SCORE_KEY: &str = "score_out_of_10";
/// Files whose `score_out_of_10` is the review mark, latest review first.
const SCORE_FILES: [&str; 3] = [
    "recheck/findings.json",
    "verify/findings.json",
    "review/findings.json",
];
const MAX_SCORE_FILE_BYTES: u64 = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 8;
/// Stages holding the final picture, best first.
const COVER_STAGES: [&str; 4] = ["export", "fix", "animate", "generate"];

/// One picture of a run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArtImage {
    pub path: String,
    pub size: u64,
    pub kind: FileKind,
    pub stage: String,
}

/// One run of the gallery.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtRun {
    pub run_id: String,
    pub template_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Best picture to show on a card.
    pub cover: ArtImage,
    pub images: Vec<ArtImage>,
}

/// Query of the route.
#[derive(Debug, Default, Clone)]
pub struct ArtQuery {
    pub limit: usize,
    pub before: Option<String>,
    pub issue: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
}

impl ArtQuery {
    pub fn from_params(params: &HashMap<String, String>) -> Self {
        let text = |key: &str| {
            params
                .get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let limit = params
            .get("limit")
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .map_or(DEFAULT_LIMIT, |n| n.min(MAX_LIMIT));
        ArtQuery {
            limit,
            before: text("before"),
            issue: text("issue"),
            status: text("status"),
            q: text("q"),
        }
    }
}

/// What the scan of a run found, valid while its stamp is unchanged.
#[derive(Clone)]
struct Scan {
    stamp: String,
    images: Vec<ArtImage>,
    score: Option<f64>,
}

fn cache() -> &'static Mutex<HashMap<String, Scan>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Scan>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_art(run: &NativeRunView) -> bool {
    let id = run.template_id.as_str();
    id.starts_with(ART_PREFIX)
        || id == PIXELART_TEMPLATE
        || run.request.inputs.get(ROUTE_INPUT).and_then(Value::as_str) == Some(ART_ROUTE)
}

/// Ticket key the run is tracked under.
fn ticket_of(run: &NativeRunView) -> Option<String> {
    let input = |key: &str| {
        run.request
            .inputs
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    run.request
        .ticket
        .clone()
        .or_else(|| run.request.target.clone())
        .or_else(|| input("feature"))
        .or_else(|| input("jira_key"))
        .filter(|t| !t.trim().is_empty())
}

fn find_score(value: &Value, depth: usize) -> Option<f64> {
    if depth > MAX_JSON_DEPTH {
        return None;
    }
    match value {
        Value::Object(map) => map
            .get(SCORE_KEY)
            .and_then(Value::as_f64)
            .or_else(|| map.values().find_map(|v| find_score(v, depth + 1))),
        Value::Array(items) => items.iter().find_map(|v| find_score(v, depth + 1)),
        _ => None,
    }
}

fn read_score(run_dir: &std::path::Path) -> Option<f64> {
    SCORE_FILES.iter().find_map(|file| {
        let path = run_dir.join(file);
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_SCORE_FILE_BYTES {
            return None;
        }
        let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        find_score(&value, 0)
    })
}

fn scan_dir(run: &NativeRunView) -> Scan {
    let images = list_files(&run.run_dir)
        .into_iter()
        .filter(|f| matches!(f.kind, FileKind::Png | FileKind::Gif))
        .map(|f| ArtImage {
            path: f.path,
            size: f.size,
            kind: f.kind,
            stage: f.stage,
        })
        .collect();
    Scan {
        stamp: String::new(),
        images,
        score: read_score(&run.run_dir),
    }
}

fn scan(run: &NativeRunView) -> Scan {
    let stamp = format!(
        "{}|{}",
        run.status.as_str(),
        run.ended_at.as_deref().unwrap_or("")
    );
    let terminal = run.status.is_terminal();
    if terminal {
        if let Some(hit) = cache()
            .lock()
            .ok()
            .and_then(|c| c.get(&run.run_id).cloned())
        {
            if hit.stamp == stamp {
                return hit;
            }
        }
    }
    let mut fresh = scan_dir(run);
    fresh.stamp = stamp;
    if terminal {
        if let Ok(mut c) = cache().lock() {
            c.insert(run.run_id.clone(), fresh.clone());
        }
    }
    fresh
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Final picture of a run: an export or imported file first, then the latest stage, previews last.
fn pick_cover(images: &[ArtImage]) -> Option<&ArtImage> {
    let rank = |i: &ArtImage| {
        let preview = file_name(&i.path).to_ascii_lowercase().contains("preview");
        let stage = COVER_STAGES
            .iter()
            .position(|s| *s == i.stage)
            .unwrap_or(COVER_STAGES.len());
        let final_file = i.path.contains("/files/") || i.stage == "export";
        // Lower is better.
        (preview, !final_file, i.kind != FileKind::Png, stage, i.path.len())
    };
    images.iter().min_by_key(|i| rank(i))
}

fn matches_text(run: &ArtRun, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    run.ticket
        .iter()
        .chain([&run.run_id, &run.template_id])
        .any(|s| s.to_lowercase().contains(&needle))
        || run
            .images
            .iter()
            .any(|i| i.path.to_lowercase().contains(&needle))
}

fn matches_issue(run: &ArtRun, issue: &str) -> bool {
    let issue = issue.to_lowercase();
    run.ticket.as_deref().is_some_and(|t| {
        let t = t.to_lowercase();
        t == issue || t.ends_with(&format!("-{issue}")) || t.ends_with(&format!("#{issue}"))
    })
}

/// The runs (newest first) matching `query`, plus the cursor of the next page.
pub fn gallery(runs: &[NativeRunView], query: &ArtQuery) -> (Vec<ArtRun>, Option<String>) {
    let mut sorted: Vec<&NativeRunView> = runs.iter().filter(|r| is_art(r)).collect();
    sorted.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    let mut out: Vec<ArtRun> = Vec::new();
    let mut more = false;
    for run in sorted {
        if query
            .before
            .as_deref()
            .is_some_and(|b| run.started_at.as_str() >= b)
        {
            continue;
        }
        if query
            .status
            .as_deref()
            .is_some_and(|s| !run.status.as_str().eq_ignore_ascii_case(s))
        {
            continue;
        }
        let found = scan(run);
        let Some(cover) = pick_cover(&found.images).cloned() else {
            continue;
        };
        let item = ArtRun {
            run_id: run.run_id.clone(),
            template_id: run.template_id.clone(),
            status: run.status.as_str(),
            ticket: ticket_of(run),
            started_at: run.started_at.clone(),
            ended_at: run.ended_at.clone(),
            score: found.score,
            cover,
            images: found.images,
        };
        if query.issue.as_deref().is_some_and(|i| !matches_issue(&item, i)) {
            continue;
        }
        if query.q.as_deref().is_some_and(|q| !matches_text(&item, q)) {
            continue;
        }
        if out.len() == query.limit {
            more = true;
            break;
        }
        out.push(item);
    }
    let next = if more {
        out.last().map(|r| r.started_at.clone())
    } else {
        None
    };
    (out, next)
}

/// Answers `GET /api/art/assets`, or `None` for any other request.
pub fn handle(
    manager: &NativeRunManager,
    method: &Method,
    path: &str,
    params: &HashMap<String, String>,
) -> Option<(u16, Value)> {
    if *method != Method::Get || path.trim_end_matches('/') != ROUTE {
        return None;
    }
    let (runs, next) = gallery(&manager.list(), &ArtQuery::from_params(params));
    Some((200, serde_json::json!({ "runs": runs, "next": next })))
}

#[cfg(test)]
#[path = "../tests/unit/art_assets_tests.rs"]
mod tests;
