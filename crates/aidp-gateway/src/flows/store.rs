//! Persistence under `Pipelines/<repo>/<KEY>/flow/` plus the `_flow.md`
//! note; the run index lives in the state dir.

pub use crate::models::Runtime;
pub use crate::models::{InboxItem, IndexEntry, JournalLine, WaitRecord};
use crate::store::{append_jsonl, read_json, read_jsonl, write_json};
use crate::sync::lock;
use aidp_flows::{FlowState, FlowStatus, StepStatus, FLOW_LABEL};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const PIPELINES_DIR: &str = "Pipelines";
const FLOW_DIR: &str = "flow";
const STATE_FILE: &str = "state.json";
const JOURNAL_FILE: &str = "journal.jsonl";
const RUNTIME_FILE: &str = "runtime.json";
const REPORTS_DIR: &str = "reports";
const INPUTS_DIR: &str = "inputs";
const NOTE_FILE: &str = "_flow.md";
const TEMP_EXTENSION: &str = "tmp";
const INDEX_DIR: &str = "flows";
const INDEX_FILE: &str = "index.json";
const REPORT_EXTENSION: &str = "md";
const TICKET_SERVICE: &str = "service";
const NOTE_TAG: &str = "aidp/flow";
const FRONTMATTER_FENCE: &str = "---\n";
const STEPS_TABLE_HEADER: &str = "| Step | Phase | Status | Reason | Iteration |\n";
const STEPS_TABLE_RULE: &str = "|---|---|---|---|---|\n";
const NO_VALUE: &str = "-";
const TABLE_ROW_SEPARATOR: &str = " | ";

fn ticket_service(state: &FlowState) -> Option<String> {
    state
        .ticket
        .get(TICKET_SERVICE)
        .and_then(Value::as_str)
        .or_else(|| super::ticket_snapshot::project_name(&state.ticket))
        .map(str::to_string)
}

/// Serialises every flow file access of the process: on Windows a rewrite
/// removes the target before renaming, and a reader must not see that gap.
static IO_GUARD: Mutex<()> = Mutex::new(());

/// File layout and atomic reads and writes of flow runs.
pub struct FlowStore {
    vault_root: PathBuf,
    state_dir: PathBuf,
    frozen: AtomicBool,
}

impl FlowStore {
    pub fn new(vault_root: PathBuf, state_dir: PathBuf) -> Self {
        FlowStore {
            vault_root,
            state_dir,
            frozen: AtomicBool::new(false),
        }
    }

    /// Stops persistence after the last in-progress file operation has completed.
    pub fn freeze(&self) {
        let _guard = lock(&IO_GUARD);
        self.frozen.store(true, Ordering::SeqCst);
    }

    fn require_writable(&self) -> anyhow::Result<()> {
        if self.frozen.load(Ordering::SeqCst) {
            return Err(crate::session_error::SessionError::ShuttingDown.into());
        }
        Ok(())
    }

    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    /// `<vault>/Pipelines/<repo>/<KEY>`.
    pub fn ticket_dir(&self, repo: &str, key: &str) -> PathBuf {
        self.vault_root.join(PIPELINES_DIR).join(repo).join(key)
    }

    /// `<vault>/Pipelines/<repo>/<KEY>/flow`.
    pub fn flow_dir(&self, repo: &str, key: &str) -> PathBuf {
        self.ticket_dir(repo, key).join(FLOW_DIR)
    }

    pub fn inputs_dir(&self, repo: &str, key: &str) -> PathBuf {
        self.flow_dir(repo, key).join(INPUTS_DIR)
    }

    fn index_file(&self) -> PathBuf {
        self.state_dir.join(INDEX_DIR).join(INDEX_FILE)
    }

    pub fn load_state(&self, repo: &str, key: &str) -> Option<FlowState> {
        let _guard = lock(&IO_GUARD);
        self.load_state_unlocked(repo, key)
    }

    fn load_state_unlocked(&self, repo: &str, key: &str) -> Option<FlowState> {
        read_json::<Option<FlowState>>(&self.flow_dir(repo, key).join(STATE_FILE), None)
    }

    /// Writes `state.json` atomically, refreshes the index entry and the generated note.
    pub fn save_state(&self, repo: &str, state: &FlowState, now: &str) -> anyhow::Result<()> {
        let _guard = lock(&IO_GUARD);
        self.require_writable()?;
        write_json(&self.flow_dir(repo, &state.key).join(STATE_FILE), state)?;
        self.upsert_index(repo, state, now)?;
        self.write_note(repo, state);
        Ok(())
    }

    pub fn append_journal(&self, repo: &str, key: &str, line: &JournalLine) {
        let _guard = lock(&IO_GUARD);
        if self.frozen.load(Ordering::SeqCst) {
            return;
        }
        append_jsonl(&self.flow_dir(repo, key).join(JOURNAL_FILE), line);
    }

    pub fn read_journal(&self, repo: &str, key: &str) -> Vec<JournalLine> {
        let _guard = lock(&IO_GUARD);
        read_jsonl(&self.flow_dir(repo, key).join(JOURNAL_FILE))
    }

    pub fn load_runtime(&self, repo: &str, key: &str) -> Runtime {
        let _guard = lock(&IO_GUARD);
        read_json(
            &self.flow_dir(repo, key).join(RUNTIME_FILE),
            Runtime::default(),
        )
    }

