//! The ticket domain model: `Ticket` and its enums, the status transition
//! matrix, and the derived-field recomputation. Pure data + pure functions —
//! no I/O. Parsing/rendering the note format lives in `note.rs`; ordering
//! lives in `order.rs`; journalling a transition lives in `ledger.rs`.

use crate::models::enums::adoption_reason::AdoptionReason;
use crate::models::enums::cli_kind::{SourceToken, StatusToken};
use crate::models::enums::pr_state::PrState;
use crate::models::track::Track;
use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A ticket's lifecycle state (vision §6).
///
/// `failed` is deliberately **not** a variant here, even though the state
/// diagram in vision §6 draws a `failed` node. It is not a status a ticket
/// ever sits in — it is one of the `autopilot.blocked_reason` values
/// (`needs-routing`, `failed`, `gate:<stage>`, `no-pr`, `pr-closed`,
/// `cancelled-by-user`). The lot's decision: a retry is modelled as the
/// ordinary transition `in-progress -> todo`, with the caller setting
/// `reason: "failed"` on the journalled event and incrementing
/// `autopilot.attempts` — see `retry_is_in_progress_to_todo` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Todo,
    Queued,
    InProgress,
    InReview,
    Done,
    Blocked,
    Cancelled,
}

impl Status {
    /// The wire form (also used in tags and error messages), e.g. `in-progress`.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Queued => "queued",
            Status::InProgress => "in-progress",
            Status::InReview => "in-review",
            Status::Done => "done",
            Status::Blocked => "blocked",
            Status::Cancelled => "cancelled",
        }
    }

    /// The explicit adjacency matrix from vision §6. Not a heuristic: every
    /// legal edge is listed, and anything not listed (including every
    /// self-edge) is illegal.
    fn allowed_targets(from: Status) -> &'static [Status] {
        use Status::*;
        match from {
            Todo => &[Queued, InProgress, Blocked, Cancelled],
            Queued => &[InProgress, Todo, Blocked, Cancelled],
            InProgress => &[InReview, Done, Blocked, Todo, Cancelled],
            InReview => &[Done, Blocked, Cancelled],
            Blocked => &[Todo, Queued, InProgress, InReview, Cancelled],
            Done => &[Todo, Cancelled],
            Cancelled => &[Todo],
        }
    }

    /// Is `from -> to` a legal edge of the matrix?
    ///
    /// `actor` is accepted for signature stability (a future lot may make the
    /// matrix actor-dependent) but is not consulted here: the matrix itself
    /// is the same for every actor in this lot. The one actor-specific rule
    /// — a human needs `--force` to leave `in-progress` — lives in [`guard`],
    /// which is the function callers should actually use to decide.
    pub fn can_transition(from: Status, to: Status, actor: Actor) -> bool {
        let _ = actor;
        from != to && Self::allowed_targets(from).contains(&to)
    }
}

impl FromStr for Status {
    type Err = anyhow::Error;

