//! Tests for the bug tickets of failed tests: creation, sub-issue link, cap, marker, and the parent
//! that waits for its bug, against an in-memory fake of the GitHub port.

use super::*;
use crate::github_sync::{has_acceptance_criteria, sync, SyncOptions};
use crate::model::{Status, Ticket};
use crate::models::constants::github::EXTRA_BUGS_AFTER_CAP;
use crate::models::created_issue::{CreatedIssue, SubIssue};
use crate::models::github_issue::IssueDependency;
use aidp_testkit::TempWorkspace;
use std::sync::Mutex;

const REPO: &str = "Litybe/PixelHood";
const SERVICE: &str = "PixelHood";
const PARENT: u64 = 272;
const PARENT_KEY: &str = "GH-272";
const PARENT_BRANCH: &str = "feat/GH-272";
const CONFIG: &str = "github:\n  repos: [Litybe/PixelHood]\n";
const ROUTING: &str = "\
rules:
  - { match: { labels: [pixelart] }, service: pixelhood-art }
  - { match: { labels: [dev] }, service: pixelhood-code }
  - { match: { labels: [\"bug:art\"] }, service: pixelhood-bug-art }
repos:
  pixelhood-code: { route: code, flow: game-code, test_flow: game-test-code }
  pixelhood-art: { route: art, flow: game-art, test_flow: game-test-art }
  pixelhood-bug-art: { route: art, flow: game-bug-art }
";

/// An in-memory GitHub with issues, sub-issues and the writes it received.
#[derive(Default)]
struct FakeGithub {
    issues: Mutex<Vec<GithubIssue>>,
    sub_issues: Mutex<Vec<(u64, SubIssue)>>,
    created: Mutex<Vec<(String, String, Vec<String>)>>,
    comments: Mutex<Vec<(u64, String)>>,
    fail_link: Mutex<bool>,
}

impl FakeGithub {
    fn close(&self, number: u64) {
        let mut issues = self.issues.lock().unwrap();
        let issue = issues.iter_mut().find(|i| i.number == number).unwrap();
        issue.state = IssueState::Closed;
        for (_, sub) in self.sub_issues.lock().unwrap().iter_mut() {
            if sub.number == number {
                sub.state = IssueState::Closed;
            }
        }
    }

    /// A bug of `PARENT` already raised on GitHub, in `state`.
    fn old_bug(&self, number: u64, state: IssueState) {
        self.sub_issues.lock().unwrap().push((
            PARENT,
            SubIssue {
                number,
                state,
                labels: vec!["bug".to_string()],
            },
        ));
    }
}

impl GithubPort for FakeGithub {
    fn list_open(&self, _repo: &str, _limit: u32) -> anyhow::Result<Vec<GithubIssue>> {
        Ok(self
            .issues
            .lock()
            .unwrap()
            .iter()
            .filter(|i| i.state == IssueState::Open)
            .cloned()
            .collect())
    }

    fn view(&self, _repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>> {
        Ok(self
            .issues
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.number == number)
            .cloned())
    }

    fn blocked_by(&self, _repo: &str, _number: u64) -> anyhow::Result<Vec<IssueDependency>> {
        Ok(Vec::new())
    }

    fn edit_labels(
        &self,
        _repo: &str,
        _number: u64,
        _add: &[String],
        _remove: &[String],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn comment(&self, _repo: &str, number: u64, body: &str) -> anyhow::Result<Option<String>> {
        self.comments
            .lock()
            .unwrap()
            .push((number, body.to_string()));
        Ok(None)
    }

    fn create_issue(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> anyhow::Result<CreatedIssue> {
        let mut issues = self.issues.lock().unwrap();
        let number = 300 + issues.len() as u64;
        issues.push(GithubIssue {
            number,
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            state: IssueState::Open,
            url: format!("https://github.com/{repo}/issues/{number}"),
            ..GithubIssue::default()
        });
        self.created
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string(), labels.to_vec()));
        Ok(CreatedIssue {
            number,
            id: 9000 + number,
            url: format!("https://github.com/{repo}/issues/{number}"),
        })
    }

    fn add_sub_issue(&self, _repo: &str, parent: u64, sub_issue_id: u64) -> anyhow::Result<()> {
        if *self.fail_link.lock().unwrap() {
            anyhow::bail!("sub-issues are not enabled");
        }
        let number = sub_issue_id - 9000;
        let labels = self
            .issues
            .lock()
            .unwrap()
            .iter()
            .find(|i| i.number == number)
            .map(|i| i.labels.clone())
            .unwrap_or_default();
        self.sub_issues.lock().unwrap().push((
            parent,
            SubIssue {
                number,
                state: IssueState::Open,
                labels,
            },
        ));
        Ok(())
    }

    fn sub_issues(&self, _repo: &str, parent: u64) -> anyhow::Result<Vec<SubIssue>> {
        Ok(self
            .sub_issues
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| *p == parent)
            .map(|(_, sub)| sub.clone())
            .collect())
    }

    fn close_issue(&self, _repo: &str, number: u64) -> anyhow::Result<()> {
        self.close(number);
        Ok(())
    }
}

fn issue(number: u64, title: &str, body: &str, labels: &[&str]) -> GithubIssue {
    GithubIssue {
        number,
        title: title.to_string(),
        body: body.to_string(),
        labels: labels.iter().map(|l| l.to_string()).collect(),
        state: IssueState::Open,
        url: format!("https://github.com/{REPO}/issues/{number}"),
        ..GithubIssue::default()
    }
}

/// A vault whose mirror `GH-272` is an art ticket already synced from `fake`.
fn vault_with_parent(name: &str, fake: &FakeGithub) -> TempWorkspace {
    let workspace = TempWorkspace::new(name);
    let todo = workspace.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(todo.join("config.yaml"), CONFIG).unwrap();
    std::fs::write(todo.join("routing.yaml"), ROUTING).unwrap();
    fake.issues.lock().unwrap().insert(
        0,
        issue(
            PARENT,
            "Sprite du héros",
            "CA-1 le sprite existe",
            &["pixelart", "aidp:to-test"],
        ),
    );
    sync_now(&workspace, fake);
    workspace
}

fn sync_now(workspace: &TempWorkspace, fake: &FakeGithub) {
    let store = TicketStore::open(workspace.path()).unwrap();
    let report = sync(&store, fake, &SyncOptions::default()).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

fn mirror(workspace: &TempWorkspace, key: &str) -> Ticket {
    let store = TicketStore::open(workspace.path()).unwrap();
    store.get(key).unwrap().0
}

fn blockers_of(ticket: &Ticket) -> Vec<(String, IssueState)> {
    ticket
        .github
        .as_ref()
        .unwrap()
        .blocked_by
        .iter()
        .map(|link| (link.key.clone(), link.state))
        .collect()
}

fn raise(workspace: &TempWorkspace, fake: &FakeGithub) -> BugOutcome {
    let store = TicketStore::open(workspace.path()).unwrap();
    create(
        &store,
        fake,
        PARENT_KEY,
        PARENT_BRANCH,
        "le héros ne s'affiche pas",
        "",
        "## Test\n- CA-1 ko : sprite absent",
    )
    .unwrap()
}

#[test]
fn the_marker_reads_back_what_the_body_wrote() {
    let body = bug_body("GH-272", 272, "feat/GH-272", "rapport");
    assert_eq!(
        parse_marker(&body),
        BugMarker {
            parent: Some(272),
            base: Some("feat/GH-272".to_string()),
        }
    );
    assert_eq!(parse_marker("pas de marqueur"), BugMarker::default());
    assert_eq!(
        parse_marker("<!-- aidp:bug parent=x base= -->"),
        BugMarker::default()
    );
}

#[test]
fn the_title_is_a_conventional_fix_on_one_line() {
    assert_eq!(
        bug_title("GH-272", "  le héros\nne s'affiche   pas "),
        "fix(GH-272): le héros ne s'affiche pas"
    );
    assert_eq!(bug_title("GH-272", " "), "fix(GH-272): test en échec");
    let long = bug_title("GH-272", &"x".repeat(500));
    assert_eq!(long.chars().count(), "fix(GH-272): ".len() + 100);
}

#[test]
fn a_failed_test_raises_a_labelled_bug_linked_under_its_parent() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-create", &fake);

    let outcome = raise(&workspace, &fake);

    assert!(outcome.changed && outcome.linked && !outcome.capped);
    assert_eq!(outcome.bug, "GH-301");
    assert_eq!(outcome.number, 301);
    assert_eq!(outcome.count, 1);
    assert_eq!(outcome.base, PARENT_BRANCH);
    let created = fake.created.lock().unwrap();
    assert_eq!(created.len(), 1);
    let (title, body, labels) = &created[0];
    assert_eq!(title, "fix(GH-272): le héros ne s'affiche pas");
    // The bug takes the route of its parent (art), and is ready for the dispatcher.
    assert_eq!(labels, &["bug", "bug:art", "aidp:ready"]);
    assert!(body.contains("Parente : #272"), "{body}");
    assert!(body.contains("- CA-1 ko : sprite absent"), "{body}");
    assert!(has_acceptance_criteria(body));
    assert_eq!(
        parse_marker(body),
        BugMarker {
            parent: Some(PARENT),
            base: Some(PARENT_BRANCH.to_string()),
        }
    );
    // Attached as a sub-issue of the parent, which is told.
    let subs = fake.sub_issues.lock().unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!((subs[0].0, subs[0].1.number), (PARENT, 301));
    assert!(fake.comments.lock().unwrap()[0].1.contains("#301"));
    // The parent mirror waits for the bug right away, before any sync.
    assert_eq!(
        blockers_of(&mirror(&workspace, PARENT_KEY)),
        vec![("GH-301".to_string(), IssueState::Open)]
    );
}

#[test]
fn a_link_that_fails_still_leaves_the_bug_and_its_blocker() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-link-fails", &fake);
    *fake.fail_link.lock().unwrap() = true;

    let outcome = raise(&workspace, &fake);

    assert!(outcome.changed && !outcome.linked);
    assert_eq!(outcome.bug, "GH-301");
    assert_eq!(blockers_of(&mirror(&workspace, PARENT_KEY)).len(), 1);
}

#[test]
fn an_open_bug_is_reused_instead_of_raising_a_second_one() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-reuse", &fake);
    let first = raise(&workspace, &fake);

    let again = raise(&workspace, &fake);

    assert!(!again.changed && !again.capped);
    assert_eq!(again.bug, first.bug);
    assert_eq!(fake.created.lock().unwrap().len(), 1);
}

