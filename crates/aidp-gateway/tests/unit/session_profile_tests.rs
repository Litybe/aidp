use super::*;
use crate::models::constants::session_profile::{DENY_RULES, PERMISSIONS_FILE};
use crate::models::enums::permission_policy::PermissionPolicy;

const AUTONOMOUS_FIXTURE: &str = include_str!("../fixtures/claude_argv/autonomous_session.json");
const ENV_ENABLED: &str = "1";
const ENV_DISABLED: &str = "0";
const BASE_ARG: &str = "-p";
const POLICY_KEY: &str = "policy";
const POLICY_ALLOW_WIRE: &str = "allow";

#[derive(Deserialize)]
struct AutonomousFixture {
    launch_tail: Vec<String>,
    permissions: SessionPermissions,
}

fn env_with(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(AUTONOMOUS_ENV.to_string(), value.to_string())])
}

fn fixture() -> AutonomousFixture {
    serde_json::from_str(AUTONOMOUS_FIXTURE).expect("fixture parses")
}

#[test]
fn profile_is_autonomous_only_when_env_is_one() {
    assert_eq!(
        SessionProfile::from_env(&env_with(ENV_ENABLED)),
        SessionProfile::Autonomous
    );
    assert_eq!(
        SessionProfile::from_env(&env_with(ENV_DISABLED)),
        SessionProfile::Interactive
    );
    assert_eq!(
        SessionProfile::from_env(&BTreeMap::new()),
        SessionProfile::Interactive
    );
}

#[test]
fn interactive_launch_tail_is_unchanged() {
    let tail = vec![BASE_ARG.to_string()];
    assert_eq!(
        SessionProfile::Interactive.apply_to_launch_tail(tail.clone()),
        tail
    );
}

/// The autonomous tail disallows AskUserQuestion and every deny rule from the fixture.
#[test]
fn autonomous_launch_tail_disallows_ask_user_question_and_deny_rules() {
    let tail = SessionProfile::Autonomous.apply_to_launch_tail(vec![BASE_ARG.to_string()]);
    let loaded = fixture();
    let expected: Vec<String> = std::iter::once(BASE_ARG.to_string())
        .chain(loaded.launch_tail)
        .chain(loaded.permissions.deny)
        .collect();
    assert_eq!(tail, expected);
}

/// Every constant deny rule reaches the launch tail.
#[test]
fn autonomous_launch_tail_contains_each_deny_rule() {
    let tail = SessionProfile::Autonomous.apply_to_launch_tail(Vec::new());
    for rule in DENY_RULES {
        assert!(tail.contains(&rule.to_string()), "missing {rule}");
    }
}

#[test]
fn autonomous_permissions_match_fixture_and_never_ask() {
    let permissions = SessionPermissions::autonomous();
    assert_eq!(permissions, fixture().permissions);
    assert_ne!(permissions.policy, PermissionPolicy::Ask);
}

/// The policy serializes as its lowercase wire value.
#[test]
fn permissions_serialize_policy_as_allow() {
    let value = serde_json::to_value(SessionPermissions::autonomous()).expect("serialize");
    assert_eq!(value[POLICY_KEY], POLICY_ALLOW_WIRE);
}

#[test]
fn autonomous_session_writes_permissions_file_to_the_state_dir() {
    let state_dir = tempfile::tempdir().expect("tempdir");
    SessionProfile::Autonomous
        .write_permissions(state_dir.path())
        .expect("write");
    let written: SessionPermissions = serde_json::from_str(
        &std::fs::read_to_string(state_dir.path().join(PERMISSIONS_FILE)).expect("read"),
    )
    .expect("parse");
    assert_eq!(written, fixture().permissions);
    assert_eq!(written.policy, PermissionPolicy::Allow);
}

const PR_FIX_FIXTURE: &str = include_str!("../fixtures/claude_argv/autonomous_pr_fix_session.json");
const INVALID_BRANCHES: &[&str] = &["", "a b", "a\tb", "a..b", "-x", "a*", "a)b"];
const OTHER_BRANCH: &str = "fix/x";

#[derive(Deserialize)]
struct PrFixFixture {
    branch: String,
    launch_tail: Vec<String>,
    extra_deny: Vec<String>,
}

fn pr_fix_fixture() -> PrFixFixture {
    serde_json::from_str(PR_FIX_FIXTURE).expect("pr fix fixture parses")
}

#[test]
fn push_branch_tail_matches_fixture() {
    let loaded = pr_fix_fixture();
    let tail = SessionProfile::Autonomous
        .apply_to_launch_tail_for_push_branch(Vec::new(), Some(&loaded.branch))
        .expect("tail");
    assert_eq!(tail, loaded.launch_tail);
}

#[test]
fn push_branch_permissions_add_the_fixture_deny_rules() {
    let loaded = pr_fix_fixture();
    let permissions = SessionProfile::Autonomous
        .permissions_for_push_branch(Some(&loaded.branch))
        .expect("permissions");
    let base = SessionPermissions::autonomous().deny;
    assert_eq!(permissions.deny[..base.len()], base[..]);
    assert_eq!(permissions.deny[base.len()..], loaded.extra_deny[..]);
}

#[test]
fn without_a_branch_output_equals_the_existing_fixture() {
    let tail = SessionProfile::Autonomous
        .apply_to_launch_tail_for_push_branch(Vec::new(), None)
        .expect("tail");
    let loaded = fixture();
    let expected: Vec<String> = loaded
        .launch_tail
        .into_iter()
        .chain(loaded.permissions.deny)
        .collect();
    assert_eq!(tail, expected);
    assert_eq!(
        SessionProfile::Autonomous
            .permissions_for_push_branch(None)
            .expect("permissions"),
        fixture().permissions
    );
}

#[test]
fn invalid_push_branch_names_are_rejected() {
    for branch in INVALID_BRANCHES {
        assert!(
            SessionProfile::validate_push_branch(branch).is_err(),
            "accepted {branch:?}"
        );
        assert!(SessionProfile::push_branch_allow_rules(branch).is_err());
        assert!(SessionProfile::Autonomous
            .permissions_for_push_branch(Some(branch))
            .is_err());
    }
}

#[test]
fn interactive_tail_ignores_the_push_branch() {
    let tail = vec![BASE_ARG.to_string()];
    let result = SessionProfile::Interactive
        .apply_to_launch_tail_for_push_branch(tail.clone(), Some(OTHER_BRANCH))
        .expect("tail");
    assert_eq!(result, tail);
}

#[test]
fn interactive_session_writes_no_permissions_file() {
    let state_dir = tempfile::tempdir().expect("tempdir");
    SessionProfile::Interactive
        .write_permissions(state_dir.path())
        .expect("write");
    assert!(!state_dir.path().join(PERMISSIONS_FILE).exists());
}
