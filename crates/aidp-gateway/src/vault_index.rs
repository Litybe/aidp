//! In-memory index of the pipeline runs under `<vault>/Pipelines/<repo>/<ticket>/<run>/`: for each
//! run its route, its stage token rows and its Pixellab usage and balance. It is built once, then
//! refreshed incrementally on a background thread: a run is read again only when its
//! `run-state.json` or its directory changed. The cost, usage and Pixellab routes read it instead
//! of walking the vault.

use crate::cost_routes::route_of_state;
use crate::models::constants::pixellab::{MAX_RUNS_PER_TICKET, PIPELINES_DIR};
use crate::models::constants::vault_index::{
    INDEX_REFRESH_TTL, MAX_SCAN_WORKERS, REFRESH_THREAD_NAME, WARM_THREAD_NAME,
};
use crate::pixellab_usage::{
    newest_of, summarize_run, ticket_usage_of, BalanceSnapshot, RunPixellabUsage,
    TicketPixellabUsage,
};
use crate::sync::lock;
use crate::ticket_cost::{stage_rows_of, StageRow};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

/// What decides that a run must be read again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RunSignature {
    state_modified: Option<SystemTime>,
    state_len: u64,
    dir_modified: Option<SystemTime>,
}

/// One pipeline run, as the routes need it.
#[derive(Clone, Debug)]
pub struct RunSummary {
    pub repo: String,
    pub run_id: String,
    pub route: Option<String>,
    pub stages: Vec<StageRow>,
    pub pixellab: RunPixellabUsage,
    pub balance: Option<BalanceSnapshot>,
    signature: RunSignature,
}

#[derive(Clone, Debug)]
struct TicketDir {
    key: String,
    runs: Vec<Arc<RunSummary>>,
}

/// The runs of the vault at one moment.
#[derive(Debug, Default)]
pub struct VaultSnapshot {
    dirs: HashMap<PathBuf, TicketDir>,
    tickets: HashMap<String, Vec<Arc<RunSummary>>>,
    balance: Option<BalanceSnapshot>,
}

impl VaultSnapshot {
    fn assemble(dirs: HashMap<PathBuf, TicketDir>) -> Self {
        let mut ordered: Vec<(&PathBuf, &TicketDir)> = dirs.iter().collect();
        ordered.sort_by(|a, b| a.0.cmp(b.0));
        let mut tickets: HashMap<String, Vec<Arc<RunSummary>>> = HashMap::new();
        let mut candidates = Vec::new();
        for (_, dir) in ordered {
            tickets
                .entry(dir.key.clone())
                .or_default()
                .extend(dir.runs.iter().cloned());
            candidates.extend(dir.runs.iter().filter_map(|run| run.balance.clone()));
        }
        for runs in tickets.values_mut() {
            runs.sort_by(|a, b| b.run_id.cmp(&a.run_id).then_with(|| a.repo.cmp(&b.repo)));
        }
        VaultSnapshot {
            balance: newest_of(candidates.into_iter()),
            tickets,
            dirs,
        }
    }

    /// The runs of `key` across the repos, newest first.
    pub fn runs_of(&self, key: &str) -> &[Arc<RunSummary>] {
        self.tickets.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The route of the newest run of `key` that holds one.
    pub fn route_of(&self, key: &str) -> Option<String> {
        self.runs_of(key).iter().find_map(|run| run.route.clone())
    }

    /// The route of run `run_id` of ticket `key`.
    pub fn route_of_run(&self, key: &str, run_id: &str) -> Option<String> {
        self.runs_of(key)
            .iter()
            .find(|run| run.run_id == run_id)
            .and_then(|run| run.route.clone())
    }

    /// The stage rows of the newest runs of `key`, [`MAX_RUNS_PER_TICKET`] per repo.
    pub fn stage_rows(&self, key: &str) -> Vec<StageRow> {
        let mut per_repo: BTreeMap<&str, usize> = BTreeMap::new();
        let mut rows = Vec::new();
        for run in self.runs_of(key) {
            let taken = per_repo.entry(run.repo.as_str()).or_default();
            if *taken >= MAX_RUNS_PER_TICKET {
                continue;
            }
            *taken += 1;
            rows.extend(run.stages.iter().cloned());
        }
        rows
    }

    /// The Pixellab usage of ticket `key`.
    pub fn pixellab_of(&self, key: &str) -> TicketPixellabUsage {
        ticket_usage_of(
            key,
            self.runs_of(key).iter().map(|run| run.pixellab.clone()),
        )
    }

    /// The newest balance any run recorded.
    pub fn latest_balance(&self) -> Option<&BalanceSnapshot> {
        self.balance.as_ref()
    }

    /// Runs indexed.
    pub fn run_count(&self) -> usize {
        self.tickets.values().map(Vec::len).sum()
    }
}

fn modified_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
}