#[test]
fn the_fourth_bug_is_refused_once_three_were_raised() {
    let fake = FakeGithub::default();
    for number in [281, 282, 283] {
        fake.old_bug(number, IssueState::Closed);
    }
    let workspace = vault_with_parent("bug-cap", &fake);

    let outcome = raise(&workspace, &fake);

    assert!(outcome.capped && !outcome.changed);
    assert_eq!(outcome.count, 3);
    assert!(outcome.message.contains("cap 3"));
    assert!(fake.created.lock().unwrap().is_empty());
    assert!(fake.comments.lock().unwrap().is_empty());
    assert!(blockers_of(&mirror(&workspace, PARENT_KEY)).is_empty());
}

#[test]
fn two_closed_bugs_still_allow_a_third() {
    let fake = FakeGithub::default();
    for number in [281, 282] {
        fake.old_bug(number, IssueState::Closed);
    }
    let workspace = vault_with_parent("bug-two-closed", &fake);

    let outcome = raise(&workspace, &fake);

    assert!(outcome.changed);
    assert_eq!(outcome.count, 3);
}

#[test]
fn the_sync_gives_the_bug_its_base_and_parent_and_makes_the_parent_wait() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-sync", &fake);
    raise(&workspace, &fake);

    sync_now(&workspace, &fake);

    let bug = mirror(&workspace, "GH-301");
    let github = bug.github.as_ref().unwrap();
    assert_eq!(github.base_branch.as_deref(), Some(PARENT_BRANCH));
    assert_eq!(github.parent, Some(PARENT));
    assert_eq!(bug.r#type, crate::model::IssueType::Bug);
    // Routed on its own flow, not on the parent's, and ready for the dispatcher.
    assert_eq!(bug.flow.as_deref(), Some("game-bug-art"));
    assert_eq!(bug.route.as_deref(), Some("art"));
    assert_eq!(bug.service, SERVICE);
    assert_eq!(bug.status, Status::Todo);
    // The parent keeps its open blocker, derived from the marker alone.
    let parent = mirror(&workspace, PARENT_KEY);
    assert_eq!(
        blockers_of(&parent),
        vec![("GH-301".to_string(), IssueState::Open)]
    );
    assert_eq!(parent.github.as_ref().unwrap().base_branch, None);
}

