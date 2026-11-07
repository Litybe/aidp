use super::*;
use crate::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_testkit::TempWorkspace;
use std::fs;

const VIEWER_SERVICE: &str = "aidp-viewer";
const VIEWER_TEMPLATE: &str = "dev-aidp-viewer";
const FIRST_KEY: &str = "LOC-1";

fn viewer_vault(workspace: &TempWorkspace) -> &Path {
    let vault = workspace.path();
    fs::create_dir_all(vault.join("Pipelines").join(VIEWER_SERVICE)).unwrap();
    vault
}

fn new_params(service: &str, summary: &str, allow_unknown_service: bool) -> NewTicket {
    NewTicket {
        service: service.to_string(),
        summary: summary.to_string(),
        issue_type: None,
        priority: None,
        body: BodySource::Template,
        labels: None,
        template: None,
        track: None,
        epic: None,
        allow_unknown_service,
    }
}

fn create(vault: &Path, params: NewTicket) -> anyhow::Result<serde_json::Value> {
    new_ticket(&TicketStore::open(vault)?, params)
}

#[test]
fn body_source_follows_the_documented_precedence() {
    let path = PathBuf::from("body.md");
    assert_eq!(
        BodySource::from_flags(Some(path.clone()), Some("x".into()), true),
        BodySource::File(path)
    );
    assert_eq!(
        BodySource::from_flags(None, Some("x".into()), true),
        BodySource::Inline("x".into())
    );
    assert_eq!(BodySource::from_flags(None, None, true), BodySource::Stdin);
    assert_eq!(
        BodySource::from_flags(None, None, false),
        BodySource::Template
    );
}

#[test]
fn end_to_end_new_list_show_edit_status() {
    let workspace = TempWorkspace::new("e2e");
    let vault = viewer_vault(&workspace);

    let created = create(vault, new_params(VIEWER_SERVICE, "Page Tickets", false)).unwrap();
    assert_eq!(created["key"], FIRST_KEY);

    let store = TicketStore::open(vault).unwrap();
    let listed = store.list(&ListFilter::default());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, FIRST_KEY);
    assert_eq!(listed[0].summary, "Page Tickets");

    let edited = edit(
        &store,
        FIRST_KEY,
        TicketEdit {
            priority: Some(Priority::High),
            ..TicketEdit::default()
        },
    )
    .unwrap();
    assert_eq!(edited["key"], FIRST_KEY);

    let (ticket, body, _) = store.get(FIRST_KEY).unwrap();
    assert_eq!(ticket.priority, Priority::High);
    assert!(
        body.contains("Page Tickets"),
        "edit must leave the body untouched: {body:?}"
    );

    let moved = status(
        &store,
        FIRST_KEY,
        Status::Blocked,
        Some("attente specs".to_string()),
        false,
    )
    .unwrap();
    assert_eq!(moved["to"], "blocked");

    let (ticket, _, _) = store.get(FIRST_KEY).unwrap();
    assert_eq!(ticket.status, Status::Blocked);

    let events = ledger::read(vault, Some(FIRST_KEY), 10);
    assert!(events
        .iter()
        .any(|e| e.to == Status::Blocked && e.reason.as_deref() == Some("attente specs")));
}

#[test]
fn list_text_is_empty_without_tickets_and_one_line_per_ticket_otherwise() {
    let workspace = TempWorkspace::new("list-text");
    let vault = viewer_vault(&workspace);
    let store = TicketStore::open(vault).unwrap();
    assert_eq!(list(&store, ListQuery::default(), Fmt::Text).unwrap(), "");

    create(vault, new_params(VIEWER_SERVICE, "One", false)).unwrap();
    create(vault, new_params(VIEWER_SERVICE, "Two", false)).unwrap();

    let text = list(&store, ListQuery::default(), Fmt::Text).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains("One"));
}

