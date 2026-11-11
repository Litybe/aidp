//! Preconditions of the doctor's actions that read data only. Pure: the daemon checks them again
//! right before an action runs, and the tests need no daemon.

use crate::models::constants::doctor::{
    GIT_UNMERGED_BOTH_ADDED, GIT_UNMERGED_MARKERS, META_EXTENSION, PR_KEY_MERGEABLE, PR_MERGEABLE,
    UNITY_HEARTBEAT_STALE_SECONDS, UNITY_KEY_EDITOR, UNITY_KEY_HEARTBEAT, UNITY_KEY_HEARTBEAT_AT,
    UNITY_KEY_RUNNING, UNITY_KEY_STALE_AFTER, VERDICT_STEP_FRAGMENTS,
};
use crate::models::constants::flow_actions::{ACTION_GH_MERGE, ACTION_UNITY_REFRESH};
use aidp_flows::{FlowState, FlowStatus, Step, StepKind, StepReason, StepState, StepStatus};
use serde_json::Value;

const OUTPUT_DECISION: &str = "decision";
const OUTPUT_OK: &str = "ok";
const MILLIS_PER_SECOND: i64 = 1000;
const SECONDS_PER_MINUTE: i64 = 60;

/// The definition and the state of `step`, when it is the current step of the flow.
fn current<'a>(state: &'a FlowState, step: &str) -> Result<(Step, &'a StepState), String> {
    let definition = crate::flows::executor::step_at_cursor(state);
    let row = state.steps.get(step);
    match (state.current.as_deref() == Some(step), definition, row) {
        (true, Some(definition), Some(row)) => Ok((definition, row)),
        _ => Err(format!(
            "`{step}` is not the current step of `{}`",
            state.key
        )),
    }
}

fn refuse_human_and_merge(definition: &Step, step: &str) -> Result<(), String> {
    if definition.kind == StepKind::Human {
        return Err(format!(
            "`{step}` is a human step: the doctor never decides one"
        ));
    }
    if definition.use_.as_deref() == Some(ACTION_GH_MERGE) {
        return Err(format!(
            "`{step}` merges a pull request: the doctor never merges"
        ));
    }
    Ok(())
}

fn failed(row: &StepState) -> bool {
    matches!(row.status, StepStatus::Failed | StepStatus::Escalated)
}

fn rejected_by_a_human(row: &StepState) -> bool {
    row.reason.as_deref() == Some(StepReason::Rejected.as_str())
}

/// True when the step produces the verdict or the report of a stage.
pub fn is_verdict_step(definition: &Step) -> bool {
    let named = format!(
        "{} {}",
        definition.id,
        definition.use_.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    VERDICT_STEP_FRAGMENTS
        .iter()
        .any(|fragment| named.contains(fragment))
}

/// `mark_step_passed`: a failed report or verdict step, never a human or merge step, with a pull
/// request for the ticket and a clean recheck.
pub fn mark_passed(
    state: &FlowState,
    step: &str,
    has_pull_request: bool,
    recheck_clean: bool,
) -> Result<(), String> {
    let (definition, row) = current(state, step)?;
    refuse_human_and_merge(&definition, step)?;
    if !failed(row) || rejected_by_a_human(row) {
        return Err(format!("`{step}` did not fail on its own"));
    }
    if !is_verdict_step(&definition) {
        return Err(format!("`{step}` is not a report or verdict step"));
    }
    if !has_pull_request {
        return Err("the ticket has no pull request".to_string());
    }
    if !recheck_clean {
        return Err("the recheck of the evidence is not clean".to_string());
    }
    Ok(())
}

/// `skip_optional_step`: a failing step the template marks `optional`.
pub fn skip_optional(state: &FlowState, step: &str) -> Result<(), String> {
    let (definition, row) = current(state, step)?;
    refuse_human_and_merge(&definition, step)?;
    if !definition.optional {
        return Err(format!("`{step}` is not marked `optional` in the template"));
    }
    let waiting = row.status == StepStatus::WaitingExternal;
    if !(failed(row) || waiting) || rejected_by_a_human(row) {
        return Err(format!("`{step}` is neither failed nor waiting"));
    }
    Ok(())
}

/// True when the flow already holds a human approval: a human step answered `approve`, `ok` or `click`.
pub fn holds_approval(state: &FlowState) -> bool {
    state.steps.values().any(|row| {
        row.status == StepStatus::Done
            && row.outputs.contains_key(OUTPUT_DECISION)
            && row.outputs.get(OUTPUT_OK) == Some(&Value::Bool(true))
    })
}

/// True when the live PR status says the pull request merges.
pub fn pr_mergeable(status: Option<&Value>) -> bool {
    status
        .and_then(|status| status.get(PR_KEY_MERGEABLE))
        .and_then(Value::as_str)
        .is_some_and(|text| text.eq_ignore_ascii_case(PR_MERGEABLE))
}

/// `retry_merge_step`: the current `gh.merge` step, a flow that holds its approval, a PR that merges.
pub fn retry_merge(state: &FlowState, step: &str, mergeable: bool) -> Result<(), String> {
    let (definition, row) = current(state, step)?;
    if definition.use_.as_deref() != Some(ACTION_GH_MERGE) {
        return Err(format!("`{step}` is not a `{ACTION_GH_MERGE}` step"));
    }
    let waiting = row.status == StepStatus::WaitingExternal;
    if !(failed(row) || waiting) {
        return Err(format!("`{step}` is neither failed nor waiting"));
    }
    if !holds_approval(state) {
        return Err("the flow holds no human approval: the doctor never approves".to_string());
    }
    if !mergeable {
        return Err("the pull request is not mergeable".to_string());
    }
    Ok(())
}

/// `reconcile_step`: the current step is still `running`.
pub fn reconcile(state: &FlowState, step: &str) -> Result<(), String> {
    let (_, row) = current(state, step)?;
    if row.status == StepStatus::Running {
        Ok(())
    } else {
        Err(format!("`{step}` is not running"))
    }
}

/// `nudge_budget`: `Ok(false)` for a step that waits for the budget guard, `Ok(true)` for one held by
/// the ceiling of its ticket (a raise is needed).
pub fn nudge_budget(state: &FlowState, step: &str) -> Result<bool, String> {
    let (_, row) = current(state, step)?;
    let reason = row.reason.as_deref();
    match row.status {
        StepStatus::WaitingExternal if reason == Some(StepReason::Budget.as_str()) => Ok(false),
        StepStatus::WaitingHuman if reason == Some(StepReason::TicketBudget.as_str()) => Ok(true),
        _ => Err(format!("`{step}` does not wait for budget")),
    }
}

/// `extend_unity_wait`: the current step is a Unity compile gate that stopped or waits.
pub fn extend_wait(state: &FlowState, step: &str) -> Result<(), String> {
    let (definition, row) = current(state, step)?;
    if definition.use_.as_deref() != Some(ACTION_UNITY_REFRESH) {
        return Err(format!("`{step}` is not a `{ACTION_UNITY_REFRESH}` step"));
    }
    if failed(row) || row.status == StepStatus::WaitingExternal {
        Ok(())
    } else {
        Err(format!("`{step}` is neither failed nor waiting"))
    }
}

/// `retry_with_fallback`: a fallback model is set.
pub fn fallback_model(configured: Option<&str>) -> Result<String, String> {
    configured
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "no `doctor.fallback_model` is set".to_string())
}

