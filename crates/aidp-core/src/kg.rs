//! Knowledge graph (Graphify) configuration, run-state types, and the
//! in-process `ensure` step.
//!
//! Unlike v3 — which split this across the separate `aidp-graph-ensure`,
//! `aidp-run-init` and `aidp-render-task-prompt` binaries and shuttled paths
//! between them via CLI flags — v4 is a single `aidp` binary. So the build is a
//! plain library function ([`ensure`]) called from [`crate::run::init`], the
//! result lands directly in `RunState.knowledge_graph`, and the prompt renderer
//! reads it straight off `&RunState` (see `crate::prompt`). No helper binary.
//!
//! Contract: **non-blocking**. Whatever happens (graphify missing, crash, stale
//! cache, disabled), the run proceeds. A `failed` status simply means no graph
//! paths get injected into Task prompts, so sub-agents fall back to Grep/Read.
//!
//! Mirrors `settings.knowledge_graph` in `schema/pipeline.schema.json` and the
//! optional top-level `knowledge_graph` block in `schema/run-state.schema.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::clock::{Clock, SystemClock};
use crate::models::constants::kg::{CODE_ONLY_FLAG, EXCLUDE_FLAG, GRAPHIFY_NOT_FOUND, OUTPUT_FLAG};
use crate::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec};

/// Cache freshness policy for the workspace knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Refresh {
    /// Reuse the cache when no tracked file is newer than `graph.json`, else rebuild.
    Lazy,
    /// Rebuild the graph every run (slow but always fresh).
    Always,
    /// Use an existing graph as-is and NEVER call `graphify` — no build, ever. The
    /// default: if a usable graph is present it is injected, otherwise the run
    /// falls back to Grep/Read. The graph is an opt-in artefact the caller builds
    /// or commits out-of-band, not something a run builds on the fly.
    #[default]
    Never,
}

impl Refresh {
    pub fn as_str(self) -> &'static str {
        match self {
            Refresh::Lazy => "lazy",
            Refresh::Always => "always",
            Refresh::Never => "never",
        }
    }
}

/// Default exclude globs applied when `settings.knowledge_graph.exclude` is
/// omitted. Mirrors the `default` block in `schema/pipeline.schema.json`.
///
/// v4 keeps run artefacts under `.aidp/pipelines/<run_id>/` (v3 used
/// `.cursor/pipelines/*/`), so `.aidp/**` replaces the legacy patterns.
pub fn default_exclude() -> Vec<String> {
    vec![
        ".aidp/**".to_string(),
        ".cursor/graphify-out/**".to_string(),
        "node_modules/**".to_string(),
        ".git/**".to_string(),
        "dist/**".to_string(),
        "build/**".to_string(),
        "target/**".to_string(),
        "out/**".to_string(),
        ".next/**".to_string(),
        ".venv/**".to_string(),
        "venv/**".to_string(),
        "__pycache__/**".to_string(),
    ]
}

fn default_cache_dir() -> String {
    ".cursor/graphify-out".to_string()
}

fn default_graphify_bin() -> String {
    "graphify".to_string()
}

fn default_code_only() -> bool {
    true
}

/// Pipeline-level Graphify settings. Lives under `settings.knowledge_graph`.
///
/// All fields are optional in the YAML; serde defaults pick up the same values
/// the JSON Schema documents. `deny_unknown_fields` catches typos early — same
/// convention as the rest of the pipeline types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeGraphSettings {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    #[serde(default)]
    pub refresh: Refresh,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,

    #[serde(default = "default_code_only")]
    pub code_only: bool,

    #[serde(default = "default_graphify_bin")]
    pub graphify_bin: String,
}

impl Default for KnowledgeGraphSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_dir: default_cache_dir(),
            refresh: Refresh::Never,
            include: Vec::new(),
            exclude: default_exclude(),
            code_only: default_code_only(),
            graphify_bin: default_graphify_bin(),
        }
    }
}

impl KnowledgeGraphSettings {
    /// True when the orchestrator should actually shell out to `graphify`.
    pub fn should_run(&self) -> bool {
        self.enabled
    }
}

/// One of the four terminal states. Never `running` — by construction `ensure`
/// either produces a usable cache (`fresh` / `reused`), declines (`skipped`),
/// or could not produce one (`failed`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KnowledgeGraphStatus {
    /// Newly built by this run.
    Fresh,
    /// Lazy cache hit — reused an existing graph.json.
    Reused,
    /// `refresh = never` and no cache, or the graph was disabled.
    Skipped,
    /// Build attempted but errored (graphify missing, crash, ...). Pipeline keeps going.
    Failed,
}