    pub fn save_runtime(&self, repo: &str, key: &str, runtime: &Runtime) -> anyhow::Result<()> {
        let _guard = lock(&IO_GUARD);
        self.require_writable()?;
        write_json(&self.flow_dir(repo, key).join(RUNTIME_FILE), runtime)
    }

    /// Stores a test report as `reports/<n>.md` and returns its path.
    pub fn write_report(&self, repo: &str, key: &str, text: &str) -> anyhow::Result<PathBuf> {
        let _guard = lock(&IO_GUARD);
        self.require_writable()?;
        let dir = self.flow_dir(repo, key).join(REPORTS_DIR);
        std::fs::create_dir_all(&dir)?;
        let count = std::fs::read_dir(&dir)?.count();
        let path = dir.join(format!("{}.{REPORT_EXTENSION}", count + 1));
        std::fs::write(&path, text)?;
        Ok(path)
    }

    /// Every run known to the daemon. A missing index is rebuilt from the vault.
    pub fn index(&self) -> BTreeMap<String, IndexEntry> {
        let _guard = lock(&IO_GUARD);
        self.read_index()
    }

    pub fn index_get(&self, key: &str) -> Option<IndexEntry> {
        self.index().remove(key)
    }

    fn read_index(&self) -> BTreeMap<String, IndexEntry> {
        let file = self.index_file();
        if file.exists() {
            return read_json(&file, BTreeMap::new());
        }
        self.scan_vault()
    }

    fn upsert_index(&self, repo: &str, state: &FlowState, now: &str) -> anyhow::Result<()> {
        let mut index = self.read_index();
        let service = ticket_service(state);
        index.insert(
            state.key.clone(),
            IndexEntry {
                key: state.key.clone(),
                repo: repo.to_string(),
                service,
                flow: state.def.id.clone(),
                status: state.status,
                current: state.current.clone(),
                updated_at: now.to_string(),
            },
        );
        write_json(&self.index_file(), &index)
    }

    /// Rebuilds the index from every `Pipelines/<repo>/<KEY>/flow/state.json`.
    fn scan_vault(&self) -> BTreeMap<String, IndexEntry> {
        let mut found = BTreeMap::new();
        let pipelines = self.vault_root.join(PIPELINES_DIR);
        for repo_entry in read_dir_paths(&pipelines) {
            let repo = file_name_of(&repo_entry);
            for ticket_entry in read_dir_paths(&repo_entry) {
                let key = file_name_of(&ticket_entry);
                if let Some(state) = self.load_state_unlocked(&repo, &key) {
                    let service = ticket_service(&state);
                    found.insert(
                        key.clone(),
                        IndexEntry {
                            key,
                            repo: repo.clone(),
                            service,
                            flow: state.def.id.clone(),
                            status: state.status,
                            current: state.current.clone(),
                            updated_at: state.started_at.clone().unwrap_or_default(),
                        },
                    );
                }
            }
        }
        found
    }

    fn write_note(&self, repo: &str, state: &FlowState) {
        let path = self.ticket_dir(repo, &state.key).join(NOTE_FILE);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = write_text(&path, &render_note(state));
    }
}

/// Writes a text file through a sibling temp file and a rename.
fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    let temp = path.with_extension(TEMP_EXTENSION);
    std::fs::write(&temp, text)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&temp, path)
}

fn read_dir_paths(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn or_dash(value: Option<&str>) -> &str {
    value.filter(|text| !text.is_empty()).unwrap_or(NO_VALUE)
}

/// The generated Obsidian note of a run: frontmatter and a step table.
pub fn render_note(state: &FlowState) -> String {
    let mut text = String::new();
    text.push_str(FRONTMATTER_FENCE);
    text.push_str(&format!("aidp: {FLOW_LABEL}\n"));
    text.push_str(&format!("key: {}\n", state.key));
    text.push_str(&format!("flow: {}\n", state.def.id));
    text.push_str(&format!("version: {}\n", state.def.version));
    text.push_str(&format!("status: {}\n", state.status.as_wire()));
    text.push_str(&format!("current: {}\n", or_dash(state.current.as_deref())));
    text.push_str(&format!("tags: [{NOTE_TAG}]\n"));
    text.push_str(FRONTMATTER_FENCE);
    text.push('\n');
    text.push_str(&format!("# Flow {}\n\n", state.key));
    text.push_str(STEPS_TABLE_HEADER);
    text.push_str(STEPS_TABLE_RULE);
    for (rid, step) in &state.steps {
        let row = [
            rid.as_str(),
            or_dash(step.phase.as_deref()),
            step.status.as_wire(),
            or_dash(step.reason.as_deref()),
            &step.iteration.to_string(),
        ]
        .join(TABLE_ROW_SEPARATOR);
        text.push_str(&format!("| {row} |\n"));
    }
    text
}

/// True while the run still owns its ticket: anything but done or aborted.
/// A failed or escalated run waits for a retry, a skip or an abort.
pub fn is_active(status: FlowStatus) -> bool {
    !matches!(status, FlowStatus::Done | FlowStatus::Aborted)
}

/// True when the step is parked on something other than the executor.
pub fn is_waiting(status: StepStatus) -> bool {
    matches!(
        status,
        StepStatus::WaitingHuman | StepStatus::WaitingExternal
    )
}
