//! The daemon side of the GitHub ticket source: `POST /api/tickets/sync?source=github`, the
//! periodic sync (`github.sync_minutes`) and the write-back of the `aidp:*` state labels that
//! follows every dispatcher tick (plan §3.2, G1 and G2).
//!
//! Every `gh` call runs on a background thread, never inside the dispatcher's tick lock.

use aidp_integrations::github_issues::GhIssuePort;
use aidp_tickets::github_sync::{self, SyncOptions};
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The only ticket source that exists.
pub const SOURCE_GITHUB: &str = "github";

const REFUSED_SOURCE: &str = "unknown ticket source (only `github`)";
const NO_REPOS: &str = "no GitHub repository configured: set `github.repos` in TODO/config.yaml";

/// One background pass at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// When the last periodic pull ended.
static LAST_PULL: Mutex<Option<Instant>> = Mutex::new(None);

/// The query of `POST /api/tickets/sync`.
#[derive(Debug, Clone, Default)]
pub struct SyncRequest {
    pub source: Option<String>,
    pub repo: Option<String>,
    pub limit: Option<u32>,
    pub dry_run: bool,
}

/// A refusal the HTTP layer turns into a 400.
#[derive(Debug)]
pub struct SyncRefused(pub String);

impl std::fmt::Display for SyncRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyncRefused {}

/// Runs a sync now, on the calling thread, over `port`, and returns the report as JSON.
pub fn run(
    store: &TicketStore,
    port: &dyn GithubPort,
    request: &SyncRequest,
) -> anyhow::Result<Value> {
    if request
        .source
        .as_deref()
        .is_some_and(|source| !source.eq_ignore_ascii_case(SOURCE_GITHUB))
    {
        return Err(SyncRefused(REFUSED_SOURCE.to_string()).into());
    }
    if request.repo.is_none() && store.config().github.repos.is_empty() {
        return Err(SyncRefused(NO_REPOS.to_string()).into());
    }
    let report = github_sync::sync(
        store,
        port,
        &SyncOptions {
            dry_run: request.dry_run,
            repo: request.repo.clone(),
            limit: request.limit,
        },
    )?;
    Ok(serde_json::to_value(report)?)
}

/// `POST /api/tickets/sync` over the real `gh`.
pub fn handle(store: &TicketStore, request: &SyncRequest) -> anyhow::Result<Value> {
    run(store, &GhIssuePort::new(), request)
}

/// A ticket as the API shows it: its frontmatter, plus `githubNumber` and `url` for a mirror
/// (`labels`, `milestone`, `route` and `flow` are frontmatter fields already).
pub fn ticket_view(ticket: &aidp_tickets::model::Ticket) -> Value {
    let mut value = serde_json::to_value(ticket).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        let meta = ticket.github.as_ref();
        object.insert(
            "githubNumber".to_string(),
            meta.map_or(Value::Null, |m| json!(m.number)),
        );
        object.insert(
            "url".to_string(),
            meta.map_or(Value::Null, |m| json!(m.url)),
        );
        object.insert("route".to_string(), json!(ticket.route));
        object.insert("milestone".to_string(), json!(ticket.milestone));
        object.insert("labels".to_string(), json!(ticket.labels));
    }
    value
}

/// Called by every dispatcher tick: starts, on a background thread, the periodic pull when it is
/// due and the label write-back. No repository configured: nothing happens.
pub fn after_tick(vault_root: &Path) {
    let Ok(store) = TicketStore::open(vault_root) else {
        return;
    };
    let cfg = store.config().github.clone();
    if cfg.repos.is_empty() {
        return;
    }
    let pull_due = cfg.sync_minutes > 0 && {
        let last = LAST_PULL.lock().unwrap_or_else(|e| e.into_inner());
        last.is_none_or(|at| at.elapsed() >= Duration::from_secs(u64::from(cfg.sync_minutes) * 60))
    };
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let vault = PathBuf::from(vault_root);
    std::thread::spawn(move || {
        let port = Arc::new(GhIssuePort::new());
        background_pass(&vault, port.as_ref(), pull_due);
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// The body of the background thread: the pull when due, then the write-back.
pub fn background_pass(vault_root: &Path, port: &dyn GithubPort, pull: bool) -> Value {
    let Ok(store) = TicketStore::open(vault_root) else {
        return json!({ "error": "cannot open the ticket store" });
    };
    if pull {
        let result = github_sync::sync(&store, port, &SyncOptions::default());
        *LAST_PULL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        return match result {
            Ok(report) => json!({ "sync": report }),
            Err(e) => json!({ "error": format!("{e:#}") }),
        };
    }
    let (pushed, errors) = github_sync::push_configured_labels(&store, port);
    json!({ "labelsPushed": pushed, "errors": errors })
}

#[cfg(test)]
#[path = "../tests/unit/ticket_sync_tests.rs"]
mod tests;
