//! `TODO/config.yaml` (dispatcher/budget/local settings, vision §8) and
//! `TODO/routing.yaml` (service -> template routing, vision §5.3 / lot 7a's
//! "Template par service"). Both loaders are tolerant: a missing, empty, or
//! partial file yields defaults for whatever is absent, and a syntactically
//! invalid file is reported on stderr rather than failing the caller.
//!
//! `github` names the repositories whose issues are mirrored (see
//! [`crate::github_sync`]) and the state labels written back on each
//! transition. Every section is modelled in full, matching
//! [`Config::default`].

use crate::models::constants::github::{
    DEFAULT_KEY_PREFIX, DEFAULT_LABEL_PREFIX, DEFAULT_LIMIT, DEFAULT_SYNC_MINUTES,
    NEEDS_REFINE_REASON, NEEDS_ROUTING_REASON, REPO_SEPARATOR, SKIP_REASON,
    STATUS_REASON_SEPARATOR,
};
use crate::models::enums::dispatcher_launch::DispatcherLaunch;
use crate::models::enums::state_label::StateLabel;
use crate::models::enums::BudgetRoute;
use crate::models::token_amount::TokenAmount;
use crate::models::track::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Default cap on live worktrees per project.
pub const DEFAULT_MAX_WORKTREES_PER_PROJECT: usize = 3;

/// Glob of the per-developer Claude settings copied into a new worktree.
pub const WORKTREE_COPY_CLAUDE_SETTINGS: &str = ".claude/settings.local.json";

/// Glob of the aidp connection configs copied into a new worktree.
pub const WORKTREE_COPY_AIDP_JSON: &str = ".aidp/*.json";

/// Autopilot dispatcher settings (read from lot 9a on; `slots` and
/// `permission_mode` are the ones the dispatcher tick actually consults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DispatcherConfig {
    #[serde(default = "DispatcherConfig::default_slots")]
    pub slots: u32,
    /// Test lane: test flows of one project at once (they share its main checkout and Unity
    /// editor, so the default is 1). Dev flows are bounded by `slots`.
    #[serde(default = "DispatcherConfig::default_test_slots")]
    pub test_slots: u32,
    #[serde(default = "DispatcherConfig::default_tick_seconds")]
    pub tick_seconds: u32,
    #[serde(default = "DispatcherConfig::default_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "DispatcherConfig::default_model")]
    pub model: String,
    #[serde(default = "DispatcherConfig::default_auto_approve_gates")]
    pub auto_approve_gates: Vec<String>,
    #[serde(default = "DispatcherConfig::default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "DispatcherConfig::default_gate_timeout_hours")]
    pub gate_timeout_hours: u32,
    #[serde(default)]
    pub paused: bool,
    /// Most tickets of one project that may own a worktree at once.
    #[serde(default = "DispatcherConfig::default_max_worktrees_per_project")]
    pub max_worktrees_per_project: usize,
    /// Untracked files copied from the main checkout into a new worktree.
    #[serde(default = "DispatcherConfig::default_worktree_copy")]
    pub worktree_copy: Vec<String>,
    /// How a queued ticket is launched: a pipeline session or a flow run.
    #[serde(default)]
    pub launch: DispatcherLaunch,
    /// Night mode: heavy (opus) runs only inside the night window.
    #[serde(default)]
    pub night: NightConfig,
    /// Stops every launch once the 7-day window reaches this percentage (`85` = 85 %).
    #[serde(default)]
    pub stop_at_7d_percent: Option<f64>,
}

/// Night-mode settings: when the dispatcher may start the heavy runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NightConfig {
    pub enabled: bool,
    /// Local `HH:MM` the night window opens at.
    pub start: String,
    /// Local `HH:MM` the night window closes at; earlier than `start` wraps past midnight.
    pub end: String,
    /// Heavy runs launch only in the first minutes after a 5-hour window reset.
    pub heavy_after_reset: bool,
    /// How many minutes after a reset still count as "just after".
    pub heavy_window_minutes: u32,
}

impl Default for NightConfig {
    fn default() -> Self {
        NightConfig {
            enabled: false,
            start: "22:00".to_string(),
            end: "07:00".to_string(),
            heavy_after_reset: true,
            heavy_window_minutes: 30,
        }
    }
}

