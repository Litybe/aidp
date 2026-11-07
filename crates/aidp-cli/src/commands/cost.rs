use super::run_loader::load_run;
use super::CommandOutput;
use crate::args::CostOp;
use crate::ctx::Ctx;
use aidp_core::cost::anomaly::{is_anomaly, median_and_mad, modified_z_scores};
use aidp_core::cost::repricing::{
    reprice_usage, ALL_BASELINES_SELECTOR, BASELINE_LABEL_PREFIX, COUNTERFACTUAL_BASELINES,
};
use aidp_core::knowledge::{LEGACY_KNOWLEDGE_DIR, ROUTING_DECISIONS_FILE, ROUTING_DIR};
use aidp_core::run::{ComplexityBucket, UsageSource, RUN_STATE_FILE};
use aidp_core::ExitStatus;
use std::path::{Path, PathBuf};

const COST_LEDGER_PRICING_UNAVAILABLE: &str =
    "aidp cost ledger: pricing grid unavailable, cost_usd written as null";
const REPRICE_PRICING_UNAVAILABLE: &str = "cannot reprice the run";
const CLAUDE_PROJECTS_ENV: &str = "AIDP_CLAUDE_PROJECTS_DIR";
const CLAUDE_PROJECTS_RELATIVE: &str = ".claude/projects";
const LEDGER_MAX_LINES: usize = 20_000;
const ANOMALY_MIN_POINTS: usize = 3;
const LEDGER_SCHEMA_VERSION: u32 = 1;
const NO_VAULT_REASON: &str = "no vault resolved";
const NO_COST_LEDGER_NOTE: &str =
    "no cost ledger attached to this run (unavailable) — nothing to replay";
const COST_ONLY_WARNING: &str =
    "contrefactuel de COÛT uniquement : il ne dit rien de la qualité qu'aurait eue l'autre modèle";

pub fn run(ctx: &Ctx, op: CostOp) -> anyhow::Result<CommandOutput> {
    match op {
        CostOp::Show { run } => show(ctx, &run),
        CostOp::Ticket { key, url } => ticket(ctx, &key, &url),
        CostOp::Ledger { run, out } => ledger(ctx, &run, out),
        CostOp::Counterfactual { run, baseline } => counterfactual(ctx, &run, &baseline),
        CostOp::Anomaly {
            repo,
            template,
            last,
        } => anomaly(ctx, &repo, template, last),
    }
}

/// Locate the Claude Code projects directory holding agent transcripts.
fn claude_projects_dir(ctx: &Ctx) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(CLAUDE_PROJECTS_ENV) {
        let pb = PathBuf::from(p);
        return pb.is_dir().then_some(pb);
    }
    let dir = ctx.home.as_ref()?.join(CLAUDE_PROJECTS_RELATIVE);
    dir.is_dir().then_some(dir)
}

/// `aidp cost ticket <key>`: the daemon's cost report, or its footer line with `--format footer`.
fn ticket(ctx: &Ctx, key: &str, url: &Option<String>) -> anyhow::Result<CommandOutput> {
    let value = aidp_gateway::client::ticket_cost(url, key).map_err(super::flow::daemon_failure)?;
    if ctx.footer() {
        let cost: aidp_gateway::ticket_cost::TicketCost = serde_json::from_value(value)?;
        return Ok(CommandOutput::text(
            aidp_gateway::ticket_cost::ticket_footer(&cost),
        ));
    }
    Ok(CommandOutput::json(ctx.format(), value))
}

fn tokens_json(usage: &aidp_core::run::StageUsage) -> serde_json::Value {
    serde_json::json!({
        "input": usage.input,
        "output": usage.output,
        "cache_creation": usage.cache_creation,
        "cache_read": usage.cache_read,
    })
}

