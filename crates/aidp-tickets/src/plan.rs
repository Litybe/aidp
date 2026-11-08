//! The dispatcher's decision: given the eligible candidates, what is already
//! running, the budget guard's verdict and the template catalog, decide what
//! to launch next. [`plan`] is the ONLY place this decision is made — both
//! the daemon's tick and `aidp ticket plan` call it, so the two can never
//! disagree.
//!
//! Pure by construction: no filesystem writes, no network, no process
//! spawning, no clock reads beyond what the caller hands in via `running`.
//! The one non-pure corner is [`TemplateCatalog::from_skill_root`], a
//! separate constructor the caller uses to build one of `plan`'s inputs
//! ahead of time — `plan` itself never touches disk.
//!
//! This module must not depend on `aidp-core` or `aidp-gateway` — see this
//! crate's top-level doc comment for why. `TemplateCatalog` is therefore a
//! minimal, hand-rolled read of `templates/*.yaml`, not a reuse of whatever
//! richer template model those crates already have.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::blockers;
use crate::model::{Source, Status, Ticket};
use crate::models::enums::LaunchDetail;
use crate::models::track::Track;

/// Template input carrying a GitHub issue number (issue mode of `dev-unity` and its siblings).
pub const ISSUE_INPUT: &str = "issue";

/// Template input carrying a free-text demand (prompt mode).
pub const PROMPT_INPUT: &str = "prompt";

/// Wire prefix of [`SkipReason::WaitingOn`].
pub const SKIP_WAITING_ON_PREFIX: &str = "waiting-on:";

/// Separator between blocker keys in the wire form of [`SkipReason::WaitingOn`].
pub const WAITING_ON_SEPARATOR: &str = ",";

/// A dispatch session already running, as tracked by the daemon (its
/// session table, not this crate's store). `origin_kind` distinguishes a
/// session the dispatcher itself launched (`"dispatcher"`) from one a human
/// started interactively (e.g. `"ui"`): per vision D4, a UI session still
/// occupies its project (so the dispatcher will not double-launch it) but
/// does NOT consume a dispatcher slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningSlot {
    pub project: String,
    pub origin_kind: String,
    pub ticket_key: Option<String>,
    pub started_at_ms: i64,
}

/// The demand a launch carries into its pipeline run, mirroring the
/// dual-mode `issue` / `prompt` inputs of `dev-unity` and its siblings.
///
/// `plan` has no access to the ticket store, so a `Prompt` demand's
/// `prompt` field is filled in here with an EMPTY STRING as a placeholder —
/// it is the caller's job (the daemon, via `TicketStore::get`) to substitute
/// the ticket's rendered body for it before writing the pipeline's inputs
/// file. This is the one place in this contract a reader is likely to be
/// surprised by, so it is spelled out here rather than only at the call
/// site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Demand {
    /// The GitHub issue the run reads itself (`issue` input).
    Issue {
        number: u64,
    },
    Prompt {
        prompt: String,
        feature: String,
    },
}

/// One ticket the plan decided to launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    pub key: String,
    pub service: String,
    pub template: String,
    pub track: String,
    pub demand: Demand,
    pub manual: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<LaunchDetail>,
    /// Delivery flow the dispatcher starts instead of a session, from the ticket's route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// True for a launch of the test lane (the `test_flow`, in the project's main checkout).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub test: bool,
}

/// Why a candidate was NOT launched this tick.
///
/// [`SkipReason::as_str`] returns the wire strings — a contract shared with
/// the daemon's state file, its HTTP API and the docs. Do not change them
/// without updating all three.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    ProjectBusy,
    NoSlot,
    NeedsRouting,
    NoTemplate,
    TemplateWithoutDemandMode,
    Budget(String),
    Paused,
    WorktreeCap(Vec<String>),
    BranchCheckedOutElsewhere,
    WaitingOn(Vec<String>),
    /// The test lane of the project is full (`dispatcher.test_slots`).
    NoTestSlot,
}

/// Wire slug of [`SkipReason::WorktreeCap`].
pub const SKIP_WORKTREE_CAP: &str = "worktree-cap";

/// Wire slug of [`SkipReason::BranchCheckedOutElsewhere`].
pub const SKIP_BRANCH_CHECKED_OUT_ELSEWHERE: &str = "branch-checked-out-elsewhere";

