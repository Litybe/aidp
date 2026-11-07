//! Local per-machine marks on pull requests, persisted in `pr-marks.json`.

use crate::store::write_json;
use crate::sync::lock;
use aidp_integrations::models::pr_mark::PrMark;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MARKS_FILE: &str = "pr-marks.json";

/// Marks keyed by `owner/repo#number`; writes are serialized and atomic.
pub struct PrMarks {
    file: PathBuf,
    writer: Mutex<()>,
}

pub fn read_marks(file: &Path) -> BTreeMap<String, PrMark> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(error = %error, file = %file.display(), "corrupt pr marks file ignored");
        BTreeMap::new()
    })
}

impl PrMarks {
    pub fn new(state_dir: &Path) -> Self {
        PrMarks {
            file: state_dir.join(MARKS_FILE),
            writer: Mutex::new(()),
        }
    }

    /// Every stored mark; a missing or corrupt file is an empty map.
    pub fn all(&self) -> BTreeMap<String, PrMark> {
        read_marks(&self.file)
    }

    /// Applies `change` to the mark of `key`, creating it when absent, and saves the file.
    pub fn update(&self, key: &str, change: impl FnOnce(&mut PrMark)) -> anyhow::Result<PrMark> {
        let _guard = lock(&self.writer);
        let mut marks = read_marks(&self.file);
        let mark = marks.entry(key.to_string()).or_default();
        change(mark);
        let updated = mark.clone();
        write_json(&self.file, &marks)?;
        Ok(updated)
    }
}
