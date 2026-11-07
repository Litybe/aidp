//! Best-effort link between a gateway session and the vault run it drives.
//! Matches on the run id captured from the session stream; unreadable run states are skipped.

use crate::models::enums::SessionStatus;
use crate::models::{AidpCommand, CurrentGate};
use aidp_core::models::run_state::{RunState, StageStatus};
use aidp_core::run::{read_resume_anchor, ResumeAnchor, WORKFLOW_RESULT_FILE};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const RUN_STATE_FILE: &str = "run-state.json";
const PIPELINES_DIR: &str = "Pipelines";
const GENERATED_PREFIX: char = '_';
const MATCH_BY_RUN_ID: &str = "run_id";
const SESSION_RUN_EVENT: &str = "session.run";
const LAST_WORKFLOW_FAILED: &str = "failed";
const TRUNCATION_MARKER: &str = "\n\n[... tronqué]";
const FENCE: &str = "````";
const CHAIN_HEADING: &str = "## Contexte du run précédent\n";
const CHAIN_SUMMARIES_HEADING: &str = "\n### Résumés par étape\n";

/// How a session was linked to its run, serialised as the `match.by` wire value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchBy {
    RunId,
}

impl MatchBy {
    fn as_str(&self) -> &'static str {
        match self {
            MatchBy::RunId => MATCH_BY_RUN_ID,
        }
    }
}

/// Normalised status of a stage or a whole run, serialised as its wire value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Pending,
    Running,
    Done,
    Cancelled,
    Skipped,
    Paused,
    Failed,
}

impl RunStatus {
    /// The wire value this status serialises to.
    pub const fn as_str(self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Done => "done",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Skipped => "skipped",
            RunStatus::Paused => "paused",
            RunStatus::Failed => "failed",
        }
    }

    /// Whether a stage in this status needs no further work.
    const fn is_terminal(self) -> bool {
        matches!(
            self,
            RunStatus::Done | RunStatus::Skipped | RunStatus::Cancelled
        )
    }
}

/// A resolved vault run.
pub struct RunRef {
    pub run_id: String,
    pub repo: String,
    pub feature: String,
    pub run_dir: PathBuf,
    pub workspace_root: String,
    pub started_at: String,
    pub workflow_run_id: Option<String>,
}

/// Compare two paths loosely: separators, trailing slash and case are ignored.
fn same_path(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        let mut s = s.replace('\\', "/");
        if s.len() > 1 && s.ends_with('/') {
            s.pop();
        }
        s.to_lowercase()
    }
    norm(a) == norm(b)
}

struct Candidate {
    run_ref: RunRef,
    started_at_parsed: Option<chrono::DateTime<chrono::FixedOffset>>,
}

/// Largest `run-state.json` prefix read; a longer file fails to parse.
const RUN_STATE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// `run-state.json` under `run_dir` as a typed `RunState`, or `None` when the
/// file is missing, oversized, unparseable or lacks a required field.
pub(crate) fn read_run_state(run_dir: &Path) -> Option<RunState> {
    let text = read_bounded(&run_dir.join(RUN_STATE_FILE), RUN_STATE_MAX_BYTES)?;
    serde_json::from_str::<RunState>(&text).ok()
}

/// Cap on the most recent runs read per feature directory (ids are timestamp-prefixed).
const MAX_RUNS_PER_FEATURE_DIR: usize = 50;

/// Read `path` as UTF-8 text, at most `max_bytes` of it.
/// Invalid UTF-8 from a mid-character cut is lossily repaired.
fn read_bounded(path: &Path, max_bytes: usize) -> Option<String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).ok()?;
    if (meta.len() as usize) <= max_bytes {
        return std::fs::read_to_string(path).ok();
    }
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(max_bytes);
    file.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Collect run candidates from the vault, scoped to `repo_hint` when given.