/// Outcome of the per-project worktree cap for one ticket.
#[derive(Debug, Clone, PartialEq)]
pub enum CapVerdict {
    Allow,
    Cap(Vec<String>),
}

/// Decide whether a project at `alive` worktrees may open one more.
/// A ticket that already owns its worktree (`has_own`) is always allowed.
pub fn cap_verdict(alive: &[String], max: usize, has_own: bool) -> CapVerdict {
    if has_own || alive.len() < max {
        CapVerdict::Allow
    } else {
        CapVerdict::Cap(alive.to_vec())
    }
}

impl SkipReason {
    /// The wire form, e.g. `project-busy` or `budget:<detail>`.
    pub fn as_str(&self) -> String {
        match self {
            SkipReason::ProjectBusy => "project-busy".to_string(),
            SkipReason::NoSlot => "no-slot".to_string(),
            SkipReason::NoTestSlot => "no-test-slot".to_string(),
            SkipReason::NeedsRouting => "needs-routing".to_string(),
            SkipReason::NoTemplate => "no-template".to_string(),
            SkipReason::TemplateWithoutDemandMode => "template-without-demand-mode".to_string(),
            SkipReason::Budget(detail) => format!("budget:{detail}"),
            SkipReason::Paused => "paused".to_string(),
            SkipReason::WorktreeCap(_) => SKIP_WORKTREE_CAP.to_string(),
            SkipReason::BranchCheckedOutElsewhere => SKIP_BRANCH_CHECKED_OUT_ELSEWHERE.to_string(),
            SkipReason::WaitingOn(keys) => {
                format!(
                    "{SKIP_WAITING_ON_PREFIX}{}",
                    keys.join(WAITING_ON_SEPARATOR)
                )
            }
        }
    }

    /// Human text kept apart from the stable slug, when the reason has any.
    pub fn detail(&self) -> Option<String> {
        match self {
            SkipReason::WorktreeCap(holders) => Some(holders.join(", ")),
            _ => None,
        }
    }
}

/// The full result of one dispatch decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchPlan {
    pub launches: Vec<Launch>,
    pub skipped: Vec<(String, SkipReason)>,
    pub slots_used: usize,
    pub slots_max: usize,
    /// Test flows running (all projects) and the per-project ceiling.
    #[serde(default)]
    pub test_slots_used: usize,
    #[serde(default)]
    pub test_slots_max: usize,
}

/// What the flow engine runs right now, per lane, for [`plan_with_lanes`].
#[derive(Debug, Clone, Default)]
pub struct LaneLoad<'a> {
    /// Routing table: gives the `test_flow` of a ticket in the test lane.
    pub routing: Option<&'a crate::config::Routing>,
    /// Dev flows running (worktrees): they use `dispatcher.slots` like dispatcher sessions.
    pub dev_flows: usize,
    /// Test flows holding or waiting for the main checkout, per project.
    pub test_flows: HashMap<String, usize>,
}

/// The token-budget guard's verdict for this tick — a STUB for lot 10a,
/// which owns the real cost-aware implementation. Lot 10a replaces
/// [`Verdict::unconstrained`] and [`Verdict::blocked`] (and presumably adds
/// more constructors of its own); it does not change this shape, which is
/// the contract [`plan`] consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub max_launches: usize,
    pub reason: Option<String>,
}

impl Verdict {
    /// No budget constraint at all: every free slot may be used.
    pub fn unconstrained() -> Verdict {
        Verdict {
            max_launches: usize::MAX,
            reason: None,
        }
    }

    /// The budget refuses every launch this tick, for `reason`.
    pub fn blocked(reason: impl Into<String>) -> Verdict {
        Verdict {
            max_launches: 0,
            reason: Some(reason.into()),
        }
    }

    pub fn max_launches(&self) -> usize {
        self.max_launches
    }
}

/// A minimal, read-only view over the pipeline templates: for each template
/// id, the set of top-level `inputs:` keys it declares. Built without
/// `aidp-core` — see this module's doc comment.
#[derive(Debug, Clone, Default)]
pub struct TemplateCatalog {
    inputs_by_template: HashMap<String, Vec<String>>,
}

