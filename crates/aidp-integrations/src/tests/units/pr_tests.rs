use super::*;
use serde_json::json;

const ACTIONABLE_THREADS: &str = include_str!("fixtures/pr/graphql_actionable_threads.json");
const EXPECTED_ACTIONABLE: [&str; 5] = [
    "human-root:human-root-comment",
    "own-root:own-root-comment",
    "author-root:author-root-comment",
    "human-followup:human-followup-comment",
    "outdated:outdated-comment",
];
const SUBMITTED_BOT: &str = "copilot-pull-request-reviewer[bot]";
const OPEN_PENDING: &str = include_str!("fixtures/pr/graphql_open_pending.json");
const DRAFT: &str = include_str!("fixtures/pr/graphql_draft.json");
const MERGED: &str = include_str!("fixtures/pr/graphql_merged.json");
const NO_CONTEXTS: &str = include_str!("fixtures/pr/graphql_no_contexts.json");
const RULES_ACTIVE: &str = include_str!("fixtures/pr/rules_thread_resolution.json");
const RULES_NONE: &str = include_str!("fixtures/pr/rules_none.json");
const COMPARE: &str = include_str!("fixtures/pr/compare.json");
const RULES_COUNT_ZERO: &str = include_str!("fixtures/pr/rules_approval_count_zero.json");
const RULES_NO_PULL_REQUEST: &str = include_str!("fixtures/pr/rules_no_pull_request.json");
const RULES_COUNT_TWO: &str = include_str!("fixtures/pr/rules_approval_count_two.json");
const RULES_NOT_FOUND: &str = include_str!("fixtures/pr/rules_not_found.json");
const OPEN_NO_DECISION: &str = include_str!("fixtures/pr/graphql_open_no_decision.json");
const OPEN_ONE_APPROVAL: &str =
    include_str!("fixtures/pr/graphql_open_one_approval_no_decision.json");

const DECISION_KEY: &str = "reviewDecision";
const PULL_REQUEST_KEY: &str = "pullRequest";
const REPOSITORY_KEY: &str = "repository";
const DECISION_APPROVED: &str = "APPROVED";
const DECISION_REVIEW_REQUIRED: &str = "REVIEW_REQUIRED";
const DECISION_CHANGES_REQUESTED: &str = "CHANGES_REQUESTED";
const REASON_KEY: &str = "reason";
const ITEMS_KEY: &str = "items";
const ID_KEY: &str = "id";
const APPROVED_ID: &str = "approved";
const ONE_REQUIRED: u64 = 1;
const TWO_REQUIRED: u64 = 2;
const ZERO_REQUIRED: u64 = 0;

const READ_AT: &str = "2026-09-28T14:00:00Z";
const NO_RULES: BranchRules = BranchRules {
    require_thread_resolution: false,
    required_approving_review_count: 0,
    degraded: false,
};
const ONE_APPROVAL_RULES: BranchRules = BranchRules {
    require_thread_resolution: false,
    required_approving_review_count: 1,
    degraded: false,
};
const ACTIVE_RULES: BranchRules = BranchRules {
    require_thread_resolution: true,
    required_approving_review_count: 0,
    degraded: false,
};
const BROKEN_RULES: BranchRules = BranchRules {
    require_thread_resolution: false,
    required_approving_review_count: 0,
    degraded: true,
};

fn rollup(nodes: Value) -> Value {
    json!({"contexts": {"nodes": nodes}})
}

fn check_run(name: &str, status: &str, conclusion: Option<&str>, started: &str) -> Value {
    json!({"__typename": "CheckRun", "name": name, "status": status,
           "conclusion": conclusion, "detailsUrl": "u", "startedAt": started})
}

fn status_context(name: &str, state: &str) -> Value {
    json!({"__typename": "StatusContext", "context": name, "state": state, "targetUrl": "u"})
}

fn review(login: &str, state: &str) -> Value {
    json!({"author": {"login": login}, "state": state, "submittedAt": "t"})
}

fn pr_with_reviews(decision: Value, reviews: Vec<Value>) -> Value {
    json!({"reviewDecision": decision, "latestOpinionatedReviews": {"nodes": reviews},
           "reviewRequests": {"nodes": []}})
}