fn show(ctx: &Ctx, run: &str) -> anyhow::Result<CommandOutput> {
    let format = ctx.format();
    let run_state = load_run(ctx, &ctx.workspace(), run)?;

    let mut stages = Vec::new();
    let mut total_usd = 0.0f64;
    let mut any_known = false;
    let mut all_unavailable = true;

    for stage in &run_state.stages {
        let row = match &stage.usage {
            Some(u) => {
                if u.source == UsageSource::Jsonl {
                    all_unavailable = false;
                }
                if let Some(c) = u.cost_usd {
                    total_usd += c;
                    any_known = true;
                }
                serde_json::json!({
                    "stage": stage.name,
                    "cost_usd": u.cost_usd,
                    "source": u.source.as_str(),
                    "model": u.model,
                    "tokens": tokens_json(u),
                })
            }
            None => serde_json::json!({
                "stage": stage.name,
                "cost_usd": null,
                "source": UsageSource::Unavailable.as_str(),
                "model": null,
                "tokens": null,
            }),
        };
        stages.push(row);
    }

    let by_model: Vec<serde_json::Value> = run_state
        .cost
        .as_ref()
        .map(|c| {
            c.by_model
                .iter()
                .map(|(model, u)| {
                    serde_json::json!({
                        "model": model,
                        "cost_usd": u.cost_usd,
                        "tokens": tokens_json(u),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let total_source = if any_known && !all_unavailable {
        UsageSource::Jsonl
    } else if stages
        .iter()
        .any(|r| r["source"] == UsageSource::Budget.as_str())
    {
        UsageSource::Budget
    } else {
        UsageSource::Unavailable
    };

    Ok(CommandOutput::json(
        format,
        serde_json::json!({
            "run_id": run_state.run_id,
            "pricing_version": run_state.cost.as_ref().and_then(|c| c.pricing_version.clone()),
            "stages": stages,
            "by_model": by_model,
            "total_usd": if any_known { Some(total_usd) } else { None },
            "total_source": total_source.as_str(),
        }),
    ))
}

/// Where the routing ledger goes: `--out` wins outright, else the durable per-repo
/// location under the vault, else the legacy in-repo knowledge directory.
fn ledger_destination(
    out: Option<PathBuf>,
    resolved: &Option<aidp_core::models::vault::Vault>,
    repo: &str,
    workspace: &Path,
) -> PathBuf {
    if let Some(o) = out {
        return o;
    }
    let knowledge_dir = match resolved {
        Some(vault) => vault.knowledge_dir(repo),
        None => workspace.join(LEGACY_KNOWLEDGE_DIR),
    };
    knowledge_dir.join(ROUTING_DIR).join(ROUTING_DECISIONS_FILE)
}

/// One ledger line per sub-agent. Every field is numeric or categorical: no prompt text.
fn ledger_row(
    run_state: &aidp_core::run::RunState,
    agent: &aidp_core::tokens::AgentUsage,
    repo: &str,
    pricing_available: bool,
) -> serde_json::Value {
    let stage_state = run_state
        .stages
        .iter()
        .find(|s| Some(&s.name) == agent.stage.as_ref());
    let status = stage_state
        .map(|s| serde_json::to_value(s.status).unwrap_or(serde_json::Value::Null))
        .unwrap_or(serde_json::Value::Null);
    let loop_iters = agent
        .stage
        .as_ref()
        .and_then(|s| run_state.loops.get(s))
        .map(|l| l.iter);
    let gate = run_state
        .gates
        .iter()
        .find(|g| Some(&g.after_stage) == agent.stage.as_ref())
        .map(|g| g.decision.as_str().to_string());
    let tier = agent
        .model
        .as_deref()
        .map_or(aidp_core::cost::ModelTier::Unknown, aidp_core::cost::tier);
    let cost_usd =
        pricing_available.then(|| aidp_core::cost::cost_usd(tier.as_str(), &agent.usage, None));
    serde_json::json!({
        "v": LEDGER_SCHEMA_VERSION,
        "run_id": run_state.run_id,
        "repo": repo,
        "template": run_state.pipeline_id,
        "stage": agent.stage,
        "task_id": agent.task_id,
        "agent_id": agent.agent_id,
        "agent_type": agent.agent_type,
        "bucket": ComplexityBucket::Medium.as_str(),
        "model_pinned": agent.model,
        "model_used": agent.model,
        "tokens": {
            "input": agent.usage.input_tokens,
            "output": agent.usage.output_tokens,
            "cache_creation": agent.usage.cache_creation_input_tokens,
            "cache_read": agent.usage.cache_read_input_tokens,
        },
        "tokens_source": UsageSource::Jsonl.as_str(),
        "cost_usd": cost_usd,
        "pricing_version": aidp_core::cost::pricing_version(),
        "status": status,
        "schema_valid": true,
        "has_blocking_findings": serde_json::Value::Null,
        "loop_iters": loop_iters,
        "gate": gate,
    })
}

/// Appends one row per agent to the ledger, returning the number written.
fn append_ledger_rows(
    ctx: &Ctx,
    run_state: &aidp_core::run::RunState,
    workspace: &Path,
    dest: &Path,
    repo: &str,
    diagnostics: &mut Vec<String>,
) -> anyhow::Result<u64> {
    use std::io::Write;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    rotate_ledger_if_needed(dest);
    let agents: Vec<aidp_core::tokens::AgentUsage> = match claude_projects_dir(ctx) {
        Some(dir) => aidp_core::tokens::scan_run_agents(&dir, workspace, &run_state.run_id),
        None => Vec::new(),
    };
    let pricing_load_error = aidp_core::cost::load_error();
    if let Some(reason) = pricing_load_error {
        diagnostics.push(format!("{COST_LEDGER_PRICING_UNAVAILABLE}: {reason}"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dest)?;
    let mut lines_written: u64 = 0;
    for agent in &agents {
        let row = ledger_row(run_state, agent, repo, pricing_load_error.is_none());
        writeln!(file, "{}", serde_json::to_string(&row)?)?;
        lines_written += 1;
    }
    Ok(lines_written)
}

fn ledger(ctx: &Ctx, run: &str, out: Option<PathBuf>) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let resolved = ctx.vault_override();
    let run_state = load_run(ctx, &workspace, run)?;
    let repo = run_state
        .repo
        .clone()
        .unwrap_or_else(|| aidp_core::vault::repo_segment(&workspace));
    let dest = ledger_destination(out, &resolved, &repo, &workspace);

    let mut diagnostics = Vec::new();
    let mut lines_written: u64 = 0;
    let mut write_error: Option<String> = None;
    match append_ledger_rows(ctx, &run_state, &workspace, &dest, &repo, &mut diagnostics) {
        Ok(written) => lines_written = written,
        Err(e) => {
            write_error = Some(e.to_string());
            diagnostics.push(format!("aidp cost ledger: write failed (swallowed): {e}"));
        }
    }
    let summary = serde_json::json!({
        "ledger_path": dest.display().to_string(),
        "lines_written": lines_written,
        "write_error": write_error,
    });
    Ok(CommandOutput::text(serde_json::to_string_pretty(&summary)?).with_diagnostics(diagnostics))
}

/// Rotate the ledger at 20 000 lines by renaming it to `<name>.1` (clobbering
/// any previous `.1`).
fn rotate_ledger_if_needed(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    if content.lines().count() < LEDGER_MAX_LINES {
        return;
    }
    let rotated = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.1"),
        None => "1".to_string(),
    });
    let _ = std::fs::rename(path, rotated);
}

fn counterfactual(ctx: &Ctx, run: &str, baseline: &str) -> anyhow::Result<CommandOutput> {
    let format = ctx.format();
    let run_state = load_run(ctx, &ctx.workspace(), run)?;

    let Some(by_model) = run_state.cost.as_ref().map(|c| &c.by_model) else {
        return Ok(CommandOutput::json(
            format,
            serde_json::json!({ "run_id": run, "note": NO_COST_LEDGER_NOTE }),
        ));
    };
    let actual_total: f64 = by_model.values().filter_map(|u| u.cost_usd).sum();
    let baselines: Vec<String> = if baseline == ALL_BASELINES_SELECTOR {
        COUNTERFACTUAL_BASELINES.iter().map(|b| b.label()).collect()
    } else {
        vec![baseline.to_string()]
    };
    if let Some(reason) = aidp_core::cost::load_error() {
        anyhow::bail!("{REPRICE_PRICING_UNAVAILABLE}: {reason}");
    }
    let reprice_at = |tier: &str| -> f64 {
        let price = aidp_core::cost::price(tier);
        by_model.values().map(|u| reprice_usage(u, &price)).sum()
    };

    let mut results = Vec::new();
    for label in baselines {
        let counterfactual = reprice_at(label.trim_start_matches(BASELINE_LABEL_PREFIX));
        let savings = actual_total - counterfactual;
        let savings_pct = if actual_total > 0.0 {
            savings / actual_total * 100.0
        } else {
            0.0
        };
        let interpretation = if savings < 0.0 {
            format!("negative savings: {label} would have cost MORE than what was actually spent")
        } else {
            format!("{label} would have cost less by {savings_pct:.1}%")
        };
        results.push(serde_json::json!({
            "baseline": label,
            "actual": actual_total,
            "counterfactual": counterfactual,
            "savings": savings,
            "savings_pct": savings_pct,
            "interpretation": interpretation,
        }));
    }
    Ok(CommandOutput::json(
        format,
        serde_json::json!({
            "run_id": run,
            "pricing_version_used": run_state.cost.as_ref().and_then(|c| c.pricing_version.clone()),
            "results": results,
            "warning": COST_ONLY_WARNING,
        }),
    ))
}

/// Outcome of `aidp cost anomaly`.
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum AnomalyStatus {
    Ok,
    InsufficientData,
}

#[derive(serde::Serialize)]
struct AnomalyPoint {
    run_id: String,
    total_usd: f64,
}

/// The total cost of every run of `repo` that recorded one, oldest first.
/// Run ids sort lexicographically because they start with a timestamp.
fn cost_points(repo_dir: &Path, template: &Option<String>) -> Vec<AnomalyPoint> {
    let mut points: Vec<AnomalyPoint> = Vec::new();
    let Ok(features) = std::fs::read_dir(repo_dir) else {
        return points;
    };
    for feature in features.flatten() {
        let feature_path = feature.path();
        if !feature_path.is_dir() {
            continue;
        }
        let Ok(runs) = std::fs::read_dir(&feature_path) else {
            continue;
        };
        for run_entry in runs.flatten() {
            let state_path = run_entry.path().join(RUN_STATE_FILE);
            if !state_path.is_file() {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&state_path) else {
                continue;
            };
            let Ok(state) = serde_json::from_str::<aidp_core::run::RunState>(&raw) else {
                continue;
            };
            if template.as_ref().is_some_and(|t| &state.pipeline_id != t) {
                continue;
            }
            if let Some(total) = state.cost.as_ref().and_then(|c| c.total_usd) {
                points.push(AnomalyPoint {
                    run_id: state.run_id.clone(),
                    total_usd: total,
                });
            }
        }
    }
    points.sort_by(|a, b| a.run_id.cmp(&b.run_id));
    points
}

fn anomaly(
    ctx: &Ctx,
    repo: &str,
    template: Option<String>,
    last: Option<usize>,
) -> anyhow::Result<CommandOutput> {
    let format = ctx.format();
    let Some(vault) = ctx.vault_override() else {
        return Ok(CommandOutput::json(
            format,
            serde_json::json!({ "status": AnomalyStatus::InsufficientData, "n": 0, "reason": NO_VAULT_REASON }),
        ));
    };
    let repo_dir = vault.repo_dir(repo);
    let mut points = if repo_dir.is_dir() {
        cost_points(&repo_dir, &template)
    } else {
        Vec::new()
    };
    if let Some(n) = last {
        if points.len() > n {
            points = points.split_off(points.len() - n);
        }
    }
    let n = points.len();
    if n < ANOMALY_MIN_POINTS {
        return Ok(CommandOutput::json(
            format,
            serde_json::json!({ "status": AnomalyStatus::InsufficientData, "n": n }),
        ));
    }
    Ok(anomaly_report(format, &points))
}

/// Flags each point by its modified z-score. A zero MAD flags nothing.
/// Exits with a negative verdict when any point is flagged.
fn anomaly_report(format: Option<aidp_core::Format>, points: &[AnomalyPoint]) -> CommandOutput {
    let values: Vec<f64> = points.iter().map(|p| p.total_usd).collect();
    let (median, mad) = median_and_mad(&values);
    let scores = modified_z_scores(&values);
    let mut any_flagged = false;
    let mut rows = Vec::new();
    for ((point, total), z) in points.iter().zip(values.iter()).zip(scores) {
        let flagged = mad != 0.0 && is_anomaly(z);
        any_flagged |= flagged;
        rows.push(serde_json::json!({
            "run_id": point.run_id,
            "total_usd": total,
            "modified_z": z,
            "flagged": flagged,
        }));
    }
    let output = CommandOutput::json(
        format,
        serde_json::json!({
            "status": AnomalyStatus::Ok,
            "n": points.len(),
            "median": median,
            "mad": mad,
            "points": rows,
            "any_flagged": any_flagged,
        }),
    );
    if any_flagged {
        return output.with_exit(ExitStatus::NegativeVerdict);
    }
    output
}

#[cfg(test)]
#[path = "../../tests/unit/cost_cmd_tests.rs"]
mod tests;