impl DispatcherConfig {
    fn default_max_worktrees_per_project() -> usize {
        DEFAULT_MAX_WORKTREES_PER_PROJECT
    }
    fn default_worktree_copy() -> Vec<String> {
        vec![
            WORKTREE_COPY_CLAUDE_SETTINGS.to_string(),
            WORKTREE_COPY_AIDP_JSON.to_string(),
        ]
    }
    fn default_slots() -> u32 {
        3
    }
    fn default_test_slots() -> u32 {
        1
    }
    fn default_tick_seconds() -> u32 {
        60
    }
    fn default_permission_mode() -> String {
        "bypassPermissions".to_string()
    }
    fn default_model() -> String {
        "claude-sonnet-5".to_string()
    }
    fn default_auto_approve_gates() -> Vec<String> {
        vec!["pr".to_string()]
    }
    fn default_max_attempts() -> u32 {
        2
    }
    fn default_gate_timeout_hours() -> u32 {
        12
    }
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        DispatcherConfig {
            slots: Self::default_slots(),
            test_slots: Self::default_test_slots(),
            tick_seconds: Self::default_tick_seconds(),
            permission_mode: Self::default_permission_mode(),
            model: Self::default_model(),
            auto_approve_gates: Self::default_auto_approve_gates(),
            max_attempts: Self::default_max_attempts(),
            gate_timeout_hours: Self::default_gate_timeout_hours(),
            paused: false,
            max_worktrees_per_project: Self::default_max_worktrees_per_project(),
            worktree_copy: Self::default_worktree_copy(),
            launch: DispatcherLaunch::default(),
            night: NightConfig::default(),
            stop_at_7d_percent: None,
        }
    }
}

/// Token-budget guard settings, read for real as of lot 10a (the dispatcher's
/// budget guard).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    #[serde(default = "BudgetConfig::default_max_utilization_5h")]
    pub max_utilization_5h: f64,
    #[serde(default = "BudgetConfig::default_max_utilization_7d")]
    pub max_utilization_7d: f64,
    #[serde(default = "BudgetConfig::default_reserve_per_run_5h")]
    pub reserve_per_run_5h: f64,
    #[serde(default = "BudgetConfig::default_reserve_per_run_7d")]
    pub reserve_per_run_7d: f64,
    #[serde(default = "BudgetConfig::default_unknown_policy")]
    pub unknown_policy: String,
    #[serde(default = "BudgetConfig::default_learn")]
    pub learn: bool,
    /// Case-insensitive substrings matched against a finished session's last
    /// text to decide that the session died of a usage limit rather than of
    /// a real failure. Kept in `config.yaml` (not hardcoded) so a wording
    /// change in the `claude` CLI can be absorbed by a hand-edit instead of
    /// a release.
    #[serde(default = "BudgetConfig::default_rate_limit_patterns")]
    pub rate_limit_patterns: Vec<String>,
    /// Token ceiling of one ticket, per route.
    #[serde(default)]
    pub per_ticket: PerTicketBudget,
    /// Pixellab credit guard.
    #[serde(default)]
    pub pixellab: PixellabBudget,
}

/// Token ceiling per ticket and route; `0` disables the ceiling of that route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerTicketBudget {
    pub code: TokenAmount,
    pub art: TokenAmount,
    pub map: TokenAmount,
    pub doc: TokenAmount,
    /// Ceiling of a ticket whose route is unknown.
    pub default: TokenAmount,
}

impl Default for PerTicketBudget {
    fn default() -> Self {
        PerTicketBudget {
            code: TokenAmount(3_000_000),
            art: TokenAmount(1_500_000),
            map: TokenAmount(2_000_000),
            doc: TokenAmount(500_000),
            default: TokenAmount(2_000_000),
        }
    }
}

impl PerTicketBudget {
    /// The ceiling, in tokens, of a ticket of `route` (`0` = unlimited).
    pub fn limit_for(&self, route: Option<BudgetRoute>) -> u64 {
        match route {
            Some(BudgetRoute::Code) => self.code.0,
            Some(BudgetRoute::Art) => self.art.0,
            Some(BudgetRoute::Map) => self.map.0,
            Some(BudgetRoute::Doc) => self.doc.0,
            None => self.default.0,
        }
    }
}