fn data(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

fn status_of(raw: &str, behind: Option<u64>, rules: BranchRules) -> PrStatus {
    build_status("o/r", &data(raw), behind, rules, READ_AT.to_string()).unwrap()
}

fn facts() -> GateFacts {
    GateFacts {
        state: PrState::Open,
        is_draft: false,
        review_decision: ReviewDecision::Approved,
        reviews_degraded: false,
        checks_verdict: ChecksVerdict::Passed,
        checks_degraded: false,
        mergeable: Mergeable::Mergeable,
        unresolved_threads: 0,
        require_thread_resolution: false,
        review_decision_from_github: true,
        required_approvals: Some(ONE_REQUIRED),
        approving_reviewers: 1,
    }
}

fn with_decision(raw: &str, decision: Value) -> Value {
    let mut document = data(raw);
    document[REPOSITORY_KEY][PULL_REQUEST_KEY][DECISION_KEY] = decision;
    document
}

fn status_with(document: &Value, rules: BranchRules) -> PrStatus {
    build_status("o/r", document, None, rules, READ_AT.to_string()).unwrap()
}

fn approved_item(gate: &Gate) -> &GateItem {
    gate.items
        .iter()
        .find(|entry| entry.id == GateItemId::Approved)
        .unwrap()
}

fn approval_facts(
    decision: ReviewDecision,
    from_github: bool,
    required: Option<u64>,
    approvals: u64,
) -> GateFacts {
    let mut input = facts();
    input.review_decision = decision;
    input.review_decision_from_github = from_github;
    input.required_approvals = required;
    input.approving_reviewers = approvals;
    input
}

fn count_sentence(have: u64, required: u64) -> String {
    format!(
        "{}{have}{}{required}{}",
        reason::APPROVALS_PREFIX,
        reason::APPROVALS_OF,
        reason::APPROVALS_SUFFIX
    )
}

fn failing_item(gate: &Gate) -> Option<GateItemId> {
    gate.items
        .iter()
        .find(|entry| !entry.ok)
        .map(|entry| entry.id)
}

fn item_ok(gate: &Gate, id: GateItemId) -> bool {
    gate.items.iter().find(|entry| entry.id == id).unwrap().ok
}

#[test]
fn checks_verdict_table() {
    let cases: Vec<(&str, Value, ChecksVerdict)> = vec![
        ("null rollup", Value::Null, ChecksVerdict::None),
        ("empty contexts", rollup(json!([])), ChecksVerdict::None),
        (
            "failed run rerun green",
            rollup(json!([
                check_run(
                    "build",
                    "COMPLETED",
                    Some("FAILURE"),
                    "2026-01-01T00:00:00Z"
                ),
                check_run(
                    "build",
                    "COMPLETED",
                    Some("SUCCESS"),
                    "2026-01-01T01:00:00Z"
                ),
            ])),
            ChecksVerdict::Passed,
        ),
        (
            "latest run is the failure",
            rollup(json!([
                check_run(
                    "build",
                    "COMPLETED",
                    Some("SUCCESS"),
                    "2026-01-01T00:00:00Z"
                ),
                check_run(
                    "build",
                    "COMPLETED",
                    Some("FAILURE"),
                    "2026-01-01T01:00:00Z"
                ),
            ])),
            ChecksVerdict::Failed,
        ),
        (
            "pending beats failed",
            rollup(json!([
                check_run("a", "IN_PROGRESS", None, "t1"),
                check_run("b", "COMPLETED", Some("FAILURE"), "t2"),
            ])),
            ChecksVerdict::Pending,
        ),
        (
            "neutral and skipped pass",
            rollup(json!([
                check_run("a", "COMPLETED", Some("NEUTRAL"), "t1"),
                check_run("b", "COMPLETED", Some("SKIPPED"), "t2"),
            ])),
            ChecksVerdict::Passed,
        ),
        (
            "status context error alone",
            rollup(json!([status_context("ci", "ERROR")])),
            ChecksVerdict::Failed,
        ),
        (
            "status context success",
            rollup(json!([status_context("ci", "SUCCESS")])),
            ChecksVerdict::Passed,
        ),
        (
            "status context expected",
            rollup(json!([status_context("ci", "EXPECTED")])),
            ChecksVerdict::Pending,
        ),
        (
            "status context pending",
            rollup(json!([status_context("ci", "PENDING")])),
            ChecksVerdict::Pending,
        ),
        (
            "status context failure",
            rollup(json!([status_context("ci", "FAILURE")])),
            ChecksVerdict::Failed,
        ),
    ];
    for (label, input, expected) in cases {
        let report = checks_verdict(&input);
        assert_eq!(report.verdict, expected, "{label}");
        assert!(!report.degraded, "{label}");
    }
}

#[test]
fn checks_verdict_keeps_one_run_per_name() {
    let report = checks_verdict(&rollup(json!([
        check_run(
            "build",
            "COMPLETED",
            Some("FAILURE"),
            "2026-01-01T00:00:00Z"
        ),
        check_run(
            "build",
            "COMPLETED",
            Some("SUCCESS"),
            "2026-01-01T01:00:00Z"
        ),
    ])));
    assert_eq!(report.runs.len(), 1);
    assert_eq!(report.runs[0].conclusion, Some(CheckConclusion::Success));
}

#[test]
fn missing_contexts_key_is_degraded_but_empty_array_is_not() {
    assert!(checks_verdict(&json!({})).degraded);
    assert!(!checks_verdict(&rollup(json!([]))).degraded);
}

#[test]
fn review_state_table() {
    let cases: Vec<(&str, Value, ReviewDecision, ReviewSource)> = vec![
        (
            "github decision wins",
            pr_with_reviews(json!("CHANGES_REQUESTED"), vec![review("a", "APPROVED")]),
            ReviewDecision::ChangesRequested,
            ReviewSource::Github,
        ),
        (
            "null decision with approval",
            pr_with_reviews(Value::Null, vec![review("alice", "APPROVED")]),
            ReviewDecision::Approved,
            ReviewSource::Derived,
        ),
        (
            "approval and changes requested",
            pr_with_reviews(
                Value::Null,
                vec![
                    review("alice", "APPROVED"),
                    review("bob", "CHANGES_REQUESTED"),
                ],
            ),
            ReviewDecision::ChangesRequested,
            ReviewSource::Derived,
        ),
        (
            "dismissed and commented do not approve",
            pr_with_reviews(
                Value::Null,
                vec![review("a", "DISMISSED"), review("b", "COMMENTED")],
            ),
            ReviewDecision::ReviewRequired,
            ReviewSource::Derived,
        ),
        (
            "no reviews",
            pr_with_reviews(Value::Null, vec![]),
            ReviewDecision::ReviewRequired,
            ReviewSource::Derived,
        ),
    ];
    for (label, input, decision, source) in cases {
        let report = review_state(&input);
        assert_eq!(report.decision, decision, "{label}");
        assert_eq!(report.source, source, "{label}");
        assert!(!report.degraded, "{label}");
    }
}

#[test]
fn unreadable_reviews_are_degraded() {
    assert!(review_state(&json!({"reviewDecision": null})).degraded);
}

#[test]
fn gate_is_ok_when_every_condition_holds() {
    let gate = merge_gate(&facts());
    assert!(gate.ok);
    assert_eq!(gate.reason, None);
    let ids: Vec<GateItemId> = gate.items.iter().map(|entry| entry.id).collect();
    assert_eq!(
        ids,
        [
            GateItemId::Open,
            GateItemId::NotDraft,
            GateItemId::NoChangesRequested,
            GateItemId::Approved,
            GateItemId::Checks,
            GateItemId::NoConflict,
            GateItemId::Threads,
        ]
    );
}

#[test]
fn gate_blocks_on_the_first_failing_item() {
    type Tweak = fn(&mut GateFacts);
    let cases: Vec<(&str, Tweak, GateItemId, &str)> = vec![
        (
            "draft",
            |f| f.is_draft = true,
            GateItemId::NotDraft,
            reason::DRAFT,
        ),
        (
            "merged",
            |f| f.state = PrState::Merged,
            GateItemId::Open,
            reason::MERGED,
        ),
        (
            "closed",
            |f| f.state = PrState::Closed,
            GateItemId::Open,
            reason::CLOSED,
        ),
        (
            "unreadable state",
            |f| f.state = PrState::Unknown,
            GateItemId::Open,
            reason::STATE_UNREADABLE,
        ),
        (
            "changes requested",
            |f| f.review_decision = ReviewDecision::ChangesRequested,
            GateItemId::NoChangesRequested,
            reason::CHANGES_REQUESTED,
        ),
        (
            "not approved",
            |f| f.review_decision = ReviewDecision::ReviewRequired,
            GateItemId::Approved,
            reason::NOT_APPROVED,
        ),
        (
            "reviews degraded",
            |f| f.reviews_degraded = true,
            GateItemId::Approved,
            reason::REVIEWS_UNREADABLE,
        ),
        (
            "checks pending",
            |f| f.checks_verdict = ChecksVerdict::Pending,
            GateItemId::Checks,
            reason::CHECKS_PENDING,
        ),
        (
            "checks failed",
            |f| f.checks_verdict = ChecksVerdict::Failed,
            GateItemId::Checks,
            reason::CHECKS_FAILED,
        ),
        (
            "checks degraded",
            |f| f.checks_degraded = true,
            GateItemId::Checks,
            reason::CHECKS_UNREADABLE,
        ),
        (
            "conflict",
            |f| f.mergeable = Mergeable::Conflicting,
            GateItemId::NoConflict,
            reason::CONFLICT,
        ),
        (
            "two threads with rule",
            |f| {
                f.unresolved_threads = 2;
                f.require_thread_resolution = true;
            },
            GateItemId::Threads,
            "2 review conversations are still unresolved",
        ),
        (
            "one thread with rule",
            |f| {
                f.unresolved_threads = 1;
                f.require_thread_resolution = true;
            },
            GateItemId::Threads,
            reason::ONE_THREAD,
        ),
    ];
    for (label, tweak, item_id, sentence) in cases {
        let mut input = facts();
        tweak(&mut input);
        let gate = merge_gate(&input);
        assert!(!gate.ok, "{label}");
        assert_eq!(failing_item(&gate), Some(item_id), "{label}");
        assert_eq!(gate.reason.as_deref(), Some(sentence), "{label}");
    }
}

#[test]
fn unknown_mergeable_and_unruled_threads_do_not_block() {
    let mut input = facts();
    input.mergeable = Mergeable::Unknown;
    input.unresolved_threads = 2;
    input.require_thread_resolution = false;
    assert!(merge_gate(&input).ok);
}

#[test]
fn earlier_items_win_over_later_ones() {
    let mut input = facts();
    input.is_draft = true;
    input.checks_verdict = ChecksVerdict::Failed;
    assert_eq!(merge_gate(&input).reason.as_deref(), Some(reason::DRAFT));
}

#[test]
fn rules_helper_reads_the_pull_request_rule() {
    assert!(rules_require_thread_resolution(&data(RULES_ACTIVE)));
    assert!(!rules_require_thread_resolution(&data(RULES_NONE)));
    assert!(!rules_require_thread_resolution(&data(RULES_NOT_FOUND)));
}

#[test]
fn open_pr_with_pending_checks_blocks_on_checks() {
    let behind = data(COMPARE)["behind_by"].as_u64();
    let status = status_of(OPEN_PENDING, behind, NO_RULES);
    assert_eq!(status.repo, "o/r");
    assert_eq!(status.number, 123);
    assert_eq!(status.state, PrState::Open);
    assert_eq!(status.viewer.as_deref(), Some("me"));
    assert_eq!(status.review.source, ReviewSource::Derived);
    assert_eq!(status.review.decision, ReviewDecision::Approved);
    assert_eq!(status.review.requested, ["bob", "core-team"]);
    assert_eq!(status.checks.verdict, ChecksVerdict::Pending);
    assert_eq!(status.checks.runs.len(), 3);
    assert_eq!(status.behind_by, Some(3));
    assert_eq!(status.threads.unresolved, 2);
    assert!(!status.threads.require_resolution);
    assert_eq!(status.gate.reason.as_deref(), Some(reason::CHECKS_PENDING));
    assert!(status.degraded.is_empty());
}

#[test]
fn two_open_threads_block_only_when_the_rule_is_active() {
    let without = status_of(OPEN_PENDING, None, NO_RULES);
    assert!(item_ok(&without.gate, GateItemId::Threads));
    let with = status_of(OPEN_PENDING, None, ACTIVE_RULES);
    assert!(with.threads.require_resolution);
    assert!(!item_ok(&with.gate, GateItemId::Threads));
}

#[test]
fn rules_error_is_degraded_and_threads_do_not_block() {
    let status = status_of(OPEN_PENDING, None, BROKEN_RULES);
    assert_eq!(status.degraded, [DegradedRead::Rules]);
    assert!(item_ok(&status.gate, GateItemId::Threads));
}

#[test]
fn draft_pr_blocks_on_not_draft() {
    let status = status_of(DRAFT, None, NO_RULES);
    assert_eq!(failing_item(&status.gate), Some(GateItemId::NotDraft));
    assert_eq!(status.gate.reason.as_deref(), Some(reason::DRAFT));
    assert_eq!(status.checks.verdict, ChecksVerdict::None);
    assert!(item_ok(&status.gate, GateItemId::Checks));
    assert!(item_ok(&status.gate, GateItemId::NoConflict));
}

#[test]
fn merged_pr_blocks_on_open_and_drops_behind_by() {
    let status = status_of(MERGED, Some(5), NO_RULES);
    assert_eq!(failing_item(&status.gate), Some(GateItemId::Open));
    assert_eq!(status.behind_by, None);
    assert!(!needs_compare(status.state));
    let serialized = serde_json::to_value(&status).unwrap();
    assert!(serialized.get("behindBy").is_none());
}

#[test]
fn missing_contexts_degrade_checks_and_block() {
    let status = status_of(NO_CONTEXTS, None, NO_RULES);
    assert_eq!(status.degraded, [DegradedRead::Checks]);
    assert_eq!(failing_item(&status.gate), Some(GateItemId::Checks));
}

#[test]
fn status_serializes_the_camel_case_contract() {
    let status = status_of(OPEN_PENDING, Some(3), NO_RULES);
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(value["isDraft"], false);
    assert_eq!(value["head"]["ref"], "feat/X-1");
    assert_eq!(value["head"]["sha"], "abc123");
    assert_eq!(value["base"]["ref"], "master");
    assert_eq!(value["mergeable"], "mergeable");
    assert_eq!(value["behindBy"], 3);
    assert_eq!(value["threads"]["requireResolution"], false);
    assert_eq!(value["gate"]["items"][0], json!({"id": "open", "ok": true}));
    assert_eq!(value["readAt"], READ_AT);
}

#[test]
fn rules_approval_count_reads_each_fixture() {
    let cases: Vec<(&str, &str, u64)> = vec![
        ("one", RULES_NONE, ONE_REQUIRED),
        ("zero", RULES_COUNT_ZERO, ZERO_REQUIRED),
        ("no pull request rule", RULES_NO_PULL_REQUEST, ZERO_REQUIRED),
        ("max of two rules", RULES_COUNT_TWO, TWO_REQUIRED),
    ];
    for (label, raw, expected) in cases {
        assert_eq!(
            rules_required_approving_review_count(&data(raw)),
            expected,
            "{label}"
        );
    }
    assert_eq!(
        rules_required_approving_review_count(&data(RULES_NOT_FOUND)),
        ZERO_REQUIRED
    );
}

#[test]
fn github_approved_passes_without_a_note() {
    let gate = merge_gate(&approval_facts(
        ReviewDecision::Approved,
        true,
        Some(ONE_REQUIRED),
        0,
    ));
    assert!(gate.ok);
    assert_eq!(gate.reason, None);
    assert!(approved_item(&gate).ok);
    assert_eq!(approved_item(&gate).reason, None);
}

#[test]
fn github_review_required_fails_even_with_count_zero() {
    for required in [Some(ZERO_REQUIRED), None] {
        let gate = merge_gate(&approval_facts(
            ReviewDecision::ReviewRequired,
            true,
            required,
            5,
        ));
        assert!(!gate.ok);
        assert!(!approved_item(&gate).ok);
        assert_eq!(
            approved_item(&gate).reason.as_deref(),
            Some(reason::NOT_APPROVED)
        );
        assert_eq!(gate.reason.as_deref(), Some(reason::NOT_APPROVED));
    }
}

#[test]
fn changes_requested_blocks_in_every_rules_case() {
    for from_github in [true, false] {
        for required in [Some(ZERO_REQUIRED), Some(ONE_REQUIRED), None] {
            let gate = merge_gate(&approval_facts(
                ReviewDecision::ChangesRequested,
                from_github,
                required,
                1,
            ));
            assert!(!gate.ok);
            assert_eq!(failing_item(&gate), Some(GateItemId::NoChangesRequested));
            assert_eq!(gate.reason.as_deref(), Some(reason::CHANGES_REQUESTED));
            assert!(!item_ok(&gate, GateItemId::Approved));
        }
    }
}

#[test]
fn empty_decision_with_threshold_one_and_no_approval_fails_with_the_count() {
    let status = status_with(&data(OPEN_NO_DECISION), ONE_APPROVAL_RULES);
    assert_eq!(status.review.source, ReviewSource::Derived);
    let item = approved_item(&status.gate);
    let expected = count_sentence(0, ONE_REQUIRED);
    assert!(!item.ok);
    assert_eq!(item.reason.as_deref(), Some(expected.as_str()));
    assert_eq!(status.gate.reason.as_deref(), Some(expected.as_str()));
}

#[test]
fn empty_decision_with_threshold_one_and_one_approval_passes() {
    let status = status_with(&data(OPEN_ONE_APPROVAL), ONE_APPROVAL_RULES);
    let item = approved_item(&status.gate);
    assert!(item.ok);
    assert_eq!(item.reason, None);
}

#[test]
fn threshold_two_needs_two_approvals() {
    let rules = BranchRules {
        required_approving_review_count: rules_required_approving_review_count(&data(
            RULES_COUNT_TWO,
        )),
        ..NO_RULES
    };
    assert_eq!(rules.required_approving_review_count, TWO_REQUIRED);
    let status = status_with(&data(OPEN_ONE_APPROVAL), rules);
    let item = approved_item(&status.gate);
    let expected = count_sentence(1, TWO_REQUIRED);
    assert!(!item.ok);
    assert_eq!(item.reason.as_deref(), Some(expected.as_str()));
}

#[test]
fn empty_decision_without_approval_rule_passes_with_a_note() {
    let cases: Vec<(&str, &str)> = vec![
        ("count zero", RULES_COUNT_ZERO),
        ("no pull request rule", RULES_NO_PULL_REQUEST),
    ];
    for (label, raw) in cases {
        let rules = BranchRules {
            required_approving_review_count: rules_required_approving_review_count(&data(raw)),
            ..NO_RULES
        };
        let status = status_with(&data(OPEN_NO_DECISION), rules);
        let item = approved_item(&status.gate);
        assert!(item.ok, "{label}");
        assert_eq!(
            item.reason.as_deref(),
            Some(reason::NO_APPROVAL_RULE),
            "{label}"
        );
    }
}

#[test]
fn empty_decision_with_unreadable_rules_passes_and_stays_degraded() {
    let status = status_with(&data(OPEN_NO_DECISION), BROKEN_RULES);
    assert_eq!(status.degraded, [DegradedRead::Rules]);
    let item = approved_item(&status.gate);
    assert!(item.ok);
    assert_eq!(item.reason.as_deref(), Some(reason::NO_APPROVAL_RULE));
}

#[test]
fn github_decision_from_a_fixture_overrides_the_rules() {
    let approved = status_with(
        &with_decision(OPEN_NO_DECISION, json!(DECISION_APPROVED)),
        ONE_APPROVAL_RULES,
    );
    assert_eq!(approved.review.source, ReviewSource::Github);
    assert!(approved_item(&approved.gate).ok);
    let required = status_with(
        &with_decision(OPEN_ONE_APPROVAL, json!(DECISION_REVIEW_REQUIRED)),
        NO_RULES,
    );
    assert_eq!(
        approved_item(&required.gate).reason.as_deref(),
        Some(reason::NOT_APPROVED)
    );
    let changes = status_with(
        &with_decision(OPEN_ONE_APPROVAL, json!(DECISION_CHANGES_REQUESTED)),
        NO_RULES,
    );
    assert_eq!(
        changes.gate.reason.as_deref(),
        Some(reason::CHANGES_REQUESTED)
    );
}

#[test]
fn gate_reason_ignores_notes_on_passing_items() {
    let gate = merge_gate(&approval_facts(
        ReviewDecision::ReviewRequired,
        false,
        Some(ZERO_REQUIRED),
        0,
    ));
    assert!(approved_item(&gate).ok);
    assert_eq!(
        approved_item(&gate).reason.as_deref(),
        Some(reason::NO_APPROVAL_RULE)
    );
    assert!(gate.ok);
    assert_eq!(gate.reason, None);
}

#[test]
fn serialized_gate_carries_the_approved_item_reason() {
    let gate = merge_gate(&approval_facts(
        ReviewDecision::ReviewRequired,
        false,
        None,
        0,
    ));
    let value = serde_json::to_value(&gate).unwrap();
    let approved = value[ITEMS_KEY]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[ID_KEY] == APPROVED_ID)
        .unwrap();
    assert_eq!(approved[REASON_KEY], reason::NO_APPROVAL_RULE);
    assert!(value[ITEMS_KEY][0].get(REASON_KEY).is_none());
}