/// Scoping avoids an unbounded vault walk on every per-request poll.
fn collect_candidates_in(vault: &Path, repo_hint: Option<&str>) -> Vec<Candidate> {
    let mut out = Vec::new();
    let pipelines_dir = vault.join(PIPELINES_DIR);
    let repo_dirs: Vec<PathBuf> = if let Some(repo) = repo_hint {
        let dir = pipelines_dir.join(repo);
        if dir.is_dir() {
            vec![dir]
        } else {
            Vec::new()
        }
    } else {
        let Ok(repos) = std::fs::read_dir(&pipelines_dir) else {
            return out;
        };
        repos
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    };
    for repo_dir in repo_dirs {
        let Ok(features) = std::fs::read_dir(&repo_dir) else {
            continue;
        };
        for feature_entry in features.flatten() {
            let feature_dir = feature_entry.path();
            let feature_name = feature_entry.file_name().to_string_lossy().to_string();
            if !feature_dir.is_dir() || feature_name.starts_with(GENERATED_PREFIX) {
                continue;
            }
            let Ok(runs) = std::fs::read_dir(&feature_dir) else {
                continue;
            };
            let mut run_entries: Vec<_> = runs
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    e.path().is_dir() && !name.starts_with(GENERATED_PREFIX)
                })
                .collect();
            run_entries.sort_by_key(|b| std::cmp::Reverse(b.file_name()));
            run_entries.truncate(MAX_RUNS_PER_FEATURE_DIR);
            for run_entry in run_entries {
                let run_dir = run_entry.path();
                let Some(state) = read_run_state(&run_dir) else {
                    continue;
                };
                let (Some(repo), Some(feature)) = (state.repo, state.feature) else {
                    continue;
                };
                let started_at_parsed =
                    chrono::DateTime::parse_from_rfc3339(&state.started_at).ok();
                out.push(Candidate {
                    run_ref: RunRef {
                        run_id: state.run_id,
                        repo,
                        feature,
                        run_dir: run_dir.clone(),
                        workspace_root: state.workspace_root.to_string_lossy().into_owned(),
                        started_at: state.started_at,
                        workflow_run_id: state.workflow_run_id,
                    },
                    started_at_parsed,
                });
            }
        }
    }
    out
}

