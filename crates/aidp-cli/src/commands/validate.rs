use super::CommandOutput;
use crate::ctx::Ctx;
use crate::models::pipeline_validation_failure::PipelineValidationFailure;
use crate::models::validate_all_summary::ValidateAllSummary;
use crate::pipelines::{discover_pipelines, resolve_pipeline_ref, validate_pipeline_file};
use aidp_agents::ProvidersConfig;
use aidp_core::pipeline::Pipeline;
use std::path::{Path, PathBuf};

const SETTINGS_PROVIDERS_LABEL: &str = "settings.providers";

pub struct ValidateArgs {
    pub pipeline: Option<String>,
    pub all: bool,
    pub inputs: Option<String>,
    pub workspace_root: Option<PathBuf>,
}

pub fn run(ctx: &Ctx, args: ValidateArgs) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(args.workspace_root);
    if args.all {
        return validate_all(ctx, &workspace, args.inputs.as_deref());
    }
    let pipeline = args
        .pipeline
        .ok_or_else(|| anyhow::anyhow!("pipeline id or path required"))?;
    let path = resolve_pipeline_ref(ctx, &workspace, &pipeline)?;
    let report = validate_pipeline_file(ctx, &path, &workspace, args.inputs.as_deref())?;
    check_pipeline_providers(&path)?;
    Ok(CommandOutput::text(report.to_string()))
}

/// Loads the pipeline and the providers config, then rejects any unknown provider id.
fn check_pipeline_providers(path: &Path) -> anyhow::Result<()> {
    let pipeline = aidp_core::pipeline::load(path)?;
    let config = aidp_core::vault::home_dir()
        .and_then(|home| ProvidersConfig::load(&home).ok())
        .unwrap_or_default();
    unknown_provider_errors(&pipeline, &config)
}

/// Fails naming each provider id used by a stage or by `settings.providers` that the config does not know.
pub(crate) fn unknown_provider_errors(
    pipeline: &Pipeline,
    config: &ProvidersConfig,
) -> anyhow::Result<()> {
    let mut problems: Vec<String> = Vec::new();
    for (role, id) in &pipeline.settings.providers {
        if !config.knows(id) {
            problems.push(format!(
                "unknown provider `{id}` in {SETTINGS_PROVIDERS_LABEL}.{role}"
            ));
        }
    }
    for stage in aidp_core::pipeline::flat_stages(pipeline) {
        if let Some(id) = stage.provider.as_deref().filter(|id| !config.knows(id)) {
            problems.push(format!("unknown provider `{id}` in stage `{}`", stage.name));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    anyhow::bail!(problems.join("; "))
}

/// Validates every discoverable pipeline, exactly the set `aidp pipeline list` shows.
fn validate_all(
    ctx: &Ctx,
    workspace: &std::path::Path,
    inputs: Option<&str>,
) -> anyhow::Result<CommandOutput> {
    let found = discover_pipelines(ctx, workspace)?;
    let mut results = Vec::new();
    let mut failed = 0usize;
    let mut diagnostics = Vec::new();
    for (id, path) in &found {
        let validated = validate_pipeline_file(ctx, path, workspace, inputs)
            .and_then(|report| check_pipeline_providers(path).map(|()| report));
        match validated {
            Ok(mut pipeline_json) => {
                pipeline_json["id"] = serde_json::json!(id);
                results.push(pipeline_json);
            }
            Err(e) => {
                failed += 1;
                diagnostics.push(format!("{id}: {e}"));
                results.push(serde_json::to_value(PipelineValidationFailure {
                    error: e.to_string(),
                    id,
                    ok: false,
                    path: path.display().to_string(),
                })?);
            }
        }
    }
    let total = results.len();
    let summary = serde_json::to_string(&ValidateAllSummary {
        failed,
        ok: failed == 0,
        total,
        results,
    })?;
    let output = CommandOutput::text(summary).with_diagnostics(diagnostics);
    if failed > 0 {
        return Ok(output.with_failure(anyhow::anyhow!(
            "{failed} of {total} pipeline(s) failed validation"
        )));
    }
    Ok(output)
}

#[cfg(test)]
#[path = "../../tests/unit/validate_providers_tests.rs"]
mod validate_providers_tests;
