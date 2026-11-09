//! Bug tickets raised by a failed test: the issue the test flow creates under its parent, the marker
//! that ties the two together, and the cap on bugs per parent. No process or network access of its
//! own: GitHub goes through [`GithubPort`].
//!
//! The marker `<!-- aidp:bug parent=272 base=feat/GH-272 -->` in the body of the bug issue is the
//! source of truth for the sync: it gives the bug mirror its parent and its base branch, and makes
//! the parent wait while the bug is open (the sync adds the bug to the parent's blockers).

use crate::github_sync::github_of;
use crate::model::Source;
use crate::models::blocker_link::BlockerLink;
use crate::models::constants::github::{
    BUG_DEFAULT_SUMMARY, BUG_LABEL, BUG_MARKER_BASE_KEY, BUG_MARKER_PARENT_KEY, BUG_MARKER_PATTERN,
    BUG_ROUTE_LABEL_PREFIX, BUG_SUMMARY_MAX_CHARS, DEFAULT_KEY_PREFIX, MAX_BUGS_PER_PARENT,
    VALID_REGEX,
};
use crate::models::enums::issue_state::IssueState;
use crate::models::github_issue::GithubIssue;
use crate::models::github_port::GithubPort;
use crate::store::{ListFilter, TicketStore};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

/// Route family a bug takes when its parent has none the routing knows.
const DEFAULT_FAMILY: &str = "code";
/// Label that makes a new issue ready for the dispatcher.
const READY_STATE: &str = "ready";

/// What the marker of a bug body says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BugMarker {
    pub parent: Option<u64>,
    pub base: Option<String>,
}

/// The marker of `body`, empty when the body carries none.
pub fn parse_marker(body: &str) -> BugMarker {
    let marker = Regex::new(BUG_MARKER_PATTERN).expect(VALID_REGEX);
    let Some(captures) = marker.captures(body) else {
        return BugMarker::default();
    };
    let mut parsed = BugMarker::default();
    for pair in captures[1].split_whitespace() {
        match pair.split_once('=') {
            Some((BUG_MARKER_PARENT_KEY, value)) => parsed.parent = value.parse().ok(),
            Some((BUG_MARKER_BASE_KEY, value)) if !value.is_empty() => {
                parsed.base = Some(value.to_string());
            }
            _ => {}
        }
    }
    parsed
}

/// The marker line of a bug under issue `parent`, whose branch starts from `base`.
pub fn marker_line(parent: u64, base: &str) -> String {
    format!("<!-- aidp:bug {BUG_MARKER_PARENT_KEY}={parent} {BUG_MARKER_BASE_KEY}={base} -->")
}

/// Open issues that name a parent in their marker, by parent number.
pub fn open_children(issues: &[GithubIssue]) -> HashMap<u64, Vec<u64>> {
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    for issue in issues {
        if let Some(parent) = parse_marker(&issue.body).parent {
            if parent != issue.number {
                children.entry(parent).or_default().push(issue.number);
            }
        }
    }
    children
}

/// `fix(<parent key>): <summary>`, the summary on one line and cut to a sensible length.
pub fn bug_title(parent_key: &str, summary: &str) -> String {
    let one_line = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary = if one_line.is_empty() {
        BUG_DEFAULT_SUMMARY.to_string()
    } else {
        one_line.chars().take(BUG_SUMMARY_MAX_CHARS).collect()
    };
    format!("fix({parent_key}): {summary}")
}

/// The body of a bug issue: marker, link to the parent, failure report and acceptance criteria.
pub fn bug_body(parent_key: &str, parent_number: u64, base: &str, report: &str) -> String {
    let report = report.trim();
    let report = if report.is_empty() {
        "Aucun rapport fourni par le test."
    } else {
        report
    };
    format!(
        "{marker}\n\
         Bug remonté par l'échec du test de {parent_key}. Parente : #{parent_number}.\n\
         Branche de base : `{base}` (la PR de ce bug vise cette branche, jamais `main`).\n\n\
         ## Rapport d'échec\n\n{report}\n\n\
         ## Critères d'acceptation\n\n\
         - CA-1 : le test qui a échoué sur #{parent_number} réussit après le correctif.\n\
         - CA-2 : aucun autre test de #{parent_number} ne régresse.\n",
        marker = marker_line(parent_number, base),
    )
}

/// What `bug-create` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BugOutcome {
    /// Mirror key of the bug (`GH-300`), empty when capped.
    pub bug: String,
    pub number: u64,
    pub url: String,
    pub parent: String,
    pub base: String,
    /// Bugs the parent holds after this call.
    pub count: usize,
    /// The parent already holds [`MAX_BUGS_PER_PARENT`] bugs: nothing was created.
    pub capped: bool,
    /// True when this call created the issue, false for a capped call or an open bug already there.
    pub changed: bool,
    /// The issue is attached as a sub-issue of the parent.
    pub linked: bool,
    pub message: String,
}

fn route_family(route: Option<&str>) -> &str {
    match route {
        Some("art") => "art",
        Some("map") => "map",
        _ => DEFAULT_FAMILY,
    }
}

fn is_bug(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(BUG_LABEL))
}