/// Pixellab credit guard: the balance the `art` route must keep, and the generations one ticket may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PixellabBudget {
    /// No `art` launch while the last known balance is below this value (`0` = no reserve).
    pub min_reserve: f64,
    /// Generations one ticket may use before its flow asks to continue (`0` = unlimited).
    pub per_ticket: u32,
}

impl Default for PixellabBudget {
    fn default() -> Self {
        PixellabBudget {
            min_reserve: 0.0,
            per_ticket: 10,
        }
    }
}

/// Model policy: route, then stage id, then the tier that stage runs on.
pub use aidp_core::model_policy::ModelPolicy;

impl BudgetConfig {
    fn default_max_utilization_5h() -> f64 {
        0.80
    }
    fn default_max_utilization_7d() -> f64 {
        0.90
    }
    fn default_reserve_per_run_5h() -> f64 {
        0.10
    }
    fn default_reserve_per_run_7d() -> f64 {
        0.02
    }
    fn default_unknown_policy() -> String {
        "conservative".to_string()
    }
    fn default_learn() -> bool {
        true
    }
    fn default_rate_limit_patterns() -> Vec<String> {
        vec![
            "rate limit".to_string(),
            "usage limit".to_string(),
            "hit your limit".to_string(),
            "429".to_string(),
            "rate_limit".to_string(),
        ]
    }
}

impl Default for BudgetConfig {
    fn default() -> Self {
        BudgetConfig {
            max_utilization_5h: Self::default_max_utilization_5h(),
            max_utilization_7d: Self::default_max_utilization_7d(),
            reserve_per_run_5h: Self::default_reserve_per_run_5h(),
            reserve_per_run_7d: Self::default_reserve_per_run_7d(),
            unknown_policy: Self::default_unknown_policy(),
            learn: Self::default_learn(),
            rate_limit_patterns: Self::default_rate_limit_patterns(),
            per_ticket: PerTicketBudget::default(),
            pixellab: PixellabBudget::default(),
        }
    }
}

/// Local-ticket settings — the only section this lot actually reads (key
/// allocation and the archive sweep).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalConfig {
    #[serde(default = "LocalConfig::default_key_prefix")]
    pub key_prefix: String,
    #[serde(default = "LocalConfig::default_archive_after_days")]
    pub archive_after_days: u32,
}

impl LocalConfig {
    fn default_key_prefix() -> String {
        "LOC".to_string()
    }
    fn default_archive_after_days() -> u32 {
        30
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        LocalConfig {
            key_prefix: Self::default_key_prefix(),
            archive_after_days: Self::default_archive_after_days(),
        }
    }
}

/// Pipeline doctor settings: the daemon investigates a broken flow or run by itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DoctorConfig {
    #[serde(default = "DoctorConfig::default_enabled")]
    pub enabled: bool,
    /// Minutes a step may run without a live run, session or process before it counts as stuck.
    #[serde(default = "DoctorConfig::default_stuck_minutes")]
    pub stuck_minutes: u32,
    /// Investigations one ticket may get over a sliding 24 hours.
    #[serde(default = "DoctorConfig::default_max_attempts_per_ticket")]
    pub max_attempts_per_ticket: u32,
    /// Key of the dedicated harness issue (for example `GH-1`): bugs of a failure shared by several tickets are raised under it, never under a game ticket. Unset: the doctor asks a human.
    #[serde(default)]
    pub harness_ticket: Option<String>,
}

impl DoctorConfig {
    fn default_enabled() -> bool {
        true
    }
    fn default_stuck_minutes() -> u32 {
        10
    }
    fn default_max_attempts_per_ticket() -> u32 {
        2
    }
}

impl Default for DoctorConfig {
    fn default() -> Self {
        DoctorConfig {
            enabled: Self::default_enabled(),
            stuck_minutes: Self::default_stuck_minutes(),
            max_attempts_per_ticket: Self::default_max_attempts_per_ticket(),
            harness_ticket: None,
        }
    }
}

