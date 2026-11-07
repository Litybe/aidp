//! Progress reporting for a headless gateway session's `Workflow` tool call.
//!
//! The `Workflow` tool returns almost immediately (0.0-0.9s measured across
//! 20 real sessions) with a text result saying the pipeline was launched in
//! the background. The pipeline itself then runs for tens of minutes with
//! nothing on the session's stdout stream. This module turns the run's
//! on-disk journal (`journal.jsonl`, written into the transcript dir the
//! tool_result names) into progress the gateway can publish on the bus
//! while the session waits.
//!
//! Pure-function-first, like the rest of this crate: `parse_workflow_launch`
//! and `progress_from_line` are the unit-tested core, exercised over
//! `serde_json::json!` fixtures with no process, daemon or filesystem
//! needed. Only `drain_journal` (file IO) and `watch` (the thread) touch
//! the outside world.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

const RUN_ID_DATE_DIGITS: usize = 8;
const RUN_ID_TIME_DIGITS: usize = 6;
const TEXT_KEY: &str = "text";
const RUN_ID_KEY: &str = "run_id";
const RUN_DIR_KEY: &str = "run_dir";

/// Everything `parse_workflow_launch` extracts from the `Workflow` tool's
/// `tool_result` text: enough to find and tail that run's journal.
#[derive(Debug, Clone)]
pub struct WorkflowLaunch {
    pub run_id: String,
    pub transcript_dir: PathBuf,
}

/// Parse the `Workflow` tool's `tool_result` text into a `WorkflowLaunch`.
///
/// Requires the literal marker `Workflow launched in background` somewhere
/// in the content, then reads `transcript_dir` from the line starting with
/// `Transcript dir:` and `run_id` from the line starting with `Run ID:`
/// (both trimmed). When `Run ID:` is absent (or empty), `run_id` falls back
/// to the last path component of the transcript dir.
///
/// Returns `None` when the marker is missing or the transcript dir is
/// absent/empty — this is what keeps the gateway from watching anything for
/// the two real failure shapes: the plan-mode refusal `Review dynamic
/// workflow before running`, and `<tool_use_error>scriptPath must be a
/// script path this tool returned, ...</tool_use_error>`. Neither carries
/// the marker.
///
/// Paths in the result are Windows-style backslash paths; the `PathBuf` is
/// built from the string as-is, with no separator normalisation.
pub fn parse_workflow_launch(content: &str) -> Option<WorkflowLaunch> {
    if !content.contains("Workflow launched in background") {
        return None;
    }

    let mut transcript_dir: Option<String> = None;
    let mut run_id: Option<String> = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Transcript dir:") {
            transcript_dir = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Run ID:") {
            run_id = Some(rest.trim().to_string());
        }
    }

    let transcript_dir = transcript_dir.filter(|s| !s.is_empty())?;
    let run_id = match run_id {
        Some(id) if !id.is_empty() => id,
        _ => last_path_component(&transcript_dir),
    };

    Some(WorkflowLaunch {
        run_id,
        transcript_dir: PathBuf::from(transcript_dir),
    })
}

/// The `run_id` of the `aidp run start` JSON envelope inside a tool result, if any.
/// Looks at the whole text, then each line, then the text blocks of an array result.
pub fn parse_run_start_run_id(content: &str) -> Option<String> {
    let whole = serde_json::from_str::<Value>(content).ok();
    if let Some(Value::Array(blocks)) = &whole {
        return blocks
            .iter()
            .filter_map(|block| block[TEXT_KEY].as_str())
            .find_map(parse_run_start_run_id);
    }
    whole.as_ref().and_then(run_id_of_envelope).or_else(|| {
        content
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('{'))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|envelope| run_id_of_envelope(&envelope))
    })
}

