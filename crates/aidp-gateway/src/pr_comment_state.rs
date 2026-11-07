//! Per-PR comment state kept on this machine: ignored points and ids already handed to a run.

use crate::models::enums::PointState;
use crate::models::pr_comment_state_file::PrCommentStateFile;
use crate::store::write_json;
use crate::sync::lock;
use aidp_core::safe_name;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const STATE_SUBDIR: &str = "prs";

const FILE_SEPARATOR: &str = "__";
const FILE_EXTENSION: &str = "json";
const REPO_SEPARATOR: char = '/';
const OWNER_FIELD: &str = "repo owner";
const NAME_FIELD: &str = "repo name";

/// State file of one PR; owner and name must be safe path components.
pub fn state_file(state_dir: &Path, repo: &str, number: u64) -> anyhow::Result<PathBuf> {
    let (owner, name) = repo
        .split_once(REPO_SEPARATOR)
        .ok_or_else(|| anyhow::anyhow!("repo `{repo}` must be owner/name"))?;
    safe_name::check(OWNER_FIELD, owner)?;
    safe_name::check(NAME_FIELD, name)?;
    let file = format!("{owner}{FILE_SEPARATOR}{name}{FILE_SEPARATOR}{number}.{FILE_EXTENSION}");
    Ok(state_dir.join(STATE_SUBDIR).join(file))
}

/// Read-only load: a missing, unsafe or corrupt file is an empty state.
pub fn load(state_dir: &Path, repo: &str, number: u64) -> PrCommentStateFile {
    let Ok(file) = state_file(state_dir, repo, number) else {
        return PrCommentStateFile::default();
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        return PrCommentStateFile::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(error = %error, file = %file.display(), "corrupt pr comment state ignored");
        PrCommentStateFile::default()
    })
}

/// Comment state of every PR; writes are serialized and atomic.
pub struct PrCommentState {
    state_dir: PathBuf,
    writer: Mutex<()>,
}

impl PrCommentState {
    pub fn new(state_dir: &Path) -> Self {
        PrCommentState {
            state_dir: state_dir.to_path_buf(),
            writer: Mutex::new(()),
        }
    }

    pub fn read(&self, repo: &str, number: u64) -> anyhow::Result<PrCommentStateFile> {
        state_file(&self.state_dir, repo, number)?;
        Ok(load(&self.state_dir, repo, number))
    }

    /// Marks a point ignored, or clears it with `none`.
    pub fn set_point(
        &self,
        repo: &str,
        number: u64,
        point_id: &str,
        state: PointState,
    ) -> anyhow::Result<PrCommentStateFile> {
        self.update(repo, number, |file| {
            if state == PointState::None {
                file.points.remove(point_id);
            } else {
                file.points.insert(point_id.to_string(), state);
            }
        })
    }

    /// Records ids sent to a run, without duplicates.
    pub fn add_handed(
        &self,
        repo: &str,
        number: u64,
        ids: &[String],
    ) -> anyhow::Result<PrCommentStateFile> {
        self.update(repo, number, |file| {
            for id in ids {
                if !file.handed.contains(id) {
                    file.handed.push(id.clone());
                }
            }
        })
    }

    fn update(
        &self,
        repo: &str,
        number: u64,
        change: impl FnOnce(&mut PrCommentStateFile),
    ) -> anyhow::Result<PrCommentStateFile> {
        let path = state_file(&self.state_dir, repo, number)?;
        let _guard = lock(&self.writer);
        let mut file = load(&self.state_dir, repo, number);
        change(&mut file);
        write_json(&path, &file)?;
        Ok(file)
    }
}

#[cfg(test)]
#[path = "../tests/unit/pr_comment_state_tests.rs"]
mod pr_comment_state_tests;

#[cfg(test)]
#[path = "../tests/unit/pr_comments_routes_tests.rs"]
mod pr_comments_routes_tests;
