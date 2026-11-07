//! Human-readable progress and summary renderings.

use super::fold::plan_task_accounting_is_inconsistent;
use super::{RunState, StageStatus};
use crate::models::constants::cost::UNKNOWN_LABEL;
use crate::plan::TaskStatus;

fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    if ms < 60_000 {
        return format!("{:.1}s", ms as f64 / 1_000.0);
    }
    let secs = ms / 1_000;
    format!("{}m {:02}s", secs / 60, secs % 60)
}

fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        return format!("{:.1}k", n as f64 / 1_000.0);
    }
    format!("{:.1}M", n as f64 / 1_000_000.0)
}

pub fn render_progress(state: &RunState) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# Run {} — {}\n\n",
        state.run_id, state.pipeline_id
    ));
    if let Some(ws) = &state.last_workflow_status {
        let done = state
            .stages
            .iter()
            .filter(|s| s.status == StageStatus::Success)
            .count();
        s.push_str(&format!(
            "- last workflow status: {} ({}/{} stages)\n\n",
            ws,
            done,
            state.stages.len()
        ));
    }
    s.push_str(
        "| Stage | Status | Started | Finished | Duration | Tokens |\n|---|---|---|---|---|---|\n",
    );
    for st in &state.stages {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            st.name,
            st.status,
            st.started_at.as_deref().unwrap_or("-"),
            st.finished_at.as_deref().unwrap_or("-"),
            st.duration_ms
                .map(format_duration)
                .as_deref()
                .unwrap_or("-"),
            st.tokens_used.map(format_tokens).as_deref().unwrap_or("-")
        ));
    }
    if !state.plan.is_empty() {
        s.push_str("\n## Plan (layers)\n");
        for (i, layer) in state.plan.iter().enumerate() {
            s.push_str(&format!("- L{}: {}\n", i, layer.join(", ")));
        }
    }
    for sched in state.plan_schedules.values() {
        s.push_str(&format!(
            "\n## Plan tasks — stage `{}` ({} wave(s), up to {} concurrent)\n\n",
            sched.stage,
            sched.waves.len(),
            sched.max_parallel.min(sched.width.max(1) as u32)
        ));
        s.push_str("| Task | Kind | Wave | After | Status | Tokens |\n|---|---|---|---|---|---|\n");
        for t in &sched.tasks {
            s.push_str(&format!(
                "| {} — {} | {} | {} | {} | {} | {} |\n",
                t.id,
                t.title,
                t.kind,
                t.wave + 1,
                if t.deps.is_empty() {
                    "-".to_string()
                } else {
                    t.deps.join(", ")
                },
                t.status,
                t.tokens_used.map(format_tokens).as_deref().unwrap_or("-")
            ));
        }
        for ser in &sched.serializations {
            s.push_str(&format!(
                "\n- ordering inserted by aidp: `{}` after `{}` (both write {})\n",
                ser.after,
                ser.before,
                ser.files.join(", ")
            ));
        }
    }
    s
}

pub fn render_summary(state: &RunState) -> String {
    let total = state.stages.len();
    let done = state
        .stages
        .iter()
        .filter(|s| s.status == StageStatus::Success)
        .count();
    let failed = state
        .stages
        .iter()
        .filter(|s| s.status == StageStatus::Cancel)
        .count();
    let skipped = state
        .stages
        .iter()
        .filter(|s| s.status == StageStatus::Skipped)
        .count();
    let total_duration: u64 = state.stages.iter().filter_map(|s| s.duration_ms).sum();
    let total_tokens: u64 = state.stages.iter().filter_map(|s| s.tokens_used).sum();
    let mut s = format!(
        "# Summary {}\n\n- pipeline: {}\n- stages success: {}/{}\n- cancel: {}\n- skipped: {}\n- total duration: {}\n- total tokens: {}\n- run_dir: {}\n",
        state.run_id, state.pipeline_id, done, total, failed, skipped,
        format_duration(total_duration), format_tokens(total_tokens),
        state.run_dir.display()
    );
    if let Some(cost) = &state.cost {
        match cost.total_usd {
            Some(usd) => s.push_str(&format!(
                "- cost: {usd:.4} $ (pricing {})\n",
                cost.pricing_version.as_deref().unwrap_or(UNKNOWN_LABEL)
            )),
            None => s.push_str("- cost: — (unavailable)\n"),
        }
    }
    if let Some(ws) = &state.last_workflow_status {
        s.push_str(&format!("- last workflow status: {}\n", ws));
    }
    if let Some(pg) = &state.pending_gate {
        s.push_str(&format!(
            "- awaiting gate: {} (resume re-proposes this gate for approval)\n",
            pg
        ));
    }
    if !state.scope_violations.is_empty() {
        s.push_str(&format!(
            "- scope: {} violation(s) — see `aidp scope verify --run {}`\n",
            state.scope_violations.len(),
            state.run_id
        ));
    }
    if !state.vault_leaks.is_empty() {
        s.push_str(&format!(
            "- ⛔ vault leak: {} run/doc artefact(s) written into the repo tree instead of the vault — see `aidp scope verify --run {}`\n",
            state.vault_leaks.len(),
            state.run_id
        ));
        s.push_str(&format!(
            "  {}\n",
            crate::vault_leak::describe(&state.vault_leaks)
        ));
    }
    for stage in state
        .stages
        .iter()
        .filter(|st| !st.skills_not_loaded.is_empty())
    {
        s.push_str(&format!(
            "warning: {} did not load skills: {}
",
            stage.name,
            stage.skills_not_loaded.join(", ")
        ));
    }
    for sched in state.plan_schedules.values() {
        let done = sched
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Success)
            .count();
        s.push_str(&format!(
            "- plan `{}`: {}/{} task(s) succeeded — {} wave(s), up to {} concurrent{}\n",
            sched.stage,
            done,
            sched.tasks.len(),
            sched.waves.len(),
            sched.max_parallel.min(sched.width.max(1) as u32),
            if sched.serializations.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} ordering(s) inserted for write conflicts",
                    sched.serializations.len()
                )
            }
        ));
        if plan_task_accounting_is_inconsistent(state, sched) {
            s.push_str(
                "  - ⚠ inconsistent: the stage itself succeeded, so this count is a reporting \
                 failure, not a work failure — check what the run produced before redoing it\n",
            );
        }
    }
    s
}
