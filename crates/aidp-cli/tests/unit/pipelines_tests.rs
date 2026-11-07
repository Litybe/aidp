use super::*;

/// Adapts the context-taking function to the signature the shared unit tests call.
fn validate_pipeline_file(
    path: &Path,
    workspace: &Path,
    inputs: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let ctx = Ctx::for_test(workspace.to_path_buf(), None);
    super::validate_pipeline_file(&ctx, path, workspace, inputs)
}

#[path = "validate_all_tests.rs"]
mod validate_all_tests;

#[test]
fn resolve_names_the_skill_root_variable_when_none_resolved() {
    let ctx = Ctx::for_test(std::env::temp_dir(), None);
    let err = resolve_pipeline_ref(&ctx, &ctx.cwd, "no-such-pipeline-id").unwrap_err();
    assert!(err.to_string().contains(SKILL_ROOT_ENV));
}