/// One GitHub repository whose open issues are mirrored under `TODO/<service>/`.
///
/// Written either as a bare `owner/name` string or as a mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "GithubRepoEntry")]
pub struct GithubRepo {
    /// Repository as `owner/name`.
    pub repo: String,
    /// Service (gateway project and `TODO/` folder) of the mirrors; the repository name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Prefix of the mirror keys, `GH` by default: issue 12 is `GH-12`.
    pub key_prefix: String,
}

impl GithubRepo {
    /// A repository with the default service and key prefix.
    pub fn named(repo: &str) -> GithubRepo {
        GithubRepo {
            repo: repo.to_string(),
            service: None,
            key_prefix: DEFAULT_KEY_PREFIX.to_string(),
        }
    }

    /// The service the mirrors are filed under: `service`, else the repository name.
    pub fn service(&self) -> String {
        if let Some(service) = self.service.as_deref().filter(|s| !s.trim().is_empty()) {
            return service.to_string();
        }
        self.repo
            .rsplit(REPO_SEPARATOR)
            .next()
            .unwrap_or(&self.repo)
            .to_string()
    }

    /// The mirror key of issue `number`, e.g. `GH-12`.
    pub fn key_of(&self, number: u64) -> String {
        format!("{}-{number}", self.key_prefix)
    }
}

/// The two spellings of a `github.repos` entry.
#[derive(Deserialize)]
#[serde(untagged)]
enum GithubRepoEntry {
    Name(String),
    Full {
        repo: String,
        #[serde(default)]
        service: Option<String>,
        #[serde(default)]
        key_prefix: Option<String>,
    },
}

impl From<GithubRepoEntry> for GithubRepo {
    fn from(entry: GithubRepoEntry) -> Self {
        match entry {
            GithubRepoEntry::Name(repo) => GithubRepo::named(&repo),
            GithubRepoEntry::Full {
                repo,
                service,
                key_prefix,
            } => GithubRepo {
                repo,
                service,
                key_prefix: key_prefix
                    .filter(|prefix| !prefix.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_KEY_PREFIX.to_string()),
            },
        }
    }
}

/// GitHub issues as the ticket source, and the state labels written back on each transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GithubConfig {
    /// Repositories whose open issues are mirrored. Empty: the GitHub source is off.
    #[serde(default)]
    pub repos: Vec<GithubRepo>,
    /// Minutes between two daemon syncs; `0` keeps only the manual sync.
    #[serde(default = "GithubConfig::default_sync_minutes")]
    pub sync_minutes: u32,
    /// Most open issues read per repository and sync.
    #[serde(default = "GithubConfig::default_limit")]
    pub limit: u32,
    /// Read GitHub's native issue dependencies (one call per open issue) besides the body.
    #[serde(default = "GithubConfig::default_native_dependencies")]
    pub native_dependencies: bool,
    /// A complete issue without a state label waits for `aidp:ready` (`blocked: not-ready`).
    #[serde(default = "GithubConfig::default_ready_required")]
    pub ready_required: bool,
    /// Prefix shared by every state label; at most one of them sits on an issue.
    #[serde(default = "GithubConfig::default_label_prefix")]
    pub label_prefix: String,
    /// Ticket status, or `blocked:<reason>`, to the state label it shows on GitHub.
    #[serde(default = "GithubConfig::default_status_labels")]
    pub status_labels: BTreeMap<String, String>,
    /// Labels a flow sets while the ticket stays `in-progress`.
    #[serde(default = "GithubConfig::default_phase_labels")]
    pub phase_labels: Vec<String>,
}

