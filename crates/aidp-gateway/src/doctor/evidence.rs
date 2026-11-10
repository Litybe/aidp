//! The evidence bundle of an incident: copies of what broke, gathered by the daemon so the
//! investigation reads files instead of reaching into the daemon.

use crate::models::constants::doctor::{
    DAEMON_ERR_LOG, EVIDENCE_DIR, EVIDENCE_INDEX_FILE, LOG_LEVELS, LOG_TAIL_BYTES, LOG_TAIL_LINES,
};
use crate::models::native_run_view::NativeRunView;
use crate::models::DoctorCandidate;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const FLOW_FILES: [&str; 3] = ["state.json", "journal.jsonl", "runtime.json"];
const RUN_FILES: [&str; 2] = ["run-state.json", "native-run.json"];
const STAGE_FILE_EXTENSIONS: [&str; 2] = ["json", "md"];
const STAGE_FILE_MAX_BYTES: u64 = 64 * 1024;
const FLOW_PREFIX: &str = "flow-";
const STAGES_DIR: &str = "stages";
const LOG_FILE: &str = "daemon.err.log.tail";
const PR_FILE: &str = "pr-status.json";
const UNITY_FILE: &str = "unity-status.json";
const FILES_KEY: &str = "files";
const INCIDENT_KEY: &str = "incident";

/// What the bundle is built from.
pub struct EvidenceSource<'a> {
    pub candidate: &'a DoctorCandidate,
    /// `Pipelines/<repo>/<KEY>/flow/` of the flow, when the incident has one.
    pub flow_dir: Option<PathBuf>,
    pub run: Option<NativeRunView>,
    pub state_dir: &'a Path,
    pub pr_status: Option<Value>,
    pub unity_status: Option<Value>,
    /// The evidence the daemon computed about bugs that do not converge: file name and content.
    pub convergence: Vec<(&'static str, Value)>,
}

/// Writes the bundle under `bundle/evidence/` and returns the list of files written.
pub fn write_bundle(bundle: &Path, source: &EvidenceSource<'_>) -> anyhow::Result<Vec<String>> {
    let evidence = bundle.join(EVIDENCE_DIR);
    std::fs::create_dir_all(&evidence)?;
    let mut files = Vec::new();
    if let Some(flow_dir) = &source.flow_dir {
        for name in FLOW_FILES {
            copy_into(
                &flow_dir.join(name),
                &evidence.join(format!("{FLOW_PREFIX}{name}")),
                &mut files,
            );
        }
    }
    if let Some(run) = &source.run {
        for name in RUN_FILES {
            copy_into(&run.run_dir.join(name), &evidence.join(name), &mut files);
        }
        copy_stage_files(&run.run_dir, &evidence.join(STAGES_DIR), &mut files);
    }
    let needles = needles(source.candidate);
    let log = filtered_log_tail(&source.state_dir.join(DAEMON_ERR_LOG), &needles);
    write_text(&evidence.join(LOG_FILE), &log, &mut files);
    for (name, value) in [
        (PR_FILE, &source.pr_status),
        (UNITY_FILE, &source.unity_status),
    ] {
        let text = serde_json::to_string_pretty(value.as_ref().unwrap_or(&Value::Null))?;
        write_text(&evidence.join(name), &text, &mut files);
    }
    for (name, value) in &source.convergence {
        let text = serde_json::to_string_pretty(value)?;
        write_text(&evidence.join(name), &text, &mut files);
    }
    let index = json!({ INCIDENT_KEY: source.candidate, FILES_KEY: files });
    write_text(
        &evidence.join(EVIDENCE_INDEX_FILE),
        &serde_json::to_string_pretty(&index)?,
        &mut files,
    );
    Ok(files)
}

fn needles(candidate: &DoctorCandidate) -> Vec<String> {
    [
        Some(&candidate.key),
        Some(&candidate.ticket),
        candidate.run_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect()
}

fn copy_into(from: &Path, to: &Path, files: &mut Vec<String>) {
    if std::fs::copy(from, to).is_ok() {
        files.push(to.to_string_lossy().to_string());
    }
}

fn write_text(to: &Path, text: &str, files: &mut Vec<String>) {
    if std::fs::write(to, text).is_ok() {
        files.push(to.to_string_lossy().to_string());
    }
}

/// The small `.json` and `.md` files of each stage directory of the run: replies and results.
fn copy_stage_files(run_dir: &Path, to: &Path, files: &mut Vec<String>) {
    let Ok(stages) = std::fs::read_dir(run_dir) else {
        return;
    };
    for stage in stages
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
    {
        let Some(name) = stage.file_name() else {
            continue;
        };
        let target = to.join(name);
        let Ok(entries) = std::fs::read_dir(&stage) else {
            continue;
        };
        for file in entries.flatten() {
            let path = file.path();
            let small = file
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() <= STAGE_FILE_MAX_BYTES);
            let wanted = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| STAGE_FILE_EXTENSIONS.contains(&extension));
            if !(small && wanted) || std::fs::create_dir_all(&target).is_err() {
                continue;
            }
            if let Some(file_name) = path.file_name() {
                copy_into(&path, &target.join(file_name), files);
            }
        }
    }
}

/// The last lines of the daemon's error log that name the incident or carry a warning or an error.
pub fn filtered_log_tail(log: &Path, needles: &[String]) -> String {
    let Ok(mut file) = std::fs::File::open(log) else {
        return String::new();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = length.saturating_sub(LOG_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let kept: Vec<&str> = text
        .lines()
        .skip(usize::from(start > 0))
        .filter(|line| {
            needles.iter().any(|needle| line.contains(needle.as_str()))
                || LOG_LEVELS.iter().any(|level| line.contains(level))
        })
        .collect();
    let first = kept.len().saturating_sub(LOG_TAIL_LINES);
    kept[first..].join("\n")
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_evidence_tests.rs"]
mod tests;
