//! The state label a GitHub mirror shows (`aidp:*`, one at a time) and how a
//! label set on GitHub reads back as an intent. Pure: no I/O.
//!
//! Write-back (plan §3.1, G2): every status maps to one label through
//! `github.status_labels` (`blocked:<reason>` keys first, then the bare
//! status). `in-progress` also accepts the flow phase labels
//! (`github.phase_labels`), so a flow that labels `aidp:review` is not undone.
//! A ticket blocked with reason `label:<name>` keeps that label: it was
//! mirrored from a state aidp does not own. `cancelled` leaves the labels
//! alone. The family is every label starting with `github.label_prefix`;
//! an edit removes every other member, so at most one stays.

use crate::config::GithubConfig;
use crate::model::{Phase, Status, Ticket};
use crate::models::constants::github::{
    LABEL_REASON_PREFIX, NEEDS_REFINE_REASON, NEEDS_ROUTING_REASON, NOT_READY_REASON, SKIP_REASON,
    STATUS_REASON_SEPARATOR,
};
use crate::models::enums::state_label::StateLabel;

/// The state label a ticket wants on its issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WantedLabel {
    /// Leave the labels as they are.
    Keep,
    /// Exactly this state label.
    Exactly(String),
    /// Any one of `accepted`; `primary` when none of them is there.
    OneOf {
        accepted: Vec<String>,
        primary: String,
    },
    /// No state label at all.
    Nothing,
}

/// The labels to add and remove on an issue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelEdit {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

/// What a state label set on GitHub asks of its mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelIntent {
    /// No state label.
    Unlabelled,
    /// Ready (or queued): a candidate for the dispatcher.
    Ready,
    /// The triage label: the gates (route, criteria) decide.
    Triage,
    /// `to-test`: development done, the issue waits for the test lane.
    ToTest,
    /// Never dispatched.
    Skip,
    /// Any other state label: the issue is held in that state outside aidp.
    Held(String),
}

/// True when `label` belongs to the state label family.
pub fn is_state_label(label: &str, cfg: &GithubConfig) -> bool {
    label
        .to_ascii_lowercase()
        .starts_with(&cfg.label_prefix.to_ascii_lowercase())
}

/// The state labels among `labels`, in their order.
pub fn state_family(labels: &[String], cfg: &GithubConfig) -> Vec<String> {
    labels
        .iter()
        .filter(|label| is_state_label(label, cfg))
        .cloned()
        .collect()
}

/// The first state label among `labels`.
pub fn current(labels: &[String], cfg: &GithubConfig) -> Option<String> {
    labels
        .iter()
        .find(|label| is_state_label(label, cfg))
        .cloned()
}

fn status_label(cfg: &GithubConfig, key: &str) -> Option<String> {
    cfg.status_labels
        .get(key)
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty())
}

fn blocked_key(reason: &str) -> String {
    format!(
        "{}{STATUS_REASON_SEPARATOR}{reason}",
        Status::Blocked.as_str()
    )
}

/// The label `ticket` wants, from its status and blocked reason.
pub fn wanted(ticket: &Ticket, cfg: &GithubConfig) -> WantedLabel {
    let reason = ticket.autopilot.blocked_reason.as_deref();
    let exactly = |label: Option<String>| label.map_or(WantedLabel::Keep, WantedLabel::Exactly);
    if ticket.autopilot.phase() == Phase::ToTest
        && matches!(
            ticket.status,
            Status::Todo | Status::Queued | Status::InProgress
        )
    {
        return WantedLabel::Exactly(StateLabel::ToTest.label(&cfg.label_prefix));
    }
    match ticket.status {
        Status::Cancelled => WantedLabel::Keep,
        Status::Blocked => match reason {
            Some(held) if held.starts_with(LABEL_REASON_PREFIX) => {
                WantedLabel::Exactly(held[LABEL_REASON_PREFIX.len()..].to_string())
            }
            Some(NOT_READY_REASON) => WantedLabel::Nothing,
            Some(reason) => exactly(
                status_label(cfg, &blocked_key(reason))
                    .or_else(|| status_label(cfg, Status::Blocked.as_str())),
            ),
            None => exactly(status_label(cfg, Status::Blocked.as_str())),
        },
        Status::InProgress => match status_label(cfg, Status::InProgress.as_str()) {
            Some(primary) => {
                let mut accepted = vec![primary.clone()];
                accepted.extend(cfg.phase_labels.iter().cloned());
                WantedLabel::OneOf { accepted, primary }
            }
            None => WantedLabel::Keep,
        },
        status => exactly(status_label(cfg, status.as_str())),
    }
}

fn contains(labels: &[String], wanted: &str) -> bool {
    labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(wanted))
}

/// The edit that brings `labels` to `wanted`, `None` when they already show it.
pub fn edit(labels: &[String], wanted: &WantedLabel, cfg: &GithubConfig) -> Option<LabelEdit> {
    let family = state_family(labels, cfg);
    let keep = match wanted {
        WantedLabel::Keep => return None,
        WantedLabel::Nothing => None,
        WantedLabel::Exactly(label) => Some(label.clone()),
        WantedLabel::OneOf { accepted, primary } => Some(
            family
                .iter()
                .find(|label| contains(accepted, label))
                .cloned()
                .unwrap_or_else(|| primary.clone()),
        ),
    };
    let remove: Vec<String> = family
        .iter()
        .filter(|label| {
            keep.as_deref()
                .is_none_or(|k| !label.eq_ignore_ascii_case(k))
        })
        .cloned()
        .collect();
    let add: Vec<String> = keep
        .into_iter()
        .filter(|label| !contains(&family, label))
        .collect();
    if add.is_empty() && remove.is_empty() {
        return None;
    }
    Some(LabelEdit { add, remove })
}

/// `labels` once `edit` is applied.
pub fn apply(labels: &[String], edit: &LabelEdit) -> Vec<String> {
    let mut out: Vec<String> = labels
        .iter()
        .filter(|label| !contains(&edit.remove, label))
        .cloned()
        .collect();
    for label in &edit.add {
        if !contains(&out, label) {
            out.push(label.clone());
        }
    }
    out
}

/// What the state label `label` (or its absence) asks of the mirror.
pub fn intent(label: Option<&str>, cfg: &GithubConfig) -> LabelIntent {
    let Some(label) = label else {
        return LabelIntent::Unlabelled;
    };
    let is = |key: &str| status_label(cfg, key).is_some_and(|l| l.eq_ignore_ascii_case(label));
    if label.eq_ignore_ascii_case(&StateLabel::ToTest.label(&cfg.label_prefix)) {
        return LabelIntent::ToTest;
    }
    if is(Status::Todo.as_str()) || is(Status::Queued.as_str()) {
        return LabelIntent::Ready;
    }
    if is(&blocked_key(SKIP_REASON)) {
        return LabelIntent::Skip;
    }
    if is(&blocked_key(NEEDS_REFINE_REASON)) || is(&blocked_key(NEEDS_ROUTING_REASON)) {
        return LabelIntent::Triage;
    }
    LabelIntent::Held(label.to_string())
}

/// The blocked reason that hands a ticket to the state label `label`.
pub fn held_reason(label: &str) -> String {
    format!("{LABEL_REASON_PREFIX}{label}")
}

#[cfg(test)]
#[path = "../tests/unit/state_labels_tests.rs"]
mod tests;
