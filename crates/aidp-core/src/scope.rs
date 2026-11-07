//! Scope enforcement: decides what a path may touch and audits finished runs.
//! [`verify_run_report`] classifies git's changed paths against each stage's scope.

use crate::git::{Git, GitCli};
use crate::models::constants::scope::{
    DOCS_DIR, FIELD_ALLOWED, FIELD_DENIED, JS_TESTS_DIR, MODE_CUSTOM, MODE_PRODUCTION,
    MODE_REPO_WIDE, MODE_STAGE_DIR_ONLY, MODE_TESTS, NO_RULES_REASON, SCOPE_JSON_FILE, TESTS_DIR,
};
pub use crate::models::scope::{ResolvedScope, ScopeReport, ScopeRules, Violation};
use crate::pipeline::{self, ScopeSpec};
use crate::run::{RunState, StageStatus};
use crate::Error;
use std::path::{Path, PathBuf};

impl ScopeRules {
    fn require_in_workspace(&self, file: &Path, path: &str) -> Result<(), Error> {
        let candidate = if file.is_relative() {
            self.workspace_root.join(file)
        } else {
            file.to_path_buf()
        };
        if path_under(&candidate, &self.workspace_root) {
            Ok(())
        } else {
            Err(Error::ScopeViolation(format!(
                "not under workspace_root: {path}"
            )))
        }
    }

    pub fn check(&self, file: &Path) -> Result<(), Error> {
        let path = file.to_string_lossy();
        if self.denied.iter().any(|pattern| pattern.matches(&path)) {
            return Err(Error::ScopeViolation(format!("denied: {path}")));
        }
        match self.mode.as_str() {
            MODE_STAGE_DIR_ONLY => {
                if path_under(file, &self.stage_dir) {
                    Ok(())
                } else {
                    Err(Error::ScopeViolation(format!(
                        "not under stage_dir: {path}"
                    )))
                }
            }
            MODE_PRODUCTION => {
                self.require_in_workspace(file, &path)?;
                if has_dir_component(&path, TESTS_DIR) || has_dir_component(&path, DOCS_DIR) {
                    Err(Error::ScopeViolation(format!(
                        "production scope excludes tests/docs: {path}"
                    )))
                } else {
                    Ok(())
                }
            }
            MODE_TESTS => {
                self.require_in_workspace(file, &path)?;
                let is_test = has_dir_component(&path, TESTS_DIR)
                    || has_dir_component(&path, JS_TESTS_DIR)
                    || looks_like_test_file(file);
                if is_test {
                    Ok(())
                } else {
                    Err(Error::ScopeViolation(format!("not in tests: {path}")))
                }
            }
            MODE_REPO_WIDE => Ok(()),
            MODE_CUSTOM => {
                if self.allowed.iter().any(|pattern| pattern.matches(&path)) {
                    Ok(())
                } else {
                    Err(Error::ScopeViolation(format!(
                        "not in custom allow: {path}"
                    )))
                }
            }
            other => Err(Error::Other(format!("unknown scope mode: {other}"))),
        }
    }
}

/// True if any `/`- or `\`-separated component of `path` equals `dir`
/// (case-insensitive). Cross-platform; substring-safe.
fn has_dir_component(path: &str, dir: &str) -> bool {
    path.split(['/', '\\'])
        .any(|segment| segment.eq_ignore_ascii_case(dir))
}

/// Whether a file name follows a test convention (`FooTests.cs`, `foo.test.ts`).
/// Lookalikes such as `latest.cs` are excluded.
fn looks_like_test_file(file: &Path) -> bool {
    let Some(raw) = file.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lowered = raw.to_ascii_lowercase();
    raw.contains("Test")
        || raw.contains("Spec")
        || lowered.contains(".test.")
        || lowered.contains(".spec.")
        || lowered.contains("_test.")
        || lowered.contains("_spec.")
        || lowered.starts_with("test_")
}