    /// Tolerant command-line spelling, see [`StatusToken`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.parse::<StatusToken>()?.status())
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who is asking for a transition. Recorded on every journalled event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Actor {
    Cli,
    Dispatcher,
    Sync,
    Human,
    Flow,
}

impl Actor {
    pub fn as_str(self) -> &'static str {
        match self {
            Actor::Cli => "cli",
            Actor::Dispatcher => "dispatcher",
            Actor::Sync => "sync",
            Actor::Human => "human",
            Actor::Flow => "flow",
        }
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decide whether `from -> to` may proceed for `actor`, composing the
/// transition matrix with the one actor rule of vision §6: a human-initiated
/// transition that leaves `in-progress` is refused unless `force` is `true`.
/// This is what `aidp ticket status` calls, and the returned error is what it
/// prints verbatim on refusal.
///
/// In this lot `force` only forces this one transition and lets the caller
/// journal it; it does not touch any session, because there is no dispatcher
/// or daemon yet to cancel one. A later lot that adds `--force` killing a
/// running session does that around this call, not inside it.
pub fn guard(from: Status, to: Status, actor: Actor, force: bool) -> anyhow::Result<()> {
    guard_with_reason(from, to, actor, force, None)
}

/// [`guard`] that also accepts the adoption shortcuts out of `todo`/`queued`,
/// recognised by `reason` and `actor`.
pub fn guard_with_reason(
    from: Status,
    to: Status,
    actor: Actor,
    force: bool,
    reason: Option<AdoptionReason>,
) -> anyhow::Result<()> {
    if is_adoption(from, to, actor, reason) {
        return Ok(());
    }
    if !Status::can_transition(from, to, actor) {
        let allowed = Status::allowed_targets(from)
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(crate::ticket_error::TicketError::Conflict(
            crate::models::constants::messages::illegal_transition(
                &from.to_string(),
                &to.to_string(),
                &allowed,
            ),
        )
        .into());
    }
    if actor == Actor::Human && from == Status::InProgress && !force {
        return Err(crate::ticket_error::TicketError::Conflict(
            crate::models::constants::messages::force_required(
                &from.to_string(),
                &to.to_string(),
                &actor.to_string(),
            ),
        )
        .into());
    }
    Ok(())
}

/// True for the adoption edges: an already delivered ticket skips the run.
fn is_adoption(from: Status, to: Status, actor: Actor, reason: Option<AdoptionReason>) -> bool {
    if !matches!(from, Status::Todo | Status::Queued) {
        return false;
    }
    matches!(
        (to, actor, reason),
        (
            Status::InReview,
            Actor::Dispatcher,
            Some(AdoptionReason::Adopted)
        ) | (
            Status::Done,
            Actor::Dispatcher,
            Some(AdoptionReason::AdoptedMerged)
        ) | (Status::InReview, Actor::Human, Some(AdoptionReason::Adopt))
    )
}

/// Ticket priority, most to least urgent (`prio:*` labels map onto it).
///
/// `Deserialize` is hand-written below (delegating to [`FromStr`]) rather
/// than derived: the derived impl would only accept the exact `PascalCase`
/// wire form, so a hand-edited `priority: high` in Obsidian would fail
/// `serde_yaml::from_value::<Ticket>` and drop the *whole ticket* from every
/// listing (see `note::parse` / `store::collect_notes`). `Serialize` stays
/// derived: rendered notes still write the canonical `High` etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum Priority {
    Highest,
    High,
    Medium,
    Low,
    Lowest,
}

impl Priority {
    /// 1 (`Highest`) .. 5 (`Lowest`).
    pub fn rank(self) -> u8 {
        match self {
            Priority::Highest => 1,
            Priority::High => 2,
            Priority::Medium => 3,
            Priority::Low => 4,
            Priority::Lowest => 5,
        }
    }

    /// The serde wire form, e.g. `High` — the single source of truth for a
    /// priority's label, used by both `cli.rs::list_text_line` and
    /// `index.rs::dequeue_table` instead of each hand-rolling its own match.
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Highest => "Highest",
            Priority::High => "High",
            Priority::Medium => "Medium",
            Priority::Low => "Low",
            Priority::Lowest => "Lowest",
        }
    }
}

impl FromStr for Priority {
    type Err = anyhow::Error;

    /// Case-insensitive name (`high`), `P1`..`P5`, or bare `1`..`5`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "highest" | "p1" | "1" => Ok(Priority::Highest),
            "high" | "p2" | "2" => Ok(Priority::High),
            "medium" | "p3" | "3" => Ok(Priority::Medium),
            "low" | "p4" | "4" => Ok(Priority::Low),
            "lowest" | "p5" | "5" => Ok(Priority::Lowest),
            _ => anyhow::bail!(
                "unknown priority `{s}`: expected highest/high/medium/low/lowest, P1..P5, or 1..5"
            ),
        }
    }
}

impl<'de> Deserialize<'de> for Priority {
    /// Delegates to [`FromStr`] — case-insensitive, and tolerant of
    /// `P1`..`P5`/`1`..`5` too — for both a quoted string and an unquoted
    /// YAML integer. See the type's doc comment for why this is hand-written
    /// instead of derived.
    ///
    /// `deserialize_any` (rather than `deserialize_str`) is what makes the
    /// unquoted form work: a hand-edited `priority: 1` reaches `note::parse`
    /// as a `serde_yaml::Value::Number`, not a string, and serde_yaml does
    /// not coerce a number into a string on its own. Without this, that
    /// integer would fail deserialization, and since serde has no partial
    /// success, the *whole* ticket — not just this field — would drop out of
    /// `note::parse` and disappear from every listing.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PriorityVisitor;

        impl<'de> Visitor<'de> for PriorityVisitor {
            type Value = Priority;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a priority name such as `high`, `P1`..`P5`, or a bare 1..5, quoted or not",
                )
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Priority::from_str(v).map_err(de::Error::custom)
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&v.to_string())
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&v.to_string())
            }
        }

        deserializer.deserialize_any(PriorityVisitor)
    }
}