impl KnowledgeGraphStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            KnowledgeGraphStatus::Fresh => "fresh",
            KnowledgeGraphStatus::Reused => "reused",
            KnowledgeGraphStatus::Skipped => "skipped",
            KnowledgeGraphStatus::Failed => "failed",
        }
    }
}

/// Run-state snapshot of the knowledge graph. Persisted at
/// `run-state.json#/knowledge_graph` by [`crate::run::init`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeGraphState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_json: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_report: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_html: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_at: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_indexed: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communities: Option<u64>,

    pub status: KnowledgeGraphStatus,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<KgFailureKind>,
}

/// Why a knowledge-graph build failed, so callers branch on a value, not on message text.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KgFailureKind {
    /// A `graphify_bin` or `cache_dir` setting was refused by the containment checks.
    ContainmentRefused,
    /// The graphify binary could not be found.
    BinaryMissing,
    #[default]
    Other,
}

impl KnowledgeGraphState {
    /// Constructor for a clean decline (graph disabled / no cache under `never`).
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self::bare(KnowledgeGraphStatus::Skipped, Some(reason.into()))
    }

    /// Constructor for non-fatal build failures.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::bare(KnowledgeGraphStatus::Failed, Some(reason.into()))
    }

    /// Constructor for a failure carrying its typed cause.
    pub fn failed_with_kind(reason: impl Into<String>, kind: KgFailureKind) -> Self {
        let mut state = Self::failed(reason);
        state.failure_kind = Some(kind);
        state
    }

    /// The reason of a failure that must abort the init: a containment refusal.
    pub fn containment_refusal(&self) -> Option<String> {
        if self.status != KnowledgeGraphStatus::Failed {
            return None;
        }
        match self.failure_kind {
            Some(KgFailureKind::ContainmentRefused) => self.reason.clone(),
            _ => None,
        }
    }

    fn bare(status: KnowledgeGraphStatus, reason: Option<String>) -> Self {
        Self {
            graph_json: None,
            graph_report: None,
            graph_html: None,
            cache_dir: None,
            built_at: None,
            files_indexed: None,
            communities: None,
            status,
            reason,
            failure_kind: None,
        }
    }

    /// True when the prompt renderer should hand the graph paths to sub-agents.
    /// Any usable cache counts — including a `skipped` reuse under
    /// `refresh = never` (committed graph). Only `failed` (no paths) is excluded.
    pub fn has_graph(&self) -> bool {
        self.graph_json.is_some() || self.graph_report.is_some()
    }
}

/// Build or refresh the workspace knowledge graph and return its run-state
/// snapshot. Never panics; never returns an `Err` — failures are encoded as
/// `status = failed`. See module docs for the non-blocking contract.
pub fn ensure(
    settings: &KnowledgeGraphSettings,
    workspace_root: &Path,
    vault_root: Option<&Path>,
) -> KnowledgeGraphState {
    ensure_with(
        settings,
        workspace_root,
        vault_root,
        &SystemClock,
        &OsProcessRunner,
    )
}

