//! Local review drafts per pull request, persisted in `pr-drafts.json`.

use crate::models::pr_drafts_file::PrDraftsFile;
use crate::store::write_json;
use crate::sync::lock;
use aidp_integrations::models::review_draft::ReviewDraft;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const DRAFTS_FILE: &str = "pr-drafts.json";

/// Draft store; writes are serialized and atomic, so drafts survive a restart.
pub struct PrDrafts {
    file: PathBuf,
    writer: Mutex<()>,
}

/// Key of a pull request in the stores: `owner/repo#number`.
pub fn draft_key(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

fn read_drafts(file: &Path) -> PrDraftsFile {
    let Ok(text) = std::fs::read_to_string(file) else {
        return PrDraftsFile::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(error = %error, file = %file.display(), "corrupt pr drafts file ignored");
        PrDraftsFile::default()
    })
}

impl PrDrafts {
    pub fn new(state_dir: &Path) -> Self {
        PrDrafts {
            file: state_dir.join(DRAFTS_FILE),
            writer: Mutex::new(()),
        }
    }

    /// Drafts of one pull request; empty when none are stored.
    pub fn get(&self, repo: &str, number: u64) -> Vec<ReviewDraft> {
        read_drafts(&self.file)
            .0
            .remove(&draft_key(repo, number))
            .unwrap_or_default()
    }

    /// Replaces every draft of the pull request; an empty list removes the entry.
    pub fn put(&self, repo: &str, number: u64, drafts: Vec<ReviewDraft>) -> anyhow::Result<()> {
        let _guard = lock(&self.writer);
        let mut stored = read_drafts(&self.file);
        let key = draft_key(repo, number);
        if drafts.is_empty() {
            stored.0.remove(&key);
        } else {
            stored.0.insert(key, drafts);
        }
        write_json(&self.file, &stored)
    }

    /// Drops the drafts of the pull request; call it only after a successful submit.
    pub fn clear(&self, repo: &str, number: u64) -> anyhow::Result<()> {
        self.put(repo, number, Vec::new())
    }
}

#[cfg(test)]
#[path = "tests/units/pr_drafts_tests.rs"]
mod pr_drafts_tests;