/// Where a ticket's frontmatter came from — a mirror of a GitHub issue, or
/// authored locally by `aidp ticket new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Github,
    Local,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Github => "github",
            Source::Local => "local",
        }
    }

    /// GitHub ranks ahead of Local in the dequeue order: 0, 1.
    pub fn rank(self) -> u8 {
        match self {
            Source::Github => 0,
            Source::Local => 1,
        }
    }
}

impl FromStr for Source {
    type Err = anyhow::Error;

    /// Case-insensitive `github` or `local`, see [`SourceToken`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.parse::<SourceToken>()?.source())
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ticket type (a `bug` label makes a GitHub mirror a `Bug`).
///
/// `Deserialize` is hand-written below, for the same reason as [`Priority`]:
/// the derived impl only accepts exact `PascalCase`, and a hand-edited
/// `type: bug` in Obsidian must not sink the whole ticket. `Serialize` stays
/// derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum IssueType {
    Story,
    Task,
    Bug,
    Spike,
}

impl FromStr for IssueType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "story" => Ok(IssueType::Story),
            "task" => Ok(IssueType::Task),
            "bug" => Ok(IssueType::Bug),
            "spike" => Ok(IssueType::Spike),
            _ => anyhow::bail!("unknown issue type `{s}`: expected story/task/bug/spike"),
        }
    }
}

impl<'de> Deserialize<'de> for IssueType {
    /// Deserialize a string and delegate to [`FromStr`] — case-insensitive.
    /// See the type's doc comment for why this is hand-written instead of
    /// derived.
    ///
    /// Unlike [`Priority`], this stays a plain `String::deserialize` and does
    /// not accept an unquoted YAML integer. That's deliberate, not an
    /// oversight: `IssueType::from_str` advertises no numeric shorthand — it
    /// only ever accepts `story`/`task`/`bug`/`spike` — so there is no bare
    /// number this impl needs to tolerate. The day `IssueType` grows a
    /// numeric form of its own, this impl has to move to the same
    /// `deserialize_any` + `Visitor` shape `Priority` uses above.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// A pull request associated with an autopilot attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pr {
    pub url: String,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub state: Option<PrState>,
    /// Set by the caller once the PR link comment was posted on the issue.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub commented: bool,
}

/// Lane of a ticket: `dev` (worktree flows, in parallel), `to-test` (dev done, waits for the Unity
/// test lane) or `testing` (the test flow runs in the main checkout of the project).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    #[default]
    Dev,
    ToTest,
    Testing,
}

impl Phase {
    /// The wire form: `dev`, `to-test`, `testing`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Dev => "dev",
            Phase::ToTest => "to-test",
            Phase::Testing => "testing",
        }
    }

    /// The phase named `text`, ignoring case.
    pub fn parse(text: &str) -> Option<Phase> {
        [Phase::Dev, Phase::ToTest, Phase::Testing]
            .into_iter()
            .find(|phase| phase.as_str().eq_ignore_ascii_case(text.trim()))
    }
}

/// The dispatcher/autopilot bookkeeping carried on every ticket, whether or
/// not any autopilot lot has run yet.
///
/// `#[serde(default)]` sits on the struct AND on every field, deliberately
/// redundant: it is the same forward/backward compatibility rule as
/// `SessionRecord` in `aidp-gateway::sessions` — a note written before a
/// field existed still parses (the field defaults), and a note written by a
/// newer `aidp` with a field this build does not know about round-trips
/// through `serde_yaml::Mapping` at the `note.rs` layer rather than being
/// rejected here.
///
/// Every field but `attempts` renders explicitly as `null` when absent
/// (no `skip_serializing_if`) so a freshly created ticket's frontmatter shows
/// the whole shape up front, matching the vision §5.1 example.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Autopilot {
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub queued_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub pr: Option<Pr>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// Set by a manual `aidp ticket start` (lot 9a) to jump the dequeue
    /// order. Carried here so the field survives a round-trip through this
    /// lot, but `order.rs` in this lot does not read it yet — a ticket with
    /// `manual_priority: true` sorts exactly like one without it until the
    /// dispatcher lot implements the jump.
    #[serde(default)]
    pub manual_priority: Option<bool>,
    /// True when the ticket was found already delivered and moved to
    /// review or done without a run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adopted: bool,
    /// Merge commit of the PR, set when the release watch is armed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_sha: Option<String>,
    /// Release tag of the merged PR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_tag: Option<String>,
    /// Deadline after which the release watch gives up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_watch_until: Option<DateTime<Utc>>,
    /// Delivery flow that drives this ticket instead of a pipeline session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// Lane of the ticket (`dev` when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// When the ticket entered its current phase: orders the test lane (oldest first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_at: Option<DateTime<Utc>>,
}