/// [`ensure`], taking `built_at` from `clock` and spawning `graphify` through
/// `runner` instead of reading the wall clock and `std::process::Command`
/// directly.
pub fn ensure_with(
    settings: &KnowledgeGraphSettings,
    workspace_root: &Path,
    vault_root: Option<&Path>,
    clock: &dyn Clock,
    runner: &dyn ProcessRunner,
) -> KnowledgeGraphState {
    let bin = match graphify_program(&settings.graphify_bin) {
        Ok(b) => b,
        Err(reason) => {
            return KnowledgeGraphState::failed_with_kind(reason, KgFailureKind::ContainmentRefused)
        }
    };
    let cache_dir = match resolve_cache_dir(workspace_root, &settings.cache_dir) {
        Ok(d) => d,
        Err(reason) => {
            return KnowledgeGraphState::failed_with_kind(reason, KgFailureKind::ContainmentRefused)
        }
    };

    // Create the cache dir up front so `graphify --output` has a target and the
    // lazy freshness walk has a stable reference point.
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        return KnowledgeGraphState::failed(format!(
            "failed to create cache dir {}: {e}",
            cache_dir.display()
        ));
    }

    let graph_json = cache_dir.join("graph.json");
    let graph_report = cache_dir.join("GRAPH_REPORT.md");
    let graph_html = cache_dir.join("graph.html");
    let cache_present = graph_json.is_file();

    // Always exclude the cache dir itself so the lazy mtime check never compares
    // graph.json against the sibling files it just wrote.
    let mut excludes = settings.exclude.clone();
    excludes.extend(derive_cache_excludes(workspace_root, &cache_dir));
    // v7: run artefacts live in the Obsidian vault, normally outside the
    // workspace — in which case the walker never sees them and this adds
    // nothing. It only matters when the vault sits *inside* the repo, where
    // indexing generated notes would poison the graph the same way `.aidp/**`
    // used to.
    if let Some(v) = vault_root {
        excludes.extend(derive_cache_excludes(workspace_root, v));
    }

    match settings.refresh {
        Refresh::Never => {
            if cache_present && graph_is_usable(&graph_json) {
                KnowledgeGraphState {
                    graph_json: Some(graph_json.to_string_lossy().into_owned()),
                    graph_report: opt_path(&graph_report),
                    graph_html: opt_path(&graph_html),
                    cache_dir: Some(cache_dir.to_string_lossy().into_owned()),
                    built_at: file_mtime_iso(&graph_json),
                    files_indexed: None,
                    communities: None,
                    status: KnowledgeGraphStatus::Skipped,
                    reason: Some(
                        "refresh=never: reusing existing cache without re-running graphify".into(),
                    ),
                    failure_kind: None,
                }
            } else if cache_present {
                // A committed but empty/corrupt graph must not be injected as if
                // usable — agents would be pointed at a dead graph (WI-P2-9).
                KnowledgeGraphState::skipped(
                    "refresh=never: committed graph.json is empty or unparseable; skipping injection",
                )
            } else {
                KnowledgeGraphState::skipped(
                    "refresh=never and no cache on disk: nothing to inject",
                )
            }
        }
        Refresh::Lazy
            if cache_present
                && graph_is_usable(&graph_json)
                && cache_is_fresh(
                    &graph_json,
                    workspace_root,
                    &settings.include,
                    &excludes,
                    settings.code_only,
                ) =>
        {
            KnowledgeGraphState {
                graph_json: Some(graph_json.to_string_lossy().into_owned()),
                graph_report: opt_path(&graph_report),
                graph_html: opt_path(&graph_html),
                cache_dir: Some(cache_dir.to_string_lossy().into_owned()),
                built_at: file_mtime_iso(&graph_json),
                files_indexed: None,
                communities: None,
                status: KnowledgeGraphStatus::Reused,
                reason: None,
                failure_kind: None,
            }
        }
        Refresh::Lazy | Refresh::Always => run_graphify(
            &bin,
            workspace_root,
            &cache_dir,
            settings.code_only,
            &excludes,
            clock,
            runner,
        ),
    }
}

/// Which graphify binary to spawn. `Ok(None)` = not found on PATH (the
/// non-blocking "missing" path); `Err` = refused setting.
fn graphify_program(setting: &str) -> Result<Option<PathBuf>, String> {
    // Explicit override from the user's own environment: taken as-is, like
    // AIDP_CLAUDE_BIN.
    if let Ok(v) = std::env::var("AIDP_GRAPHIFY_BIN") {
        if !v.is_empty() {
            return Ok(Some(PathBuf::from(v)));
        }
    }
    validate_bare_program(setting)?;
    Ok(crate::program::resolve_program(setting))
}

/// A repo-supplied `graphify_bin` must be a bare program name, resolved via
/// PATH only — never a path that could point into the repo itself.
fn validate_bare_program(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        return Err(format!(
            "settings.knowledge_graph.graphify_bin must be a bare program name (no path); got {name:?}. Set AIDP_GRAPHIFY_BIN for an explicit path."
        ));
    }
    Ok(())
}