/// The `<vault>/Pipelines/<repo>` directory name: the basename of a session's `cwd`.
fn repo_dir_name_for(cwd: &str) -> Option<String> {
    let norm = cwd.replace('\\', "/");
    let trimmed = norm.trim_end_matches('/');
    Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

/// Run of the session's workspace matched by exact run id.
fn resolve_candidate(cwd: &str, wanted: Option<&str>) -> Option<Candidate> {
    let wanted = wanted?;
    let vault = aidp_core::vault::resolve_global()?.root;
    let repo_hint = repo_dir_name_for(cwd);
    collect_candidates_in(&vault, repo_hint.as_deref())
        .into_iter()
        .filter(|c| same_path(&c.run_ref.workspace_root, cwd))
        .find(|c| {
            c.run_ref.run_id == wanted || c.run_ref.workflow_run_id.as_deref() == Some(wanted)
        })
}

/// Resolve the vault run a session is driving.
/// Pass the record's captured run id; without one the session is not linked to a run.
pub fn resolve_run(cwd: &str, workflow_run_id: Option<&str>) -> Option<RunRef> {
    resolve_candidate(cwd, workflow_run_id).map(|c| c.run_ref)
}

/// Map a stage status to its normalised value (`in_progress` becomes running, `success` done).
pub fn stage_status(status: StageStatus) -> RunStatus {
    match status {
        StageStatus::Pending => RunStatus::Pending,
        StageStatus::InProgress => RunStatus::Running,
        StageStatus::Success => RunStatus::Done,
        StageStatus::Cancel => RunStatus::Cancelled,
        StageStatus::Skipped => RunStatus::Skipped,
    }
}

/// The gate a run is stopped at when only `workflow-result.json` reports it.
/// Any read or parse error yields `None`.
fn unpersisted_anchor_gate(run_dir: &Path, run_state: &RunState) -> Option<ResumeAnchor> {
    let has_workflow_run_id = run_state
        .workflow_run_id
        .as_deref()
        .is_some_and(|id| !id.is_empty());
    if has_workflow_run_id {
        return None;
    }
    read_resume_anchor(run_dir)
        .ok()
        .flatten()
        .filter(|anchor| anchor.pending_gate.is_some())
}

/// `currentGate` payload for a gate whose resume anchor was never persisted.
fn unpersisted_gate_payload(run_dir: &Path, run_state: &RunState, anchor: ResumeAnchor) -> Value {
    let result_path = run_dir.join(WORKFLOW_RESULT_FILE);
    let resume_command =
        AidpCommand::record_resume_from_result(&run_state.run_id, &result_path).to_string();
    CurrentGate::unpersisted(anchor.pending_gate, anchor.workflow_run_id, resume_command).to_value()
}

/// Re-read `run-state.json` under `run_dir` as the camelCase payload the HTTP layer serves.
pub fn normalize(run_dir: &Path) -> Option<Value> {
    let state = read_run_state(run_dir)?;

    let mut stages = Vec::with_capacity(state.stages.len());
    let mut done_count = 0usize;
    let mut any_running = false;
    let mut normalized_all_terminal = true;
    let mut any_done = false;
    for stage in &state.stages {
        let status = stage_status(stage.status);
        if status == RunStatus::Done {
            done_count += 1;
            any_done = true;
        }
        if status == RunStatus::Running {
            any_running = true;
        }
        if !status.is_terminal() {
            normalized_all_terminal = false;
        }
        stages.push(json!({
            "name": stage.name,
            "status": status.as_str(),
            "startedAt": stage.started_at,
            "endedAt": stage.finished_at,
            "durationMs": stage.duration_ms,
            "tokensUsed": stage.tokens_used,
            "stageDir": stage.stage_dir.to_string_lossy(),
        }));
    }

    let pending_gate = state.pending_gate.as_deref().filter(|s| !s.is_empty());
    let last_workflow_status = state.last_workflow_status.as_deref();

    let unpersisted_anchor = if pending_gate.is_none() {
        unpersisted_anchor_gate(run_dir, &state)
    } else {
        None
    };

    let status = if pending_gate.is_some() || unpersisted_anchor.is_some() {
        RunStatus::Paused
    } else if last_workflow_status == Some(LAST_WORKFLOW_FAILED) {
        RunStatus::Failed
    } else if any_running {
        RunStatus::Running
    } else if normalized_all_terminal && any_done {
        RunStatus::Done
    } else {
        RunStatus::Pending
    };

    let current_gate = match (pending_gate, unpersisted_anchor) {
        (Some(stage), _) => Some(CurrentGate::persisted(stage).to_value()),
        (None, Some(anchor)) => Some(unpersisted_gate_payload(run_dir, &state, anchor)),
        (None, None) => None,
    };

    Some(json!({
        "runId": state.run_id,
        "pipelineId": state.pipeline_id,
        "repo": state.repo,
        "feature": state.feature,
        "runDir": run_dir.to_string_lossy(),
        "workspaceRoot": state.workspace_root.to_string_lossy(),
        "startedAt": state.started_at,
        "workflowRunId": state.workflow_run_id,
        "status": status.as_str(),
        "currentGate": current_gate,
        "stageCount": stages.len(),
        "doneCount": done_count,
        "stages": stages,
    }))
}

/// Resolve then normalise a session's run; the function the HTTP route calls.
/// `workflow_run_id` is the run id captured on the session record.
pub fn for_session_with_run_id(
    rec_id: &str,
    cwd: &str,
    started_at: &str,
    workflow_run_id: Option<&str>,
) -> Option<Value> {
    let candidate = resolve_candidate(cwd, workflow_run_id)?;
    let delta_seconds = candidate
        .started_at_parsed
        .zip(chrono::DateTime::parse_from_rfc3339(started_at).ok())
        .map(|(run_ts, session_ts)| (run_ts - session_ts).num_seconds());

    let mut normalized = normalize(&candidate.run_ref.run_dir)?;
    if let Value::Object(ref mut m) = normalized {
        m.insert("sessionId".to_string(), json!(rec_id));
        m.insert(
            "match".to_string(),
            json!({ "by": MatchBy::RunId.as_str(), "deltaSeconds": delta_seconds }),
        );
    }
    Some(normalized)
}

/// Context gathered from a previous run's vault directory for a chained session's prompt.
#[derive(Debug, Clone, Default)]
pub struct ChainArtefacts {
    pub run_id: String,
    pub feature: String,
    pub pipeline_id: String,
    /// (stage name, summary text), in run-state order, skipping empty summaries.
    pub stage_summaries: Vec<(String, String)>,
    /// (relative label like `plan/plan.md`, file body), in allowlist order.
    pub documents: Vec<(String, String)>,
}

/// Allowlisted basenames read from each stage directory, in priority order.
/// `plan.json` is excluded: `plan.md` is its readable twin.
const CHAIN_DOC_BASENAMES: [&str; 4] = ["study.md", "plan.md", "context.md", "exploration.md"];

/// Read a run directory into a `ChainArtefacts` bundle, in `run-state.json` order.
/// Returns `None` when the run state is unreadable; unreadable files are skipped.
pub fn read_chain_artefacts(run_dir: &Path) -> Option<ChainArtefacts> {
    let state = read_run_state(run_dir)?;

    let mut stage_summaries = Vec::new();
    let mut documents = Vec::new();

    for stage in &state.stages {
        let name = stage.name.clone();

        if let Some(summary) = stage.summary.as_deref() {
            let trimmed = summary.trim();
            if !trimmed.is_empty() {
                stage_summaries.push((name.clone(), trimmed.to_string()));
            }
        }

        if stage.stage_dir.as_os_str().is_empty() {
            continue;
        }
        let stage_dir = if stage.stage_dir.is_absolute() {
            stage.stage_dir.clone()
        } else {
            run_dir.join(&stage.stage_dir)
        };
        for basename in CHAIN_DOC_BASENAMES {
            let path = stage_dir.join(basename);
            if let Some(body) = read_bounded(&path, CHAIN_DOC_MAX_CHARS * 4) {
                documents.push((format!("{name}/{basename}"), body));
            }
        }
    }

    Some(ChainArtefacts {
        run_id: state.run_id,
        feature: state.feature.unwrap_or_default(),
        pipeline_id: state.pipeline_id,
        stage_summaries,
        documents,
    })
}

/// Hard budget for one injected document body, in characters.
pub const CHAIN_DOC_MAX_CHARS: usize = 8_000;
/// Hard budget for the whole assembled context block, in characters.
pub const CHAIN_BLOCK_MAX_CHARS: usize = 24_000;

/// Truncate `s` to at most `max` characters (never bytes, to respect char boundaries).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(TRUNCATION_MARKER);
    out
}

