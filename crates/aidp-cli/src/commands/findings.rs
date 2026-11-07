use super::CommandOutput;
use crate::args::FindingsOp;
use crate::ctx::Ctx;
use crate::models::envelopes::{ChecksSummary, FindingsMergeEnvelope};
use aidp_core::ExitStatus;
use std::path::PathBuf;

const DEFAULT_AXIS_LABEL: &str = "axis";

pub fn run(ctx: &Ctx, op: FindingsOp) -> anyhow::Result<CommandOutput> {
    match op {
        FindingsOp::Merge {
            inputs,
            labels,
            out,
        } => merge(inputs, labels, out, ctx.format()),
    }
}

/// The merge inputs named by `--input` (label from the file stem) and `--label axis=path`.
fn merge_inputs(
    inputs: Vec<PathBuf>,
    labels: Vec<String>,
) -> anyhow::Result<Vec<aidp_core::findings::MergeInput>> {
    let mut merge_inputs: Vec<aidp_core::findings::MergeInput> = Vec::new();
    for p in inputs {
        let label = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(DEFAULT_AXIS_LABEL)
            .to_string();
        merge_inputs.push(aidp_core::findings::MergeInput { label, path: p });
    }
    for l in labels {
        let (axis, path) = l
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--label must be `axis=path`, got `{l}`"))?;
        merge_inputs.push(aidp_core::findings::MergeInput {
            label: axis.to_string(),
            path: PathBuf::from(path),
        });
    }
    Ok(merge_inputs)
}

/// `aidp findings merge`: writes the canonical findings.json to `--out`.
/// The merge log travels in the stdout envelope, since the schema forbids extra keys.
fn merge(
    inputs: Vec<PathBuf>,
    labels: Vec<String>,
    out: PathBuf,
    format: Option<aidp_core::Format>,
) -> anyhow::Result<CommandOutput> {
    let merge_inputs = merge_inputs(inputs, labels)?;
    if merge_inputs.is_empty() {
        return Err(aidp_core::TypedExit {
            status: ExitStatus::Usage,
            message: "findings merge needs at least one --input or --label axis=path".into(),
        }
        .into());
    }
    let outcome = aidp_core::findings::merge(&merge_inputs)?;
    let text = serde_json::to_string_pretty(&outcome.findings)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&out, text).map_err(|e| anyhow::anyhow!("write {}: {e}", out.display()))?;

    let envelope = FindingsMergeEnvelope {
        checks_summary: checks_summary(&outcome.findings),
        degraded_axes: &outcome.degraded_axes,
        has_blocking_findings: outcome.findings.has_blocking_findings,
        merge_log: &outcome.merge_log,
        out: out.display().to_string(),
        severity_counts: &outcome.findings.severity_counts,
    };
    Ok(CommandOutput::json(
        format,
        serde_json::to_value(&envelope)?,
    ))
}

fn checks_summary(findings: &aidp_core::findings::Findings) -> ChecksSummary {
    let mut summary = ChecksSummary {
        fail: 0,
        pass: 0,
        skip: 0,
    };
    for check in &findings.checks {
        match check.status {
            aidp_core::findings::CheckStatus::Pass => summary.pass += 1,
            aidp_core::findings::CheckStatus::Fail => summary.fail += 1,
            aidp_core::findings::CheckStatus::Skip => summary.skip += 1,
        }
    }
    summary
}