/// Creates the bug issue of a failed test of ticket `parent_key`, or says the cap is reached.
///
/// `base` is the branch of the parent: the bug starts from it and its PR targets it.
/// An open bug of the parent is reused instead of creating a second one.
pub fn create(
    store: &TicketStore,
    port: &dyn GithubPort,
    parent_key: &str,
    base: &str,
    summary: &str,
    report: &str,
) -> anyhow::Result<BugOutcome> {
    let (parent, meta) = github_of(store, parent_key)?;
    let cfg = &store.config().github;
    let repo = cfg.repo(&meta.repo).cloned();
    let key_of = |number: u64| match &repo {
        Some(repo) => repo.key_of(number),
        None => format!("{DEFAULT_KEY_PREFIX}-{number}"),
    };

    let remote = port.sub_issues(&meta.repo, meta.number).unwrap_or_default();
    let remote_bugs: Vec<_> = remote.iter().filter(|sub| is_bug(&sub.labels)).collect();
    let local_children: Vec<_> = store
        .list(&ListFilter {
            source: Some(Source::Github),
            ..ListFilter::default()
        })
        .into_iter()
        .filter_map(|ticket| ticket.github)
        .filter(|github| {
            github.repo.eq_ignore_ascii_case(&meta.repo) && github.parent == Some(meta.number)
        })
        .collect();
    let mut numbers: Vec<u64> = remote_bugs.iter().map(|sub| sub.number).collect();
    for local in &local_children {
        if !numbers.contains(&local.number) {
            numbers.push(local.number);
        }
    }
    let count = numbers.len();

    let open_remote = remote_bugs
        .iter()
        .find(|sub| sub.state == IssueState::Open)
        .map(|sub| sub.number);
    let open_local = local_children
        .iter()
        .find(|local| local.state == IssueState::Open)
        .map(|local| local.number);
    if let Some(number) = open_remote.or(open_local) {
        return Ok(BugOutcome {
            bug: key_of(number),
            number,
            url: local_children
                .iter()
                .find(|local| local.number == number)
                .map(|local| local.url.clone())
                .unwrap_or_default(),
            parent: parent_key.to_string(),
            base: base.to_string(),
            count,
            message: format!("bug #{number} already open for {parent_key}"),
            ..BugOutcome::default()
        });
    }
    if count >= MAX_BUGS_PER_PARENT {
        return Ok(BugOutcome {
            parent: parent_key.to_string(),
            base: base.to_string(),
            count,
            capped: true,
            message: format!(
                "{parent_key} already raised {count} bugs (cap {MAX_BUGS_PER_PARENT})"
            ),
            ..BugOutcome::default()
        });
    }

    let family = route_family(parent.route.as_deref());
    let mut labels = vec![
        BUG_LABEL.to_string(),
        format!("{BUG_ROUTE_LABEL_PREFIX}{family}"),
    ];
    labels.extend(cfg.state_label(READY_STATE));
    let created = port.create_issue(
        &meta.repo,
        &bug_title(parent_key, summary),
        &bug_body(parent_key, meta.number, base, report),
        &labels,
    )?;
    let linked = port
        .add_sub_issue(&meta.repo, meta.number, created.id)
        .is_ok();
    let bug_key = key_of(created.number);
    let note = format!(
        "Le test de ce ticket a échoué : bug #{} créé (branche de base `{base}`). Ce ticket attend sa correction, puis repasse en test.",
        created.number
    );
    let _ = port.comment(&meta.repo, meta.number, &note);

    // The sync derives the same blocker from the marker; writing it now closes the gap until then.
    let (mut fresh, body, extra) = store.get(parent_key)?;
    if let Some(github) = fresh.github.as_mut() {
        if !github.blocked_by.iter().any(|link| link.key == bug_key) {
            github.blocked_by.push(BlockerLink {
                key: bug_key.clone(),
                state: IssueState::Open,
            });
        }
    }
    store.save(&mut fresh, &body, &extra)?;

    Ok(BugOutcome {
        bug: bug_key,
        number: created.number,
        url: created.url,
        parent: parent_key.to_string(),
        base: base.to_string(),
        count: count + 1,
        capped: false,
        changed: true,
        linked,
        message: String::new(),
    })
}

/// Marks the blocker `key` closed on the mirror of the parent of `meta`, so the parent does not wait for
/// the next sync to go back to the test lane. Best effort: the sync derives the same state.
fn release_parent(store: &TicketStore, key: &str, meta: &crate::models::github_meta::GithubMeta) {
    let Some(parent_number) = meta.parent else {
        return;
    };
    let cfg = &store.config().github;
    let parent_key = match cfg.repo(&meta.repo) {
        Some(repo) => repo.key_of(parent_number),
        None => format!("{DEFAULT_KEY_PREFIX}-{parent_number}"),
    };
    let Ok((mut parent, body, extra)) = store.get(&parent_key) else {
        return;
    };
    let mut changed = false;
    if let Some(github) = parent.github.as_mut() {
        for link in github.blocked_by.iter_mut().filter(|link| link.key == key) {
            if link.state != IssueState::Closed {
                link.state = IssueState::Closed;
                changed = true;
            }
        }
    }
    if changed {
        let _ = store.save(&mut parent, &body, &extra);
    }
}

/// What `close` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloseOutcome {
    pub key: String,
    pub number: u64,
    pub changed: bool,
}

/// Closes the issue of mirror `key` (a bug whose PR merged into the parent branch: GitHub only closes
/// an issue by itself when the PR reaches the default branch). Already closed: nothing changes.
pub fn close(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
) -> anyhow::Result<CloseOutcome> {
    let (_, meta) = github_of(store, key)?;
    let open = match port.view(&meta.repo, meta.number)? {
        Some(issue) => issue.state == IssueState::Open,
        None => false,
    };
    if open {
        port.close_issue(&meta.repo, meta.number)?;
    }
    release_parent(store, key, &meta);
    Ok(CloseOutcome {
        key: key.to_string(),
        number: meta.number,
        changed: open,
    })
}

#[cfg(test)]
#[path = "../tests/unit/bug_ticket_tests.rs"]
mod tests;