/// Anchor `cache_dir` under the workspace: an absolute path, or a relative one
/// whose lexical normalization escapes via `..`, is refused.
fn resolve_cache_dir(workspace_root: &Path, cache_dir: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    let p = Path::new(cache_dir);
    let refuse = || {
        format!(
            "settings.knowledge_graph.cache_dir must be a relative path inside the workspace; got {cache_dir:?}"
        )
    };
    if p.is_absolute() || p.has_root() {
        return Err(refuse());
    }
    let mut depth: usize = 0;
    for c in p.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => depth = depth.checked_sub(1).ok_or_else(refuse)?,
            Component::RootDir | Component::Prefix(_) => return Err(refuse()),
        }
    }
    let joined = workspace_root.join(p);

    // LOC-27: the lexical `..` check above stops a literal `../..` in
    // `cache_dir`, but not a symlinked ancestor (e.g. a component of the
    // joined path that is itself a symlink pointing outside the workspace).
    // Walk up to the deepest ancestor that actually exists on disk,
    // canonicalize it and the workspace root, and require the former to stay
    // under the latter before any directory gets created under it.
    let mut probe: &Path = &joined;
    let existing_ancestor = loop {
        if probe.exists() {
            break Some(probe.to_path_buf());
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => break None,
        }
    };
    if let Some(ancestor) = existing_ancestor {
        let canon_ancestor = ancestor.canonicalize().map_err(|e| {
            format!("settings.knowledge_graph.cache_dir: failed to resolve {ancestor:?}: {e}")
        })?;
        let canon_ws = workspace_root.canonicalize().map_err(|e| {
            format!("settings.knowledge_graph.cache_dir: failed to resolve workspace root {workspace_root:?}: {e}")
        })?;
        if !canon_ancestor.starts_with(&canon_ws) {
            return Err(refuse());
        }
    }
    Ok(joined)
}

/// Glob patterns covering the cache directory (relative to the workspace) so
/// the freshness walk does not count files we just produced.
fn derive_cache_excludes(workspace_root: &Path, cache_dir: &Path) -> Vec<String> {
    let ws = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let cache_abs = cache_dir
        .canonicalize()
        .unwrap_or_else(|_| cache_dir.to_path_buf());
    let Ok(rel) = cache_abs.strip_prefix(&ws) else {
        return Vec::new(); // cache lives outside the workspace; the walker won't see it
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        return Vec::new();
    }
    vec![format!("{rel_str}/"), format!("{rel_str}/**")]
}

/// Known non-code / documentation / data / asset extensions. Under `code_only`
/// (the v7 default), graphify indexes only code, so a change to one of these
/// files can never be reflected in the graph and must NOT invalidate the cache.
/// The list is intentionally a DENYLIST of well-known non-code types: anything
/// not listed (including unknown extensions and extension-less files) still
/// invalidates, so we can never reuse a graph that might be stale.
const NON_CODE_EXTS: &[&str] = &[
    "md", "markdown", "txt", "rst", "adoc", "json", "yaml", "yml", "toml", "ini", "cfg", "conf",
    "csv", "tsv", "xml", "html", "htm", "css", "scss", "sass", "less", "lock", "log", "svg", "png",
    "jpg", "jpeg", "gif", "ico", "webp", "bmp", "pdf", "zip", "gz", "tgz", "tar", "7z", "woff",
    "woff2", "ttf", "otf", "eot", "mp4", "mov", "mp3", "wav", "map",
];