impl Autopilot {
    /// The phase, `dev` when none is recorded.
    pub fn phase(&self) -> Phase {
        self.phase.unwrap_or_default()
    }

    /// Records `phase` and the time it began.
    pub fn set_phase(&mut self, phase: Phase, now: DateTime<Utc>) {
        if self.phase() != phase || self.phase_at.is_none() {
            self.phase_at = Some(now);
        }
        self.phase = Some(phase);
    }
}

/// The frontmatter of a ticket note (vision §5.1). The note's own `aidp:
/// ticket` marker is not a field here — `note.rs` owns that marker and the
/// Markdown body around this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    pub key: String,
    pub source: Source,
    #[serde(rename = "type")]
    pub r#type: IssueType,
    pub summary: String,
    pub status: Status,
    pub priority: Priority,
    /// Derived by [`Ticket::recompute_derived`]; defaults to `0` so a note
    /// written before this field existed still parses (it is fixed on the
    /// next save).
    #[serde(default)]
    pub priority_rank: u8,
    pub service: String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub track: Option<Track>,
    /// Route that matched in `routing.yaml` (the `repos:` entry), absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Delivery flow the dispatcher starts for this ticket, resolved from its route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<crate::models::milestone::Milestone>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// Present only for `source: github` — a local ticket has no `github:`
    /// key at all in its rendered frontmatter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<crate::models::github_meta::GithubMeta>,
    #[serde(default)]
    pub autopilot: Autopilot,
    /// Derived by [`Ticket::recompute_derived`].
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Ticket {
    /// Recompute `priority_rank` and `tags` from the ticket's current
    /// `priority`, `status`, `service` and `source`.
    ///
    /// Called by the store on every save — never by the note renderer, which
    /// only ever writes what is already on the struct. Consequence: editing
    /// `priority:` by hand in Obsidian leaves `priority_rank` (and the
    /// `aidp/ticket/<status>` tag, if `status:` was also hand-edited) stale
    /// until the next `aidp ticket` command touches the note. That is
    /// accepted behaviour for this lot, not a bug to fix with a file
    /// watcher: `aidp` is the only writer of derived fields, by design.
    pub fn recompute_derived(&mut self) {
        self.priority_rank = self.priority.rank();
        self.tags = vec![
            "aidp/ticket".to_string(),
            format!("aidp/ticket/{}", self.status.as_str()),
            format!("service/{}", self.service),
            format!("source/{}", self.source.as_str()),
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ticket() -> Ticket {
        let now = Utc::now();
        Ticket {
            key: "LOC-1".to_string(),
            source: Source::Local,
            r#type: IssueType::Task,
            summary: "Sample ticket".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            priority_rank: 0,
            service: "aidp-viewer".to_string(),
            template: None,
            track: None,
            route: None,
            flow: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            milestone: None,
            created: now,
            updated: now,
            github: None,
            autopilot: Autopilot::default(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn status_kebab_case_round_trip() {
        let cases = [
            (Status::Todo, "\"todo\""),
            (Status::Queued, "\"queued\""),
            (Status::InProgress, "\"in-progress\""),
            (Status::InReview, "\"in-review\""),
            (Status::Done, "\"done\""),
            (Status::Blocked, "\"blocked\""),
            (Status::Cancelled, "\"cancelled\""),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json, "serialising {status:?}");
            let back: Status = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn transition_matrix_legal_and_illegal() {
        use Status::*;

        // One legal edge per source state, taken straight from the matrix.
        assert!(Status::can_transition(Todo, Queued, Actor::Cli));
        assert!(Status::can_transition(
            Queued,
            InProgress,
            Actor::Dispatcher
        ));
        assert!(Status::can_transition(
            InProgress,
            InReview,
            Actor::Dispatcher
        ));
        assert!(Status::can_transition(InReview, Done, Actor::Sync));
        assert!(Status::can_transition(Blocked, Todo, Actor::Human));
        assert!(Status::can_transition(Done, Todo, Actor::Cli));
        assert!(Status::can_transition(Cancelled, Todo, Actor::Human));

        // No self-edges, for any state.
        for s in [Todo, Queued, InProgress, InReview, Done, Blocked, Cancelled] {
            assert!(
                !Status::can_transition(s, s, Actor::Cli),
                "{s:?} -> {s:?} must be illegal"
            );
        }

        // Edges absent from the matrix.
        assert!(!Status::can_transition(Todo, Done, Actor::Cli));
        assert!(!Status::can_transition(InReview, Queued, Actor::Cli));
        assert!(!Status::can_transition(Done, Blocked, Actor::Cli));
        assert!(!Status::can_transition(Cancelled, Done, Actor::Cli));
        assert!(!Status::can_transition(Cancelled, Blocked, Actor::Cli));
    }

    #[test]
    fn human_needs_force_to_leave_in_progress() {
        let refused = guard(Status::InProgress, Status::Done, Actor::Human, false).unwrap_err();
        let msg = refused.to_string();
        assert!(msg.contains("force"), "message should mention force: {msg}");
        assert!(msg.contains("in-progress"));
        assert!(
            refused
                .downcast_ref::<crate::ticket_error::TicketError>()
                .is_some(),
            "the --force refusal must downcast to TicketError, got: {refused:?}"
        );

        assert!(guard(Status::InProgress, Status::Done, Actor::Human, true).is_ok());

        // Non-human actors are not subject to the force rule.
        assert!(guard(Status::InProgress, Status::Done, Actor::Dispatcher, false).is_ok());

        // An illegal transition is refused regardless of actor or force.
        let illegal = guard(Status::Todo, Status::Done, Actor::Human, true).unwrap_err();
        assert!(illegal.to_string().contains("not a legal transition"));
    }

    #[test]
    fn retry_is_in_progress_to_todo() {
        // The lot's retry transition: in-progress -> todo. The dispatcher is
        // not subject to the human/force rule, so a retry never needs it.
        assert!(Status::can_transition(
            Status::InProgress,
            Status::Todo,
            Actor::Dispatcher
        ));
        assert!(guard(Status::InProgress, Status::Todo, Actor::Dispatcher, false).is_ok());

        let mut autopilot = Autopilot {
            attempts: 0,
            ..Default::default()
        };
        autopilot.attempts += 1;
        autopilot.blocked_reason = Some("failed".to_string());
        assert_eq!(autopilot.attempts, 1);
        assert_eq!(autopilot.blocked_reason.as_deref(), Some("failed"));
    }

    #[test]
    fn priority_parse_tolerant() {
        for s in ["High", "high", "P2", "p2", "2"] {
            assert_eq!(
                s.parse::<Priority>().unwrap(),
                Priority::High,
                "parsing {s:?}"
            );
        }
        assert_eq!("1".parse::<Priority>().unwrap(), Priority::Highest);
        assert_eq!("p5".parse::<Priority>().unwrap(), Priority::Lowest);
        assert!("6".parse::<Priority>().is_err());
        assert!("urgent".parse::<Priority>().is_err());
    }

    #[test]
    fn issue_type_parse_tolerant() {
        for s in ["Bug", "bug", "BUG", "bUg"] {
            assert_eq!(
                s.parse::<IssueType>().unwrap(),
                IssueType::Bug,
                "parsing {s:?}"
            );
        }
        assert_eq!("story".parse::<IssueType>().unwrap(), IssueType::Story);
        assert!("epic".parse::<IssueType>().is_err());
    }

    #[test]
    fn derived_tags_and_priority_rank() {
        let mut t = sample_ticket();
        t.priority = Priority::High;
        t.status = Status::InProgress;
        t.service = "cheat-alerts-service".to_string();
        t.source = Source::Github;

        t.recompute_derived();

        assert_eq!(t.priority_rank, 2);
        assert_eq!(
            t.tags,
            vec![
                "aidp/ticket".to_string(),
                "aidp/ticket/in-progress".to_string(),
                "service/cheat-alerts-service".to_string(),
                "source/github".to_string(),
            ]
        );
    }

    #[test]
    fn priority_deserializes_case_insensitively() {
        let base = match serde_yaml::to_value(sample_ticket()).unwrap() {
            serde_yaml::Value::Mapping(m) => m,
            _ => unreachable!("Ticket always serialises to a YAML mapping"),
        };

        for s in ["high", "HIGH", "High"] {
            let mut mapping = base.clone();
            mapping.insert(
                serde_yaml::Value::String("priority".to_string()),
                serde_yaml::Value::String(s.to_string()),
            );
            let parsed: Ticket = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping))
                .unwrap_or_else(|e| {
                    panic!("a ticket with `priority: {s}` must still deserialize: {e}")
                });
            assert_eq!(parsed.priority, Priority::High, "priority {s:?}");
        }
    }

    #[test]
    fn priority_deserializes_an_unquoted_yaml_integer() {
        let base = match serde_yaml::to_value(sample_ticket()).unwrap() {
            serde_yaml::Value::Mapping(m) => m,
            _ => unreachable!("Ticket always serialises to a YAML mapping"),
        };

        let with_priority = |value: serde_yaml::Value| -> anyhow::Result<Ticket> {
            let mut mapping = base.clone();
            mapping.insert(serde_yaml::Value::String("priority".to_string()), value);
            serde_yaml::from_value(serde_yaml::Value::Mapping(mapping)).map_err(Into::into)
        };

        // An unquoted YAML integer — `Value::Number`, not `Value::String` —
        // is what a hand-edited `priority: 1` actually deserializes to.
        let parsed = with_priority(serde_yaml::Value::Number(1.into()))
            .unwrap_or_else(|e| panic!("`priority: 1` (unquoted) must still deserialize: {e}"));
        assert_eq!(parsed.priority, Priority::Highest);

        let parsed = with_priority(serde_yaml::Value::Number(5.into()))
            .unwrap_or_else(|e| panic!("`priority: 5` (unquoted) must still deserialize: {e}"));
        assert_eq!(parsed.priority, Priority::Lowest);

        // The quoted form keeps working too.
        let parsed = with_priority(serde_yaml::Value::String("1".to_string()))
            .unwrap_or_else(|e| panic!("`priority: \"1\"` (quoted) must still deserialize: {e}"));
        assert_eq!(parsed.priority, Priority::Highest);

        // Out of range still fails.
        assert!(
            with_priority(serde_yaml::Value::Number(9.into())).is_err(),
            "priority `9` is out of the 1..5 range and must be refused"
        );
    }

    #[test]
    fn issue_type_deserializes_case_insensitively() {
        let base = match serde_yaml::to_value(sample_ticket()).unwrap() {
            serde_yaml::Value::Mapping(m) => m,
            _ => unreachable!("Ticket always serialises to a YAML mapping"),
        };

        for s in ["bug", "BUG", "Bug"] {
            let mut mapping = base.clone();
            mapping.insert(
                serde_yaml::Value::String("type".to_string()),
                serde_yaml::Value::String(s.to_string()),
            );
            let parsed: Ticket = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping))
                .unwrap_or_else(|e| {
                    panic!("a ticket with `type: {s}` must still deserialize: {e}")
                });
            assert_eq!(parsed.r#type, IssueType::Bug, "type {s:?}");
        }
    }

    #[test]
    fn issue_type_refuses_an_unquoted_integer() {
        // Unlike `Priority`, `IssueType` advertises no numeric shorthand —
        // this pins the deliberate asymmetry so a future reader does not
        // "fix" it by giving `IssueType` the same Visitor as `Priority`.
        let base = match serde_yaml::to_value(sample_ticket()).unwrap() {
            serde_yaml::Value::Mapping(m) => m,
            _ => unreachable!("Ticket always serialises to a YAML mapping"),
        };

        let mut mapping = base.clone();
        mapping.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::Number(1.into()),
        );
        let result: Result<Ticket, _> = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping));
        assert!(
            result.is_err(),
            "`type: 1` (unquoted) has no numeric shorthand and must still be refused"
        );
    }

    #[test]
    fn priority_as_str_matches_the_serde_wire_form() {
        for p in [
            Priority::Highest,
            Priority::High,
            Priority::Medium,
            Priority::Low,
            Priority::Lowest,
        ] {
            let yaml = serde_yaml::to_string(&p).unwrap();
            assert_eq!(yaml.trim(), p.as_str(), "serialising {p:?}");
        }
    }

    #[test]
    fn autopilot_defaults_when_fields_absent() {
        // An explicit empty object.
        let from_empty: Autopilot = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty, Autopilot::default());

        // An absent `autopilot` key entirely, on the containing struct.
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            #[serde(default)]
            autopilot: Autopilot,
        }
        let wrapper: Wrapper = serde_json::from_str("{}").unwrap();
        assert_eq!(wrapper.autopilot, Autopilot::default());
    }
}

#[cfg(test)]
#[path = "../tests/unit/model_tests.rs"]
mod model_tests;
