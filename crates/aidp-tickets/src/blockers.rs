//! Pure blocker logic: which linked issues still block a ticket, which issues
//! a body names as blockers, and whether a ticket sits in a blocker cycle.

use crate::models::blocker_link::BlockerLink;
use crate::models::constants::github::{
    BLOCKER_SENTENCE_PATTERN, ISSUE_REFERENCE_PATTERN, VALID_REGEX,
};
use crate::models::enums::issue_state::IssueState;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

/// One issue a body names as a blocker: `repo` is `None` for a bare `#12` (the same repository).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BodyReference {
    pub repo: Option<String>,
    pub number: u64,
}

/// Keys of the blockers that are still open.
pub fn open(blocked_by: &[BlockerLink]) -> Vec<String> {
    blocked_by
        .iter()
        .filter(|blocker| blocker.state == IssueState::Open)
        .map(|blocker| blocker.key.clone())
        .collect()
}

/// The issues a body declares as blockers: every `#n` or `owner/name#n` that follows
/// « Dépend de », « Depends on », « Blocked by » or « Bloqué par » on the same line,
/// in order of appearance, without duplicates.
pub fn body_references(body: &str) -> Vec<BodyReference> {
    let sentence = Regex::new(BLOCKER_SENTENCE_PATTERN).expect(VALID_REGEX);
    let reference = Regex::new(ISSUE_REFERENCE_PATTERN).expect(VALID_REGEX);
    let mut found: Vec<BodyReference> = Vec::new();
    for line in body.lines() {
        let Some(start) = sentence.find(line) else {
            continue;
        };
        for capture in reference.captures_iter(&line[start.end()..]) {
            let Some(number) = capture.get(2).and_then(|m| m.as_str().parse::<u64>().ok()) else {
                continue;
            };
            let candidate = BodyReference {
                repo: capture.get(1).map(|m| m.as_str().to_string()),
                number,
            };
            if !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

/// True when `key` reaches itself through the open blocker edges of `graph`.
pub fn in_cycle(key: &str, graph: &BTreeMap<String, Vec<String>>) -> bool {
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = graph
        .get(key)
        .map(|next| next.iter().map(String::as_str).collect())
        .unwrap_or_default();
    while let Some(current) = stack.pop() {
        if current == key {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(next) = graph.get(current) {
            stack.extend(next.iter().map(String::as_str));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(repo: Option<&str>, number: u64) -> BodyReference {
        BodyReference {
            repo: repo.map(str::to_string),
            number,
        }
    }

    #[test]
    fn french_and_english_sentences_name_blockers() {
        let body =
            "Contexte.\n\nDépend de #12 et #14.\nBlocked by Litybe/aidp#3\nbloquée par #12\n";
        assert_eq!(
            body_references(body),
            vec![
                reference(None, 12),
                reference(None, 14),
                reference(Some("Litybe/aidp"), 3)
            ]
        );
    }

    #[test]
    fn depends_on_is_case_insensitive_and_a_bare_reference_is_ignored() {
        let body = "Voir #7 pour le contexte.\n- DEPENDS ON #8\n- depend de #9\n";
        assert_eq!(
            body_references(body),
            vec![reference(None, 8), reference(None, 9)]
        );
    }

    #[test]
    fn only_open_blockers_block() {
        let links = vec![
            BlockerLink {
                key: "GH-1".to_string(),
                state: IssueState::Open,
            },
            BlockerLink {
                key: "GH-2".to_string(),
                state: IssueState::Closed,
            },
        ];
        assert_eq!(open(&links), vec!["GH-1".to_string()]);
    }
}