#[test]
fn closing_the_bug_frees_the_parent() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-close", &fake);
    raise(&workspace, &fake);
    sync_now(&workspace, &fake);

    let store = TicketStore::open(workspace.path()).unwrap();
    let closed = close(&store, &fake, "GH-301").unwrap();

    assert!(closed.changed);
    // Right away, in the parent mirror...
    assert_eq!(
        blockers_of(&mirror(&workspace, PARENT_KEY)),
        vec![("GH-301".to_string(), IssueState::Closed)]
    );
    // ... and after the next sync, where the closed bug is no longer a blocker at all.
    sync_now(&workspace, &fake);
    assert!(blockers_of(&mirror(&workspace, PARENT_KEY)).is_empty());
    // Closing twice changes nothing.
    assert!(!close(&store, &fake, "GH-301").unwrap().changed);
}

fn raise_allowing(workspace: &TempWorkspace, fake: &FakeGithub, extra: usize) -> BugOutcome {
    let store = TicketStore::open(workspace.path()).unwrap();
    create_allowing(
        &store,
        fake,
        PARENT_KEY,
        PARENT_BRANCH,
        "le spawner n'instancie pas le héros",
        "",
        "## Test\n- CA-2 ko",
        extra,
    )
    .unwrap()
}

fn rearm_at(workspace: &TempWorkspace, fake: &FakeGithub, at: &str) -> RearmOutcome {
    let store = TicketStore::open(workspace.path()).unwrap();
    rearm(
        &store,
        fake,
        PARENT_KEY,
        DateTime::parse_from_rfc3339(at)
            .unwrap()
            .with_timezone(&Utc),
    )
    .unwrap()
}