impl GithubConfig {
    fn default_sync_minutes() -> u32 {
        DEFAULT_SYNC_MINUTES
    }
    fn default_limit() -> u32 {
        DEFAULT_LIMIT
    }
    fn default_native_dependencies() -> bool {
        true
    }
    fn default_ready_required() -> bool {
        true
    }
    fn default_label_prefix() -> String {
        DEFAULT_LABEL_PREFIX.to_string()
    }
    fn default_status_labels() -> BTreeMap<String, String> {
        let label = |state: StateLabel| state.label(DEFAULT_LABEL_PREFIX);
        let blocked = |reason: &str| format!("blocked{STATUS_REASON_SEPARATOR}{reason}");
        BTreeMap::from([
            ("todo".to_string(), label(StateLabel::Ready)),
            ("queued".to_string(), label(StateLabel::Queued)),
            ("in-progress".to_string(), label(StateLabel::InProgress)),
            ("in-review".to_string(), label(StateLabel::AwaitingMerge)),
            ("done".to_string(), label(StateLabel::Done)),
            ("blocked".to_string(), label(StateLabel::Blocked)),
            (blocked(NEEDS_REFINE_REASON), label(StateLabel::Triage)),
            (blocked(NEEDS_ROUTING_REASON), label(StateLabel::Triage)),
            (blocked(SKIP_REASON), label(StateLabel::Skip)),
        ])
    }
    fn default_phase_labels() -> Vec<String> {
        StateLabel::PHASES
            .iter()
            .map(|state| state.label(DEFAULT_LABEL_PREFIX))
            .collect()
    }

    /// The configured repository whose `repo` equals `repo`, ignoring case.
    pub fn repo(&self, repo: &str) -> Option<&GithubRepo> {
        self.repos
            .iter()
            .find(|candidate| candidate.repo.eq_ignore_ascii_case(repo))
    }

    /// The label a state suffix (`review`) or a full label name (`aidp:review`) designates,
    /// when it is one of the configured state labels.
    pub fn state_label(&self, state: &str) -> Option<String> {
        let wanted = state.trim();
        let full = if wanted.starts_with(&self.label_prefix) {
            wanted.to_string()
        } else {
            format!("{}{wanted}", self.label_prefix)
        };
        self.known_state_labels()
            .into_iter()
            .find(|label| label.eq_ignore_ascii_case(&full))
    }

    /// Every configured state label: `status_labels` values then `phase_labels`, deduplicated.
    pub fn known_state_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = Vec::new();
        let built_in = [StateLabel::ToTest.label(&self.label_prefix)];
        for label in self
            .status_labels
            .values()
            .chain(self.phase_labels.iter())
            .chain(built_in.iter())
        {
            if !labels.iter().any(|known| known.eq_ignore_ascii_case(label)) {
                labels.push(label.clone());
            }
        }
        labels
    }
}

impl Default for GithubConfig {
    fn default() -> Self {
        GithubConfig {
            repos: Vec::new(),
            sync_minutes: Self::default_sync_minutes(),
            limit: Self::default_limit(),
            native_dependencies: Self::default_native_dependencies(),
            ready_required: Self::default_ready_required(),
            label_prefix: Self::default_label_prefix(),
            status_labels: Self::default_status_labels(),
            phase_labels: Self::default_phase_labels(),
        }
    }
}

/// `TODO/config.yaml` in full (vision §8). `#[serde(default)]` sits on every
/// section (container-level) and on every leaf field (field-level,
/// deliberately redundant, the same double-belt convention as
/// `model::Autopilot`): a missing section, a section present but missing one
/// field, an empty file, or a missing file all yield the documented defaults
/// for whatever is absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dispatcher: DispatcherConfig,
    pub budget: BudgetConfig,
    pub local: LocalConfig,
    pub github: GithubConfig,
    /// `models: { <route>: { <stageId>: haiku|sonnet|opus } }`, applied when a pipeline is compiled.
    pub models: ModelPolicy,
    /// The pipeline doctor: automatic investigation of broken flows and runs.
    pub doctor: DoctorConfig,
}

/// Load `<vault>/TODO/config.yaml`. A missing file, an empty file, or a file
/// that only sets a few keys all yield [`Config::default`] for whatever is
/// absent. A syntactically invalid file (bad YAML) is reported on stderr and
/// the full default `Config` is returned — never fatal, so a hand-edit typo
/// degrades the dispatcher to "run with defaults" rather than crashing every
/// `aidp ticket` invocation.
pub fn load(vault_root: &Path) -> Config {
    let file = vault_root.join("TODO").join("config.yaml");
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(_) => return Config::default(),
    };
    if text.trim().is_empty() {
        return Config::default();
    }
    match serde_yaml::from_str(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("invalid {}: {e}; falling back to defaults", file.display());
            Config::default()
        }
    }
}

