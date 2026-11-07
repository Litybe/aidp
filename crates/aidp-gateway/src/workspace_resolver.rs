//! Resolves a [`WorkspaceRequest`] to a directory before the admission lock is taken.
//! Every git call goes through [`ProcessRunner`] so tests can script it.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use aidp_core::process_runner::ProcessRunner;
use aidp_core::safe_name;

use crate::models::enums::ApiStatus;
use crate::models::{ApiError, ProjectRef, WorkspaceRequest};
use crate::worktrees::{self, EnsureError, Workspace, WorkspaceKind};

/// Environment variable listing the roots a `path` workspace may live under.
pub const ALLOWED_ROOTS_ENV: &str = "AIDP_ALLOWED_ROOTS";
/// Longest scratch name accepted, in characters.
pub const SCRATCH_NAME_MAX_LEN: usize = 41;

const TICKET_FIELD: &str = "ticket";
const NAME_SEPARATOR: char = '-';
const MISSING_TICKET_MESSAGE: &str = "workspace kind ticket needs a ticket key";
const MISSING_NAME_MESSAGE: &str = "workspace kind scratch needs a name";
const PR_WORKTREE_MESSAGE: &str = "workspace kind prworktree is created by the pr fix route only";
const MISSING_PATH_MESSAGE: &str = "workspace kind path needs a path";
const INVALID_NAME_MESSAGE: &str = "scratch name must match ^[a-z0-9][a-z0-9-]{0,40}$";
const NO_VAULT_MESSAGE: &str = "no vault configured: ticket and scratch workspaces need one";
const NO_TICKET_WORKTREE_MESSAGE: &str = "no worktree exists for ticket";
const RELATIVE_PATH_MESSAGE: &str = "workspace path must be absolute";
const PARENT_PATH_MESSAGE: &str = "workspace path must not contain `..`";
const MISSING_DIR_MESSAGE: &str = "workspace path is not an existing directory";
const OUTSIDE_ROOTS_MESSAGE: &str = "workspace path is outside the allowed roots";

fn failure(status: ApiStatus, message: impl Into<String>) -> ApiError {
    ApiError {
        status,
        message: message.into(),
    }
}

/// True for `^[a-z0-9][a-z0-9-]{0,40}$`.
pub fn is_valid_scratch_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= SCRATCH_NAME_MAX_LEN
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == NAME_SEPARATOR)
}

/// Roots from the raw env value, else every project path plus the vault root.
pub fn allowed_roots(
    env_value: Option<OsString>,
    project_paths: &[PathBuf],
    vault: Option<&Path>,
) -> Vec<PathBuf> {
    match env_value {
        Some(value) if !value.is_empty() => std::env::split_paths(&value).collect(),
        _ => project_paths
            .iter()
            .cloned()
            .chain(vault.map(Path::to_path_buf))
            .collect(),
    }
}

/// Canonical directory of `raw` when it is absolute, free of `..`, existing and under one of `roots`.
pub fn check_path_allowed(raw: &str, roots: &[PathBuf]) -> Result<PathBuf, ApiError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(failure(ApiStatus::BadRequest, RELATIVE_PATH_MESSAGE));
    }
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(failure(ApiStatus::Forbidden, PARENT_PATH_MESSAGE));
    }
    let canonical = std::fs::canonicalize(path)
        .ok()
        .filter(|resolved| resolved.is_dir())
        .ok_or_else(|| failure(ApiStatus::BadRequest, MISSING_DIR_MESSAGE))?;
    let allowed = roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| canonical.starts_with(root));
    if allowed {
        Ok(canonical)
    } else {
        Err(failure(ApiStatus::Forbidden, OUTSIDE_ROOTS_MESSAGE))
    }
}

fn ensure_failure(error: EnsureError) -> ApiError {
    let status = match error {
        EnsureError::BranchCheckedOutElsewhere { .. } => ApiStatus::Conflict,
        EnsureError::InvalidKey(_) => ApiStatus::BadRequest,
        EnsureError::Git(_) => ApiStatus::Upstream,
        EnsureError::Io(_) => ApiStatus::Io,
    };
    failure(status, error.to_string())
}

fn resolve_ticket(
    runner: &dyn ProcessRunner,
    key: &str,
    project: &ProjectRef,
    vault: Option<&Path>,
) -> Result<Workspace, ApiError> {
    safe_name::check(TICKET_FIELD, key)
        .map_err(|error| failure(ApiStatus::BadRequest, error.to_string()))?;
    let vault = vault.ok_or_else(|| failure(ApiStatus::Config, NO_VAULT_MESSAGE))?;
    worktrees::existing_for_branch(
        runner,
        &project.path,
        vault,
        &project.repo,
        &worktrees::branch_for(key),
    )
    .ok_or_else(|| {
        failure(
            ApiStatus::Conflict,
            format!("{NO_TICKET_WORKTREE_MESSAGE} {key}"),
        )
    })
}

fn resolve_scratch(
    runner: &dyn ProcessRunner,
    name: &str,
    project: &ProjectRef,
    vault: Option<&Path>,
) -> Result<Workspace, ApiError> {
    if !is_valid_scratch_name(name) {
        return Err(failure(ApiStatus::BadRequest, INVALID_NAME_MESSAGE));
    }
    let vault = vault.ok_or_else(|| failure(ApiStatus::Config, NO_VAULT_MESSAGE))?;
    worktrees::ensure_scratch(runner, &project.path, vault, &project.repo, name)
        .map(|ensured| ensured.workspace)
        .map_err(ensure_failure)
}

fn resolve_path(
    raw: &str,
    project_paths: &[PathBuf],
    vault: Option<&Path>,
) -> Result<Workspace, ApiError> {
    let roots = allowed_roots(std::env::var_os(ALLOWED_ROOTS_ENV), project_paths, vault);
    check_path_allowed(raw, &roots).map(|path| Workspace {
        kind: WorkspaceKind::Path,
        path,
        branch: None,
        name: None,
    })
}

/// Resolve a workspace request. Meant to run before the admission mutex is held.
pub fn resolve_workspace(
    runner: &dyn ProcessRunner,
    request: &WorkspaceRequest,
    project: &ProjectRef,
    project_paths: &[PathBuf],
    vault: Option<&Path>,
) -> Result<Workspace, ApiError> {
    match request.kind {
        WorkspaceKind::Project => Ok(Workspace {
            kind: WorkspaceKind::Project,
            path: project.path.clone(),
            branch: None,
            name: None,
        }),
        WorkspaceKind::Ticket => {
            let key = request
                .ticket
                .as_deref()
                .ok_or_else(|| failure(ApiStatus::BadRequest, MISSING_TICKET_MESSAGE))?;
            resolve_ticket(runner, key, project, vault)
        }
        WorkspaceKind::Scratch => {
            let name = request
                .name
                .as_deref()
                .ok_or_else(|| failure(ApiStatus::BadRequest, MISSING_NAME_MESSAGE))?;
            resolve_scratch(runner, name, project, vault)
        }
        WorkspaceKind::Path => {
            let raw = request
                .path
                .as_deref()
                .ok_or_else(|| failure(ApiStatus::BadRequest, MISSING_PATH_MESSAGE))?;
            resolve_path(raw, project_paths, vault)
        }
        WorkspaceKind::PrWorktree => Err(failure(ApiStatus::BadRequest, PR_WORKTREE_MESSAGE)),
    }
}

#[cfg(test)]
#[path = "../tests/unit/workspace_resolver_tests.rs"]
mod tests;