/// Render `a` into the markdown block injected into a chained prompt.
/// Returns an empty string when there is nothing to inject.
pub fn build_context_block(a: &ChainArtefacts) -> String {
    if a.stage_summaries.is_empty() && a.documents.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(CHAIN_HEADING);
    out.push_str(&format!(
        "Run `{}` — pipeline `{}`, feature `{}`.\n",
        a.run_id, a.pipeline_id, a.feature
    ));

    if !a.stage_summaries.is_empty() {
        out.push_str(CHAIN_SUMMARIES_HEADING);
        for (stage, summary) in &a.stage_summaries {
            out.push_str(&format!("- **{stage}** — {summary}\n"));
        }
    }

    for (label, body) in &a.documents {
        out.push_str(&format!("\n### {label}\n"));
        let truncated = truncate_chars(body, CHAIN_DOC_MAX_CHARS);
        out.push_str(FENCE);
        out.push('\n');
        out.push_str(&truncated);
        out.push('\n');
        out.push_str(FENCE);
        out.push('\n');
    }

    truncate_chars(&out, CHAIN_BLOCK_MAX_CHARS)
}

/// The run's non-empty `pending_gate`, or `None` (also when the state is unreadable).
pub fn pending_gate_stage(run_dir: &Path) -> Option<String> {
    read_run_state(run_dir)?
        .pending_gate
        .filter(|stage| !stage.is_empty())
}

/// `true` when the run under `run_dir` has a non-empty `pending_gate`.
/// `false` when the state cannot be read.
pub fn run_is_gate_blocked(run_dir: &Path) -> bool {
    pending_gate_stage(run_dir).is_some()
}

/// Snapshot of one running session's run, used by `start_watcher` to detect changes.
struct WatchState {
    run_dir: PathBuf,
    mtime: Option<SystemTime>,
    stages: Vec<(String, String)>,
    current_gate: Value,
}

/// Delay between two watcher passes.
const WATCHER_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Wait before rescanning the vault after a session's first miss.
const NEGATIVE_CACHE_INITIAL_TTL: Duration = Duration::from_secs(10);

/// Longest wait between two vault scans for a session that has no run.
const NEGATIVE_CACHE_MAX_TTL: Duration = Duration::from_secs(300);

/// A session whose vault scan found no run: when to rescan, and the miss count.
struct NegativeEntry {
    next_check: Instant,
    misses: u32,
}

/// Wait after the `misses`-th empty scan: the initial TTL doubled per miss, capped.
fn negative_cache_ttl(misses: u32) -> Duration {
    let doublings = misses.saturating_sub(1);
    let factor = 1u32.checked_shl(doublings).unwrap_or(u32::MAX);
    NEGATIVE_CACHE_INITIAL_TTL
        .saturating_mul(factor)
        .min(NEGATIVE_CACHE_MAX_TTL)
}

/// Sleep up to `total`, waking every ~200ms to check the shutdown flag.
/// Returns false when interrupted, true when the full duration elapsed.
fn sleep_unless_stopped(shutdown: &crate::shutdown::Shutdown, total: std::time::Duration) -> bool {
    let mut elapsed = std::time::Duration::ZERO;
    let slice = std::time::Duration::from_millis(200);
    while elapsed < total {
        if shutdown.is_set() {
            return false;
        }
        let remaining = total - elapsed;
        let to_sleep = std::cmp::min(remaining, slice);
        std::thread::sleep(to_sleep);
        elapsed += to_sleep;
    }
    true
}

