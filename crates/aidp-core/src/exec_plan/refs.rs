//! Parsing of `stages.<stage>.<segment>.<name>` references shared by the compiler and the emitter.

use crate::expr;
use crate::models::constants::refs::{
    HANDLEBARS_CLOSE, HANDLEBARS_OPEN, REF_OUTPUTS, REF_SEPARATOR, REF_SIGNALS, REF_STAGES,
    STAGE_REF_PARTS,
};
use crate::models::enums::AgentRole;
use crate::pipeline::{Pipeline, Stage};
use crate::run::RunState;
use std::path::PathBuf;

/// Splits `stages.<stage>.<segment>.<name>` into `(stage, name)`.
fn parse_stage_ref(raw: &str, segment: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = strip_handlebars(raw)
        .split(REF_SEPARATOR)
        .map(str::trim)
        .collect();
    let well_formed = parts.len() == STAGE_REF_PARTS
        && parts[0] == REF_STAGES
        && parts[2] == segment
        && !parts[1].is_empty()
        && !parts[3].is_empty();
    well_formed.then(|| (parts[1].to_string(), parts[3].to_string()))
}

/// The expression inside optional `{{ }}` wrapping.
pub(crate) fn strip_handlebars(raw: &str) -> &str {
    raw.trim()
        .trim_start_matches(HANDLEBARS_OPEN)
        .trim_end_matches(HANDLEBARS_CLOSE)
        .trim()
}

/// Parses `fan_out.from` into `(stage, signal property)`.
pub(crate) fn parse_fanout_from(stage: &str, raw: &str) -> anyhow::Result<(String, String)> {
    parse_stage_ref(raw, REF_SIGNALS).ok_or_else(|| {
        anyhow::anyhow!(
            "stage `{stage}`: fan_out.from `{}` must be of shape `stages.<stage>.signals.<prop>` (an upstream array signal)",
            raw.trim()
        )
    })
}

/// Parses `plan_tasks.from` into `(stage, output name)`.
pub(crate) fn parse_plan_from(stage: &str, raw: &str) -> anyhow::Result<(String, String)> {
    parse_stage_ref(raw, REF_OUTPUTS).ok_or_else(|| {
        anyhow::anyhow!(
            "stage `{stage}`: plan_tasks.from `{}` must be of shape `stages.<stage>.outputs.<name>` (the upstream plan.json artifact)",
            raw.trim()
        )
    })
}

/// Where an upstream stage's output artifact lives.
pub(crate) fn plan_artifact_path(
    state: &RunState,
    pipe: &Pipeline,
    producer: &str,
    output: &str,
) -> PathBuf {
    let eval = expr::EvalCtx::new(state, pipe);
    PathBuf::from(expr::render_token(
        &format!("{REF_STAGES}.{producer}.{REF_OUTPUTS}.{output}"),
        &eval,
    ))
}

/// Whether the stage runs as a reviewer.
pub(crate) fn is_reviewer(stage: &Stage) -> bool {
    stage.agent_role() == AgentRole::Reviewer
}
