//! The holder of a pull request branch, kept as a state file in the vault.

use crate::models::constants::pr_workspace_holder::{
    HOLDER_FILE_NAME, HOLDER_TEMP_SUFFIX, INPUT_PR, INPUT_PR_NUMBER, INPUT_TARGET,
    PR_REFERENCE_PREFIX, PULL_URL_SEGMENT,
};
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
use crate::worktrees::pr_worktree_path;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::warn;

const PR_INPUT_NAMES: [&str; 3] = [INPUT_PR_NUMBER, INPUT_PR, INPUT_TARGET];

/// The pull request an input map names, read from `pr_number`, `pr` then `target`.
/// Each accepts a number, a numeric string, `#123` or a URL ending in `/pull/123`.
pub fn pr_number_of(inputs: &BTreeMap<String, Value>) -> Option<u64> {
    PR_INPUT_NAMES
        .iter()
        .filter_map(|name| inputs.get(*name))
        .find_map(pr_number_from_value)
}

fn pr_number_from_value(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => pr_number_from_text(text),
        _ => None,
    }
}

fn pr_number_from_text(text: &str) -> Option<u64> {
    let trimmed = text.trim().trim_end_matches('/');
    let digits = match trimmed.rsplit_once(PULL_URL_SEGMENT) {
        Some((_, tail)) => tail,
        None => trimmed.strip_prefix(PR_REFERENCE_PREFIX).unwrap_or(trimmed),
    };
    digits.parse().ok()
}

/// `<vault>/Pipelines/<repo>/PR-<number>/workspace-holder.json`.
pub fn holder_path(vault: &Path, repo_segment: &str, number: u64) -> PathBuf {
    let directory = pr_worktree_path(vault, repo_segment, number)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    directory.join(HOLDER_FILE_NAME)
}

/// Writes the holder file atomically: a temporary sibling renamed over the target.
pub fn write(
    vault: &Path,
    repo_segment: &str,
    number: u64,
    holder: &PrWorkspaceHolder,
) -> std::io::Result<()> {
    let target = holder_path(vault, repo_segment, number);
    if let Some(directory) = target.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = target.with_file_name(format!("{HOLDER_FILE_NAME}{HOLDER_TEMP_SUFFIX}"));
    std::fs::write(&temporary, serde_json::to_vec_pretty(holder)?)?;
    std::fs::rename(&temporary, &target)
}

/// The recorded holder, `None` when the file is absent or unreadable.
pub fn read(vault: &Path, repo_segment: &str, number: u64) -> Option<PrWorkspaceHolder> {
    let path = holder_path(vault, repo_segment, number);
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&text) {
        Ok(holder) => Some(holder),
        Err(error) => {
            warn!(path = %path.display(), %error, "unparseable workspace holder file");
            None
        }
    }
}

/// Removes the holder file; an absent file is not an error.
pub fn remove(vault: &Path, repo_segment: &str, number: u64) {
    let path = holder_path(vault, repo_segment, number);
    if let Err(error) = std::fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(path = %path.display(), %error, "workspace holder file not removed");
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/pr_workspace_holder_tests.rs"]
mod tests;
