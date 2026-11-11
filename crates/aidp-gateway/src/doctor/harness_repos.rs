//! Which repositories a harness patch may target, and where their local clones are.
//! A clone is trusted only when its `origin` remote names the repository.

use crate::doctor::ports::{DoctorSettings, HarnessRepos};
use crate::models::constants::doctor::{
    CLONE_REMOTE, ENV_SOURCE_ROOT, GITHUB_HOST, HARNESS_REPOS, HARNESS_REPO_AIDP,
    HARNESS_REPO_SKILL,
};
use aidp_tickets::config::DoctorConfig;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// The allowed repository `name` is, with the spelling of the allowlist (the built-in ones, then
/// `doctor.harness_repos`); the match ignores case.
pub fn canonical_repo(name: &str, settings: &DoctorConfig) -> Option<String> {
    HARNESS_REPOS
        .iter()
        .copied()
        .chain(settings.harness_repos.keys().map(String::as_str))
        .find(|known| known.eq_ignore_ascii_case(name.trim()))
        .map(str::to_string)
}

/// `owner/name` of a GitHub remote URL (https or ssh form, optional `.git`), lower-cased.
pub fn remote_repo(url: &str) -> Option<String> {
    let url = url.trim();
    let start = url.to_ascii_lowercase().find(GITHUB_HOST)? + GITHUB_HOST.len();
    let rest = url[start..]
        .trim_start_matches([':', '/'])
        .trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}").to_ascii_lowercase())
}

/// The clone when it is a git repository whose `origin` names `repo`, else why not.
fn verified(repo: &str, clone: &Path) -> Result<PathBuf, String> {
    if !clone.join(".git").exists() {
        return Err(format!("{} is not a git repository", clone.display()));
    }
    let output = Command::new("git")
        .args(["remote", "get-url", CLONE_REMOTE])
        .current_dir(clone)
        .output()
        .map_err(|error| format!("git cannot run in {}: {error}", clone.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} has no `{CLONE_REMOTE}` remote",
            clone.display()
        ));
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match remote_repo(&url) {
        Some(found) if found.eq_ignore_ascii_case(repo) => Ok(clone.to_path_buf()),
        _ => Err(format!(
            "`{CLONE_REMOTE}` of {} is `{url}`, not `{repo}`",
            clone.display()
        )),
    }
}

/// The unverified clone of `repo`: the config entry, else the environment's.
fn candidate(
    repo: &str,
    settings: &DoctorConfig,
    source: &Option<PathBuf>,
    skill: &Option<PathBuf>,
) -> Result<PathBuf, String> {
    let configured = settings
        .harness_repos
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(repo))
        .map(|(_, path)| PathBuf::from(path));
    if let Some(path) = configured {
        return if path.exists() {
            Ok(path)
        } else {
            Err(format!("configured path {} does not exist", path.display()))
        };
    }
    if repo.eq_ignore_ascii_case(HARNESS_REPO_AIDP) {
        source
            .clone()
            .ok_or_else(|| format!("{ENV_SOURCE_ROOT} is not set"))
    } else if repo.eq_ignore_ascii_case(HARNESS_REPO_SKILL) {
        skill
            .clone()
            .ok_or_else(|| "the skill root is unknown".to_string())
    } else {
        Err("no `doctor.harness_repos` entry".to_string())
    }
}

/// Resolves a repository to its verified local clone. `settings` is read at each call, so a
/// config edit applies without a restart.
pub fn resolver(
    settings: DoctorSettings,
    source: Option<PathBuf>,
    skill: Option<PathBuf>,
) -> HarnessRepos {
    Arc::new(move |repo: &str| {
        let config = settings();
        match candidate(repo, &config, &source, &skill).and_then(|clone| verified(repo, &clone)) {
            Ok(clone) => Some(clone),
            Err(reason) => {
                tracing::warn!(%repo, %reason, "no usable local clone of harness repository");
                None
            }
        }
    })
}

#[cfg(test)]
#[path = "../../tests/unit/harness_repos_tests.rs"]
mod tests;