fn signature_of(run_dir: &Path) -> RunSignature {
    let state = std::fs::metadata(run_dir.join(crate::runs::RUN_STATE_FILE)).ok();
    RunSignature {
        state_modified: state.as_ref().and_then(|meta| meta.modified().ok()),
        state_len: state.map_or(0, |meta| meta.len()),
        dir_modified: modified_of(run_dir),
    }
}

fn sub_directories(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()))
        })
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    found.sort();
    found
}

fn summarize(repo: &str, run_id: &str, dir: &Path, signature: RunSignature) -> RunSummary {
    let state = crate::runs::read_run_state(dir);
    let usage = summarize_run(run_id, dir);
    RunSummary {
        repo: repo.to_string(),
        run_id: run_id.to_string(),
        route: state.as_ref().and_then(route_of_state),
        stages: state.as_ref().map(stage_rows_of).unwrap_or_default(),
        pixellab: usage.usage,
        balance: usage.balance,
        signature,
    }
}

fn scan_ticket_dir(
    repo: &str,
    key: &str,
    path: &Path,
    previous: Option<&TicketDir>,
    force: bool,
) -> TicketDir {
    let runs = sub_directories(path)
        .into_iter()
        .map(|(run_id, run_dir)| {
            let signature = signature_of(&run_dir);
            let reusable = previous
                .filter(|_| !force)
                .and_then(|dir| dir.runs.iter().find(|run| run.run_id == run_id))
                .filter(|run| run.signature == signature);
            match reusable {
                Some(run) => Arc::clone(run),
                None => Arc::new(summarize(repo, &run_id, &run_dir, signature)),
            }
        })
        .collect();
    TicketDir {
        key: key.to_string(),
        runs,
    }
}