/// Whether `file` is `stage_dir` or below it, after lexical and symlink resolution.
/// A path whose `..` climbs above the root is never under anything.
fn path_under(file: &Path, stage_dir: &Path) -> bool {
    match (resolve_path(file), resolve_path(stage_dir)) {
        (Some(resolved_file), Some(resolved_dir)) => resolved_file.starts_with(&resolved_dir),
        _ => false,
    }
}

/// Canonicalize `path` when it exists; otherwise normalize it lexically,
/// canonicalize its deepest existing ancestor and re-append the rest.
fn resolve_path(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }
    let norm = normalize_lexical(path)?;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = norm.as_path();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(cur) {
            let mut out = canonical;
            for segment in tail.iter().rev() {
                out.push(segment);
            }
            return Some(out);
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return Some(norm),
        }
    }
}

/// Fold `.` and `..` without touching the filesystem. `None` when a `..`
/// would climb above the root, the prefix, or the start of a relative path.
fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut anchor = PathBuf::new();
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => anchor.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(segment) => parts.push(segment.to_os_string()),
        }
    }
    for segment in parts {
        anchor.push(segment);
    }
    Some(anchor)
}

pub fn rules_for(state: &RunState, stage_name: &str) -> anyhow::Result<ScopeRules> {
    let pipe = pipeline::load(&state.pipeline_path)?;
    let stage = pipeline::find_stage(&pipe, stage_name).ok_or_else(|| {
        crate::Error::NotFound(format!(
            "{} {stage_name}",
            crate::models::constants::messages::UNKNOWN_STAGE
        ))
    })?;
    let spec = stage.scope.clone().unwrap_or(ScopeSpec {
        mode: Some(crate::models::constants::messages::DEFAULT_SCOPE_MODE.into()),
        allowed: vec![],
        denied: vec![],
        writes: vec![],
        reads: vec![],
    });
    Ok(ScopeRules {
        mode: spec
            .mode
            .unwrap_or_else(|| crate::models::constants::messages::DEFAULT_SCOPE_MODE.into()),
        stage_dir: state.stage_dir(stage_name, stage.placement()),
        workspace_root: state.workspace_root.clone(),
        allowed: compile_globs(FIELD_ALLOWED, &spec.allowed)?,
        denied: compile_globs(FIELD_DENIED, &spec.denied)?,
    })
}

/// Compile every pattern or fail naming the offending one: a glob silently
/// dropped from `denied` would widen the scope without anyone noticing.
fn compile_globs(field: &str, patterns: &[String]) -> anyhow::Result<Vec<glob::Pattern>> {
    patterns
        .iter()
        .map(|pattern| {
            glob::Pattern::new(pattern).map_err(|error| {
                anyhow::anyhow!("invalid scope.{field} glob \"{pattern}\": {error}")
            })
        })
        .collect()
}

/// Resolve `stage_name`'s scope into the `aidp scope resolve` shape.
/// Pure: writes nothing; pair with [`write_scope_json`].
pub fn scope_json(state: &RunState, stage_name: &str) -> anyhow::Result<ResolvedScope> {
    let rules = rules_for(state, stage_name)?;
    Ok(ResolvedScope {
        stage: stage_name.to_string(),
        mode: rules.mode,
        stage_dir: rules.stage_dir,
        allowed: rules
            .allowed
            .iter()
            .map(|pattern| pattern.as_str().to_string())
            .collect(),
        denied: rules
            .denied
            .iter()
            .map(|pattern| pattern.as_str().to_string())
            .collect(),
    })
}

pub fn write_scope_json(
    state: &RunState,
    stage_name: &str,
    report: &ResolvedScope,
) -> anyhow::Result<PathBuf> {
    let stage_dir = rules_for(state, stage_name)?.stage_dir;
    std::fs::create_dir_all(&stage_dir)?;
    let path = stage_dir.join(SCOPE_JSON_FILE);
    std::fs::write(&path, serde_json::to_vec_pretty(report)?)?;
    Ok(path)
}