/// Spawn a detached thread polling each running session's vault run.
/// It emits `session.run` on the bus when a run changes and never panics.
pub fn start_watcher(
    sessions: Arc<crate::sessions::SessionManager>,
    bus: Arc<crate::bus::Bus>,
    shutdown: Arc<crate::shutdown::Shutdown>,
) {
    std::thread::spawn(move || {
        let _loop_guard = crate::threads::enter_loop(crate::threads::RUNS_WATCHER);
        let mut cache: HashMap<String, WatchState> = HashMap::new();
        let mut misses: HashMap<String, NegativeEntry> = HashMap::new();
        loop {
            if shutdown.is_set() {
                break;
            }
            if !sleep_unless_stopped(&shutdown, WATCHER_POLL_INTERVAL) {
                break;
            }
            crate::threads::guarded(crate::threads::RUNS_WATCHER, || {
                watch_running_sessions(&sessions, &bus, &mut cache, &mut misses);
            });
        }
    });
}

/// One watcher pass: forget finished sessions, emit `session.run` for changed runs.
fn watch_running_sessions(
    sessions: &crate::sessions::SessionManager,
    bus: &crate::bus::Bus,
    cache: &mut HashMap<String, WatchState>,
    misses: &mut HashMap<String, NegativeEntry>,
) {
    let running = sessions.list(Some(SessionStatus::Running), None, 0);
    let running_ids: std::collections::HashSet<String> =
        running.iter().map(|record| record.id.clone()).collect();
    cache.retain(|id, _| running_ids.contains(id));
    misses.retain(|id, _| running_ids.contains(id));

    for rec in &running {
        let Some(run_dir) = locate_run_dir(rec, cache, misses, Instant::now()) else {
            continue;
        };

        let mtime = std::fs::metadata(run_dir.join(RUN_STATE_FILE))
            .ok()
            .and_then(|m| m.modified().ok());

        let prev = cache.get(&rec.id);
        let unchanged_mtime =
            matches!((prev.and_then(|p| p.mtime), mtime), (Some(a), Some(b)) if a == b);
        if prev.is_some() && unchanged_mtime {
            continue;
        }

        let Some(normalized) = normalize(&run_dir) else {
            continue;
        };

        let stages: Vec<(String, String)> = normalized["stages"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|s| {
                        (
                            s["name"].as_str().unwrap_or("").to_string(),
                            s["status"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let current_gate = normalized["currentGate"].clone();

        let changed = match prev {
            None => true,
            Some(p) => p.stages != stages || p.current_gate != current_gate,
        };

        if changed {
            let running_stage = stages
                .iter()
                .find(|(_, st)| st == RunStatus::Running.as_str());
            let stage_name = running_stage.map(|(n, _)| n.clone());
            let stage_status = running_stage.map(|(_, st)| st.clone());
            bus.emit(
                SESSION_RUN_EVENT,
                json!({
                    "sessionId": rec.id,
                    "runId": normalized["runId"],
                    "status": normalized["status"],
                    "stage": stage_name,
                    "stageStatus": stage_status,
                    "doneCount": normalized["doneCount"],
                    "stageCount": normalized["stageCount"],
                    "currentGate": current_gate,
                }),
            );
        }

        cache.insert(
            rec.id.clone(),
            WatchState {
                run_dir,
                mtime,
                stages,
                current_gate,
            },
        );
    }
}

/// Run dir of `record`: the cached one, else a vault scan outside its miss window.
/// A miss escalates the window; a hit clears it.
fn locate_run_dir(
    record: &crate::sessions::SessionRecord,
    cache: &HashMap<String, WatchState>,
    misses: &mut HashMap<String, NegativeEntry>,
    now: Instant,
) -> Option<PathBuf> {
    if let Some(state) = cache.get(&record.id) {
        return Some(state.run_dir.clone());
    }
    if misses
        .get(&record.id)
        .is_some_and(|entry| now < entry.next_check)
    {
        return None;
    }
    match resolve_run(&record.cwd, record.workflow_run_id.as_deref()) {
        Some(resolved) => {
            misses.remove(&record.id);
            Some(resolved.run_dir)
        }
        None => {
            let entry = misses.entry(record.id.clone()).or_insert(NegativeEntry {
                next_check: now,
                misses: 0,
            });
            entry.misses = entry.misses.saturating_add(1);
            entry.next_check = now + negative_cache_ttl(entry.misses);
            None
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/runs_tests.rs"]
mod tests;