impl TemplateCatalog {
    /// Read every `*.yaml` directly under `<skill_root>/templates/` (NOT its
    /// `legacy/` subdirectory, nor any other subdirectory) as a raw
    /// [`serde_yaml::Value`], and record each template's `id` (falling back
    /// to the file stem when absent) with the KEYS of its top-level
    /// `inputs:` mapping.
    ///
    /// Tolerant by design: an unreadable file, a file that is not valid
    /// YAML, or a `templates/` directory that does not exist at all, is
    /// skipped (or yields an empty catalog) with a `tracing` warning —
    /// never an error. The daemon must keep ticking when someone leaves a
    /// broken template on disk.
    ///
    /// Only the PRESENCE of an `inputs:` key is recorded, never its value or
    /// default: `templates/dev-discovery.yaml` declares `prompt` with the
    /// literal sentinel default `false` (falsy, consumed by a `when:` guard
    /// at composition time), and treating that falsy default as "input
    /// absent" would silently break the already-shipped double-demand
    /// templates `dev-csharp`, `dev-lambda-tsgit` and `dev-discovery` itself.
    pub fn from_skill_root(skill_root: &Path) -> TemplateCatalog {
        let mut catalog = TemplateCatalog::empty();
        let dir = skill_root.join("templates");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "cannot read templates dir {}: {e}; using an empty catalog",
                    dir.display()
                );
                return catalog;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(
                        "skipping unreadable directory entry under {}: {e}",
                        dir.display()
                    );
                    continue;
                }
            };
            let path = entry.path();
            // Skips `legacy/` (and any other subdirectory) by construction:
            // `read_dir` does not recurse, so a directory entry is simply
            // never treated as a template file.
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }

            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!("skipping unreadable template {}: {e}", path.display());
                    continue;
                }
            };
            let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
                Ok(value) => value,
                Err(e) => {
                    tracing::warn!("skipping invalid template {}: {e}", path.display());
                    continue;
                }
            };

            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string()
                });
            let inputs = value
                .get("inputs")
                .and_then(|v| v.as_mapping())
                .map(|mapping| {
                    mapping
                        .keys()
                        .filter_map(|k| k.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            catalog.insert(&id, inputs);
        }

        catalog
    }

    /// An empty catalog: no template known.
    pub fn empty() -> TemplateCatalog {
        TemplateCatalog {
            inputs_by_template: HashMap::new(),
        }
    }

    /// Record `template_id` as declaring exactly `inputs` (by key). For
    /// tests, and for a CLI-side build that already has the parsed
    /// templates in hand.
    pub fn insert(&mut self, id: &str, inputs: Vec<String>) {
        self.inputs_by_template.insert(id.to_string(), inputs);
    }

    /// Does `template_id` declare a top-level input named `input`?
    pub fn declares(&self, template_id: &str, input: &str) -> bool {
        self.inputs_by_template
            .get(template_id)
            .is_some_and(|inputs| inputs.iter().any(|i| i == input))
    }

    /// Is `template_id` known to this catalog at all?
    pub fn known(&self, template_id: &str) -> bool {
        self.inputs_by_template.contains_key(template_id)
    }
}