/// Submitted comments detect Copilot without changing opinionated approval decisions.
#[test]
fn submitted_commented_reviews_preserve_the_approval_gate() {
    let status = status_of(ACTIONABLE_THREADS, None, NO_RULES);
    assert_eq!(status.review.decision, ReviewDecision::Approved);
    assert_eq!(status.review.reviewers.len(), 1);
    assert_eq!(status.review.submitted.len(), 1);
    assert_eq!(status.review.submitted[0].login, SUBMITTED_BOT);
    assert_eq!(status.review.submitted[0].state, ReviewState::Commented);
}

/// Initial and outdated comments need handling; resolved threads and author replies do not.
#[test]
fn actionable_threads_distinguish_roots_replies_and_human_followups() {
    let status = status_of(ACTIONABLE_THREADS, None, NO_RULES);
    assert_eq!(status.threads.unresolved, 7);
    assert_eq!(status.threads.actionable, Some(5));
    assert_eq!(status.threads.actionable_ids.unwrap(), EXPECTED_ACTIONABLE);
}

/// Truncated thread data remains unknown rather than clean.
#[test]
fn incomplete_thread_pages_never_report_clean() {
    let mut document = data(ACTIONABLE_THREADS);
    document[wire::REPOSITORY][wire::PULL_REQUEST][wire::REVIEW_THREADS][wire::PAGE_INFO]
        [wire::HAS_NEXT_PAGE] = Value::Bool(true);
    let status = status_with(&document, NO_RULES);
    assert_eq!(status.threads.actionable, None);
}

/// GraphQL pagination retains every page's threads and its terminal page state.
#[test]
fn paginated_thread_connections_are_combined_before_counting() {
    let mut first = data(ACTIONABLE_THREADS);
    first[wire::REPOSITORY][wire::PULL_REQUEST][wire::REVIEW_THREADS][wire::PAGE_INFO]
        [wire::HAS_NEXT_PAGE] = Value::Bool(true);
    let last = data(ACTIONABLE_THREADS);
    let pages = Value::Array(vec![json!({wire::DATA: first}), json!({wire::DATA: last})]);
    let merged = merge_graphql_pages(pages).unwrap();
    let status = status_with(&merged, NO_RULES);
    assert_eq!(status.threads.unresolved, 14);
    assert_eq!(status.threads.actionable, Some(10));
}