#[test]
fn render_returns_the_stored_body_untouched() {
    let workspace = TempWorkspace::new("render-body");
    let vault = viewer_vault(&workspace);
    let mut params = new_params(VIEWER_SERVICE, "Body", false);
    params.body = BodySource::Inline("line one\nline two\n".to_string());
    create(vault, params).unwrap();

    let store = TicketStore::open(vault).unwrap();

    assert_eq!(render(&store, FIRST_KEY).unwrap(), "line one\nline two\n");
}

/// Drives `reindex` over one fresh and one old `done` ticket.
/// The archived ticket must still appear in `_TODO.md`, proving one refresh saw it.
#[test]
fn reindex_archives_old_done_tickets_in_a_single_pass() {
    let workspace = TempWorkspace::new("reindex-single-pass");
    let vault = viewer_vault(&workspace);
    fs::create_dir_all(vault.join("TODO")).unwrap();
    fs::write(
        vault.join("TODO").join("config.yaml"),
        "local:\n  archive_after_days: 1\n",
    )
    .unwrap();

    create(vault, new_params(VIEWER_SERVICE, "Fresh ticket", false)).unwrap();

    let mut old_done = Ticket {
        key: "LOC-2".to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "Old done ticket".to_string(),
        status: Status::Done,
        priority: Priority::Medium,
        priority_rank: 0,
        service: VIEWER_SERVICE.to_string(),
        template: None,
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: Utc::now() - chrono::Duration::days(2),
        updated: Utc::now() - chrono::Duration::days(2),
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    };
    old_done.recompute_derived();
    let service_dir = vault.join("TODO").join(VIEWER_SERVICE);
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("LOC-2.md"),
        note::render(&old_done, "# body\n", &serde_yaml::Mapping::new()),
    )
    .unwrap();

    let store = TicketStore::open(vault).unwrap();
    let outcome = reindex(&store).unwrap();

    assert_eq!(
        outcome["archived"], 1,
        "exactly the old done ticket must be archived"
    );
    assert!(service_dir.join("archive").join("LOC-2.md").is_file());
    assert!(!service_dir.join("LOC-2.md").exists());

    let board = fs::read_to_string(vault.join("TODO").join("_TODO.md")).unwrap();
    assert!(
        board.contains("LOC-2"),
        "the generated _TODO.md must still mention the archived key:\n{board}"
    );
}

#[test]
fn illegal_transition_is_refused() {
    let workspace = TempWorkspace::new("illegal");
    let vault = viewer_vault(&workspace);
    create(vault, new_params(VIEWER_SERVICE, "x", false)).unwrap();

    let store = TicketStore::open(vault).unwrap();
    let before = store.get(FIRST_KEY).unwrap();

    let err = status(&store, FIRST_KEY, Status::Done, None, false).unwrap_err();
    assert!(
        err.to_string().contains("not a legal transition"),
        "unexpected error: {err}"
    );

    let after = store.get(FIRST_KEY).unwrap();
    assert_eq!(
        before.0, after.0,
        "a refused transition changes nothing on disk"
    );
    assert_eq!(before.1, after.1);
}

#[test]
fn new_without_a_known_service_is_refused_unless_allowed() {
    let workspace = TempWorkspace::new("unknown-service");
    let vault = viewer_vault(&workspace);

    let err = create(vault, new_params("totally-unknown", "x", false)).unwrap_err();
    assert!(
        err.to_string().contains("unknown service"),
        "unexpected error: {err}"
    );

    create(vault, new_params("totally-unknown", "x", true)).unwrap();
    let store = TicketStore::open(vault).unwrap();
    assert_eq!(store.list(&ListFilter::default()).len(), 1);
}