/// Build the [`Launch`] a single ticket would produce, applying only the
/// routing/demand-mode checks — unrouted service, missing/unknown template,
/// a template that declares no demand mode usable by this ticket. NOT the
/// project-busy, slot-ceiling or budget checks: those are [`plan`]'s own
/// pre-filter over the whole candidate set, orthogonal to whether a launch
/// can be BUILT for this one ticket at all.
///
/// Factored out of `plan`'s own loop so it has exactly one home: the
/// dispatcher's forced-during-wait path (`aidp ticket start --force` while
/// `mode: waiting`, vision §7.3 / lot 10a's "Attente et reprise dans le
/// dispatcher") needs this same per-ticket logic for its one forced ticket,
/// deliberately WITHOUT the slot/budget gate it is bypassing — duplicating
/// ~30 lines of routing/demand logic here and in the dispatcher was the
/// alternative, and this crate's own review history already flags that
/// drift as a recurring problem.
pub fn build_launch(t: &Ticket, templates: &TemplateCatalog) -> Result<Launch, SkipReason> {
    if t.service.is_empty() || t.service == "unrouted" {
        return Err(SkipReason::NeedsRouting);
    }

    let issue = t
        .github
        .as_ref()
        .filter(|_| t.source == Source::Github)
        .map(|github| github.number);
    let track = t.track.unwrap_or(Track::Standard).as_str().to_string();
    if let Some(flow) = t.flow.as_deref().filter(|flow| !flow.is_empty()) {
        // A routed flow chooses its pipelines itself: no template is needed.
        let demand = match issue {
            Some(number) => Demand::Issue { number },
            None => Demand::Prompt {
                prompt: String::new(),
                feature: t.key.clone(),
            },
        };
        return Ok(Launch {
            key: t.key.clone(),
            service: t.service.clone(),
            template: t.template.clone().unwrap_or_default(),
            track,
            demand,
            manual: t.autopilot.manual_priority == Some(true),
            detail: None,
            flow: Some(flow.to_string()),
            test: false,
        });
    }

    let template = match t.template.as_deref() {
        Some(tpl) if !tpl.is_empty() => tpl,
        _ => return Err(SkipReason::NoTemplate),
    };

    let issue_usable = issue.filter(|_| templates.declares(template, ISSUE_INPUT));
    let prompt_usable = templates.declares(template, PROMPT_INPUT);
    if !templates.known(template) || (issue_usable.is_none() && !prompt_usable) {
        return Err(SkipReason::TemplateWithoutDemandMode);
    }

    let demand = match issue_usable {
        Some(number) => Demand::Issue { number },
        // `prompt_usable` is guaranteed true here: the check above only let
        // this candidate through when it has an issue mode or a prompt mode.
        None => Demand::Prompt {
            prompt: String::new(),
            feature: t.key.clone(),
        },
    };

    Ok(Launch {
        key: t.key.clone(),
        service: t.service.clone(),
        template: template.to_string(),
        track,
        demand,
        manual: t.autopilot.manual_priority == Some(true),
        detail: None,
        flow: None,
        test: false,
    })
}

/// The launch of a `to-test` ticket: its route's `test_flow`, in the test lane.
pub fn build_test_launch(
    t: &Ticket,
    routing: Option<&crate::config::Routing>,
) -> Result<Launch, SkipReason> {
    if t.service.is_empty() || t.service == "unrouted" {
        return Err(SkipReason::NeedsRouting);
    }
    let flow = t
        .flow
        .as_deref()
        .zip(routing)
        .and_then(|(flow, routing)| routing.test_flow_for(flow, t.route.as_deref()))
        .ok_or(SkipReason::NoTemplate)?;
    let demand = match t.github.as_ref().filter(|_| t.source == Source::Github) {
        Some(github) => Demand::Issue {
            number: github.number,
        },
        None => Demand::Prompt {
            prompt: String::new(),
            feature: t.key.clone(),
        },
    };
    Ok(Launch {
        key: t.key.clone(),
        service: t.service.clone(),
        template: t.template.clone().unwrap_or_default(),
        track: t.track.unwrap_or(Track::Standard).as_str().to_string(),
        demand,
        manual: t.autopilot.manual_priority == Some(true),
        detail: None,
        flow: Some(flow.to_string()),
        test: true,
    })
}

/// Decide what to launch next, applying the rules in order:
///
/// 1. `cfg.paused` -> every candidate is skipped as [`SkipReason::Paused`];
///    `launches` is empty; `slots_used` is still reported honestly.
/// 2. `slots_used` counts only `running` entries with
///    `origin_kind == "dispatcher"` (vision D4: a UI session occupies its
///    project but does not consume a slot). The free budget is
///    `cfg.slots - slots_used`, further capped by `budget.max_launches()`.
/// 3. Candidates are sorted with [`crate::order::sort_key`] (manual priority
///    first) and filtered to [`Status::Todo`] / [`Status::Queued`].
/// 4. Per candidate, in order: unrouted service -> [`SkipReason::NeedsRouting`];
///    missing template -> [`SkipReason::NoTemplate`]; a template unknown to
///    `templates`, or one that declares no demand mode usable BY THIS
///    TICKET (`issue` only helps a `Source::Github` ticket; `prompt`
///    helps any ticket) -> [`SkipReason::TemplateWithoutDemandMode`]; the
///    ticket's service already occupied (by any `running` entry, or by a
///    `Launch` already added this pass) -> [`SkipReason::ProjectBusy`]; no
///    free slot -> [`SkipReason::NoSlot`], or [`SkipReason::Budget`] when
///    `budget.max_launches() == 0`; otherwise a [`Launch`] is emitted and a
///    slot consumed.
/// 5. The [`Demand`] on an emitted launch is `Demand::Issue` when the
///    template declares `issue` AND the ticket is a GitHub mirror,
///    otherwise `Demand::Prompt` (see [`Demand`]'s own doc comment for the
///    empty-`prompt`-string contract). `track` is the ticket's own `track`
///    or `"standard"`.
///
/// A local (non-mirror) ticket routed to an issue-only template legitimately
/// produces `TemplateWithoutDemandMode` — that is expected, not a bug to
/// work around with a fallback. A ticket whose route names a `flow` needs
/// no template: the launch carries the flow and the dispatcher starts it.
pub fn plan(
    candidates: &[Ticket],
    running: &[RunningSlot],
    budget: &Verdict,
    templates: &TemplateCatalog,
    cfg: &crate::config::DispatcherConfig,
) -> DispatchPlan {
    plan_with_worktrees(candidates, running, budget, templates, cfg, &HashMap::new())
}