const MORNING: &str = "2026-11-10T08:00:00Z";
const SAME_DAY: &str = "2026-11-10T20:00:00Z";
const NEXT_DAY: &str = "2026-11-11T09:00:00Z";

fn capped_parent(name: &str) -> (FakeGithub, TempWorkspace) {
    let fake = FakeGithub::default();
    for number in [281, 282, 283] {
        fake.old_bug(number, IssueState::Closed);
    }
    let workspace = vault_with_parent(name, &fake);
    (fake, workspace)
}

#[test]
fn one_more_bug_is_allowed_past_the_cap_and_only_one() {
    let (fake, workspace) = capped_parent("bug-extra");

    let extra = raise_allowing(&workspace, &fake, EXTRA_BUGS_AFTER_CAP);
    assert!(extra.changed && !extra.capped, "{extra:?}");
    assert_eq!(extra.count, 4);

    fake.close(extra.number);
    let again = raise_allowing(&workspace, &fake, EXTRA_BUGS_AFTER_CAP);
    assert!(again.capped && !again.changed, "{again:?}");
    assert_eq!(fake.created.lock().unwrap().len(), 1);
}

#[test]
fn without_the_allowance_the_cap_holds() {
    let (fake, workspace) = capped_parent("bug-no-extra");
    assert!(raise_allowing(&workspace, &fake, 0).capped);
}

#[test]
fn re_arming_forgives_the_closed_bugs_so_the_parent_may_raise_three_more() {
    let (fake, workspace) = capped_parent("bug-rearm");
    assert!(raise(&workspace, &fake).capped);

    let outcome = rearm_at(&workspace, &fake, MORNING);
    assert!(outcome.rearmed, "{outcome:?}");
    assert_eq!(outcome.forgiven, 3);

    let first = raise(&workspace, &fake);
    assert!(first.changed && !first.capped, "{first:?}");
    assert_eq!(first.count, 1);
    fake.close(first.number);
    for _ in 0..2 {
        let next = raise(&workspace, &fake);
        assert!(next.changed, "{next:?}");
        fake.close(next.number);
    }
    assert!(raise(&workspace, &fake).capped);
}

#[test]
fn the_cap_is_re_armed_once_over_24_hours() {
    let (fake, workspace) = capped_parent("bug-rearm-once");
    assert!(rearm_at(&workspace, &fake, MORNING).rearmed);

    let second = rearm_at(&workspace, &fake, SAME_DAY);
    assert!(!second.rearmed, "{second:?}");
    assert!(second.message.contains("once per 24 h"));

    let first = raise(&workspace, &fake);
    fake.close(first.number);
    let third = rearm_at(&workspace, &fake, NEXT_DAY);
    assert!(third.rearmed, "{third:?}");
    assert_eq!(third.forgiven, 1);
}