/// Classify `paths` against `(stage_name, rules)` pairs; a path accepted by any rule is fine.
/// The reason kept is the last rule's rejection message.
pub fn classify(paths: &[String], rules: &[(String, ScopeRules)]) -> Vec<Violation> {
    let mut out = Vec::new();
    for changed_path in paths {
        let candidate = PathBuf::from(changed_path);
        let mut last_reason: Option<String> = None;
        let mut accepted = false;
        for (_name, r) in rules {
            match r.check(&candidate) {
                Ok(()) => {
                    accepted = true;
                    break;
                }
                Err(error) => last_reason = Some(error.to_string()),
            }
        }
        if !accepted {
            let reason = last_reason.unwrap_or_else(|| NO_RULES_REASON.to_string());
            out.push(Violation {
                path: changed_path.clone(),
                reason,
            });
        }
    }
    out
}

/// Linked worktrees git has registered, excluding `workspace_root`.
/// Empty, never an error, when git is absent or none exist.
fn discover_extra_worktrees(workspace_root: &Path, git: &dyn Git) -> Vec<PathBuf> {
    git.worktree_list(workspace_root)
}

/// Classify every path git says changed against each successful stage's scope.
/// Read-only, and empty when `state.base_sha` is absent.
pub fn verify_run_report(state: &RunState) -> anyhow::Result<ScopeReport> {
    verify_run_report_with(state, &GitCli)
}

/// Same contract as [`verify_run_report`], through an injected [`Git`] port so
/// a test can supply canned worktree lists instead of shelling out.
pub fn verify_run_report_with(state: &RunState, git: &dyn Git) -> anyhow::Result<ScopeReport> {
    let Some(base_sha) = state.base_sha.clone() else {
        return Ok(ScopeReport::default());
    };

    let pipe = pipeline::load(&state.pipeline_path)?;
    let extra_worktrees = discover_extra_worktrees(&state.workspace_root, git);

    let mut violations = Vec::new();
    let mut union_universal = false;
    let mut uninspected_stages = Vec::new();
    let mut own_tree_rules: Vec<(String, ScopeRules)> = Vec::new();

    for stage in state
        .stages
        .iter()
        .filter(|stage| stage.status == StageStatus::Success)
    {
        let Some(sdef) = pipeline::find_stage(&pipe, &stage.name) else {
            continue;
        };
        let rules = match rules_for(state, &stage.name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rules.mode == MODE_REPO_WIDE {
            union_universal = true;
        }
        if sdef.isolation == Some(crate::models::enums::Isolation::Worktree) {
            if extra_worktrees.is_empty() {
                uninspected_stages.push(stage.name.clone());
                continue;
            }
            for worktree in &extra_worktrees {
                let paths = crate::gitlog::changed(worktree, &base_sha);
                violations.extend(classify(&paths, &[(stage.name.clone(), rules.clone())]));
            }
            continue;
        }
        own_tree_rules.push((stage.name.clone(), rules));
    }

    let own_paths = crate::gitlog::changed(&state.workspace_root, &base_sha);
    violations.extend(classify(&own_paths, &own_tree_rules));

    let mut all_paths = own_paths;
    for worktree in &extra_worktrees {
        all_paths.extend(crate::gitlog::changed(worktree, &base_sha));
    }
    let vault_leaks = crate::vault_leak::detect(&all_paths, state.vault_root.is_some());

    Ok(ScopeReport {
        violations,
        vault_leaks,
        union_universal,
        uninspected_stages,
    })
}

/// Projection of [`verify_run_report`] onto `RunState.scope_violations`.
pub fn verify_run(state: &RunState) -> anyhow::Result<Vec<Violation>> {
    Ok(verify_run_report(state)?.violations)
}

#[cfg(test)]
#[path = "../tests/unit/scope_tests.rs"]
mod tests;