/// [`plan`] with the per-project worktree cap applied; `worktree_holders` maps a project to
/// the keys of its tickets owning a worktree, and holders are never capped.
pub fn plan_with_worktrees(
    candidates: &[Ticket],
    running: &[RunningSlot],
    budget: &Verdict,
    templates: &TemplateCatalog,
    cfg: &crate::config::DispatcherConfig,
    worktree_holders: &HashMap<String, Vec<String>>,
) -> DispatchPlan {
    plan_with_blockers(
        candidates,
        running,
        budget,
        templates,
        cfg,
        worktree_holders,
    )
}

/// Open blockers of `ticket` (`waiting-on:GH-<n>`), empty for a local ticket.
fn open_blockers_of(ticket: &Ticket) -> Vec<String> {
    match ticket.github.as_ref() {
        Some(github) => blockers::open(&github.blocked_by),
        None => Vec::new(),
    }
}

/// Open-blocker edges of every candidate, keyed by ticket.
fn blocker_graph(candidates: &[Ticket]) -> BTreeMap<String, Vec<String>> {
    candidates
        .iter()
        .map(|t| (t.key.clone(), open_blockers_of(t)))
        .collect()
}

/// [`plan_with_worktrees`] that also holds back tickets with open blockers.
/// A manual ticket or one in a blocker cycle launches anyway.
pub fn plan_with_blockers(
    candidates: &[Ticket],
    running: &[RunningSlot],
    budget: &Verdict,
    templates: &TemplateCatalog,
    cfg: &crate::config::DispatcherConfig,
    worktree_holders: &HashMap<String, Vec<String>>,
) -> DispatchPlan {
    plan_with_lanes(
        candidates,
        running,
        budget,
        templates,
        cfg,
        worktree_holders,
        &LaneLoad::default(),
    )
}

/// Origin of the sessions a flow starts: governed by the flow lanes, not by project occupancy.
const FLOW_ORIGIN: &str = "flow";