/// The commented `config.yaml` the store writes when the file is absent —
/// every key of vision §8, one short comment per group, the same values as
/// [`Config::default`]. Kept honest by the test
/// `default_yaml_parses_into_the_default_config`: this text must parse to
/// exactly `Config::default()`.
pub fn default_yaml() -> &'static str {
    r#"# aidp ticket dispatcher configuration (vision §8). Edited by hand, read on
# every tick; a missing or partial file behaves exactly like the defaults
# shown here.

dispatcher:
  slots: 3                      # dev lane: flows (worktrees) running at once, across every service
  test_slots: 1                 # test lane: test flows at once per project (main checkout, Unity editor)
  tick_seconds: 60
  permission_mode: bypassPermissions
  model: claude-sonnet-5
  auto_approve_gates: [pr]      # "*" approves every gate; anything else blocks the ticket (blocked: gate:<stage>)
  max_attempts: 2
  gate_timeout_hours: 12
  paused: false
  launch: session               # session | flow: how a queued ticket is launched
  night:                        # heavy (opus) runs only in the night window
    enabled: false
    start: "22:00"              # local time
    end: "07:00"
    heavy_after_reset: true     # ...and only just after a 5h window reset
    heavy_window_minutes: 30
  # stop_at_7d_percent: 85      # no launch once the 7-day window reaches 85 %
  max_worktrees_per_project: 3  # tickets of one project that may hold a worktree at once
  worktree_copy:                # untracked files copied into each new worktree
    - ".claude/settings.local.json"
    - ".aidp/*.json"

budget:
  max_utilization_5h: 0.80
  max_utilization_7d: 0.90
  reserve_per_run_5h: 0.10       # estimated cost of one dev run on the 5h window
  reserve_per_run_7d: 0.02
  unknown_policy: conservative   # no snapshot yet: run one session at a time
  learn: true                    # refine the reserve from observed deltas
  rate_limit_patterns:           # substrings that mark a session killed by the usage limit
    - "rate limit"
    - "usage limit"
    - "hit your limit"
    - "429"
    - "rate_limit"
  per_ticket:                    # token ceiling of one ticket (3M, 500k or a number)
    code: 3M
    art: 1.5M
    map: 2M
    doc: 0.5M
    default: 2M
  pixellab:
    min_reserve: 0               # no art launch below this balance (0 = off)
    per_ticket: 10               # generations one ticket may use

local:
  key_prefix: LOC
  archive_after_days: 30         # done/cancelled moved to <service>/archive/

github:                          # GitHub issues as the ticket source (aidp ticket sync)
  repos: []                      # e.g. [Litybe/PixelHood] or [{ repo: Litybe/PixelHood, service: PixelHood, key_prefix: GH }]
  sync_minutes: 10               # daemon sync rhythm; 0 = only the manual sync
  limit: 200                     # open issues read per repository and sync
  native_dependencies: true      # also read GitHub's blocked_by dependencies (one call per issue)
  ready_required: true           # a complete issue waits for aidp:ready (blocked: not-ready)
  label_prefix: "aidp:"          # state label family: one label at a time on an issue
  status_labels:                 # ticket status (or blocked:<reason>) -> state label
    todo: "aidp:ready"
    queued: "aidp:queued"
    in-progress: "aidp:in-progress"
    in-review: "aidp:awaiting-merge"
    done: "aidp:done"
    blocked: "aidp:blocked"
    "blocked:needs-refine": "aidp:triage"
    "blocked:needs-routing": "aidp:triage"
    "blocked:skip": "aidp:skip"
  phase_labels:                  # set by a flow while the ticket stays in-progress
    - "aidp:review"
    - "aidp:fixing"
    - "aidp:to-test"
    - "aidp:testing"

doctor:                          # investigates a failed, escalated or stuck flow by itself
  enabled: true
  stuck_minutes: 10              # a running step with no live run for this long is stuck
  max_attempts_per_ticket: 2     # investigations per ticket over 24 h
  # harness_ticket: GH-1          # issue under which bugs shared by several tickets are raised

# Model policy, applied when a pipeline is compiled (CLI override, then this, then the template).
# models:
#   code: { report: haiku, review: sonnet }
#   art: { review: sonnet }
"#
}

