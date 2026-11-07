//! Tests for the GitHub sync: creations, updates, closures, blockers and label write-back,
//! against an in-memory fake of the GitHub port.

use super::*;
use crate::models::milestone::Milestone;
use aidp_testkit::TempWorkspace;
use std::sync::Mutex;

const REPO: &str = "Litybe/PixelHood";
const SERVICE: &str = "PixelHood";

const CONFIG: &str = "github:\n  repos: [Litybe/PixelHood]\n";

const ROUTING: &str = "\
defaults: { track: standard }
rules:
  - { match: { labels: [pixelart, \"pixel:*\"] }, service: pixelhood-art }
  - { match: { label: bug }, service: pixelhood-bug }
  - { match: { labels: [dev, enhancement] }, service: pixelhood-code }
  - { match: { keywords: [map, salle] }, route: pixelhood-map }
repos:
  pixelhood-code: { route: code, flow: game-code, template: dev-unity }
  pixelhood-bug: { route: code, flow: game-code, track: bug }
  pixelhood-art: { route: art, flow: game-art }
  pixelhood-map: { flow: game-map }
";

/// One recorded label edit: issue number, labels added, labels removed.
type LabelEdit = (u64, Vec<String>, Vec<String>);

/// An in-memory GitHub: open issues, closed ones, dependencies, and the writes it received.
#[derive(Default)]
struct FakeGithub {
    open: Mutex<Vec<GithubIssue>>,
    closed: Mutex<Vec<GithubIssue>>,
    dependencies: Mutex<HashMap<u64, Vec<IssueDependency>>>,
    edits: Mutex<Vec<LabelEdit>>,
    comments: Mutex<Vec<(u64, String)>>,
}

impl FakeGithub {
    fn with_open(issues: Vec<GithubIssue>) -> FakeGithub {
        let fake = FakeGithub::default();
        *fake.open.lock().unwrap() = issues;
        fake
    }

    fn set_labels(&self, number: u64, labels: &[&str]) {
        let mut open = self.open.lock().unwrap();
        let issue = open.iter_mut().find(|i| i.number == number).unwrap();
        issue.labels = labels.iter().map(|l| l.to_string()).collect();
    }

    fn close(&self, number: u64) {
        let mut open = self.open.lock().unwrap();
        let position = open.iter().position(|i| i.number == number).unwrap();
        let mut issue = open.remove(position);
        issue.state = IssueState::Closed;
        self.closed.lock().unwrap().push(issue);
    }

    fn edited(&self) -> Vec<(u64, Vec<String>, Vec<String>)> {
        self.edits.lock().unwrap().clone()
    }
}

impl GithubPort for FakeGithub {
    fn list_open(&self, _repo: &str, limit: u32) -> anyhow::Result<Vec<GithubIssue>> {
        Ok(self
            .open
            .lock()
            .unwrap()
            .iter()
            .take(limit as usize)
            .cloned()
            .collect())
    }

    fn view(&self, _repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>> {
        let open = self.open.lock().unwrap();
        let closed = self.closed.lock().unwrap();
        Ok(open
            .iter()
            .chain(closed.iter())
            .find(|i| i.number == number)
            .cloned())
    }

    fn blocked_by(&self, _repo: &str, number: u64) -> anyhow::Result<Vec<IssueDependency>> {
        Ok(self
            .dependencies
            .lock()
            .unwrap()
            .get(&number)
            .cloned()
            .unwrap_or_default())
    }

    fn edit_labels(
        &self,
        _repo: &str,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        self.edits
            .lock()
            .unwrap()
            .push((number, add.to_vec(), remove.to_vec()));
        let mut open = self.open.lock().unwrap();
        if let Some(issue) = open.iter_mut().find(|i| i.number == number) {
            issue.labels = state_labels::apply(
                &issue.labels,
                &state_labels::LabelEdit {
                    add: add.to_vec(),
                    remove: remove.to_vec(),
                },
            );
        }
        Ok(())
    }

    fn comment(&self, _repo: &str, number: u64, body: &str) -> anyhow::Result<Option<String>> {
        self.comments
            .lock()
            .unwrap()
            .push((number, body.to_string()));
        Ok(Some(format!(
            "https://github.com/{REPO}/issues/{number}#c1"
        )))
    }
}

fn issue(number: u64, title: &str, body: &str, labels: &[&str]) -> GithubIssue {
    GithubIssue {
        number,
        title: title.to_string(),
        body: body.to_string(),
        labels: labels.iter().map(|l| l.to_string()).collect(),
        milestone: None,
        state: IssueState::Open,
        url: format!("https://github.com/{REPO}/issues/{number}"),
        created_at: None,
        updated_at: None,
    }
}

fn vault(name: &str) -> TempWorkspace {
    let workspace = TempWorkspace::new(name);
    let todo = workspace.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(todo.join("config.yaml"), CONFIG).unwrap();
    std::fs::write(todo.join("routing.yaml"), ROUTING).unwrap();
    workspace
}

fn run(workspace: &TempWorkspace, port: &FakeGithub) -> Report {
    let store = TicketStore::open(workspace.path()).unwrap();
    sync(&store, port, &SyncOptions::default()).unwrap()
}

fn get(workspace: &TempWorkspace, key: &str) -> (Ticket, String) {
    let store = TicketStore::open(workspace.path()).unwrap();
    let (ticket, body, _) = store.get(key).unwrap();
    (ticket, body)
}

fn reason(ticket: &Ticket) -> Option<&str> {
    ticket.autopilot.blocked_reason.as_deref()
}

#[test]
fn first_sync_creates_mirrors_routed_triaged_or_held() {
    let workspace = vault("gh-create");
    let port = FakeGithub::with_open(vec![
        issue(1, "Dash", "CA-1 le dash marche", &["dev", "aidp:ready"]),
        issue(2, "Épingle", "Juste une idée", &["pixelart"]),
        issue(3, "Sans étiquette", "CA-1 quelque chose", &[]),
        issue(4, "Complète", "CA-1 ok", &["enhancement"]),
        issue(5, "Déjà en cours", "CA-1 ok", &["dev", "aidp:fixing"]),
        issue(6, "Jamais", "CA-1 ok", &["dev", "aidp:skip"]),
        issue(7, "Nouvelle salle du donjon", "CA-1 ok", &["aidp:ready"]),
        issue(8, "Crash au saut", "CA-1 ok", &["bug", "aidp:ready"]),
    ]);

    let report = run(&workspace, &port);

    assert_eq!(report.created, 8);
    assert_eq!(report.unrouted, 1);
    assert_eq!(report.triaged, 1);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(workspace
        .path()
        .join("TODO")
        .join(SERVICE)
        .join("GH-1.md")
        .is_file());

    let (dash, body) = get(&workspace, "GH-1");
    assert_eq!(dash.status, Status::Todo);
    assert_eq!(dash.source, Source::Github);
    assert_eq!(dash.service, SERVICE);
    assert_eq!(dash.route.as_deref(), Some("code"));
    assert_eq!(dash.flow.as_deref(), Some("game-code"));
    assert_eq!(dash.template.as_deref(), Some("dev-unity"));
    assert_eq!(dash.github.as_ref().unwrap().number, 1);
    assert!(body.starts_with("# GH-1 - Dash\n\nCA-1 le dash marche\n\n## Notes locales"));

    let art = get(&workspace, "GH-2").0;
    assert_eq!(
        (art.status, reason(&art)),
        (Status::Blocked, Some("needs-refine"))
    );
    let unrouted = get(&workspace, "GH-3").0;
    assert_eq!(reason(&unrouted), Some("needs-routing"));
    assert_eq!(unrouted.route, None);
    assert_eq!(reason(&get(&workspace, "GH-4").0), Some("not-ready"));
    assert_eq!(
        reason(&get(&workspace, "GH-5").0),
        Some("label:aidp:fixing")
    );
    assert_eq!(reason(&get(&workspace, "GH-6").0), Some("skip"));
    let map = get(&workspace, "GH-7").0;
    assert_eq!(map.route.as_deref(), Some("pixelhood-map"));
    assert_eq!(get(&workspace, "GH-8").0.route.as_deref(), Some("code"));
    assert_eq!(map.status, Status::Todo);
    let bug = get(&workspace, "GH-8").0;
    assert_eq!(bug.r#type, IssueType::Bug);
    assert_eq!(bug.track, Some(crate::models::track::Track::Bug));

    let edits = port.edited();
    assert_eq!(
        edits,
        vec![
            (2, vec!["aidp:triage".to_string()], Vec::new()),
            (3, vec!["aidp:triage".to_string()], Vec::new()),
        ]
    );
    assert_eq!(report.labels_pushed, 2);
}

#[test]
fn a_label_changed_on_github_is_an_intent_and_a_local_status_is_kept() {
    let workspace = vault("gh-intent");
    let port = FakeGithub::with_open(vec![
        issue(1, "Dash", "CA-1", &["dev", "aidp:ready"]),
        issue(4, "Complète", "CA-1", &["dev"]),
    ]);
    run(&workspace, &port);

    // The dispatcher queued GH-1 locally; GitHub still shows aidp:ready.
    let store = TicketStore::open(workspace.path()).unwrap();
    let (mut dash, body, extra) = store.get("GH-1").unwrap();
    dash.status = Status::Queued;
    store.save(&mut dash, &body, &extra).unwrap();
    // The human marks GH-4 ready.
    port.set_labels(4, &["dev", "aidp:ready"]);

    let report = run(&workspace, &port);

    assert_eq!(get(&workspace, "GH-1").0.status, Status::Queued);
    assert_eq!(get(&workspace, "GH-4").0.status, Status::Todo);
    assert!(port.edited().contains(&(
        1,
        vec!["aidp:queued".to_string()],
        vec!["aidp:ready".to_string()]
    )));
    assert_eq!((report.updated, report.unchanged), (1, 1));
}

#[test]
fn a_refined_issue_leaves_triage_for_not_ready_then_ready() {
    let workspace = vault("gh-refine");
    let port = FakeGithub::with_open(vec![issue(2, "Épingle", "idée", &["pixelart"])]);
    run(&workspace, &port);
    assert_eq!(reason(&get(&workspace, "GH-2").0), Some("needs-refine"));

    // issue-refine rewrites the body, then swaps aidp:triage for aidp:ready.
    {
        let mut open = port.open.lock().unwrap();
        open[0].body = "CA-1 l'épingle brille".to_string();
    }
    port.set_labels(2, &["pixelart", "aidp:ready"]);
    run(&workspace, &port);

    let refined = get(&workspace, "GH-2").0;
    assert_eq!(refined.status, Status::Todo);
    assert_eq!(reason(&refined), None);
}

#[test]
fn a_closed_issue_cancels_a_waiting_mirror_and_finishes_one_in_review() {
    let workspace = vault("gh-closed");
    let port = FakeGithub::with_open(vec![
        issue(1, "Dash", "CA-1", &["dev", "aidp:ready"]),
        issue(2, "PR prête", "CA-1", &["dev", "aidp:ready"]),
    ]);
    run(&workspace, &port);
    let store = TicketStore::open(workspace.path()).unwrap();
    let (mut waiting, body, extra) = store.get("GH-2").unwrap();
    waiting.status = Status::InReview;
    store.save(&mut waiting, &body, &extra).unwrap();

    port.close(1);
    port.close(2);
    let report = run(&workspace, &port);

    let cancelled = get(&workspace, "GH-1").0;
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.github.unwrap().state, IssueState::Closed);
    assert_eq!(get(&workspace, "GH-2").0.status, Status::Done);
    assert_eq!(report.cancelled, 1);

    // A settled closed mirror is not looked up again.
    let again = run(&workspace, &port);
    assert_eq!(again.cancelled + again.updated, 0);
}

#[test]
fn local_notes_survive_an_update() {
    let workspace = vault("gh-notes");
    let port = FakeGithub::with_open(vec![issue(1, "Dash", "CA-1 v1", &["dev", "aidp:ready"])]);
    run(&workspace, &port);
    let store = TicketStore::open(workspace.path()).unwrap();
    let (mut ticket, body, extra) = store.get("GH-1").unwrap();
    let noted = format!("{body}Penser au son du dash.\n");
    store.save(&mut ticket, &noted, &extra).unwrap();

    port.open.lock().unwrap()[0].body = "CA-1 v2\r\nCA-2 plus vite".to_string();
    run(&workspace, &port);

    let (_, body) = get(&workspace, "GH-1");
    assert_eq!(
        body,
        "# GH-1 - Dash\n\nCA-1 v2\nCA-2 plus vite\n\n## Notes locales\nPenser au son du dash.\n"
    );
}

#[test]
fn blockers_come_from_the_body_and_the_native_dependencies() {
    let workspace = vault("gh-blockers");
    let port = FakeGithub::with_open(vec![
        issue(1, "Socle", "CA-1", &["dev", "aidp:ready"]),
        issue(
            3,
            "Suite",
            "CA-1\nDépend de #1 et #9.\nBlocked by Other/repo#4",
            &["dev", "aidp:ready"],
        ),
    ]);
    port.dependencies.lock().unwrap().insert(
        3,
        vec![IssueDependency {
            repo: REPO.to_string(),
            number: 12,
            state: IssueState::Open,
        }],
    );

    run(&workspace, &port);

    let blocked = get(&workspace, "GH-3").0;
    let links = blocked.github.unwrap().blocked_by;
    let keys: Vec<(String, IssueState)> = links.into_iter().map(|l| (l.key, l.state)).collect();
    assert_eq!(
        keys,
        vec![
            ("GH-1".to_string(), IssueState::Open),
            ("GH-9".to_string(), IssueState::Closed),
            ("GH-12".to_string(), IssueState::Open),
        ]
    );
}

#[test]
fn milestone_and_priority_labels_are_mirrored() {
    let workspace = vault("gh-milestone");
    let mut with_milestone = issue(1, "Dash", "CA-1", &["dev", "prio:high", "aidp:ready"]);
    with_milestone.milestone = Some(Milestone {
        title: "RM-1".to_string(),
        number: Some(1),
        due_on: None,
    });
    let port = FakeGithub::with_open(vec![with_milestone]);
    run(&workspace, &port);
    let ticket = get(&workspace, "GH-1").0;
    assert_eq!(ticket.priority, Priority::High);
    assert_eq!(ticket.milestone.unwrap().title, "RM-1");
}

#[test]
fn a_dry_run_writes_nothing() {
    let workspace = vault("gh-dry");
    let port = FakeGithub::with_open(vec![issue(2, "Épingle", "idée", &["pixelart"])]);
    let store = TicketStore::open(workspace.path()).unwrap();
    let report = sync(
        &store,
        &port,
        &SyncOptions {
            dry_run: true,
            ..SyncOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.created, 1);
    assert!(store.path_of("GH-2").is_none());
    assert!(port.edited().is_empty());
}

#[test]
fn an_unchanged_issue_is_counted_unchanged() {
    let workspace = vault("gh-unchanged");
    let port = FakeGithub::with_open(vec![issue(1, "Dash", "CA-1", &["dev", "aidp:ready"])]);
    run(&workspace, &port);
    let report = run(&workspace, &port);
    assert_eq!((report.unchanged, report.updated), (1, 0));
}

#[test]
fn label_ticket_keeps_a_single_state_label() {
    let workspace = vault("gh-label");
    let port = FakeGithub::with_open(vec![issue(1, "Dash", "CA-1", &["dev", "aidp:in-progress"])]);
    run(&workspace, &port);
    let store = TicketStore::open(workspace.path()).unwrap();

    let outcome = label_ticket(&store, &port, "GH-1", "review").unwrap();

    assert!(outcome.changed);
    assert_eq!(outcome.label, "aidp:review");
    assert_eq!(outcome.removed, vec!["aidp:in-progress".to_string()]);
    assert_eq!(
        get(&workspace, "GH-1").0.labels,
        vec!["dev".to_string(), "aidp:review".to_string()]
    );
    let again = label_ticket(&store, &port, "GH-1", "aidp:review").unwrap();
    assert!(!again.changed);
    assert!(label_ticket(&store, &port, "GH-1", "nope").is_err());
}

#[test]
fn comment_ticket_reaches_the_issue_and_refuses_a_local_ticket() {
    let workspace = vault("gh-comment");
    let port = FakeGithub::with_open(vec![issue(1, "Dash", "CA-1", &["dev", "aidp:ready"])]);
    run(&workspace, &port);
    let store = TicketStore::open(workspace.path()).unwrap();
    let url = comment_ticket(&store, &port, "GH-1", "Revue : OK").unwrap();
    assert!(url.is_some());
    assert_eq!(
        port.comments.lock().unwrap().clone(),
        vec![(1, "Revue : OK".to_string())]
    );
    assert!(comment_ticket(&store, &port, "LOC-1", "x").is_err());
}

#[test]
fn closing_line_is_added_once() {
    assert_eq!(with_closing_line("Résumé", 12), "Résumé\n\nCloses #12\n");
    assert_eq!(with_closing_line("Fixes #12 enfin", 12), "Fixes #12 enfin");
    assert_eq!(with_closing_line("", 3), "Closes #3\n");
    assert_eq!(
        with_closing_line("closes #120", 12),
        "closes #120\n\nCloses #12\n"
    );
}

#[test]
fn acceptance_criteria_and_priority_helpers() {
    assert!(has_acceptance_criteria("- CA-1 : le joueur saute"));
    assert!(!has_acceptance_criteria("PIXEL-CA-x"));
    let labels = vec!["prio:low".to_string()];
    assert_eq!(priority_of(&labels), Priority::Low);
    assert_eq!(priority_of(&[]), Priority::Medium);
}