/// [`plan_with_blockers`] with the two lanes. Dev lane: `Todo`/`Queued` tickets, bounded by
/// `cfg.slots` (dispatcher sessions plus `lanes.dev_flows`). Test lane: tickets in phase `to-test`
/// (any of `Todo`/`Queued`/`InProgress`), launched with their `test_flow`, at most
/// `cfg.test_slots` per project, oldest first, a manual promotion ahead. A test launch never
/// uses a dev slot; the budget ceiling covers both.
pub fn plan_with_lanes(
    candidates: &[Ticket],
    running: &[RunningSlot],
    budget: &Verdict,
    templates: &TemplateCatalog,
    cfg: &crate::config::DispatcherConfig,
    worktree_holders: &HashMap<String, Vec<String>>,
    lanes: &LaneLoad<'_>,
) -> DispatchPlan {
    let graph = blocker_graph(candidates);
    let slots_used = running
        .iter()
        .filter(|r| r.origin_kind == "dispatcher")
        .count()
        + lanes.dev_flows;
    let slots_max = cfg.slots as usize;
    let test_slots_max = cfg.test_slots as usize;
    let test_slots_used: usize = lanes.test_flows.values().sum();

    if cfg.paused {
        let skipped = candidates
            .iter()
            .map(|t| (t.key.clone(), SkipReason::Paused))
            .collect();
        return DispatchPlan {
            launches: Vec::new(),
            skipped,
            slots_used,
            slots_max,
            test_slots_used,
            test_slots_max,
        };
    }

    let mut budget_left = budget.max_launches();
    // Projects held by a session that is not a flow's: a flow lane must not start there either.
    let session_busy: HashSet<String> = running
        .iter()
        .filter(|r| r.origin_kind != FLOW_ORIGIN)
        .map(|r| r.project.clone())
        .collect();
    let mut launches: Vec<Launch> = Vec::new();
    let mut skipped: Vec<(String, SkipReason)> = Vec::new();

    let mut test_candidates: Vec<&Ticket> = candidates
        .iter()
        .filter(|t| t.autopilot.phase() == crate::model::Phase::ToTest)
        .filter(|t| matches!(t.status, Status::Todo | Status::Queued | Status::InProgress))
        .collect();
    test_candidates.sort_by_key(|t| {
        (
            u8::from(t.autopilot.manual_priority != Some(true)),
            t.autopilot.phase_at.unwrap_or(t.updated),
            t.key.clone(),
        )
    });
    let mut test_launched: HashMap<String, usize> = HashMap::new();
    for t in test_candidates {
        let launch = match build_test_launch(t, lanes.routing) {
            Ok(launch) => launch,
            Err(skip_reason) => {
                skipped.push((t.key.clone(), skip_reason));
                continue;
            }
        };
        if session_busy.contains(&launch.service) {
            skipped.push((t.key.clone(), SkipReason::ProjectBusy));
            continue;
        }
        let in_lane = lanes.test_flows.get(&launch.service).copied().unwrap_or(0)
            + test_launched.get(&launch.service).copied().unwrap_or(0);
        if in_lane >= test_slots_max {
            skipped.push((t.key.clone(), SkipReason::NoTestSlot));
            continue;
        }
        if budget_left == 0 {
            let reason = budget.reason.clone().unwrap_or_default();
            skipped.push((t.key.clone(), SkipReason::Budget(reason)));
            continue;
        }
        budget_left = budget_left.saturating_sub(1);
        *test_launched.entry(launch.service.clone()).or_insert(0) += 1;
        launches.push(launch);
    }

    let mut free = slots_max.saturating_sub(slots_used);
    free = free.min(budget_left);

    let mut sorted: Vec<Ticket> = candidates.to_vec();
    crate::order::sort(&mut sorted);

    let mut occupied: HashSet<String> = running.iter().map(|r| r.project.clone()).collect();

    for t in sorted.iter().filter(|t| {
        matches!(t.status, Status::Todo | Status::Queued)
            && t.autopilot.phase() != crate::model::Phase::ToTest
    }) {
        let mut launch = match build_launch(t, templates) {
            Ok(launch) => launch,
            Err(skip_reason) => {
                skipped.push((t.key.clone(), skip_reason));
                continue;
            }
        };

        let open_keys = graph.get(&t.key).cloned().unwrap_or_default();
        if !launch.manual && !open_keys.is_empty() {
            if blockers::in_cycle(&t.key, &graph) {
                launch.detail = Some(LaunchDetail::BlockerCycle);
            } else {
                skipped.push((t.key.clone(), SkipReason::WaitingOn(open_keys)));
                continue;
            }
        }

        let flow_launch = launch.flow.is_some();
        let busy = if flow_launch {
            session_busy.contains(&launch.service)
        } else {
            occupied.contains(&launch.service)
        };
        if busy {
            skipped.push((t.key.clone(), SkipReason::ProjectBusy));
            continue;
        }

        let holders = worktree_holders
            .get(&launch.service)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let has_own = holders.iter().any(|key| key == &t.key);
        if let CapVerdict::Cap(keys) = cap_verdict(holders, cfg.max_worktrees_per_project, has_own)
        {
            skipped.push((t.key.clone(), SkipReason::WorktreeCap(keys)));
            continue;
        }

        if free == 0 {
            if budget_left == 0 {
                let reason = budget.reason.clone().unwrap_or_default();
                skipped.push((t.key.clone(), SkipReason::Budget(reason)));
            } else {
                skipped.push((t.key.clone(), SkipReason::NoSlot));
            }
            continue;
        }

        if !flow_launch {
            occupied.insert(launch.service.clone());
        }
        free -= 1;
        launches.push(launch);
    }

    DispatchPlan {
        launches,
        skipped,
        slots_used,
        slots_max,
        test_slots_used,
        test_slots_max,
    }
}

#[cfg(test)]
#[path = "../tests/unit/plan_tests.rs"]
mod plan_tests;
