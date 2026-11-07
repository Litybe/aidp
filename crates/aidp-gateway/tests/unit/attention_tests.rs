use super::*;
use aidp_integrations::models::squad_comment::SquadComment;

const REPO: &str = "acme/app";
const ME: &str = "alice";
const SHA: &str = "sha1";

fn comment(id: &str, author: &str, created_at: &str, body_marker: bool) -> SquadComment {
    SquadComment {
        id: id.to_string(),
        author: author.to_string(),
        created_at: created_at.to_string(),
        body_marker,
    }
}

fn own_facts() -> AttentionFacts {
    AttentionFacts {
        repo: REPO.to_string(),
        number: 7,
        head_sha: SHA.to_string(),
        me: ME.to_string(),
        author: ME.to_string(),
        last_seen: Some("2026-01-01T00:00:00Z".to_string()),
        comments: Vec::new(),
        bot_authors: Vec::new(),
        changes_requested_by: Vec::new(),
        review_requests: Vec::new(),
        conflict: false,
        gate_ok: false,
    }
}

fn kinds(found: &[AttentionSignal]) -> Vec<AttentionSignalKind> {
    found.iter().map(|item| item.kind).collect()
}

#[test]
fn new_comment_from_someone_else() {
    let mut facts = own_facts();
    facts.comments = vec![comment("c1", "bob", "2026-02-01T00:00:00Z", false)];
    let found = signals(&facts, &NotificationsPrConfig::default());
    assert_eq!(kinds(&found), vec![AttentionSignalKind::NewComment]);
    assert_eq!(found[0].fingerprint, "acme/app#7:new_comment:sha1:c1");
    assert!(found[0].notify);
}

#[test]
fn comment_exclusions() {
    let mut facts = own_facts();
    facts.bot_authors = vec!["robot".to_string()];
    facts.comments = vec![
        comment("own", ME, "2026-02-01T00:00:00Z", false),
        comment("bot", "robot", "2026-02-01T00:00:00Z", false),
        comment("suffix", "ci[bot]", "2026-02-01T00:00:00Z", false),
        comment("marker", "bob", "2026-02-01T00:00:00Z", true),
        comment("old", "bob", "2025-01-01T00:00:00Z", false),
    ];
    assert!(signals(&facts, &NotificationsPrConfig::default()).is_empty());
}

#[test]
fn two_comments_two_signals() {
    let mut facts = own_facts();
    facts.comments = vec![
        comment("c1", "bob", "2026-02-01T00:00:00Z", false),
        comment("c2", "carol", "2026-02-02T00:00:00Z", false),
    ];
    assert_eq!(signals(&facts, &NotificationsPrConfig::default()).len(), 2);
}

#[test]
fn changes_requested_per_reviewer() {
    let mut facts = own_facts();
    facts.changes_requested_by = vec!["bob".to_string()];
    let found = signals(&facts, &NotificationsPrConfig::default());
    assert_eq!(
        found[0].fingerprint,
        "acme/app#7:changes_requested:sha1:bob"
    );
}

#[test]
fn conflict_fingerprint() {
    let mut facts = own_facts();
    facts.conflict = true;
    let found = signals(&facts, &NotificationsPrConfig::default());
    let prints: Vec<&str> = found.iter().map(|item| item.fingerprint.as_str()).collect();
    assert_eq!(prints, vec!["acme/app#7:conflict:sha1"]);
}

#[test]
fn mergeable_requires_green_gate() {
    let mut facts = own_facts();
    assert!(signals(&facts, &NotificationsPrConfig::default()).is_empty());
    facts.gate_ok = true;
    let found = signals(&facts, &NotificationsPrConfig::default());
    assert_eq!(found[0].fingerprint, "acme/app#7:mergeable:sha1");
}

#[test]
fn review_requested_only_when_asked() {
    let mut facts = own_facts();
    facts.author = "bob".to_string();
    facts.conflict = true;
    assert!(signals(&facts, &NotificationsPrConfig::default()).is_empty());
    facts.review_requests = vec![ME.to_string()];
    let found = signals(&facts, &NotificationsPrConfig::default());
    assert_eq!(kinds(&found), vec![AttentionSignalKind::ReviewRequested]);
    assert_eq!(found[0].fingerprint, "acme/app#7:review_requested:sha1");
}

#[test]
fn disabled_kind_is_not_produced() {
    let mut facts = own_facts();
    facts.conflict = true;
    let config = NotificationsPrConfig {
        conflict: false,
        ..NotificationsPrConfig::default()
    };
    assert!(signals(&facts, &config).is_empty());
}

#[test]
fn new_head_sha_changes_fingerprint() {
    let mut facts = own_facts();
    facts.conflict = true;
    let before = signals(&facts, &NotificationsPrConfig::default());
    facts.head_sha = "sha2".to_string();
    let after = signals(&facts, &NotificationsPrConfig::default());
    assert_ne!(before[0].fingerprint, after[0].fingerprint);
}
