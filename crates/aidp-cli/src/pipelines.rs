use crate::ctx::{Ctx, SKILL_ROOT_ENV, TEMPLATES_DIR};
use aidp_core::ExitStatus;
use std::path::{Path, PathBuf};

const MAX_SUGGESTION_DISTANCE: usize = 3;
const YAML_EXTENSION: &str = ".yaml";
const YML_EXTENSION: &str = ".yml";

/// Every pipeline discoverable from the skill's `templates/` and the workspace flow directory.
pub fn discover_pipelines(ctx: &Ctx, ws: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let roots = ctx.pipeline_roots(ws);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    aidp_core::pipeline::discover(&refs)
}

/// Renders `aidp pipeline list` in the requested format.
pub fn list_pipelines(
    ctx: &Ctx,
    ws: &Path,
    format: Option<aidp_core::Format>,
) -> anyhow::Result<String> {
    let found = discover_pipelines(ctx, ws)?;
    let arr: Vec<_> = found
        .into_iter()
        .map(|(id, p)| serde_json::json!({"id": id, "path": p}))
        .collect();
    crate::output::render_value(format.unwrap_or_default(), &serde_json::Value::Array(arr))
}

/// Renders `aidp pipeline show` for one id or path.
pub fn show_pipeline(ctx: &Ctx, ws: &Path, id_or_path: &str) -> anyhow::Result<String> {
    let path = resolve_pipeline_ref(ctx, ws, id_or_path)?;
    let pipe = aidp_core::pipeline::load(&path)?;
    Ok(serde_json::to_string_pretty(&pipe)?)
}

/// Load, compose and inputs-check one pipeline file.
/// Returns the `{"ok", "path", "warnings"?}` report `aidp validate` prints per file.
pub fn validate_pipeline_file(
    ctx: &Ctx,
    path: &Path,
    workspace: &Path,
    inputs: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let pipe = aidp_core::pipeline::load(path)?;
    let roots = compose_roots(ctx, workspace, path);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let effective = aidp_core::compose::expand(&pipe, &refs)?;
    aidp_core::pipeline::topo_sort(&effective)?;
    if let Some(s) = inputs {
        let provided: serde_json::Value =
            serde_json::from_str(s).map_err(|e| anyhow::anyhow!("--inputs invalid JSON: {e}"))?;
        let provided: std::collections::BTreeMap<String, serde_json::Value> = match provided {
            serde_json::Value::Object(m) => m.into_iter().collect(),
            _ => anyhow::bail!("--inputs must be a JSON object"),
        };
        aidp_core::pipeline::validate_inputs(&pipe, &provided)?;
    }
    let mut warnings = inert_tools_warnings(&effective);
    if let Some(w) = check_skills(&effective, path)? {
        warnings.push(w);
    }
    let mut out = serde_json::json!({"ok": true, "path": path.display().to_string()});
    if !warnings.is_empty() {
        out["warnings"] = serde_json::json!(warnings);
    }
    Ok(out)
}

/// A warning per stage declaring `tools:`, which is documented as not wired into the emitter.
/// A warning never fails validation: the field predates the check.
fn inert_tools_warnings(effective: &aidp_core::pipeline::Pipeline) -> Vec<String> {
    aidp_core::pipeline::flat_stages(effective)
        .into_iter()
        .filter(|stage| stage.tools.is_some())
        .map(|stage| {
            format!(
                "stage {:?} declares tools: — currently inert (see pipeline::ToolsSpec doc comment)",
                stage.name
            )
        })
        .collect()
}

/// Fails when a declared skill would reach the prompt as `(not found)`,
/// naming each stage and skill.
fn check_skills(
    pipe: &aidp_core::pipeline::Pipeline,
    path: &Path,
) -> anyhow::Result<Option<String>> {
    let search = aidp_core::skills::default_search_paths();
    let searched = search
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if !search.iter().any(|p| p.is_dir()) {
        return Ok(declares_skills(pipe)
            .then(|| format!("skills not checked: no skill root found (searched: {searched})")));
    }
    let search_refs: Vec<&Path> = search.iter().map(|p| p.as_path()).collect();
    let missing = aidp_core::skills::unresolved(pipe, &search_refs);
    if missing.is_empty() {
        return Ok(None);
    }
    let lines: Vec<String> = missing
        .iter()
        .map(|u| format!("stage {:?}: skill {:?} not found", u.stage, u.skill))
        .collect();
    anyhow::bail!(
        "unresolved skill(s) in {} (searched: {searched}):\n  {}",
        path.display(),
        lines.join("\n  ")
    )
}

/// True when any stage declares a skill, in any list: against no search root
/// every declared name is unresolved.
fn declares_skills(pipe: &aidp_core::pipeline::Pipeline) -> bool {
    !aidp_core::skills::unresolved(pipe, &[]).is_empty()
}

/// Search roots for resolving composite `uses:` references.
/// Combines the skill's `templates/` dir with the core's per-pipeline roots.
pub fn compose_roots(ctx: &Ctx, workspace: &Path, pipeline_path: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(root) = &ctx.skill_root {
        roots.push(root.join(TEMPLATES_DIR));
    }
    for r in aidp_core::pipeline::discovery_roots(workspace, pipeline_path) {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    roots
}

/// Resolve a pipeline id-or-path to a YAML file path. The single resolution
/// rule shared by `validate`, `run init`, `topo` and `pipeline show`.
pub fn resolve_pipeline_ref(ctx: &Ctx, ws: &Path, id_or_path: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(id_or_path);
    if path.is_file() {
        return Ok(path);
    }
    let found = discover_pipelines(ctx, ws)?;
    for (id, p) in &found {
        if id == id_or_path {
            return Ok(p.clone());
        }
    }
    let looks_like_path = id_or_path.contains('/')
        || id_or_path.contains('\\')
        || id_or_path.ends_with(YAML_EXTENSION)
        || id_or_path.ends_with(YML_EXTENSION);
    if looks_like_path {
        if let Err(e) = std::fs::metadata(&path) {
            anyhow::bail!("{}: {e}", path.display());
        }
    }
    let mut ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
    ids.sort();
    let mut suggestions: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| aidp_core::text::levenshtein(id, id_or_path) <= MAX_SUGGESTION_DISTANCE)
        .collect();
    suggestions.sort_by_key(|id| aidp_core::text::levenshtein(id, id_or_path));
    let hint = skill_root_hint(ctx);
    let message = if suggestions.is_empty() {
        format!(
            "pipeline not found: {id_or_path} (available: {}){hint}",
            ids.join(", ")
        )
    } else {
        format!(
            "pipeline not found: {id_or_path} (did you mean: {}?){hint}",
            suggestions.join(", ")
        )
    };
    Err(aidp_core::TypedExit {
        status: ExitStatus::NotFound,
        message,
    }
    .into())
}

/// Names `AIDP_SKILL_ROOT` when no skill root resolved, since the template ids are then missing.
fn skill_root_hint(ctx: &Ctx) -> String {
    match ctx.skill_root {
        Some(_) => String::new(),
        None => format!(" — no skill root found: set {SKILL_ROOT_ENV} to the skill directory"),
    }
}

#[cfg(test)]
#[path = "../tests/unit/pipelines_tests.rs"]
mod tests;
