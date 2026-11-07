//! Pure aggregation of the viewer's "inbox": a single urgency-ordered list of
//! actionable items derived from sessions, pull request attention and automation runs
//! already in memory.
//!
//! This module is deliberately pure, following the same pure/impure split as
//! `aidp_core::gitlog` (see that module's header): no process spawning, no
//! I/O, no locks, no `Path`. The HTTP route gathers the inputs and calls
//! [`build_inbox`]; the decision logic here is unit-testable with zero setup.
//!
//! ISO-8601 UTC timestamps are compared as plain strings throughout (never
//! parsed into `chrono` types for ordering), the same convention
//! `sessions.rs::search` relies on — such strings sort lexicographically in
//! chronological order. The one exception is the 24h cutoff, which needs
//! actual duration arithmetic and is computed once via `chrono`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::automations::AutomationRun;
use crate::models::enums::{InboxAction, InboxKind, SessionStatus};
use crate::models::pr_attention_record::PrAttentionRecord;
use crate::sessions::SessionRecord;

/// Urgency of a pull request attention entry, between gates and failures.
const ATTENTION_URGENCY: u8 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InboxEntry {
    /// gate | pr.attention | failure | skippedAutomation | runningSession
    pub kind: String,
    /// reply | open | launch | dismiss
    pub action: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// ISO-8601, the instant this entry became relevant. Used for tie-breaking.
    pub at: String,
    /// 0 = most urgent. Serialized so the viewer can group without re-deriving.
    pub urgency: u8,
}

fn session_title(s: &SessionRecord) -> String {
    if let Some(t) = &s.title {
        if !t.is_empty() {
            return t.clone();
        }
    }
    if let Some(t) = &s.target {
        if !t.is_empty() {
            return t.clone();
        }
    }
    if let Some(t) = &s.template_id {
        if !t.is_empty() {
            return t.clone();
        }
    }
    s.id.clone()
}

/// Subtract 24h from `now` (RFC-3339) and format back to RFC-3339. Returns
/// `None` if `now` does not parse — callers must degrade to "unbounded"
/// rather than treat that as "exclude everything".
fn cutoff_24h_before(now: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    let cutoff = parsed - chrono::Duration::hours(24);
    Some(cutoff.to_rfc3339())
}

/// One entry per unread record that was meant to notify.
fn attention_entries(attention: &[PrAttentionRecord]) -> Vec<InboxEntry> {
    attention
        .iter()
        .filter(|record| record.notified && !record.read)
        .map(|record| InboxEntry {
            kind: InboxKind::Attention.as_str().to_string(),
            action: InboxAction::Open.as_str().to_string(),
            title: format!("{}#{}", record.repo, record.number),
            detail: Some(record.signal.as_str().to_string()),
            session_id: None,
            fingerprint: Some(record.fingerprint.clone()),
            automation_id: None,
            project_id: None,
            project_name: None,
            at: record.first_seen.clone(),
            urgency: ATTENTION_URGENCY,
        })
        .collect()
}

pub fn build_inbox(
    sessions: &[SessionRecord],
    runs: &[AutomationRun],
    attention: &[PrAttentionRecord],
    now: &str,
) -> Vec<InboxEntry> {
    let mut entries: Vec<InboxEntry> = Vec::new();

    // gate: running session with a pending question.
    for s in sessions {
        if s.status == SessionStatus::Running && s.pending_question.is_some() {
            entries.push(InboxEntry {
                kind: "gate".to_string(),
                action: "reply".to_string(),
                title: session_title(s),
                detail: None,
                session_id: Some(s.id.clone()),
                fingerprint: None,
                automation_id: None,
                project_id: Some(s.project_id.clone()),
                project_name: Some(s.project_name.clone()),
                at: s.started_at.clone(),
                urgency: 0,
            });
        }
    }

    entries.extend(attention_entries(attention));

    // failure: failed sessions within the last 24h of `now`.
    let cutoff = cutoff_24h_before(now);
    for s in sessions {
        if s.status != SessionStatus::Failed {
            continue;
        }
        let at = s.ended_at.clone().unwrap_or_else(|| s.started_at.clone());
        let in_window = match &cutoff {
            Some(c) => at.as_str() >= c.as_str(),
            None => true,
        };
        if in_window {
            entries.push(InboxEntry {
                kind: "failure".to_string(),
                action: "open".to_string(),
                title: session_title(s),
                detail: None,
                session_id: Some(s.id.clone()),
                fingerprint: None,
                automation_id: None,
                project_id: Some(s.project_id.clone()),
                project_name: Some(s.project_name.clone()),
                at,
                urgency: 2,
            });
        }
    }

    // skippedAutomation: recurrently-skipped automations.
    // Group runs by automation_id, re-deriving the latest ourselves.
    let mut by_automation: HashMap<&str, Vec<&AutomationRun>> = HashMap::new();
    for r in runs {
        by_automation
            .entry(r.automation_id.as_str())
            .or_default()
            .push(r);
    }
    for (_automation_id, mut group) in by_automation {
        group.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        let latest = match group.last() {
            Some(r) => *r,
            None => continue,
        };
        if latest.status != "skipped" {
            continue;
        }
        // Anchor is the automation's latest run's `started_at`, not `now`.
        let window_start = cutoff_24h_before(&latest.started_at);
        let skipped_in_window = group
            .iter()
            .filter(|r| {
                r.status == "skipped"
                    && match &window_start {
                        Some(c) => r.started_at.as_str() >= c.as_str(),
                        None => true,
                    }
            })
            .count();
        if skipped_in_window < 2 {
            continue;
        }
        entries.push(InboxEntry {
            kind: "skippedAutomation".to_string(),
            action: "dismiss".to_string(),
            title: latest.automation_name.clone(),
            detail: latest.note.clone(),
            session_id: None,
            fingerprint: None,
            automation_id: Some(latest.automation_id.clone()),
            project_id: None,
            project_name: None,
            at: latest.started_at.clone(),
            urgency: 3,
        });
    }

    // runningSession: running session with no pending question.
    for s in sessions {
        if s.status == SessionStatus::Running && s.pending_question.is_none() {
            entries.push(InboxEntry {
                kind: "runningSession".to_string(),
                action: "open".to_string(),
                title: session_title(s),
                detail: None,
                session_id: Some(s.id.clone()),
                fingerprint: None,
                automation_id: None,
                project_id: Some(s.project_id.clone()),
                project_name: Some(s.project_name.clone()),
                at: s.started_at.clone(),
                urgency: 4,
            });
        }
    }

    entries.sort_by(|a, b| a.urgency.cmp(&b.urgency).then_with(|| b.at.cmp(&a.at)));
    entries
}

#[cfg(test)]
#[path = "../tests/unit/inbox_tests.rs"]
mod inbox_tests;
