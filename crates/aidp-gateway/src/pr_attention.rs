//! Fingerprint store of pull request attention events, persisted in `pr-attention.json`.

use crate::models::attention_read_request::AttentionReadRequest;
use crate::models::attention_signal::AttentionSignal;
use crate::models::pr_attention_file::PrAttentionFile;
use crate::models::pr_attention_record::PrAttentionRecord;
use crate::store::write_json;
use crate::sync::lock;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const ATTENTION_FILE: &str = "pr-attention.json";

/// Records keyed by fingerprint; writes are serialized and atomic.
pub struct PrAttention {
    file: PathBuf,
    writer: Mutex<()>,
}

fn read_file(file: &Path) -> PrAttentionFile {
    let Ok(text) = std::fs::read_to_string(file) else {
        return PrAttentionFile::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(error = %error, file = %file.display(), "corrupt pr attention file ignored");
        PrAttentionFile::default()
    })
}

fn save(file: &Path, stored: &PrAttentionFile) {
    if let Err(error) = write_json(file, stored) {
        tracing::warn!(error = %error, "pr attention write failed");
    }
}

impl PrAttention {
    pub fn new(state_dir: &Path) -> Self {
        PrAttention {
            file: state_dir.join(ATTENTION_FILE),
            writer: Mutex::new(()),
        }
    }

    /// Stores signals never seen before and returns only those new records.
    pub fn upsert_new(
        &self,
        signals: &[AttentionSignal],
        repo: &str,
        number: u64,
        head_sha: &str,
        now: &str,
    ) -> Vec<PrAttentionRecord> {
        let _guard = lock(&self.writer);
        let mut stored = read_file(&self.file);
        let mut created = Vec::new();
        for signal in signals {
            if stored.records.contains_key(&signal.fingerprint) {
                continue;
            }
            let record = PrAttentionRecord {
                fingerprint: signal.fingerprint.clone(),
                repo: repo.to_string(),
                number,
                signal: signal.kind,
                head_sha: head_sha.to_string(),
                detail: signal.detail.clone(),
                first_seen: now.to_string(),
                notified: signal.notify,
                read: false,
            };
            stored
                .records
                .insert(record.fingerprint.clone(), record.clone());
            created.push(record);
        }
        if !created.is_empty() {
            save(&self.file, &stored);
        }
        created
    }

    /// Records newest first, optionally only the unread ones.
    pub fn list(&self, unread_only: bool) -> Vec<PrAttentionRecord> {
        let mut records: Vec<PrAttentionRecord> = read_file(&self.file)
            .records
            .into_values()
            .filter(|record| !unread_only || !record.read)
            .collect();
        records.sort_by(|left, right| right.first_seen.cmp(&left.first_seen));
        records
    }

    /// Marks records read and returns how many changed.
    pub fn mark_read(&self, request: &AttentionReadRequest) -> usize {
        let _guard = lock(&self.writer);
        let mut stored = read_file(&self.file);
        let everything = request.all.unwrap_or(false);
        let wanted = request.fingerprints.as_deref().unwrap_or(&[]);
        let mut changed = 0;
        for record in stored.records.values_mut() {
            let selected = everything || wanted.contains(&record.fingerprint);
            if selected && !record.read {
                record.read = true;
                changed += 1;
            }
        }
        if changed > 0 {
            save(&self.file, &stored);
        }
        changed
    }
}

#[cfg(test)]
#[path = "../tests/unit/pr_attention_tests.rs"]
mod pr_attention_tests;

#[cfg(test)]
#[path = "../tests/unit/pr_attention_routes_tests.rs"]
mod pr_attention_routes_tests;