/// Under `code_only`, true when this file's extension is a known non-code type
/// (so it must not invalidate a code-only graph). Files with no extension or an
/// unknown extension return false → they still invalidate (conservative).
fn noncode_under_code_only(path: &Path, code_only: bool) -> bool {
    if !code_only {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => NON_CODE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// Whether `graph.json` is newer than every tracked file under `--include`.
///
/// Walks the workspace with `walkdir`, pruning excluded directories at the
/// boundary (so we never recurse into `node_modules/`, `target/`, `.aidp/`,
/// the cache dir, ...), and short-circuits on the first file newer than
/// `graph.json`. Relies on OS mtime, not git. Under `code_only`, edits to
/// known non-code files (docs/data/assets) are ignored — graphify would never
/// reflect them, so they should not force a rebuild.
fn cache_is_fresh(
    graph_json: &Path,
    workspace_root: &Path,
    includes: &[String],
    excludes: &[String],
    code_only: bool,
) -> bool {
    let Ok(graph_meta) = std::fs::metadata(graph_json) else {
        return false;
    };
    let Ok(graph_mtime) = graph_meta.modified() else {
        return false;
    };
    let ws_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let walker = walkdir::WalkDir::new(&ws_root)
        .into_iter()
        .filter_entry(|e| {
            let Ok(rel) = e.path().strip_prefix(&ws_root) else {
                return true;
            };
            if rel.as_os_str().is_empty() {
                return true; // the root itself
            }
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let is_dir = e.file_type().is_dir();
            !path_is_excluded(&rel_str, is_dir, excludes)
        });

    for entry in walker.flatten() {
        if entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&ws_root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !includes.is_empty() && !matches_any(&rel_str, includes) {
            continue;
        }
        // Under code_only, a non-code file (doc/data/asset) can't be in the
        // graph, so its mtime must not invalidate the cache.
        if noncode_under_code_only(entry.path(), code_only) {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if mtime > graph_mtime {
                    return false;
                }
            }
        }
    }

    true
}

/// True when `rel` (a forward-slash relative path) is covered by any exclude.
///
/// Handles the common `base/**` form with a robust prefix check (so a `target`
/// directory is pruned by `target/**`), and falls back to `glob::Pattern` for
/// everything else — matching v4's scope-check conventions in `scope.rs`.
fn path_is_excluded(rel: &str, is_dir: bool, patterns: &[String]) -> bool {
    for p in patterns {
        if let Some(base) = p.strip_suffix("/**") {
            if rel == base || rel.starts_with(&format!("{base}/")) {
                return true;
            }
        }
        if let Ok(pat) = glob::Pattern::new(p) {
            if pat.matches(rel) {
                return true;
            }
            if is_dir && pat.matches(&format!("{rel}/")) {
                return true;
            }
        }
    }
    false
}

fn matches_any(rel: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        glob::Pattern::new(p)
            .map(|pat| pat.matches(rel))
            .unwrap_or(false)
    })
}

fn opt_path(p: &Path) -> Option<String> {
    if p.is_file() {
        Some(p.to_string_lossy().into_owned())
    } else {
        None
    }
}