fn parallel_map<T: Send, R: Send>(items: Vec<T>, work: impl Fn(T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, MAX_SCAN_WORKERS)
        .min(items.len().max(1));
    let queue = Mutex::new(items.into_iter().enumerate());
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let next = lock(&queue).next();
                let Some((index, item)) = next else { break };
                let result = work(item);
                lock(&results).push((index, result));
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

fn scan(vault: &Path, previous: Option<&VaultSnapshot>) -> VaultSnapshot {
    let mut work = Vec::new();
    for (repo, repo_dir) in sub_directories(&vault.join(PIPELINES_DIR)) {
        for (key, key_dir) in sub_directories(&repo_dir) {
            work.push((repo.clone(), key, key_dir));
        }
    }
    let scanned = parallel_map(work, |(repo, key, path)| {
        let before = previous.and_then(|snapshot| snapshot.dirs.get(&path));
        let dir = scan_ticket_dir(&repo, &key, &path, before, false);
        (path, dir)
    });
    VaultSnapshot::assemble(scanned.into_iter().collect())
}

struct Slot {
    current: Mutex<Option<(Arc<VaultSnapshot>, Instant)>>,
    build: Mutex<()>,
    refreshing: AtomicBool,
}

type Slots = Mutex<HashMap<PathBuf, Arc<Slot>>>;

fn slot_of(vault: &Path) -> Arc<Slot> {
    static SLOTS: OnceLock<Slots> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
    Arc::clone(lock(slots).entry(vault.to_path_buf()).or_insert_with(|| {
        Arc::new(Slot {
            current: Mutex::new(None),
            build: Mutex::new(()),
            refreshing: AtomicBool::new(false),
        })
    }))
}

fn start_refresh(slot: &Arc<Slot>, vault: &Path) {
    if slot
        .refreshing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let background = Arc::clone(slot);
    let root = vault.to_path_buf();
    let started = std::thread::Builder::new()
        .name(REFRESH_THREAD_NAME.to_string())
        .spawn(move || {
            let before = lock(&background.current)
                .as_ref()
                .map(|(snapshot, _)| Arc::clone(snapshot));
            let next = Arc::new(scan(&root, before.as_deref()));
            *lock(&background.current) = Some((next, Instant::now()));
            background.refreshing.store(false, Ordering::SeqCst);
        })
        .is_ok();
    if !started {
        slot.refreshing.store(false, Ordering::SeqCst);
    }
}

/// The index of `vault`. The first read builds it (later readers wait for that one build); an
/// older one is answered at once while a background thread refreshes it.
pub fn snapshot(vault: &Path) -> Arc<VaultSnapshot> {
    if vault.as_os_str().is_empty() {
        return Arc::new(VaultSnapshot::default());
    }
    let slot = slot_of(vault);
    let known = lock(&slot.current).clone();
    if let Some((snapshot, built_at)) = known {
        if built_at.elapsed() >= INDEX_REFRESH_TTL {
            start_refresh(&slot, vault);
        }
        return snapshot;
    }
    let _building = lock(&slot.build);
    if let Some((snapshot, _)) = lock(&slot.current).clone() {
        return snapshot;
    }
    let built = Arc::new(scan(vault, None));
    *lock(&slot.current) = Some((Arc::clone(&built), Instant::now()));
    built
}

/// The index with the runs of ticket `key` read again now, for the callers that decide on them
/// (the ticket ceilings). The other tickets stay as indexed.
pub fn refresh_ticket(vault: &Path, key: &str) -> Arc<VaultSnapshot> {
    let current = snapshot(vault);
    if vault.as_os_str().is_empty() {
        return current;
    }
    let mut dirs: HashMap<PathBuf, TicketDir> = current
        .dirs
        .iter()
        .filter(|(_, dir)| dir.key != key)
        .map(|(path, dir)| (path.clone(), dir.clone()))
        .collect();
    for (repo, repo_dir) in sub_directories(&vault.join(PIPELINES_DIR)) {
        let path = repo_dir.join(key);
        if path.is_dir() {
            let dir = scan_ticket_dir(&repo, key, &path, current.dirs.get(&path), true);
            dirs.insert(path, dir);
        }
    }
    let next = Arc::new(VaultSnapshot::assemble(dirs));
    let slot = slot_of(vault);
    let mut stored = lock(&slot.current);
    let built_at = stored.as_ref().map_or_else(Instant::now, |(_, at)| *at);
    *stored = Some((Arc::clone(&next), built_at));
    next
}

/// Makes the index of `vault` refresh in the background now.
pub fn invalidate(vault: &Path) {
    if vault.as_os_str().is_empty() {
        return;
    }
    let slot = slot_of(vault);
    let known = lock(&slot.current).is_some();
    if known {
        start_refresh(&slot, vault);
    }
}

/// Drops the index of `vault`: the next read builds it again.
pub fn forget(vault: &Path) {
    if vault.as_os_str().is_empty() {
        return;
    }
    *lock(&slot_of(vault).current) = None;
}

/// Builds the index of `vault` on its own thread, so the first request finds it ready.
pub fn warm(vault: &Path) {
    if vault.as_os_str().is_empty() {
        return;
    }
    let root = vault.to_path_buf();
    let _ = std::thread::Builder::new()
        .name(WARM_THREAD_NAME.to_string())
        .spawn(move || {
            snapshot(&root);
        });
}

#[cfg(test)]
#[path = "../tests/unit/vault_index_tests.rs"]
mod tests;
