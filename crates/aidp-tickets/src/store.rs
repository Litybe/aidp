//! `TicketStore`: the filesystem layer over `<vault>/TODO/` — open, list,
//! get/path_of, atomic save, key allocation, moving a ticket to another
//! service, and the archive sweep. Vision §5 and the lot plan's "Store"
//! section of `docs/plans/ticket-autopilot/01-lot-7a-jira-local.md`.
//!
//! This crate receives an already-resolved `vault_root: &Path` and must not
//! depend on `aidp-core` (see `lib.rs`), so the vault-layout literals this
//! module needs are declared locally below rather than imported from
//! `aidp_core::vault`.

use crate::config::{self, Config, Routing};
use crate::model::{Source, Status, Ticket};
use crate::{index, note, order, safe_name};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// `<vault>/TODO`. `aidp_core::vault` has no equivalent const — `TODO/` is
/// not part of the `Pipelines/`/`Knowledge/` layout that crate owns — but
/// `aidp_core::obsidian`'s dashboard writer hard-codes this exact same
/// literal (`vault_root.join("TODO")`) for the `[[TODO/_TODO|Tickets]]`
/// dashboard link. That duplication, like this one, is deliberate: a shared
/// const would require `aidp-tickets` to depend on `aidp-core`, which is
/// exactly the dependency this crate must not take.
const TODO_DIR: &str = "TODO";

/// `<vault>/TODO/.state` — machine-only bookkeeping (`counter.json`,
/// `ticket-events.jsonl`, and, from lot 9a, `dispatcher.json`). Dot-prefixed
/// so it is skipped by every directory walk in this module and hidden from
/// Obsidian's file explorer, same convention as `aidp_core::vault::STATE_DIR`.
const STATE_DIR: &str = ".state";

/// Per-service subfolder holding swept `done`/`cancelled` tickets.
const ARCHIVE_DIR: &str = "archive";

/// Sink for a Jira mirror whose service could not be deduced (lot 8a). Not a
/// real service — see [`check_service`].
const UNROUTED_DIR: &str = "unrouted";

/// Mirrors `aidp_core::vault::PIPELINES_DIR`. Duplicated for the same reason
/// as [`TODO_DIR`]: `known_services` needs to know the name of the
/// `Pipelines/` folder to enumerate known services, and importing it from
/// `aidp-core` would be the forbidden dependency.
const PIPELINES_DIR: &str = "Pipelines";

/// Mirrors `aidp_core::vault::KNOWLEDGE_DIR`. See [`PIPELINES_DIR`].
const KNOWLEDGE_DIR: &str = "Knowledge";

/// Service names that can never be a real service, whatever `--allow-unknown-service`
/// says — see [`check_service`].
const RESERVED_SERVICE_NAMES: &[&str] = &[UNROUTED_DIR, ARCHIVE_DIR, STATE_DIR];

/// The filesystem gateway over `<vault>/TODO/`. Holds no ticket data in
/// memory across calls — every method re-reads the vault, matching vision §2
/// principle 2 ("le vault est la vérité... jamais de cache d'état dans le
/// crate").
pub struct TicketStore {
    /// `<vault>/TODO`.
    root: PathBuf,
    vault_root: PathBuf,
    config: Config,
    routing: Routing,
}

/// Filter for [`TicketStore::list`].
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// Empty means "any status".
    pub status: Vec<Status>,
    pub service: Option<String>,
    pub source: Option<Source>,
    /// Include `archive/` subfolders too. Default `false`.
    pub archived: bool,
}

impl TicketStore {
    /// Open (creating if absent) `<vault_root>/TODO/` and `TODO/.state/`,
    /// write a commented `config.yaml` from [`config::default_yaml`] only
    /// when the file is absent (a user's own file is never overwritten), then
    /// load [`Config`] and [`Routing`].
    pub fn open(vault_root: &Path) -> anyhow::Result<TicketStore> {
        let root = vault_root.join(TODO_DIR);
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join(STATE_DIR))?;

        let config_path = root.join("config.yaml");
        if !config_path.exists() {
            fs::write(&config_path, config::default_yaml())?;
        }

        let config = config::load(vault_root);
        let routing = config::load_routing(vault_root);

