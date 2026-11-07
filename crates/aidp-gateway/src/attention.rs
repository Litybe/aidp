//! Pure computation of the attention signals of one pull request.

use aidp_integrations::models::notifications_pr_config::NotificationsPrConfig;
use aidp_integrations::pr_comments::is_bot;

use crate::models::attention_facts::AttentionFacts;
use crate::models::attention_signal::AttentionSignal;
use crate::models::enums::attention_signal_kind::AttentionSignalKind;

const FINGERPRINT_SEPARATOR: &str = ":";
const PR_NUMBER_PREFIX: &str = "#";

/// Builds the dedup key `<repo>#<n>:<kind>:<sha>[:<detail>]`.
pub fn fingerprint(
    repo: &str,
    number: u64,
    kind: AttentionSignalKind,
    sha: &str,
    detail: Option<&str>,
) -> String {
    let base = format!(
        "{repo}{PR_NUMBER_PREFIX}{number}{FINGERPRINT_SEPARATOR}{}{FINGERPRINT_SEPARATOR}{sha}",
        kind.as_str()
    );
    match detail {
        Some(detail) => format!("{base}{FINGERPRINT_SEPARATOR}{detail}"),
        None => base,
    }
}

/// Whether the config enables this kind.
fn enabled(kind: AttentionSignalKind, config: &NotificationsPrConfig) -> bool {
    match kind {
        AttentionSignalKind::NewComment => config.new_comment,
        AttentionSignalKind::ChangesRequested => config.changes_requested,
        AttentionSignalKind::Conflict => config.conflict,
        AttentionSignalKind::Mergeable => config.mergeable,
        AttentionSignalKind::ReviewRequested => config.review_requested,
    }
}

/// Builds a signal of `kind` with its fingerprint.
fn signal(
    facts: &AttentionFacts,
    kind: AttentionSignalKind,
    detail: Option<&str>,
    notify: bool,
) -> AttentionSignal {
    AttentionSignal {
        kind,
        fingerprint: fingerprint(&facts.repo, facts.number, kind, &facts.head_sha, detail),
        detail: detail.map(str::to_string),
        notify,
    }
}

/// One signal per fresh human comment from someone else.
fn comment_signals(facts: &AttentionFacts) -> Vec<AttentionSignal> {
    facts
        .comments
        .iter()
        .filter(|comment| comment.author != facts.me)
        .filter(|comment| !comment.body_marker)
        .filter(|comment| !is_bot(&comment.author, &facts.bot_authors))
        .filter(|comment| match &facts.last_seen {
            Some(seen) => comment.created_at.as_str() > seen.as_str(),
            None => true,
        })
        .map(|comment| {
            signal(
                facts,
                AttentionSignalKind::NewComment,
                Some(&comment.id),
                true,
            )
        })
        .collect()
}

/// Signals for a pull request authored by the viewer.
fn own_signals(facts: &AttentionFacts) -> Vec<AttentionSignal> {
    let mut signals = comment_signals(facts);
    for reviewer in &facts.changes_requested_by {
        signals.push(signal(
            facts,
            AttentionSignalKind::ChangesRequested,
            Some(reviewer),
            true,
        ));
    }
    if facts.conflict {
        signals.push(signal(facts, AttentionSignalKind::Conflict, None, true));
    }
    if facts.gate_ok {
        signals.push(signal(facts, AttentionSignalKind::Mergeable, None, true));
    }
    signals
}

/// Signals the viewer should see for one pull request, filtered by config.
pub fn signals(facts: &AttentionFacts, config: &NotificationsPrConfig) -> Vec<AttentionSignal> {
    let computed = if facts.author == facts.me {
        own_signals(facts)
    } else if facts.review_requests.contains(&facts.me) {
        vec![signal(
            facts,
            AttentionSignalKind::ReviewRequested,
            None,
            true,
        )]
    } else {
        Vec::new()
    };
    computed
        .into_iter()
        .filter(|candidate| enabled(candidate.kind, config))
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/attention_tests.rs"]
mod attention_tests;