/// The well-formed `run_id` of a JSON object that also carries `run_dir`, as `aidp run start` prints.
fn run_id_of_envelope(envelope: &Value) -> Option<String> {
    envelope.get(RUN_DIR_KEY)?;
    envelope
        .get(RUN_ID_KEY)
        .and_then(Value::as_str)
        .filter(|run_id| is_aidp_run_id(run_id))
        .map(str::to_string)
}

/// Whether `text` has the shape `YYYYMMDD-HHMMSS-<lowercase alphanumerics>`.
fn is_aidp_run_id(text: &str) -> bool {
    let mut parts = text.splitn(3, '-');
    let (Some(date), Some(time), Some(suffix)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    date.len() == RUN_ID_DATE_DIGITS
        && date.bytes().all(|b| b.is_ascii_digit())
        && time.len() == RUN_ID_TIME_DIGITS
        && time.bytes().all(|b| b.is_ascii_digit())
        && !suffix.is_empty()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// Last non-empty component of a Windows-or-Unix path string, used only as
/// the `run_id` fallback when the `Run ID:` line is absent.
fn last_path_component(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// `journal.jsonl` lives directly inside the run's transcript dir.
pub fn journal_path(transcript_dir: &Path) -> PathBuf {
    transcript_dir.join("journal.jsonl")
}

/// The `label`/`phase` last seen for one agent id, recorded on `started` so
/// a later `result`/`failed` line (which does not repeat them) can still
/// report which phase it belongs to.
#[derive(Default)]
struct AgentMeta {
    label: Option<String>,
    phase: Option<String>,
}

/// Tracks how far a journal has been read, plus the `label`/`phase` last
/// seen per agent id. Fields are private; construct with `Default`, advance
/// with `drain_journal`.
#[derive(Default)]
pub struct JournalCursor {
    offset: u64,
    agents: HashMap<String, AgentMeta>,
}

/// Parse one journal line into a bus payload fragment, or `None` when the
/// line carries no agent-scoped progress.
///
/// The journal's line vocabulary (measured over 280 real journal files,
/// 4822 lines) is exactly four `type` values:
/// - `launched` — names no agent and no phase; always `None`.
/// - `started` — records `label`/`phase` (when present) into the cursor
///   under `agentId`, then reports `status: "started"`.
/// - `result` — `status` is `result.status` when that is a string, else the
///   literal `"result"`. The `result` value itself is never copied into the
///   payload: real result lines are 800-4500 bytes of full reply envelope
///   and would flood the bus and the SSE stream.
/// - `failed` — reports `status: "failed"`; there is no message/error field
///   on this line, so none is invented.
///
/// For `result` and `failed`, `label`/`phase` come from the cursor (`null`
/// when the agent was never seen `started` — legitimate when the gateway
/// starts tailing a journal that already had lines, or on a resumed run).
///
/// Blank lines and lines that fail to parse as JSON return `None`; this
/// never panics and never propagates an error.
pub fn progress_from_line(cursor: &mut JournalCursor, line: &str) -> Option<Value> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let ty = v.get("type").and_then(Value::as_str)?;
    let agent_id = v.get("agentId").and_then(Value::as_str);

    match ty {
        "launched" => None,
        "started" => {
            let agent_id = agent_id?;
            let label = v.get("label").and_then(Value::as_str).map(str::to_string);
            let phase = v.get("phase").and_then(Value::as_str).map(str::to_string);
            cursor.agents.insert(
                agent_id.to_string(),
                AgentMeta {
                    label: label.clone(),
                    phase: phase.clone(),
                },
            );
            Some(json!({
                "agent": agent_id,
                "label": label,
                "phase": phase,
                "status": "started",
            }))
        }
        "result" => {
            let agent_id = agent_id?;
            let meta = cursor.agents.get(agent_id);
            let label = meta.and_then(|m| m.label.clone());
            let phase = meta.and_then(|m| m.phase.clone());
            let status = v
                .get("result")
                .and_then(Value::as_object)
                .and_then(|o| o.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("result")
                .to_string();
            Some(json!({
                "agent": agent_id,
                "label": label,
                "phase": phase,
                "status": status,
            }))
        }
        "failed" => {
            let agent_id = agent_id?;
            let meta = cursor.agents.get(agent_id);
            let label = meta.and_then(|m| m.label.clone());
            let phase = meta.and_then(|m| m.phase.clone());
            Some(json!({
                "agent": agent_id,
                "label": label,
                "phase": phase,
                "status": "failed",
            }))
        }
        _ => None,
    }
}

/// Read whatever is new in `journal` since `cursor.offset`, feed each
/// complete line to `progress_from_line`, and collect the payloads.
///
/// Only bytes that ended in a newline advance `cursor.offset`: a line
/// caught mid-write is re-read whole on the next poll instead of being
/// parsed truncated and lost. A missing or unreadable file yields an empty
/// `Vec` and leaves the cursor untouched — the journal does not exist yet
/// at the instant the tool_result arrives, so this is the normal
/// first-poll path, not an error.
pub fn drain_journal(cursor: &mut JournalCursor, journal: &Path) -> Vec<Value> {
    let Ok(mut file) = std::fs::File::open(journal) else {
        return Vec::new();
    };
    if file.seek(SeekFrom::Start(cursor.offset)).is_err() {
        return Vec::new();
    }

    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() || buf.is_empty() {
        return Vec::new();
    }

    let complete_len = match buf.iter().rposition(|&b| b == b'\n') {
        Some(idx) => idx + 1,
        None => 0,
    };
    if complete_len == 0 {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&buf[..complete_len]);
    let payloads: Vec<Value> = text
        .lines()
        .filter_map(|line| progress_from_line(cursor, line))
        .collect();

    cursor.offset += complete_len as u64;
    payloads
}

/// How often the watcher polls the journal.
///
/// Shorter than `runs::start_watcher`'s 5s poll: watching a `Workflow` run
/// is a liveness feature — the session's stdout stream goes silent for the
/// run's entire duration — and each poll here is an incremental byte-range
/// read of a small local file (the largest real journal is a few tens of
/// KB), not `runs.rs`'s full JSON parse of `run-state.json`.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Drain the journal and call `on_progress` once per payload, inserting
/// `runId` first.
fn drain_and_emit(
    cursor: &mut JournalCursor,
    journal: &Path,
    run_id: &str,
    on_progress: &impl Fn(Value),
) {
    for mut payload in drain_journal(cursor, journal) {
        if let Value::Object(map) = &mut payload {
            map.insert("runId".to_string(), json!(run_id));
        }
        on_progress(payload);
    }
}

/// Spawn a detached thread that tails `launch`'s journal and calls
/// `on_progress` once per payload, until `stop` is set. Modelled on the
/// fire-and-forget `session.changes` thread in `sessions.rs` (clones an
/// `Arc<Bus>` into `std::thread::spawn` so the session transport is never
/// slowed) and on the change-only emit discipline of `runs::start_watcher`.
///
/// Returns immediately; the loop runs on its own thread and never panics
/// (no `unwrap`, no `expect`, no index arithmetic that can overflow). A
/// final drain runs right after the stop flag is observed, so the last
/// lines written before the session ended are not dropped.
pub fn watch(
    launch: WorkflowLaunch,
    stop: Arc<AtomicBool>,
    on_progress: impl Fn(Value) + Send + 'static,
) {
    std::thread::spawn(move || {
        let journal = journal_path(&launch.transcript_dir);
        let mut cursor = JournalCursor::default();
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(POLL_INTERVAL);
            drain_and_emit(&mut cursor, &journal, &launch.run_id, &on_progress);
        }
        drain_and_emit(&mut cursor, &journal, &launch.run_id, &on_progress);
    });
}

#[cfg(test)]
#[path = "../tests/unit/workflow_progress_tests.rs"]
mod tests;