        Ok(TicketStore {
            root,
            vault_root: vault_root.to_path_buf(),
            config,
            routing,
        })
    }

    /// `<vault>/TODO`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The vault root this store was opened with.
    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    /// `TODO/config.yaml`, as loaded at [`TicketStore::open`] time.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// `TODO/routing.yaml`'s `repos:` table, as loaded at
    /// [`TicketStore::open`] time.
    pub fn routing(&self) -> &Routing {
        &self.routing
    }

    /// Walk the immediate child directories of `TODO/` — skipping any name
    /// starting with `_` or `.` (so `.state/` is out), `unrouted/` included —
    /// and parse every `*.md` file directly inside each (never `_`-prefixed,
    /// and never inside `archive/` unless `filter.archived`), keeping the
    /// path each note was found at alongside its parsed `Ticket` — a note
    /// that fails to parse, or does not carry `aidp: ticket`, is reported
    /// once on stderr and skipped: never fatal. The result is sorted by
    /// [`order::sort_key`] applied to each pair's `Ticket`, so this and
    /// [`TicketStore::list`] always agree on dequeue order.
    ///
    /// This is the one traversal every other listing/archiving operation in
    /// this module builds on: [`TicketStore::list`] below is a thin map that
    /// drops the path, and [`TicketStore::archive_sweep`] (and `cmd_reindex`
    /// in `cli.rs`) call this directly so the path a candidate is archived
    /// at is the one this walk already found — never a fresh `path_of`
    /// lookup per candidate.
    pub(crate) fn list_with_paths(&self, filter: &ListFilter) -> Vec<(Ticket, PathBuf)> {
        let mut out = Vec::new();

        let Ok(entries) = fs::read_dir(&self.root) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            collect_notes(&path, &filter.status, &mut out);
            if filter.archived {
                let archive = path.join(ARCHIVE_DIR);
                if archive.is_dir() {
                    collect_notes(&archive, &filter.status, &mut out);
                }
            }
        }

        let mut out: Vec<(Ticket, PathBuf)> = out
            .into_iter()
            .filter(|(t, _)| filter.service.as_deref().is_none_or(|s| t.service == s))
            .filter(|(t, _)| filter.source.is_none_or(|s| t.source == s))
            .filter(|(t, _)| filter.status.is_empty() || filter.status.contains(&t.status))
            .collect();
        out.sort_by_key(|(t, _)| order::sort_key(t));
        out
    }

    /// Ticket-only view of [`Self::list_with_paths`], for every caller that
    /// has no use for the on-disk path — the great majority. This method's
    /// PUBLIC SIGNATURE MUST NOT CHANGE: it is called from outside this
    /// crate at `aidp-cli/src/main.rs` (`aidp vault reindex`), and from
    /// within this crate by `cli.rs` and `index.rs`.
    pub fn list(&self, filter: &ListFilter) -> Vec<Ticket> {
        self.list_with_paths(filter)
            .into_iter()
            .map(|(ticket, _)| ticket)
            .collect()
    }

    /// Find a ticket note by key across every service folder (a key is
    /// unique in the whole store) and parse it.
    ///
    /// `key` is validated with [`safe_name::check`] first, so an unsafe key
    /// (e.g. a traversal like `../../etc/passwd`) fails with an explicit
    /// error naming the rule it broke, instead of the misleading "ticket not
    /// found" that `path_of` returning `None` would otherwise produce.
    pub fn get(&self, key: &str) -> anyhow::Result<(Ticket, String, serde_yaml::Mapping)> {
        safe_name::check("key", key)?;
        let path = self.path_of(key).ok_or_else(|| {
            crate::ticket_error::TicketError::NotFound(
                crate::models::constants::messages::ticket_not_found_under(
                    key,
                    &self.root.display().to_string(),
                ),
            )
        })?;
        let text = fs::read_to_string(&path)?;
        let parsed = note::parse(&text)?;
        Ok((parsed.ticket, parsed.body, parsed.extra))
    }

    /// The path a ticket note lives at, searching every service folder
    /// (direct children of `TODO/`, `unrouted/` included) and their
    /// `archive/` subfolder. `None` if no `<key>.md` exists anywhere.
    ///
    /// `key` is validated with [`safe_name::check`] first: an unsafe key
    /// (e.g. containing `..` or a path separator) never designated an
    /// existing file under `TODO/`, so it is treated exactly like any other
    /// key that doesn't resolve to a note — `None`, not an error. The return
    /// type stays `Option<PathBuf>` on purpose, to avoid churning this
    /// method's three internal callers over a signature change.
    pub fn path_of(&self, key: &str) -> Option<PathBuf> {
        safe_name::check("key", key).ok()?;
        let entries = fs::read_dir(&self.root).ok()?;
        let filename = format!("{key}.md");
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            let direct = path.join(&filename);
            if direct.is_file() {
                return Some(direct);
            }
            let archived = path.join(ARCHIVE_DIR).join(&filename);
            if archived.is_file() {
                return Some(archived);
            }
        }
        None
    }

    /// [`Self::save_reporting`] without the index report, for callers that
    /// only need the note's path.
    pub fn save(
        &self,
        ticket: &mut Ticket,
        body: &str,
        extra: &serde_yaml::Mapping,
    ) -> anyhow::Result<PathBuf> {
        self.save_reporting(ticket, body, extra)
            .map(|(dest, _report)| dest)
    }

    /// Recompute derived fields, bump `updated`, render, and write the note
    /// atomically (temp file + rename, never a direct overwrite), then
    /// refresh the generated indexes best-effort — a failed index write is
    /// returned in the [`index::RefreshReport`] and never fails the save.
    ///
    /// Writes to `TODO/<ticket.service>/<ticket.key>.md`. If `ticket.service`
    /// was changed by the caller since the note was last saved, this does
    /// **not** move or delete the note at its old location — use
    /// [`TicketStore::move_to_service`] for that.
    ///
    /// `ticket.key` and `ticket.service` are validated with
    /// [`safe_name::check`] before any directory is created or any byte is
    /// written: a `Ticket` can reach this method built from a hand-edited
    /// note's frontmatter (`note::parse` also validates, but a caller could
    /// in principle construct a `Ticket` directly), so this is the last line
    /// of defense against writing outside `<vault>/TODO/`.
    pub fn save_reporting(
        &self,
        ticket: &mut Ticket,
        body: &str,
        extra: &serde_yaml::Mapping,
    ) -> anyhow::Result<(PathBuf, index::RefreshReport)> {
        safe_name::check("key", &ticket.key)?;
        safe_name::check("service", &ticket.service)?;
        ticket.recompute_derived();
        ticket.updated = chrono::Utc::now();
        let text = note::render(ticket, body, extra);

        let service_dir = self.root.join(&ticket.service);
        fs::create_dir_all(&service_dir)?;
        let dest = service_dir.join(format!("{}.md", ticket.key));
        write_atomic(&dest, &text)?;

        let report = self.reindex();
        Ok((dest, report))
    }

    /// `<prefix>-<n>` with `prefix = config.local.key_prefix` and
    /// `n = max(counter.json, max n over the existing <prefix>-<n> notes) + 1`,
    /// then persists `TODO/.state/counter.json` atomically. Scanning the
    /// existing keys (rather than trusting the counter file alone) is what
    /// makes allocation survive a deleted or stale counter file.
    ///
    /// Like every other write in this lot, this does no cross-process
    /// locking — vision §"Risques": atomic writes now, a real lock only
    /// matters once a daemon and the CLI can race (lot 9a).
    pub fn allocate_key(&self) -> anyhow::Result<String> {
        let prefix = self.config.local.key_prefix.clone();

        let counter_path = self.root.join(STATE_DIR).join("counter.json");
        let stored = fs::read_to_string(&counter_path)
            .ok()
            .and_then(|t| serde_json::from_str::<CounterFile>(&t).ok())
            .map(|c| c.last)
            .unwrap_or(0);

        let existing_max = self.max_existing_number(&prefix);
        let n = stored.max(existing_max) + 1;

        write_atomic(
            &counter_path,
            &serde_json::to_string_pretty(&CounterFile { last: n })?,
        )?;

        Ok(format!("{prefix}-{n}"))
    }

    /// The highest `n` among existing `<prefix>-<n>.md` notes, scanning every
    /// service folder and its `archive/` subfolder (an archived ticket still
    /// occupied its key). Reads filenames only — a note need not even parse
    /// to count towards the maximum, since the key is the filename stem.
    fn max_existing_number(&self, prefix: &str) -> u64 {
        let mut max_n: u64 = 0;
        let Ok(entries) = fs::read_dir(&self.root) else {
            return max_n;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            scan_max_number(&path, prefix, &mut max_n);
            scan_max_number(&path.join(ARCHIVE_DIR), prefix, &mut max_n);
        }
        max_n
    }

    /// Move a ticket's note to another service's folder, update
    /// `ticket.service`, recompute derived fields (`tags` in particular), and
    /// reindex — since [`Self::reindex`] regenerates every service index in
    /// one pass, this reindexes both the old and the new service folder.
    ///
    /// `key` is already covered by [`Self::get`] below, which validates it.
    /// `service`'s shape is validated here with [`safe_name::check`] before
    /// it is joined onto the store root — `service` is not necessarily the
    /// same value `TicketStore::check_service` already vetted (a caller can
    /// reach this method directly), so it gets its own check rather than
    /// trusting the CLI layer.
    pub fn move_to_service(&self, key: &str, service: &str) -> anyhow::Result<PathBuf> {
        safe_name::check("service", service)?;
        let (mut ticket, body, extra) = self.get(key)?;
        let old_path = self.path_of(key).ok_or_else(|| {
            crate::ticket_error::TicketError::NotFound(
                crate::models::constants::messages::ticket_not_found_under(
                    key,
                    &self.root.display().to_string(),
                ),
            )
        })?;

        ticket.service = service.to_string();
        ticket.recompute_derived();
        ticket.updated = chrono::Utc::now();
        let text = note::render(&ticket, &body, &extra);

        let new_dir = self.root.join(service);
        fs::create_dir_all(&new_dir)?;
        let new_path = new_dir.join(format!("{key}.md"));
        write_atomic(&new_path, &text)?;

        if old_path != new_path {
            let _ = fs::remove_file(&old_path);
        }

        self.reindex();

        Ok(new_path)
    }

    /// Move logic only, no traversal and no reindex: for each `(ticket,
    /// path)` pair — typically produced by [`Self::list_with_paths`] —
    /// rename the note it names into `<service>/archive/` when `status` is
    /// `Done`/`Cancelled` and `updated` is older than
    /// `config.local.archive_after_days`, reusing `path` exactly as given
    /// rather than re-deriving it with [`Self::path_of`].
    ///
    /// `ticket.key` and `ticket.service` are re-validated with
    /// [`safe_name::check`] here, per entry, before the `<service>/archive/`
    /// destination is built: both current callers ([`Self::archive_sweep`]
    /// and `cli.rs`'s `cmd_reindex`) only ever pass entries produced by
    /// [`Self::list_with_paths`], which already went through `note::parse`'s
    /// own check — but this method builds the exact same
    /// `<service>/<key>.md` shape [`Self::save`] does, so, like `save`,
    /// `get`, `path_of` and `move_to_service`, it checks again itself rather
    /// than trusting a caller in place of the filesystem write it is about
    /// to perform. An entry that fails the check is reported once on stderr
    /// and skipped, not counted — the same discipline as every other
    /// per-entry failure below.
    ///
    /// An entry whose note already lives under `archive/` (its path's parent
    /// directory is named `archive`) is skipped and not counted: without
    /// this check, handing this an `archived: true` listing — as
    /// `cmd_reindex` in `cli.rs` does — would try to rename an already
    /// archived file onto itself.
    ///
    /// Best-effort, same discipline as the rest of this module: a failed
    /// `safe_name::check`, `create_dir_all` or `rename` for one entry is
    /// reported once on stderr and the loop `continue`s to the next entry —
    /// it never aborts the sweep, and this method itself never returns
    /// `Err`.
    pub fn archive_entries(&self, entries: &[(Ticket, PathBuf)]) -> anyhow::Result<usize> {
        let cutoff = chrono::Utc::now()
            - chrono::Duration::days(self.config.local.archive_after_days as i64);
        let mut moved = 0usize;

        for (ticket, path) in entries {
            if let Err(e) = safe_name::check("key", &ticket.key) {
                tracing::warn!("skipping entry with unsafe key {}: {e}", ticket.key);
                continue;
            }
            if let Err(e) = safe_name::check("service", &ticket.service) {
                tracing::warn!("skipping entry with unsafe service {}: {e}", ticket.service);
                continue;
            }
            if !matches!(ticket.status, Status::Done | Status::Cancelled) {
                continue;
            }
            if ticket.updated >= cutoff {
                continue;
            }
            let already_archived = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                == Some(ARCHIVE_DIR);
            if already_archived {
                continue;
            }

            let archive_dir = self.root.join(&ticket.service).join(ARCHIVE_DIR);
            if let Err(e) = fs::create_dir_all(&archive_dir) {
                tracing::warn!("failed to create {}: {e}", archive_dir.display());
                continue;
            }
            let dest = archive_dir.join(format!("{}.md", ticket.key));
            if let Err(e) = fs::rename(path, &dest) {
                tracing::warn!("failed to archive {}: {e}", path.display());
                continue;
            }
            moved += 1;
        }

        Ok(moved)
    }

    /// Move `done`/`cancelled` tickets whose `updated` is older than
    /// `config.local.archive_after_days` into `<service>/archive/`, then
    /// reindex best-effort: one [`Self::list_with_paths`] traversal feeding
    /// [`Self::archive_entries`] (see that method for the move rules
    /// themselves), then a best-effort [`Self::reindex`].
    ///
    /// Still relies on `note::parse` upstream for its own entries: every
    /// `ticket.key` and `ticket.service` this method touches comes from
    /// `self.list_with_paths(..)`, which walks `note::parse` output, and
    /// `note::parse` runs `safe_name::check` on both fields itself, so a
    /// poisoned note is dropped, with a stderr line, by `collect_notes`
    /// before it ever reaches this loop. [`Self::archive_entries`] also
    /// checks both fields itself now, per entry, as the last line of
    /// defense — see that method's doc comment — so this holds even if a
    /// future caller ever hands it entries built some other way.
    pub fn archive_sweep(&self) -> anyhow::Result<usize> {
        let entries = self.list_with_paths(&ListFilter::default());
        let moved = self.archive_entries(&entries)?;

        self.reindex();

        Ok(moved)
    }

    /// List every non-archived ticket in the store and hand it to
    /// [`index::refresh`], which prints each failed write on stderr itself.
    /// Best-effort: a failure never fails the write that triggered it.
    fn reindex(&self) -> index::RefreshReport {
        let tickets = self.list(&ListFilter::default());
        index::refresh(&self.vault_root, &tickets)
    }

    /// The immediate child directory names of `Pipelines/` and `Knowledge/`
    /// in `vault_root` — the pivot that ties a ticket's `service` to a real
    /// project (vision §5: "le nom du service est le pivot... identique au
    /// dossier `Pipelines/<repo>/` du vault").
    pub fn known_services(vault_root: &Path) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for top in [PIPELINES_DIR, KNOWLEDGE_DIR] {
            let Ok(entries) = fs::read_dir(vault_root.join(top)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.starts_with('_') || name.starts_with('.') {
                    continue;
                }
                out.insert(name.to_string());
            }
        }
        out
    }

    /// Refuse a `service` that is either a reserved name — `unrouted`,
    /// `archive`, `.state`, anything starting with `_` or `.`, or anything
    /// containing a path separator — or, absent `allow_unknown`, one that
    /// matches no `Pipelines/<service>` nor `Knowledge/<service>` folder in
    /// the vault. Reserved names are refused **even with** `allow_unknown`:
    /// `unrouted/` is a sink for unroutable Jira mirrors, never a service.
    ///
    /// The shape half of this (starts with `_`/`.`, contains a separator,
    /// contains `:`, control characters, anything `safe_name::check` catches
    /// via the `Path` component invariant) is delegated to
    /// [`safe_name::check`] — this function still raises the same "reserved
    /// name" message for a shape violation as it always has, so its error
    /// text (and the two tests that pin it) do not change.
    pub fn check_service(
        vault_root: &Path,
        service: &str,
        allow_unknown: bool,
    ) -> anyhow::Result<()> {
        if safe_name::check("service", service).is_err()
            || RESERVED_SERVICE_NAMES.contains(&service)
        {
            anyhow::bail!(
                "`{service}` is a reserved name and can never be a service (reserved: {}; also refused: `_`- or `.`-prefixed names and path separators)",
                RESERVED_SERVICE_NAMES.join(", ")
            );
        }
        if allow_unknown {
            return Ok(());
        }
        let known = Self::known_services(vault_root);
        if known.contains(service) {
            return Ok(());
        }
        let listed = if known.is_empty() {
            "none".to_string()
        } else {
            known.into_iter().collect::<Vec<_>>().join(", ")
        };
        anyhow::bail!(
            "unknown service `{service}`: no `Pipelines/{service}` or `Knowledge/{service}` folder in the vault (known services: {listed}); pass --allow-unknown-service to bypass"
        );
    }
}