#[test]
fn plan_without_a_skill_root_degrades_to_an_empty_catalog_with_a_warning() {
    let workspace = TempWorkspace::new("port-plan-none");
    let vault = viewer_vault(&workspace);
    create(vault, new_params(VIEWER_SERVICE, "Port ticket", false)).unwrap();
    let store = TicketStore::open(vault).unwrap();

    let output = plan(&store, None, Fmt::JsonCompact).unwrap();

    assert_eq!(output.warnings.len(), 1);
    let parsed: serde_json::Value = serde_json::from_str(&output.text).unwrap();
    assert!(parsed["launches"].as_array().unwrap().is_empty());
}

#[test]
fn it_plan_with_a_skill_root_reads_its_template_catalog() {
    let workspace = TempWorkspace::new("port-plan-root");
    let vault = viewer_vault(&workspace);
    let skill_root = workspace.path().join("skill-root");
    fs::create_dir_all(skill_root.join("templates")).unwrap();
    fs::write(
        skill_root
            .join("templates")
            .join(format!("{VIEWER_TEMPLATE}.yaml")),
        format!("id: {VIEWER_TEMPLATE}\ninputs:\n  prompt: {{ required: true, kind: string }}\n"),
    )
    .unwrap();
    let mut params = new_params(VIEWER_SERVICE, "Port ticket", false);
    params.template = Some(VIEWER_TEMPLATE.to_string());
    create(vault, params).unwrap();
    let store = TicketStore::open(vault).unwrap();

    let output = plan(&store, Some(&skill_root), Fmt::JsonCompact).unwrap();

    assert!(output.warnings.is_empty());
    let parsed: serde_json::Value = serde_json::from_str(&output.text).unwrap();
    let launches = parsed["launches"].as_array().unwrap();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0]["template"], VIEWER_TEMPLATE);
}

#[test]
fn plan_with_an_unreadable_skill_root_warns() {
    let workspace = TempWorkspace::new("port-plan-missing-root");
    let vault = viewer_vault(&workspace);
    let store = TicketStore::open(vault).unwrap();
    let missing = workspace.path().join("absent");

    let output = plan(&store, Some(&missing), Fmt::Text).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert!(output.text.is_empty());
}

#[test]
fn route_mode_from_unrouted_alone_is_the_batch_shape() {
    let mode = route_mode(None, None, true).unwrap();
    assert!(matches!(mode, RouteMode::FromUnrouted));
}

#[test]
fn route_mode_key_and_service_alone_is_the_one_shape() {
    let mode = route_mode(Some(FIRST_KEY.to_string()), Some("svc".to_string()), false).unwrap();
    match mode {
        RouteMode::One { key, service } => {
            assert_eq!(key, FIRST_KEY);
            assert_eq!(service, "svc");
        }
        RouteMode::FromUnrouted => panic!("expected RouteMode::One"),
    }
}

#[test]
fn route_mode_from_unrouted_with_a_key_is_an_explicit_conflict() {
    let err = route_mode(Some(FIRST_KEY.to_string()), None, true).unwrap_err();
    assert!(err.to_string().contains("--from-unrouted"), "{err}");
    assert!(err.to_string().contains("KEY"), "{err}");
}

#[test]
fn route_mode_from_unrouted_with_service_is_an_explicit_conflict() {
    let err = route_mode(None, Some("svc".to_string()), true).unwrap_err();
    assert!(err.to_string().contains("--from-unrouted"), "{err}");
    assert!(err.to_string().contains("--service"), "{err}");
}

#[test]
fn route_mode_missing_key_or_service_without_from_unrouted_is_refused() {
    let err = route_mode(None, None, false).unwrap_err();
    assert!(err.to_string().contains("--from-unrouted"), "{err}");
    assert!(
        err.to_string().contains("KEY") && err.to_string().contains("--service"),
        "{err}"
    );

    let err = route_mode(Some(FIRST_KEY.to_string()), None, false).unwrap_err();
    assert!(err.to_string().contains("--service"), "{err}");

    let err = route_mode(None, Some("svc".to_string()), false).unwrap_err();
    assert!(err.to_string().contains("KEY"), "{err}");
}
