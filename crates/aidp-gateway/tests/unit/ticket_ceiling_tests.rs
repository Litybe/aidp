use super::*;
use aidp_testkit::TempWorkspace;

const KEY: &str = "GH-12";

fn session(id: &str, ticket: &str, tokens: u64) -> SessionRecord {
    serde_json::from_value(json!({
        "id": id,
        "projectId": "p",
        "projectName": "P",
        "cwd": "/tmp",
        "engine": "v7",
        "templateId": "dev-unity",
        "prompt": "x",
        "permissionMode": "open",
        "status": "done",
        "startedAt": "2026-11-06T10:00:00Z",
        "origin": { "kind": "flow", "ref": ticket, "label": "implement" },
        "usage": { "input": tokens, "output": 0, "cacheRead": 0, "cacheCreate": 0 },
    }))
    .unwrap()
}

fn vault_with(config: &str) -> TempWorkspace {
    let vault = TempWorkspace::new("p5");
    let todo = vault.path().join("TODO");
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(todo.join("config.yaml"), config).unwrap();
    vault
}

const SMALL_CODE_CEILING: &str = "budget:\n  per_ticket:\n    code: 1000\n";

#[test]
fn a_ticket_within_its_ceiling_is_not_held() {
    let vault = vault_with(SMALL_CODE_CEILING);
    let records = vec![session("s1", KEY, 400)];

    assert_eq!(exceeded(vault.path(), KEY, &records), None);
}

#[test]
fn a_ticket_past_its_token_ceiling_is_held_with_the_question() {
    let vault = vault_with(SMALL_CODE_CEILING);
    let records = vec![session("s1", KEY, 600), session("s2", KEY, 600)];

    let hit = exceeded(vault.path(), KEY, &records).expect("a hit");

    assert_eq!((hit.used, hit.limit), (1200, 1000));
    assert!(hit.prompt.contains("Continuer ?"));
    assert!(hit.prompt.contains(KEY));
}

#[test]
fn the_sessions_of_other_tickets_do_not_count() {
    let vault = vault_with(SMALL_CODE_CEILING);
    let records = vec![session("s1", KEY, 400), session("s2", "GH-13", 5000)];

    assert_eq!(exceeded(vault.path(), KEY, &records), None);
}

#[test]
fn approving_raises_the_ceiling_of_that_ticket_only() {
    let vault = vault_with(SMALL_CODE_CEILING);
    let records = vec![session("s1", KEY, 1200), session("s2", "GH-13", 1200)];

    raise(vault.path(), KEY, &records);

    assert_eq!(exceeded(vault.path(), KEY, &records), None);
    assert!(exceeded(vault.path(), "GH-13", &records).is_some());
}

#[test]
fn a_raised_ceiling_holds_again_after_one_more_allowance() {
    let vault = vault_with(SMALL_CODE_CEILING);
    raise(vault.path(), KEY, &[session("s1", KEY, 1200)]);

    let records = vec![session("s1", KEY, 1200), session("s2", KEY, 1100)];

    let hit = exceeded(vault.path(), KEY, &records).expect("a hit");
    assert_eq!(hit.limit, 2200);
}

#[test]
fn a_zero_ceiling_never_holds() {
    let vault = vault_with("budget:\n  per_ticket:\n    code: 0\n");

    assert_eq!(
        exceeded(vault.path(), KEY, &[session("s1", KEY, 9_000_000)]),
        None
    );
}

#[test]
fn the_generations_of_a_ticket_are_capped() {
    let vault = vault_with("budget:\n  pixellab:\n    per_ticket: 3\n");
    let stage = vault
        .path()
        .join("Pipelines/repo")
        .join(KEY)
        .join("run-1/generate");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(
        stage.join("pixellab-usage.json"),
        json!({ "generations": 4, "jobs": [], "recordedAt": "2026-11-06T10:00:00Z" }).to_string(),
    )
    .unwrap();

    let hit = exceeded(vault.path(), KEY, &[]).expect("a hit");

    assert_eq!((hit.used, hit.limit), (4, 3));
}

#[test]
fn the_report_shows_the_limit_the_use_and_the_raise() {
    let vault = vault_with(SMALL_CODE_CEILING);
    let records = vec![session("s1", KEY, 1200)];
    raise(vault.path(), KEY, &records);

    let (budget, pixellab) = report(vault.path(), KEY, &records);

    assert_eq!(budget["usedTokens"], 1200);
    assert_eq!(budget["limitTokens"], 2200);
    assert_eq!(budget["raisedTo"], 2200);
    assert_eq!(budget["exceeded"], false);
    assert_eq!(pixellab["generations"], 0);
}
