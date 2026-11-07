use crate::models::squad_pr::SquadPr;
use crate::squad::{squad_prs, GhRunner};
use aidp_core::clock::FixedClock;

const DETAIL: &str = include_str!("../fixtures/squad_recent_comments.json");
const LISTING: &str = include_str!("../fixtures/squad_recent_comments_list.json");
const DETAIL_QUERY_MARKER: &str = "pullRequest(number";
const REPO: &str = "acme/svc";
const ME: &str = "alice";
const NOW: &str = "2026-10-03T09:00:00Z";
const COMMENTED_PR: u64 = 7;
const BARE_PR: u64 = 8;

struct FixtureRunner;

impl GhRunner for FixtureRunner {
    fn run(&self, args: &[String]) -> Result<String, String> {
        let detail = args.iter().any(|arg| arg.contains(DETAIL_QUERY_MARKER));
        Ok(if detail { DETAIL } else { LISTING }.to_string())
    }
}

fn pr_numbered(number: u64) -> SquadPr {
    let clock = FixedClock::from_rfc3339(NOW);
    squad_prs(&[REPO.to_string()], ME, &FixtureRunner, &clock, None)
        .prs
        .into_iter()
        .find(|pr| pr.number == number)
        .expect("pull request in fixture")
}

#[test]
fn recent_comments_carry_id_author_and_marker() {
    let comments = pr_numbered(COMMENTED_PR).recent_comments;
    let summary: Vec<(&str, &str, bool)> = comments
        .iter()
        .map(|c| (c.id.as_str(), c.author.as_str(), c.body_marker))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("IC_1", "bob", false),
            ("RC_1", "carol", false),
            ("IC_2", "alice", true)
        ]
    );
}

#[test]
fn missing_comment_fields_yield_an_empty_list() {
    assert!(pr_numbered(BARE_PR).recent_comments.is_empty());
}

#[test]
fn squad_pr_without_recent_comments_key_deserializes() {
    let mut value = serde_json::to_value(pr_numbered(BARE_PR)).unwrap();
    value.as_object_mut().unwrap().remove("recentComments");
    value.as_object_mut().unwrap().remove("recent_comments");
    let parsed: SquadPr = serde_json::from_value(value).unwrap();
    assert!(parsed.recent_comments.is_empty());
}
