//! Per-file "viewed" marks on pull requests, persisted in `pr-viewed.json`.

use crate::models::file_blob::FileBlob;
use crate::models::pr_viewed_file::PrViewedFile;
use crate::models::viewed_file_state::ViewedFileState;
use crate::pr_drafts::draft_key;
use crate::store::write_json;
use crate::sync::lock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const VIEWED_FILE: &str = "pr-viewed.json";

/// Viewed marks keyed by pull request then path, each holding the blob sha seen.
pub struct PrViewed {
    file: PathBuf,
    writer: Mutex<()>,
}

fn read_viewed(file: &Path) -> PrViewedFile {
    let Ok(text) = std::fs::read_to_string(file) else {
        return PrViewedFile::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(error = %error, file = %file.display(), "corrupt pr viewed file ignored");
        PrViewedFile::default()
    })
}

/// Paths of `now` that are absent from `at_review` or whose blob differs from it.
pub fn changed_since(at_review: &[FileBlob], now: &[FileBlob]) -> Vec<String> {
    let reviewed: BTreeMap<&str, &str> = at_review
        .iter()
        .map(|blob| (blob.path.as_str(), blob.sha.as_str()))
        .collect();
    now.iter()
        .filter(|blob| reviewed.get(blob.path.as_str()) != Some(&blob.sha.as_str()))
        .map(|blob| blob.path.clone())
        .collect()
}

impl PrViewed {
    pub fn new(state_dir: &Path) -> Self {
        PrViewed {
            file: state_dir.join(VIEWED_FILE),
            writer: Mutex::new(()),
        }
    }

    /// Records that `path` was viewed at blob `sha`.
    pub fn mark(&self, repo: &str, number: u64, path: &str, sha: &str) -> anyhow::Result<()> {
        self.change(repo, number, |paths| {
            paths.insert(path.to_string(), sha.to_string());
        })
    }

    /// Forgets the viewed mark of `path`.
    pub fn unmark(&self, repo: &str, number: u64, path: &str) -> anyhow::Result<()> {
        self.change(repo, number, |paths| {
            paths.remove(path);
        })
    }

    /// A file counts as viewed only while its current blob sha equals the stored one.
    pub fn viewed_state(
        &self,
        repo: &str,
        number: u64,
        files: &[FileBlob],
    ) -> Vec<ViewedFileState> {
        let stored = read_viewed(&self.file)
            .0
            .remove(&draft_key(repo, number))
            .unwrap_or_default();
        files
            .iter()
            .map(|blob| ViewedFileState {
                path: blob.path.clone(),
                viewed: stored.get(&blob.path) == Some(&blob.sha),
            })
            .collect()
    }

    /// Paths the viewer marked whose current blob differs from the one marked.
    pub fn since_last_review(&self, repo: &str, number: u64, now: &[FileBlob]) -> Vec<String> {
        let stored = read_viewed(&self.file)
            .0
            .remove(&draft_key(repo, number))
            .unwrap_or_default();
        let at_review: Vec<FileBlob> = stored
            .into_iter()
            .map(|(path, sha)| FileBlob { path, sha })
            .collect();
        let marked_now: Vec<FileBlob> = now
            .iter()
            .filter(|blob| at_review.iter().any(|seen| seen.path == blob.path))
            .cloned()
            .collect();
        changed_since(&at_review, &marked_now)
    }

    fn change(
        &self,
        repo: &str,
        number: u64,
        edit: impl FnOnce(&mut BTreeMap<String, String>),
    ) -> anyhow::Result<()> {
        let _guard = lock(&self.writer);
        let mut stored = read_viewed(&self.file);
        let key = draft_key(repo, number);
        let paths = stored.0.entry(key.clone()).or_default();
        edit(paths);
        if paths.is_empty() {
            stored.0.remove(&key);
        }
        write_json(&self.file, &stored)
    }
}

#[cfg(test)]
#[path = "tests/units/pr_viewed_tests.rs"]
mod pr_viewed_tests;