#[test]
fn a_bug_still_open_when_the_cap_is_re_armed_keeps_counting_and_is_reused() {
    let fake = FakeGithub::default();
    fake.old_bug(281, IssueState::Closed);
    fake.old_bug(282, IssueState::Open);
    let workspace = vault_with_parent("bug-rearm-open", &fake);

    let outcome = rearm_at(&workspace, &fake, MORNING);
    assert_eq!(outcome.forgiven, 1);
    let reused = raise(&workspace, &fake);
    assert_eq!(reused.number, 282);
    assert!(!reused.changed);
}

const WRITTEN: &str = "## Issue de base\n\nObjectif.\n\n## Description du bug\n\nCA-1 ko.\n\n## Plan à adopter\n\n1. Corriger.\n\n## Critères d'acceptation\n\n- CA-1 : le test passe.";

#[test]
fn a_written_body_keeps_the_marker_first_and_folds_the_raw_report_last() {
    let body = compose_body(
        "GH-272",
        272,
        "feat/GH-272",
        WRITTEN,
        "## Test\nsprite absent",
    );
    assert!(
        body.starts_with("<!-- aidp:bug parent=272 base=feat/GH-272 -->\n## Issue de base"),
        "{body}"
    );
    let position = |needle: &str| {
        body.find(needle)
            .unwrap_or_else(|| panic!("{needle}: {body}"))
    };
    assert!(position("## Issue de base") < position("## Description du bug"));
    assert!(position("## Description du bug") < position("## Plan à adopter"));
    assert!(position("## Plan à adopter") < position("## Critères d'acceptation"));
    assert!(position("## Critères d'acceptation") < position("<details>"));
    assert!(position("<details>") < position("sprite absent"));
    assert!(body.trim_end().ends_with("</details>"));
    assert_eq!(body.matches("aidp:bug").count(), 1);
    assert_eq!(
        parse_marker(&body),
        BugMarker {
            parent: Some(272),
            base: Some("feat/GH-272".to_string()),
        }
    );
}

#[test]
fn a_marker_inside_the_written_body_is_replaced_by_the_daemon_one() {
    let written = format!("<!-- aidp:bug parent=1 base=autre -->\n{WRITTEN}");
    let body = compose_body("GH-272", 272, "feat/GH-272", &written, "rapport");
    assert_eq!(body.matches("aidp:bug").count(), 1, "{body}");
    assert_eq!(parse_marker(&body).parent, Some(272));
}

#[test]
fn a_closing_tag_in_the_report_cannot_end_the_folded_block_early() {
    let body = compose_body("GH-272", 272, "feat/GH-272", WRITTEN, "a </details> b");
    assert_eq!(body.matches("</details>").count(), 1, "{body}");
}

#[test]
fn an_empty_report_is_said_inside_the_folded_block() {
    let body = compose_body("GH-272", 272, "feat/GH-272", WRITTEN, "  ");
    assert!(body.contains("Aucun rapport fourni par le test."), "{body}");
}

#[test]
fn without_a_written_body_the_body_is_the_one_from_before() {
    for written in ["", "  \n ", "<!-- aidp:bug parent=1 base=x -->"] {
        assert_eq!(
            compose_body("GH-272", 272, "feat/GH-272", written, "rapport"),
            bug_body("GH-272", 272, "feat/GH-272", "rapport"),
            "{written:?}"
        );
    }
}

#[test]
fn raising_a_bug_with_a_written_body_creates_the_issue_with_it() {
    let fake = FakeGithub::default();
    let workspace = vault_with_parent("bug-written-body", &fake);
    let store = TicketStore::open(workspace.path()).unwrap();
    create(
        &store,
        &fake,
        PARENT_KEY,
        PARENT_BRANCH,
        "le héros ne s'affiche pas",
        WRITTEN,
        "## Test\n- CA-1 ko",
    )
    .unwrap();
    let created = fake.created.lock().unwrap();
    let (_, body, _) = &created[0];
    assert!(
        body.contains("## Plan à adopter") && body.contains("<details>"),
        "{body}"
    );
}