fn file_mtime_iso(p: &Path) -> Option<String> {
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta.modified().ok()?;
    let dt: DateTime<Utc> = mtime.into();
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn now_iso(clock: &dyn Clock) -> String {
    clock.now_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn run_graphify(
    bin: &Option<PathBuf>,
    workspace_root: &Path,
    cache_dir: &Path,
    code_only: bool,
    excludes: &[String],
    clock: &dyn Clock,
    runner: &dyn ProcessRunner,
) -> KnowledgeGraphState {
    let Some(bin) = bin else {
        return KnowledgeGraphState::failed_with_kind(
            GRAPHIFY_NOT_FOUND,
            KgFailureKind::BinaryMissing,
        );
    };
    let bin = bin.display();
    let mut spec = ProcessSpec::new(bin.to_string())
        .arg(workspace_root.display().to_string())
        .arg(OUTPUT_FLAG)
        .arg(cache_dir.display().to_string());
    if code_only {
        spec = spec.arg(CODE_ONLY_FLAG);
    }
    for e in excludes {
        spec = spec.arg(EXCLUDE_FLAG).arg(e);
    }

    let output = match runner.output(&spec) {
        Ok(o) => o,
        Err(e) => {
            // NotFound is the common "graphifyy not installed" case; any other
            // I/O error is reported but still non-blocking.
            let reason = match e.kind() {
                std::io::ErrorKind::NotFound => format!(
                    "graphify CLI not found ({bin}). Install with `pip install graphifyy` \
                     or set AIDP_GRAPHIFY_BIN to an explicit path."
                ),
                _ => format!("failed to spawn {bin}: {e}"),
            };
            let kind = match e.kind() {
                std::io::ErrorKind::NotFound => KgFailureKind::BinaryMissing,
                _ => KgFailureKind::Other,
            };
            return KnowledgeGraphState::failed_with_kind(reason, kind);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_trim = stderr.trim();
        let reason = if stderr_trim.is_empty() {
            format!(
                "graphify exited with status {}",
                output.status.code().unwrap_or(-1)
            )
        } else {
            format!(
                "graphify exited with status {}: {}",
                output.status.code().unwrap_or(-1),
                truncate(stderr_trim, 400)
            )
        };
        return KnowledgeGraphState::failed(reason);
    }

    // Trust the on-disk artefacts, not stdout (graphify's CLI surface evolves).
    let graph_json = cache_dir.join("graph.json");
    if !graph_json.is_file() {
        return KnowledgeGraphState::failed(format!(
            "graphify completed but produced no graph.json at {}",
            graph_json.display()
        ));
    }

    let graph_report = cache_dir.join("GRAPH_REPORT.md");
    let graph_html = cache_dir.join("graph.html");
    let (files_indexed, communities) = parse_graph_metrics(&graph_json);

    KnowledgeGraphState {
        graph_json: Some(graph_json.to_string_lossy().into_owned()),
        graph_report: opt_path(&graph_report),
        graph_html: opt_path(&graph_html),
        cache_dir: Some(cache_dir.to_string_lossy().into_owned()),
        built_at: Some(now_iso(clock)),
        files_indexed,
        communities,
        status: KnowledgeGraphStatus::Fresh,
        reason: None,
        failure_kind: None,
    }
}

/// Best-effort extraction of node/community counts from a `graph.json`.
/// Graphify's exact key names may evolve; try obvious candidates, else skip.
/// A graph is usable for injection only if it parses and has at least one
/// recognized node. Empty (`{}`, `{"nodes":[]}`), unparseable, or unreadable
/// graphs return false → not injected/reused (WI-P2-9).
fn graph_is_usable(graph_json: &Path) -> bool {
    matches!(parse_graph_metrics(graph_json).0, Some(n) if n > 0)
}

fn parse_graph_metrics(graph_json: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(bytes) = std::fs::read(graph_json) else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, None);
    };

    let files_indexed =
        read_count(&v, &["files_indexed", "files", "node_count", "nodes"]).or_else(|| {
            v.get("nodes")
                .and_then(|n| n.as_array())
                .map(|a| a.len() as u64)
        });
    let communities = read_count(&v, &["communities_count", "communities", "n_communities"])
        .or_else(|| {
            v.get("communities")
                .and_then(|c| c.as_array())
                .map(|a| a.len() as u64)
        });

    (files_indexed, communities)
}

fn read_count(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|n| n.as_u64()) {
            return Some(n);
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    #[test]
    fn graphify_bin_paths_refused() {
        for bad in ["./evil", "C:evil", "..\\x", "", ".", "..", "/usr/bin/g"] {
            let e = super::validate_bare_program(bad).unwrap_err();
            assert!(e.contains("graphify_bin"), "{e}");
        }
        assert!(super::validate_bare_program("graphify").is_ok());
    }

    #[test]
    fn graphify_bin_env_override_honoured() {
        std::env::set_var("AIDP_GRAPHIFY_BIN", "C:/tools/graphify.exe");
        let r = super::graphify_program("./evil");
        std::env::remove_var("AIDP_GRAPHIFY_BIN");
        assert_eq!(
            r.unwrap(),
            Some(std::path::PathBuf::from("C:/tools/graphify.exe"))
        );
    }

    #[test]
    fn absolute_cache_dir_refused_and_not_created() {
        let ws = tmp_ws("kg-abs-cache");
        let target = ws.path().join("outside-abs");
        let s = KnowledgeGraphSettings {
            enabled: true,
            cache_dir: target.to_string_lossy().into_owned(),
            ..KnowledgeGraphSettings::default()
        };
        let st = ensure(&s, ws.path(), None);
        assert_eq!(st.status, KnowledgeGraphStatus::Failed);
        assert!(st.reason.as_deref().unwrap().contains("cache_dir"));
        assert!(!target.exists());
        assert!(super::resolve_cache_dir(ws.path(), "../x").is_err());
        assert!(super::resolve_cache_dir(ws.path(), "a/../../x").is_err());
    }

    #[test]
    fn relative_cache_dir_ok() {
        let ws = tmp_ws("kg-rel-cache");
        assert_eq!(
            super::resolve_cache_dir(ws.path(), ".aidp/graph").unwrap(),
            ws.path().join(".aidp/graph")
        );
        assert!(super::resolve_cache_dir(ws.path(), "a/../b").is_ok());
    }

    #[test]
    fn symlinked_cache_ancestor_outside_workspace_is_refused() {
        let ws = tmp_ws("kg-symlink-cache");
        let outside = tempfile::tempdir().unwrap();
        let link = ws.path().join("cache");
        #[cfg(unix)]
        let symlinked = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(windows)]
        let symlinked = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let symlinked = false;
        if !symlinked {
            // Unprivileged environment (e.g. Windows without dev mode/admin) —
            // symlink creation itself failed, nothing to test.
            return;
        }
        assert!(super::resolve_cache_dir(ws.path(), "cache/graph").is_err());
    }

    use super::*;
    use aidp_testkit::TempWorkspace;

    fn tmp_ws(label: &str) -> TempWorkspace {
        TempWorkspace::new(label)
    }

    #[test]
    fn defaults_match_schema() {
        let s = KnowledgeGraphSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.cache_dir, ".cursor/graphify-out");
        assert_eq!(s.refresh, Refresh::Never);
        assert!(s.include.is_empty());
        assert!(s.code_only);
        assert_eq!(s.graphify_bin, "graphify");
        // v4 run artefacts live under .aidp/, not .cursor/pipelines/.
        assert!(s.exclude.iter().any(|p| p == ".aidp/**"));
        assert!(s.exclude.iter().any(|p| p == ".git/**"));
        assert!(s.exclude.iter().any(|p| p == ".cursor/graphify-out/**"));
    }

    // WI-P2-9: a graph is usable only if it parses and has nodes.
    #[test]
    fn graph_usability_probe() {
        let ws = tmp_ws("usable");
        let p = ws.path().join("graph.json");
        std::fs::write(&p, r#"{"node_count":12}"#).unwrap();
        assert!(graph_is_usable(&p));
        std::fs::write(&p, r#"{"nodes":[]}"#).unwrap();
        assert!(!graph_is_usable(&p), "empty nodes array is not usable");
        std::fs::write(&p, "{}").unwrap();
        assert!(!graph_is_usable(&p), "empty object is not usable");
        std::fs::write(&p, "not json").unwrap();
        assert!(!graph_is_usable(&p), "unparseable is not usable");
        assert!(
            !graph_is_usable(&ws.path().join("missing.json")),
            "missing file is not usable"
        );
        let _ = std::fs::remove_dir_all(ws.path());
    }

    // WI-P3-3: under code_only, only known non-code files are spared from
    // invalidating the cache; unknown/extension-less files stay conservative.
    #[test]
    fn code_only_freshness_classification() {
        assert!(noncode_under_code_only(Path::new("README.md"), true));
        assert!(noncode_under_code_only(Path::new("docs/x.JSON"), true)); // case-insensitive
        assert!(!noncode_under_code_only(Path::new("src/x.rs"), true)); // code → can invalidate
        assert!(!noncode_under_code_only(Path::new("weird.xyz"), true)); // unknown → conservative
        assert!(!noncode_under_code_only(Path::new("Makefile"), true)); // no ext → conservative
        assert!(!noncode_under_code_only(Path::new("README.md"), false)); // code_only off → unchanged
    }

    #[test]
    fn refresh_round_trips() {
        for r in [Refresh::Lazy, Refresh::Always, Refresh::Never] {
            let s = serde_json::to_string(&r).unwrap();
            let back: Refresh = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
        assert_eq!(serde_json::to_string(&Refresh::Lazy).unwrap(), "\"lazy\"");
        assert_eq!(serde_json::to_string(&Refresh::Never).unwrap(), "\"never\"");
    }

    #[test]
    fn status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&KnowledgeGraphStatus::Fresh).unwrap(),
            "\"fresh\""
        );
        assert_eq!(KnowledgeGraphStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn has_graph_requires_a_path() {
        let mut s = KnowledgeGraphState::failed("x");
        assert!(!s.has_graph());
        s.graph_json = Some("/abs/graph.json".into());
        assert!(s.has_graph());
    }

    #[test]
    fn skipped_and_failed_constructors_set_status() {
        let s = KnowledgeGraphState::skipped("disabled");
        assert_eq!(s.status, KnowledgeGraphStatus::Skipped);
        assert_eq!(s.reason.as_deref(), Some("disabled"));
        assert!(s.graph_json.is_none());

        let f = KnowledgeGraphState::failed("graphify missing");
        assert_eq!(f.status, KnowledgeGraphStatus::Failed);
        assert_eq!(f.reason.as_deref(), Some("graphify missing"));
    }

    #[test]
    fn path_is_excluded_prunes_dirs_and_keeps_siblings() {
        let ex = default_exclude();
        // Directories pruned via the `base/**` prefix rule.
        assert!(path_is_excluded("node_modules", true, &ex));
        assert!(path_is_excluded("target", true, &ex));
        assert!(path_is_excluded(".aidp", true, &ex));
        assert!(path_is_excluded(
            ".aidp/pipelines/run-state.json",
            false,
            &ex
        ));
        // Real source is kept.
        assert!(!path_is_excluded("src", true, &ex));
        assert!(!path_is_excluded("src/main.rs", false, &ex));
    }

    #[test]
    fn ensure_is_non_blocking_when_graphify_missing() {
        let ws = tmp_ws("missing-bin");
        std::fs::write(ws.path().join("a.rs"), "fn main() {}").unwrap();
        let settings = KnowledgeGraphSettings {
            enabled: true,
            // Force a build attempt: the default is now `never` (no build), so
            // pin `lazy` to actually exercise the graphify-missing failure path.
            refresh: Refresh::Lazy,
            graphify_bin: "graphify-definitely-not-installed-xyz".into(),
            ..Default::default()
        };
        let state = ensure(&settings, ws.path(), None);
        assert_eq!(state.status, KnowledgeGraphStatus::Failed);
        assert!(!state.has_graph());
        assert!(state.reason.unwrap().contains("not found"));
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn ensure_never_without_cache_is_skipped() {
        let ws = tmp_ws("never-no-cache");
        let settings = KnowledgeGraphSettings {
            enabled: true,
            refresh: Refresh::Never,
            ..Default::default()
        };
        let state = ensure(&settings, ws.path(), None);
        assert_eq!(state.status, KnowledgeGraphStatus::Skipped);
        assert!(!state.has_graph());
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn ensure_never_with_cache_reuses_paths() {
        let ws = tmp_ws("never-with-cache");
        let cache = ws.path().join(".cursor/graphify-out");
        std::fs::create_dir_all(&cache).unwrap();
        // A non-trivial graph (has nodes) → usable → paths injected (WI-P2-9).
        std::fs::write(cache.join("graph.json"), r#"{"node_count":3}"#).unwrap();
        let settings = KnowledgeGraphSettings {
            enabled: true,
            refresh: Refresh::Never,
            ..Default::default()
        };
        let state = ensure(&settings, ws.path(), None);
        assert_eq!(state.status, KnowledgeGraphStatus::Skipped);
        assert!(state.has_graph()); // committed-graph CI case: paths injected
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn cache_is_fresh_true_when_nothing_newer() {
        let ws = tmp_ws("fresh-true");
        std::fs::write(ws.path().join("source.py"), "print('hi')").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let graph = ws.path().join("graph.json");
        std::fs::write(&graph, "{}").unwrap();
        assert!(cache_is_fresh(&graph, ws.path(), &[], &[], false));
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn cache_is_fresh_false_when_a_file_is_newer() {
        let ws = tmp_ws("fresh-false");
        let graph = ws.path().join("graph.json");
        std::fs::write(&graph, "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(ws.path().join("source.py"), "print('hi')").unwrap();
        assert!(!cache_is_fresh(&graph, ws.path(), &[], &[], false));
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn cache_freshness_ignores_excluded_dirs() {
        let ws = tmp_ws("fresh-excluded");
        let graph = ws.path().join("graph.json");
        std::fs::write(&graph, "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        // A newer file inside an excluded dir must NOT invalidate the cache.
        let nm = ws.path().join("node_modules/pkg");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("index.js"), "x").unwrap();
        assert!(cache_is_fresh(
            &graph,
            ws.path(),
            &[],
            &default_exclude(),
            false
        ));
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn parse_graph_metrics_reads_counts_and_arrays() {
        let ws = tmp_ws("metrics");
        let p = ws.path().join("graph.json");
        std::fs::write(&p, r#"{"node_count":144,"communities_count":6}"#).unwrap();
        assert_eq!(parse_graph_metrics(&p), (Some(144), Some(6)));
        std::fs::write(
            &p,
            r#"{"nodes":[{"id":1},{"id":2}],"communities":[{"id":"c1"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_graph_metrics(&p), (Some(2), Some(1)));
        std::fs::write(&p, r#"{"unrelated":42}"#).unwrap();
        assert_eq!(parse_graph_metrics(&p), (None, None));
        let _ = std::fs::remove_dir_all(ws.path());
    }

    #[test]
    fn truncate_handles_unicode_safely() {
        let t = truncate("abcdé", 4);
        assert!(t.starts_with("abcd"));
        assert!(t.ends_with("..."));
    }
}

#[cfg(test)]
#[path = "../tests/unit/kg_failure_kind_tests.rs"]
mod kg_failure_kind_tests;