/// Seconds since the editor heartbeat of a Unity status document, `None` when there is none.
pub fn heartbeat_silence(status: &Value, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let at = status
        .get(UNITY_KEY_EDITOR)?
        .get(UNITY_KEY_HEARTBEAT)?
        .get(UNITY_KEY_HEARTBEAT_AT)?
        .as_str()?;
    let at = chrono::DateTime::parse_from_rfc3339(at).ok()?;
    Some(now.signed_duration_since(at).num_seconds())
}

/// `dismiss_unity_modal`: the editor of the project runs and its heartbeat is silent past its limit.
pub fn editor_blocked(status: &Value, now: chrono::DateTime<chrono::Utc>) -> Result<(), String> {
    let editor = status.get(UNITY_KEY_EDITOR);
    let running = editor
        .and_then(|editor| editor.get(UNITY_KEY_RUNNING))
        .and_then(Value::as_bool)
        == Some(true);
    if !running {
        return Err(
            "no Unity editor is open on the project: the doctor never opens one".to_string(),
        );
    }
    let limit = editor
        .and_then(|editor| editor.get(UNITY_KEY_HEARTBEAT))
        .and_then(|heartbeat| heartbeat.get(UNITY_KEY_STALE_AFTER))
        .and_then(Value::as_i64)
        .unwrap_or(UNITY_HEARTBEAT_STALE_SECONDS);
    match heartbeat_silence(status, now) {
        Some(silence) if silence > limit => Ok(()),
        Some(_) => Err("the editor heartbeat is alive: no modal window blocks it".to_string()),
        None => Err("the editor gives no heartbeat: a blocked editor cannot be told".to_string()),
    }
}

/// `release_unity_lock`: the holder's flow is over, or live with a heartbeat silent for over
/// `stuck_minutes`.
pub fn lock_release(
    holder_over: bool,
    holder: &str,
    status: Option<&str>,
    silence_seconds: Option<i64>,
    stuck_minutes: u32,
) -> Result<(), String> {
    if holder_over {
        return Ok(());
    }
    let limit = i64::from(stuck_minutes) * SECONDS_PER_MINUTE;
    match silence_seconds {
        Some(silence) if silence > limit => Ok(()),
        _ => Err(format!(
            "the holder `{holder}` is {}, not terminal, and its heartbeat is not dead for {stuck_minutes} min",
            status.unwrap_or("live")
        )),
    }
}

/// The unmerged paths of a `git status --porcelain`: the Unity `.meta` files both sides added
/// (`Ok`), or, when any other path is unmerged, those other paths (`Err`).
pub fn meta_conflicts(porcelain: &str) -> Result<Vec<String>, Vec<String>> {
    let mut resolvable = Vec::new();
    let mut blocking = Vec::new();
    for line in porcelain.lines() {
        let Some((code, path)) = line.split_at_checked(2) else {
            continue;
        };
        if !GIT_UNMERGED_MARKERS.contains(&code) {
            continue;
        }
        let path = path.trim().to_string();
        if code == GIT_UNMERGED_BOTH_ADDED && path.ends_with(META_EXTENSION) {
            resolvable.push(path);
        } else {
            blocking.push(path);
        }
    }
    if blocking.is_empty() {
        Ok(resolvable)
    } else {
        Err(blocking)
    }
}

/// True when a flow's worktree may be removed: the flow is over for longer than `hours`, nothing runs on it.
pub fn worktree_prunable(
    status: FlowStatus,
    ended_ms: Option<i64>,
    now_ms: i64,
    hours: u32,
    active: bool,
) -> bool {
    let age_ms = ended_ms.map_or(0, |ended| now_ms - ended);
    status.is_terminal()
        && !active
        && ended_ms.is_some()
        && age_ms > i64::from(hours) * SECONDS_PER_MINUTE * SECONDS_PER_MINUTE * MILLIS_PER_SECOND
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_preconditions_tests.rs"]
mod tests;