/// One entry of `TODO/routing.yaml`'s `repos:` table, named after a service (a local ticket) or
/// a route target (a GitHub mirror): `template` is the pipeline a session runs, `flow` the
/// delivery flow the dispatcher starts instead when set, `track` the run depth, and `route` the
/// route family a mirror shows (`code`, `art`, `map`, `doc`; the entry name when absent).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoRoute {
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub track: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Flow the dispatcher starts in the test lane once the dev flow reached `to-test`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_flow: Option<String>,
}

/// `routing.yaml`'s top-level `defaults:` — currently just the fallback
/// `track` used by [`crate::routing::route`] when a matched rule's `repos:`
/// entry sets none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingDefaults {
    pub track: Option<Track>,
}

/// `TODO/routing.yaml` (lot 8a, vision §5.3): `rules:` are evaluated first,
/// in order, by [`crate::routing::route`] against the facts of an incoming
/// GitHub issue (or an existing ticket, for `aidp ticket route
/// --from-unrouted`); `repos:` resolves a winning rule's `template`/`track`,
/// and is also what `template_for` (used by `aidp ticket new`/`route`) reads
/// directly.
///
/// Loading is tolerant **per section, and per rule** (see [`load_routing`]):
/// `defaults:` and `repos:` are deserialized independently, so a mistake in
/// one never costs the other; the raw `rules:` value is handed to
/// [`crate::routing::compile_rules`], which validates each entry on its own
/// and drops only the ones that do not compile (unknown criterion, bad
/// `summary_regex`, wrong shape). A hand-written `rules:` from before this
/// lot — whose `match` is a bare string rather than a criteria map — is
/// exactly that case: every such entry is dropped with a stderr warning, and
/// `repos:` (and any other well-formed rule) survives intact. Only a
/// whole-file YAML syntax error falls back to the old behaviour: warn, and
/// return `Routing::default()` (an empty `repos:` too).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Routing {
    #[serde(default)]
    pub defaults: RoutingDefaults,
    #[serde(default)]
    pub rules: Vec<crate::routing::Rule>,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoRoute>,
}

impl Routing {
    /// The template configured for `service`, if any. `None` when the
    /// service has no `repos:` entry, or the entry has no `template:` —
    /// callers (`new`, `route`) then create the ticket with `template: null`.
    pub fn template_for(&self, service: &str) -> Option<&str> {
        self.repos.get(service)?.template.as_deref()
    }

    /// The delivery flow configured for `service` (or route), if any.
    pub fn flow_for(&self, service: &str) -> Option<&str> {
        self.repos.get(service)?.flow.as_deref()
    }

    /// The test flow of a ticket routed to `flow` (family `route`): the entry that names both
    /// first, else any entry that names `flow`.
    pub fn test_flow_for(&self, flow: &str, route: Option<&str>) -> Option<&str> {
        let by_flow = || {
            self.repos
                .values()
                .filter(|entry| entry.flow.as_deref() == Some(flow))
        };
        by_flow()
            .find(|entry| entry.route.as_deref() == route && entry.test_flow.is_some())
            .or_else(|| by_flow().find(|entry| entry.test_flow.is_some()))
            .and_then(|entry| entry.test_flow.as_deref())
    }
}

