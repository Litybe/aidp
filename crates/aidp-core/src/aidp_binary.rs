//! Whether the first `aidp` found on `PATH` is the one the user should run.
//!
//! [`evaluate`] is pure: it judges plain facts. The IO helpers and [`probe`]
//! gather those facts from the filesystem and the environment.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Program name looked up on `PATH`.
pub const AIDP_PROGRAM_NAME: &str = "aidp";
/// Environment variable naming the skill root that holds the Rust workspace.
pub const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
/// Names the aidp sources, for a binary installed away from the workspace it was built in.
pub const SOURCES_ENV: &str = "AIDP_SRC";

pub const ISSUE_ABSENT: &str = "absent";
pub const ISSUE_TARGET_BUILD: &str = "target_build";
pub const ISSUE_MISMATCH: &str = "mismatch";
pub const ISSUE_OLDER_THAN_RUNNING: &str = "older_than_running";
pub const ISSUE_OLDER_THAN_SOURCES: &str = "older_than_sources";

pub const KEY_OK: &str = "ok";
pub const KEY_PATH: &str = "path";
pub const KEY_MODIFIED_AT: &str = "modifiedAt";
pub const KEY_RUNNING_PATH: &str = "runningPath";
pub const KEY_RUNNING_MODIFIED_AT: &str = "runningModifiedAt";
pub const KEY_ISSUES: &str = "issues";

const TARGET_DIR: &str = "target";
const DEBUG_DIR: &str = "debug";
const WORKTREES_DIR: &str = "worktrees";
const CRATES_DIR: &str = "crates";
const SRC_DIR: &str = "src";
/// `crates/aidp-core` of the workspace this binary was compiled from.
const BUILD_CRATE_DIR: &str = env!("CARGO_MANIFEST_DIR");
/// Levels from `crates/aidp-core` up to its workspace.
const WORKSPACE_LEVELS_UP: usize = 2;
const CARGO_TOML: &str = "Cargo.toml";
const CARGO_LOCK: &str = "Cargo.lock";
const RUST_SOURCE_EXT: &str = "rs";
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
const VERBATIM_PREFIX: &str = r"\\?\";
const UNC_PREFIX: &str = r"\\";
const WINDOWS_SEPARATOR: char = '\\';
const UNIX_SEPARATOR: char = '/';

/// A reason the `aidp` on `PATH` is not the one to trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AidpBinaryIssue {
    Absent,
    TargetBuild,
    Mismatch,
    OlderThanRunning,
    OlderThanSources,
}

impl AidpBinaryIssue {
    /// The stable wire value of this issue.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Absent => ISSUE_ABSENT,
            Self::TargetBuild => ISSUE_TARGET_BUILD,
            Self::Mismatch => ISSUE_MISMATCH,
            Self::OlderThanRunning => ISSUE_OLDER_THAN_RUNNING,
            Self::OlderThanSources => ISSUE_OLDER_THAN_SOURCES,
        }
    }
}

/// Everything [`evaluate`] needs, gathered beforehand. No IO happens on it.
#[derive(Debug, Clone, Default)]
pub struct AidpBinaryFacts {
    pub resolved: Option<PathBuf>,
    pub running: Option<PathBuf>,
    pub resolved_modified: Option<SystemTime>,
    pub running_modified: Option<SystemTime>,
    pub sources_modified: Option<SystemTime>,
}

/// The verdict on the `aidp` found on `PATH`.
#[derive(Debug, Clone)]
pub struct AidpBinaryReport {
    pub resolved: Option<PathBuf>,
    pub running: Option<PathBuf>,
    pub resolved_modified: Option<SystemTime>,
    pub running_modified: Option<SystemTime>,
    pub issues: Vec<AidpBinaryIssue>,
}

impl AidpBinaryReport {
    /// True when no issue was found.
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// The camelCase JSON shape served to the doctor and the health endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(KEY_OK.into(), self.ok().into());
        object.insert(KEY_PATH.into(), path_json(&self.resolved));
        object.insert(KEY_MODIFIED_AT.into(), time_json(self.resolved_modified));
        object.insert(KEY_RUNNING_PATH.into(), path_json(&self.running));
        object.insert(
            KEY_RUNNING_MODIFIED_AT.into(),
            time_json(self.running_modified),
        );
        let slugs: Vec<&str> = self.issues.iter().map(AidpBinaryIssue::slug).collect();
        object.insert(KEY_ISSUES.into(), slugs.into());
        serde_json::Value::Object(object)
    }
}

fn path_json(path: &Option<PathBuf>) -> serde_json::Value {
    match path {
        Some(p) => p.to_string_lossy().into_owned().into(),
        None => serde_json::Value::Null,
    }
}

fn time_json(time: Option<SystemTime>) -> serde_json::Value {
    match time {
        Some(t) => chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into(),
        None => serde_json::Value::Null,
    }
}

/// Judges `facts`. A missing binary short-circuits to [`AidpBinaryIssue::Absent`];
/// otherwise every applicable issue is reported. An unknown time never yields
/// an issue.
pub fn evaluate(facts: AidpBinaryFacts) -> AidpBinaryReport {
    let mut issues = Vec::new();
    if let Some(resolved) = &facts.resolved {
        let resolved_key = normalize(resolved);
        if is_target_build(&resolved_key) {
            issues.push(AidpBinaryIssue::TargetBuild);
        }
        let mismatch = facts
            .running
            .as_deref()
            .is_some_and(|running| normalize(running) != resolved_key);
        if mismatch {
            issues.push(AidpBinaryIssue::Mismatch);
            if is_older(facts.resolved_modified, facts.running_modified) {
                issues.push(AidpBinaryIssue::OlderThanRunning);
            }
        }
        if is_older(facts.resolved_modified, facts.sources_modified) {
            issues.push(AidpBinaryIssue::OlderThanSources);
        }
    } else {
        issues.push(AidpBinaryIssue::Absent);
    }
    AidpBinaryReport {
        resolved: facts.resolved,
        running: facts.running,
        resolved_modified: facts.resolved_modified,
        running_modified: facts.running_modified,
        issues,
    }
}