/// tmp + rename so a crash never leaves a half-written note or state file.
/// Copied verbatim (as a sequence, not an import — this crate cannot depend
/// on `aidp-gateway`) from `write_json` in `aidp-gateway/src/store.rs`.
/// Windows `rename` does not overwrite an existing destination, hence the
/// explicit `remove_file` first.
///
/// `pub(crate)` rather than private: `jira_sync` reuses this exact function
/// to persist `TODO/.state/jira-sync.json` atomically, the same way every
/// ticket note and state file in this module is written.
pub(crate) fn write_atomic(dest: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, content)?;
    if dest.exists() {
        let _ = fs::remove_file(dest);
    }
    fs::rename(&tmp, dest)?;
    Ok(())
}

/// Parse every non-`_`-prefixed `*.md` file directly inside `dir` (no
/// recursion) as a ticket note, appending `(ticket, path)` successes to
/// `out` — carrying the path forward is what lets [`TicketStore::archive_entries`]
/// reuse a note's location instead of re-deriving it with
/// [`TicketStore::path_of`]. A file that fails to parse — bad YAML, missing
/// `aidp: ticket`, anything `note::parse` rejects — is reported once on
/// stderr and skipped, never fatal.
///
/// `status_filter` is a fast-path HINT, applied before the expensive full
/// `note::parse`: when it is non-empty, [`note::peek_status`] reads just the
/// frontmatter's `status` key and, if it can and the peeked status is not in
/// `status_filter`, the file is skipped entirely — no `note::parse` call, no
/// stderr warning, since being out of scope for this call is not an error.
/// A `peek_status` miss (empty filter, `None`, or a match) always falls
/// through to the existing `note::parse` call and its existing
/// success/error handling, unchanged: this hint may only skip WORK, never
/// change which tickets a caller with an empty filter sees, and
/// [`TicketStore::list_with_paths`]'s own post-collection status filter
/// remains the correctness backstop for anything this hint could not peek.
fn collect_notes(dir: &Path, status_filter: &[Status], out: &mut Vec<(Ticket, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") || name.starts_with('_') {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(text) => {
                if !status_filter.is_empty() {
                    if let Some(s) = note::peek_status(&text) {
                        if !status_filter.contains(&s) {
                            continue;
                        }
                    }
                }
                match note::parse(&text) {
                    Ok(parsed) => out.push((parsed.ticket, path.clone())),
                    Err(e) => {
                        tracing::warn!("skipping unreadable ticket note {}: {e}", path.display())
                    }
                }
            }
            Err(e) => tracing::warn!("failed to read {}: {e}", path.display()),
        }
    }
}