/// Load `<vault>/TODO/routing.yaml`. A missing or empty file yields an empty
/// [`Routing`] (same "never fatal" contract as [`load`]).
///
/// A present file is first parsed to a generic [`serde_yaml::Value`], then
/// each top-level section is deserialized on its own: `defaults:` and
/// `repos:` each fall back to their own default (plus a stderr warning
/// naming the section) if malformed, and the raw `rules:` value is handed to
/// [`crate::routing::compile_rules`], which drops only the individual rules
/// that do not validate. This is what guarantees a bad or legacy `rules:`
/// entry can never wipe out `repos:` — only a whole-file YAML syntax error
/// (the text does not parse as YAML at all) keeps the old all-or-nothing
/// behaviour: warn, and return `Routing::default()`.
pub fn load_routing(vault_root: &Path) -> Routing {
    let file = vault_root.join("TODO").join("routing.yaml");
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(_) => return Routing::default(),
    };
    if text.trim().is_empty() {
        return Routing::default();
    }

    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "invalid {}: {e}; falling back to an empty routing table",
                file.display()
            );
            return Routing::default();
        }
    };

    let defaults = value
        .get("defaults")
        .cloned()
        .map(|v| {
            serde_yaml::from_value(v).unwrap_or_else(|e| {
                tracing::warn!(
                    "invalid 'defaults' in {}: {e}; using defaults",
                    file.display()
                );
                RoutingDefaults::default()
            })
        })
        .unwrap_or_default();

    let repos = value
        .get("repos")
        .cloned()
        .map(|v| {
            serde_yaml::from_value(v).unwrap_or_else(|e| {
                tracing::warn!("invalid 'repos' in {}: {e}; ignoring it", file.display());
                BTreeMap::new()
            })
        })
        .unwrap_or_default();

    let rules = crate::routing::compile_rules(value.get("rules"));

    Routing {
        defaults,
        rules,
        repos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidp_testkit::TempWorkspace;

    #[test]
    fn defaults_when_file_absent() {
        let _workspace = TempWorkspace::new("absent");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();

        let cfg = load(vault);

        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn invalid_yaml_falls_back_to_defaults_without_panicking() {
        let _workspace = TempWorkspace::new("invalid");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();
        fs::write(vault.join("TODO").join("config.yaml"), "not: [valid: yaml").unwrap();

        let cfg = load(vault);

        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn default_yaml_parses_into_the_default_config() {
        let parsed: Config = serde_yaml::from_str(default_yaml()).expect("default_yaml() parses");

        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn routing_resolves_template_for_a_known_repo() {
        let _workspace = TempWorkspace::new("routing-known");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();
        fs::write(
            vault.join("TODO").join("routing.yaml"),
            "repos:\n  cheat-alerts-service: { template: dev-csharp }\n  aidp-viewer: { template: dev-aidp-viewer }\n",
        )
        .unwrap();

        let routing = load_routing(vault);

        assert_eq!(
            routing.template_for("cheat-alerts-service"),
            Some("dev-csharp")
        );
        assert_eq!(routing.template_for("aidp-viewer"), Some("dev-aidp-viewer"));
        assert_eq!(routing.template_for("unknown-service"), None);
    }

    #[test]
    fn routing_keeps_repos_when_a_rule_is_malformed() {
        // The pre-lot-8a shape: `match` used to be ignored entirely, so a
        // hand-written file could carry it as a plain string. That string no
        // longer deserializes into `RuleCriteria` — the anti-regression
        // guarantee is that this costs the rule its slot, and nothing else.
        let _workspace = TempWorkspace::new("routing-malformed-rule");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();
        fs::write(
            vault.join("TODO").join("routing.yaml"),
            "repos:\n  cheat-alerts-service: { template: dev-csharp }\nrules:\n  - match: \"PIXEL-*\"\n    service: cheat-alerts-service\n",
        )
        .unwrap();

        let routing = load_routing(vault);

        assert_eq!(
            routing.template_for("cheat-alerts-service"),
            Some("dev-csharp")
        );
        assert!(routing.rules.is_empty());
    }

    #[test]
    fn routing_loads_a_well_formed_full_file() {
        let _workspace = TempWorkspace::new("routing-full");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO")).unwrap();
        fs::write(
            vault.join("TODO").join("routing.yaml"),
            "defaults:\n  track: standard\n\
             rules:\n  \
             - match: { component: cheat-alerts-service }\n    service: cheat-alerts-service\n  \
             - match: { label: aidp-viewer }\n    service: aidp-viewer\n\
             repos:\n  \
             cheat-alerts-service: { template: dev-csharp }\n  \
             aidp-viewer: { template: dev-aidp-viewer }\n",
        )
        .unwrap();

        let routing = load_routing(vault);

        assert_eq!(
            routing.defaults.track,
            Some(crate::models::track::Track::Standard)
        );
        assert_eq!(routing.rules.len(), 2);
        assert_eq!(routing.rules[0].service, "cheat-alerts-service");
        assert_eq!(routing.rules[1].service, "aidp-viewer");
        assert_eq!(
            routing.template_for("cheat-alerts-service"),
            Some("dev-csharp")
        );
    }
}

#[cfg(test)]
#[path = "../tests/unit/config_tests.rs"]
mod config_tests;