/// True when both times are known and `candidate` is strictly before `reference`.
fn is_older(candidate: Option<SystemTime>, reference: Option<SystemTime>) -> bool {
    matches!((candidate, reference), (Some(c), Some(r)) if c < r)
}

/// Strips a Windows verbatim prefix, unifies separators to `/` and lowercases
/// on Windows.
fn normalize(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let without_prefix = if let Some(rest) = raw.strip_prefix(VERBATIM_UNC_PREFIX) {
        format!("{UNC_PREFIX}{rest}")
    } else if let Some(rest) = raw.strip_prefix(VERBATIM_PREFIX) {
        rest.to_string()
    } else {
        raw.into_owned()
    };
    let unified = without_prefix.replace(WINDOWS_SEPARATOR, &UNIX_SEPARATOR.to_string());
    if cfg!(windows) {
        unified.to_lowercase()
    } else {
        unified
    }
}

/// True for a `target/debug` build, or any `target` directory below a
/// `worktrees` directory. The canonical `rust/target/release` install is not
/// a target build.
fn is_target_build(normalized: &str) -> bool {
    let components: Vec<&str> = normalized
        .split(UNIX_SEPARATOR)
        .filter(|c| !c.is_empty())
        .collect();
    let debug_build = components
        .windows(2)
        .any(|pair| pair[0] == TARGET_DIR && pair[1] == DEBUG_DIR);
    let worktree_build = components
        .iter()
        .position(|c| *c == WORKTREES_DIR)
        .is_some_and(|at| components[at + 1..].contains(&TARGET_DIR));
    debug_build || worktree_build
}

/// `std::fs::canonicalize`, or the input itself when that fails.
pub fn canonical_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The modification time of `path`, when readable.
pub fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The aidp workspace to compare sources against: `AIDP_SRC` when it names one, else the
/// workspace the running binary sits in, else the one it was compiled from, if still there.
pub fn locate_rust_dir(running: &Path) -> Option<PathBuf> {
    rust_dir_from(
        std::env::var_os(SOURCES_ENV).map(PathBuf::from),
        running,
        &build_workspace(),
    )
}

/// [`locate_rust_dir`] over explicit facts.
fn rust_dir_from(explicit: Option<PathBuf>, running: &Path, build: &Path) -> Option<PathBuf> {
    if let Some(dir) = explicit.filter(|dir| is_workspace(dir)) {
        return Some(dir);
    }
    rust_dir_from_ancestors(running).or_else(|| is_workspace(build).then(|| build.to_path_buf()))
}

/// The workspace this binary was compiled from.
fn build_workspace() -> PathBuf {
    Path::new(BUILD_CRATE_DIR)
        .ancestors()
        .nth(WORKSPACE_LEVELS_UP)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// True for a directory holding a `Cargo.toml` and a `crates` directory.
fn is_workspace(dir: &Path) -> bool {
    dir.join(CARGO_TOML).is_file() && dir.join(CRATES_DIR).is_dir()
}

/// The first ancestor of `running` holding a `Cargo.toml` and a `crates` directory.
fn rust_dir_from_ancestors(running: &Path) -> Option<PathBuf> {
    running
        .ancestors()
        .find(|dir| is_workspace(dir))
        .map(Path::to_path_buf)
}

/// The newest modification time among `Cargo.toml`, `Cargo.lock` and every
/// `*.rs` file under `crates/*/src/`. Unreadable entries are skipped.
pub fn newest_source_mtime(rust_dir: &Path) -> Option<SystemTime> {
    let mut newest = [CARGO_TOML, CARGO_LOCK]
        .iter()
        .filter_map(|name| modified(&rust_dir.join(name)))
        .max();
    let crate_dirs = std::fs::read_dir(rust_dir.join(CRATES_DIR))
        .into_iter()
        .flatten()
        .flatten();
    for crate_dir in crate_dirs {
        newest = newest.max(newest_rust_file_mtime(&crate_dir.path().join(SRC_DIR)));
    }
    newest
}

/// The newest `*.rs` modification time under `dir`, recursively, never
/// entering a `target` directory.
fn newest_rust_file_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest = None;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let found = if path.is_dir() {
            if path.file_name().is_some_and(|n| n == TARGET_DIR) {
                continue;
            }
            newest_rust_file_mtime(&path)
        } else if path.extension().is_some_and(|e| e == RUST_SOURCE_EXT) {
            modified(&path)
        } else {
            None
        };
        newest = newest.max(found);
    }
    newest
}

/// Gathers the facts from `PATH`, the running executable and the sources,
/// then calls [`evaluate`].
pub fn probe() -> AidpBinaryReport {
    let resolved =
        crate::program::resolve_program(AIDP_PROGRAM_NAME).map(|path| canonical_or_self(&path));
    let running = std::env::current_exe()
        .ok()
        .map(|path| canonical_or_self(&path));
    let sources_modified = running
        .as_deref()
        .and_then(locate_rust_dir)
        .and_then(|dir| newest_source_mtime(&dir));
    let facts = AidpBinaryFacts {
        resolved_modified: resolved.as_deref().and_then(modified),
        running_modified: running.as_deref().and_then(modified),
        resolved,
        running,
        sources_modified,
    };
    evaluate(facts)
}

#[cfg(test)]
#[path = "../tests/unit/aidp_binary_tests.rs"]
mod aidp_binary_tests;