/// Scan `dir` (non-recursively; a missing `dir` — e.g. no `archive/` yet —
/// is silently a no-op) for `<prefix>-<n>.md` file names, raising `max_n`
/// to the highest `n` found.
fn scan_max_number(dir: &Path, prefix: &str, max_n: &mut u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let needle = format!("{prefix}-");
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(rest) = stem.strip_prefix(&needle) {
            if let Ok(n) = rest.parse::<u64>() {
                if n > *max_n {
                    *max_n = n;
                }
            }
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CounterFile {
    last: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Autopilot, IssueType, Priority};
    use aidp_testkit::TempWorkspace;

    fn sample_ticket(key: &str, service: &str) -> Ticket {
        let now = chrono::Utc::now();
        Ticket {
            key: key.to_string(),
            source: Source::Local,
            r#type: IssueType::Task,
            summary: format!("Ticket {key}"),
            status: Status::Todo,
            priority: Priority::Medium,
            priority_rank: 0,
            service: service.to_string(),
            template: None,
            track: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            created: now,
            updated: now,
            github: None,
            route: None,
            flow: None,
            milestone: None,
            autopilot: Autopilot::default(),
            tags: Vec::new(),
        }
    }

    /// Write a ticket note straight to disk, bypassing `save` — needed
    /// whenever a test wants to control `updated` itself (`save` always
    /// stamps it to `Utc::now()`).
    fn write_ticket_note(dir: &Path, ticket: &Ticket) {
        fs::create_dir_all(dir).unwrap();
        let mut t = ticket.clone();
        t.recompute_derived();
        let text = note::render(&t, "# body\n", &serde_yaml::Mapping::new());
        fs::write(dir.join(format!("{}.md", t.key)), text).unwrap();
    }

    #[test]
    fn open_creates_todo_state_and_a_default_config() {
        let _workspace = TempWorkspace::new("open-creates");
        let vault = _workspace.path();

        let store = TicketStore::open(vault).unwrap();

        assert!(vault.join("TODO").is_dir());
        assert!(vault.join("TODO").join(".state").is_dir());
        let config_path = vault.join("TODO").join("config.yaml");
        assert!(config_path.is_file());
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            config::default_yaml()
        );
        assert_eq!(*store.config(), Config::default());
    }

    #[test]
    fn open_does_not_overwrite_an_existing_config() {
        let _workspace = TempWorkspace::new("open-preserves");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();
        fs::write(
            vault.join("TODO").join("config.yaml"),
            "local:\n  key_prefix: ZED\n",
        )
        .unwrap();

        let store = TicketStore::open(vault).unwrap();

        assert_eq!(
            fs::read_to_string(vault.join("TODO").join("config.yaml")).unwrap(),
            "local:\n  key_prefix: ZED\n"
        );
        assert_eq!(store.config().local.key_prefix, "ZED");
    }

    #[test]
    fn list_ignores_state_archive_and_underscore_notes() {
        let _workspace = TempWorkspace::new("list-ignores");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        write_ticket_note(&todo.join("svc"), &sample_ticket("LOC-1", "svc"));
        write_ticket_note(
            &todo.join("svc").join("archive"),
            &sample_ticket("LOC-2", "svc"),
        );
        fs::create_dir_all(todo.join("svc")).unwrap();
        fs::write(
            todo.join("svc").join("_svc.md"),
            "generated, must be ignored",
        )
        .unwrap();
        fs::create_dir_all(todo.join(".state")).unwrap();
        fs::write(
            todo.join(".state").join("stray.md"),
            "must never be scanned",
        )
        .unwrap();

        let active = store.list(&ListFilter::default());
        assert_eq!(
            active.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            vec!["LOC-1"]
        );

        let archived_list = store.list(&ListFilter {
            archived: true,
            ..Default::default()
        });
        let mut with_archived: Vec<&str> = archived_list.iter().map(|t| t.key.as_str()).collect();
        with_archived.sort();
        assert_eq!(with_archived, vec!["LOC-1", "LOC-2"]);
    }

    #[test]
    fn list_skips_an_unparsable_note_without_failing() {
        let _workspace = TempWorkspace::new("list-skips-bad");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        write_ticket_note(&todo.join("svc"), &sample_ticket("LOC-1", "svc"));
        fs::write(
            todo.join("svc").join("LOC-BAD.md"),
            "not a ticket note at all\n",
        )
        .unwrap();

        let tickets = store.list(&ListFilter::default());

        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].key, "LOC-1");
    }

    #[test]
    fn list_status_filter_uses_the_peek_fast_path() {
        let _workspace = TempWorkspace::new("list-status-peek");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        write_ticket_note(&todo.join("svc"), &sample_ticket("LOC-1", "svc"));
        let mut done = sample_ticket("LOC-2", "svc");
        done.status = Status::Done;
        write_ticket_note(&todo.join("svc"), &done);

        let todo_only = store.list(&ListFilter {
            status: vec![Status::Todo],
            ..Default::default()
        });
        assert_eq!(
            todo_only.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            vec!["LOC-1"],
            "only the todo ticket must come back for a Todo-only filter"
        );
    }

    #[test]
    fn list_status_filter_excludes_a_malformed_note_by_peek_without_reporting_it() {
        // A note whose frontmatter has a `status` the peek can read, but a
        // deeper field (`priority_rank`) whose value would fail the full
        // `Ticket` deserialize. Filtering it out on the peeked status alone
        // — because it does not match — must never call `note::parse` on it
        // at all, so this is not reported as an unreadable-note error; it is
        // simply out of scope for this call, same as before the peek existed
        // it would have been reported as unparseable *if* the caller had
        // asked for `Done` tickets (which it does not, here).
        let _workspace = TempWorkspace::new("list-status-peek-malformed");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");
        fs::create_dir_all(todo.join("svc")).unwrap();
        fs::write(
            todo.join("svc").join("LOC-BAD.md"),
            concat!(
                "---\n",
                "aidp: ticket\n",
                "key: LOC-BAD\n",
                "source: local\n",
                "type: Task\n",
                "summary: \"Malformed below status\"\n",
                "status: done\n",
                "priority: Medium\n",
                "priority_rank: not-a-number\n",
                "service: svc\n",
                "created: 2026-09-10T08:12:00Z\n",
                "updated: 2026-09-10T08:12:00Z\n",
                "---\n",
                "# LOC-BAD\n",
            ),
        )
        .unwrap();
        write_ticket_note(&todo.join("svc"), &sample_ticket("LOC-1", "svc"));

        let todo_only = store.list(&ListFilter {
            status: vec![Status::Todo],
            ..Default::default()
        });
        assert_eq!(
            todo_only.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            vec!["LOC-1"],
            "the malformed done-status note must be silently excluded by the peek, \
             never reach note::parse, and never appear in a Todo-only listing"
        );
    }

    #[test]
    fn save_is_atomic_and_bumps_updated() {
        let _workspace = TempWorkspace::new("save-atomic");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let mut ticket = sample_ticket("LOC-1", "svc");
        let old_updated = ticket.updated;
        std::thread::sleep(std::time::Duration::from_millis(5));

        let path = store
            .save(&mut ticket, "# body\n", &serde_yaml::Mapping::new())
            .unwrap();

        assert!(path.is_file());
        assert!(ticket.updated > old_updated);

        let leftovers: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no leftover tmp file: {leftovers:?}");

        assert!(vault.join("TODO").join("_TODO.md").is_file());
        assert!(vault.join("TODO").join("svc").join("_svc.md").is_file());
    }

    #[test]
    fn allocate_key_survives_a_deleted_counter() {
        let _workspace = TempWorkspace::new("allocate-survives");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();

        let k1 = store.allocate_key().unwrap();
        assert_eq!(k1, "LOC-1");
        let mut t1 = sample_ticket(&k1, "svc");
        store
            .save(&mut t1, "# body\n", &serde_yaml::Mapping::new())
            .unwrap();

        let k2 = store.allocate_key().unwrap();
        assert_eq!(k2, "LOC-2");
        let mut t2 = sample_ticket(&k2, "svc");
        store
            .save(&mut t2, "# body\n", &serde_yaml::Mapping::new())
            .unwrap();

        fs::remove_file(vault.join("TODO").join(".state").join("counter.json")).unwrap();

        let k3 = store.allocate_key().unwrap();
        assert_eq!(
            k3, "LOC-3",
            "must resume from the existing LOC-2 note, not restart at LOC-1"
        );
    }

    #[test]
    fn move_to_service_moves_the_file_and_reindexes_both() {
        let _workspace = TempWorkspace::new("move-service");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let mut ticket = sample_ticket("LOC-1", "svc-a");
        store
            .save(&mut ticket, "# body\n", &serde_yaml::Mapping::new())
            .unwrap();

        let new_path = store.move_to_service("LOC-1", "svc-b").unwrap();

        assert!(new_path.ends_with(Path::new("svc-b").join("LOC-1.md")));
        assert!(!vault.join("TODO").join("svc-a").join("LOC-1.md").exists());
        assert!(vault.join("TODO").join("svc-b").join("LOC-1.md").is_file());

        let (moved, _, _) = store.get("LOC-1").unwrap();
        assert_eq!(moved.service, "svc-b");
        assert!(moved.tags.contains(&"service/svc-b".to_string()));

        assert!(vault.join("TODO").join("svc-a").join("_svc-a.md").is_file());
        assert!(vault.join("TODO").join("svc-b").join("_svc-b.md").is_file());
    }

    #[test]
    fn archive_sweep_moves_old_done_and_cancelled() {
        let _workspace = TempWorkspace::new("archive-sweep");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        let mut old_done = sample_ticket("LOC-1", "svc");
        old_done.status = Status::Done;
        old_done.updated = chrono::Utc::now() - chrono::Duration::days(60);
        write_ticket_note(&todo.join("svc"), &old_done);

        let mut old_cancelled = sample_ticket("LOC-2", "svc");
        old_cancelled.status = Status::Cancelled;
        old_cancelled.updated = chrono::Utc::now() - chrono::Duration::days(60);
        write_ticket_note(&todo.join("svc"), &old_cancelled);

        let recent_done = {
            let mut t = sample_ticket("LOC-3", "svc");
            t.status = Status::Done;
            t
        };
        write_ticket_note(&todo.join("svc"), &recent_done);

        let mut old_todo = sample_ticket("LOC-4", "svc");
        old_todo.updated = chrono::Utc::now() - chrono::Duration::days(60);
        write_ticket_note(&todo.join("svc"), &old_todo);

        let moved = store.archive_sweep().unwrap();

        assert_eq!(moved, 2);
        assert!(todo.join("svc").join("archive").join("LOC-1.md").is_file());
        assert!(todo.join("svc").join("archive").join("LOC-2.md").is_file());
        assert!(!todo.join("svc").join("LOC-1.md").exists());
        assert!(!todo.join("svc").join("LOC-2.md").exists());
        assert!(
            todo.join("svc").join("LOC-3.md").is_file(),
            "a recently-done ticket must stay active"
        );
        assert!(
            todo.join("svc").join("LOC-4.md").is_file(),
            "an old but still-todo ticket must never be swept"
        );
    }

    /// An already-archived, old, `done` note — the shape `cmd_reindex`
    /// hands `archive_entries` via an `archived: true` listing — must be
    /// left alone: not moved (there is nowhere else to rename it to that
    /// wouldn't collide with itself), not errored, and not counted.
    #[test]
    fn archive_entries_skips_already_archived_notes() {
        let _workspace = TempWorkspace::new("archive-entries-skip-archived");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        let mut old_done = sample_ticket("LOC-1", "svc");
        old_done.status = Status::Done;
        old_done.updated = chrono::Utc::now() - chrono::Duration::days(60);
        let archive_dir = todo.join("svc").join("archive");
        write_ticket_note(&archive_dir, &old_done);
        let path = archive_dir.join("LOC-1.md");
        let before = fs::read_to_string(&path).unwrap();

        let moved = store.archive_entries(&[(old_done, path.clone())]).unwrap();

        assert_eq!(moved, 0, "an already-archived note must not be counted");
        assert!(path.is_file(), "the note must stay exactly where it was");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "an already-archived note must not be rewritten"
        );
    }

    /// Without the `safe_name::check` calls added to `archive_entries`'
    /// loop body, an entry whose `key` was poisoned by a hand-edited
    /// frontmatter — bypassing `note::parse`, which is what every real
    /// caller of this method goes through today — would have its
    /// `<service>/archive/<key>.md` destination built straight from that
    /// key and handed to `fs::rename`. This test builds such a `Ticket`
    /// directly, the same way `save_refuses_a_poisoned_key_or_service`
    /// does, to prove `archive_entries` itself refuses it.
    #[test]
    fn archive_entries_skips_an_entry_whose_key_is_not_a_safe_component() {
        let _workspace = TempWorkspace::new("archive-entries-unsafe-key");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();
        let todo = vault.join("TODO");

        let mut poisoned = sample_ticket("../evil", "svc");
        poisoned.status = Status::Done;
        poisoned.updated = chrono::Utc::now() - chrono::Duration::days(60);

        // Plant the note under a safe filename of its own — `poisoned.key`
        // is exactly what must never reach `fs::rename`, so the fixture
        // can't be written via `write_ticket_note`, which derives the
        // on-disk filename from that same poisoned key.
        let dir = todo.join("svc");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("planted.md");
        let mut on_disk = poisoned.clone();
        on_disk.recompute_derived();
        fs::write(
            &path,
            note::render(&on_disk, "# body\n", &serde_yaml::Mapping::new()),
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();

        let moved = store.archive_entries(&[(poisoned, path.clone())]).unwrap();

        assert_eq!(
            moved, 0,
            "an entry whose key is not a safe path component must not be archived"
        );
        assert!(path.is_file(), "the note must stay exactly where it was");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "the note must not be rewritten or moved"
        );
    }

    #[test]
    fn check_service_refuses_unknown_and_accepts_with_allow_unknown() {
        let _workspace = TempWorkspace::new("check-service-unknown");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("Pipelines").join("aidp-viewer")).unwrap();
        fs::create_dir_all(vault.join("Knowledge").join("cheat-alerts-service")).unwrap();

        assert!(TicketStore::check_service(vault, "aidp-viewer", false).is_ok());
        assert!(TicketStore::check_service(vault, "cheat-alerts-service", false).is_ok());

        let err = TicketStore::check_service(vault, "totally-unknown", false).unwrap_err();
        assert!(err.to_string().contains("aidp-viewer"));
        assert!(err.to_string().contains("cheat-alerts-service"));

        assert!(TicketStore::check_service(vault, "totally-unknown", true).is_ok());
    }

    #[test]
    fn check_service_refuses_unrouted_even_with_allow_unknown() {
        let _workspace = TempWorkspace::new("check-service-reserved");
        let vault = _workspace.path();

        for name in ["unrouted", "archive", ".state", "_hidden"] {
            let err = TicketStore::check_service(vault, name, true).unwrap_err();
            assert!(
                err.to_string().contains("reserved"),
                "{name}: unexpected message: {err}"
            );
        }

        let err = TicketStore::check_service(vault, "svc/sub", true).unwrap_err();
        assert!(err.to_string().contains("reserved"));
    }

    /// Without the `safe_name::check` call in `path_of`, a key of the form
    /// `../../<dir>/planted` walks back out of `TODO/<service>/` and resolves
    /// to a real file planted outside the store root — this test plants that
    /// exact file and proves `path_of`/`get` no longer follow it there.
    #[test]
    fn path_of_refuses_a_traversing_key() {
        let _workspace = TempWorkspace::new("path-of-traversal");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();

        // A real service folder to give `path_of`'s scan somewhere to start
        // its traversal from — otherwise the scan simply finds no
        // directories at all and the test would prove nothing.
        fs::create_dir_all(store.root().join("svc")).unwrap();

        // A valid ticket-shaped note, planted OUTSIDE the store root: in the
        // OS temp dir that is `vault`'s own parent, not under `vault/TODO/`.
        let outside_dir = vault.parent().unwrap().join("path-of-traversal-planted");
        write_ticket_note(&outside_dir, &sample_ticket("planted", "svc"));

        // From `<vault>/TODO/svc/`, `../../../<outside_dir_name>/planted.md`
        // resolves exactly to the planted note above.
        let traversal_key = format!(
            "../../../{}/planted",
            outside_dir.file_name().unwrap().to_str().unwrap()
        );

        assert!(
            store.path_of(&traversal_key).is_none(),
            "a traversing key must never resolve to a path, even though a note \
             with that filename really exists outside the store root"
        );
        assert!(
            store.get(&traversal_key).is_err(),
            "get must refuse a traversing key with an explicit error"
        );
    }

    /// Without the `safe_name::check` calls in `save`, a `Ticket` whose
    /// `key` or `service` was poisoned by a hand-edited frontmatter (see
    /// `parse_refuses_a_traversing_key_in_frontmatter` in `note.rs` for how
    /// such a `Ticket` would normally never even get this far) writes
    /// straight through `fs::create_dir_all`/`write_atomic` to a path
    /// outside `<vault>/TODO/`. This test builds such a `Ticket` directly —
    /// bypassing `note::parse` entirely — to prove `save` itself refuses it.
    #[test]
    fn save_refuses_a_poisoned_key_or_service() {
        let _workspace = TempWorkspace::new("save-poisoned");
        let vault = _workspace.path();
        let store = TicketStore::open(vault).unwrap();

        let mut poisoned_key = sample_ticket("../../escaped", "svc");
        let err = store
            .save(&mut poisoned_key, "# body\n", &serde_yaml::Mapping::new())
            .unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "error should name the offending field: {err}"
        );
        assert!(
            !vault.join("escaped.md").exists(),
            "save must never write outside TODO/ for a poisoned key"
        );

        let mut poisoned_service = sample_ticket("LOC-1", "../..");
        let err = store
            .save(
                &mut poisoned_service,
                "# body\n",
                &serde_yaml::Mapping::new(),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("service"),
            "error should name the offending field: {err}"
        );
        assert!(
            !vault.parent().unwrap().join("LOC-1.md").exists(),
            "save must never write outside the vault for a poisoned service"
        );
    }
}
