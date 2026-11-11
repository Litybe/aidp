//! `aidp memory ingest-knowledge` and `aidp memory inbox` on a temporary vault and gateway directory.

use crate::common::*;
use serde_json::{json, Value};
use std::path::Path;

const PROJECT_DIR: &str = "game";
const EXISTING_NOTE: &str = "---\ntype: note\ndomain: unity\ntags: [jump]\nagents: []\nstatus: active\nimportance: 0.8\nupdated: 2026-01-01\nsource: skill:x\n---\n# Coyote time\n\nLe coyote window laisse sauter juste apres avoir quitte le bord de la plateforme.\n";
const EXISTING_BODY: &str =
    "Le coyote window laisse sauter juste apres avoir quitte le bord de la plateforme.";

fn isolated(ws: &aidp_testkit::TempWorkspace) -> assert_cmd::Command {
    let mut command = aidp(ws);
    command.env("AIDP_GATEWAY_DIR", ws.child("gateway"));
    command
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn prepared() -> (aidp_testkit::TempWorkspace, String) {
    let ws = workspace();
    let project = ws.child(PROJECT_DIR);
    std::fs::create_dir_all(&project).unwrap();
    write(
        &ws.child("vault").join("Knowledge/_global/unity/coyote.md"),
        EXISTING_NOTE,
    );
    let project_arg = project.to_string_lossy().to_string();
    let output = isolated(&ws)
        .args(["rag", "index", "--source", "knowledge", "--project"])
        .arg(&project_arg)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    (ws, project_arg)
}

fn proposal(overrides: Value) -> Value {
    let mut base = json!({
        "action": "add", "domain": "unity", "scope": "project", "slug": "shader-passes",
        "title": "Passes de shader", "body": "Un shader URP declare ses passes dans un bloc SubShader.",
        "evidence": "vu dans le run"
    });
    for (key, value) in overrides.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn proposals_file(ws: &aidp_testkit::TempWorkspace, proposals: Vec<Value>) -> String {
    let file = ws.child("knowledge-proposals.json");
    write(&file, &json!({ "proposals": proposals }).to_string());
    file.to_string_lossy().to_string()
}

fn run(ws: &aidp_testkit::TempWorkspace, project: &str, args: &[&str]) -> std::process::Output {
    isolated(ws)
        .args(["--format", "json", "memory"])
        .args(args)
        .arg("--project")
        .arg(project)
        .output()
        .unwrap()
}

fn json_of(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}

fn search_paths(ws: &aidp_testkit::TempWorkspace, project: &str, query: &str) -> Vec<String> {
    let output = isolated(ws)
        .args(["--format", "json", "rag", "search", query, "--project"])
        .arg(project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let mut paths: Vec<String> = json_of(&output)
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["path"].as_str().unwrap().to_string())
        .collect();
    paths.dedup();
    paths
}

fn note(ws: &aidp_testkit::TempWorkspace, rel: &str) -> String {
    std::fs::read_to_string(ws.child("vault").join(rel)).unwrap()
}

#[test]
fn a_new_note_is_written_and_searchable_at_once() {
    let (ws, project) = prepared();
    let file = proposals_file(&ws, vec![proposal(json!({}))]);
    let output = run(&ws, &project, &["ingest-knowledge", &file]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report = json_of(&output);
    assert_eq!(
        (report["files"].clone(), report["added"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(
        report["results"][0]["path"],
        "Knowledge/game/notes/unity/shader-passes.md"
    );
    let written = note(&ws, "Knowledge/game/notes/unity/shader-passes.md");
    assert!(written.contains("source: manual\n"));
    assert!(written.contains("status: active\n"));
    assert_eq!(
        search_paths(&ws, &project, "subshader passes"),
        ["Knowledge/game/notes/unity/shader-passes.md"]
    );
}

#[test]
fn a_duplicate_only_adds_evidence() {
    let (ws, project) = prepared();
    let file = proposals_file(
        &ws,
        vec![proposal(json!({
            "slug": "coyote-again", "title": "Coyote time", "body": EXISTING_BODY, "scope": "global"
        }))],
    );
    let output = run(&ws, &project, &["ingest-knowledge", &file]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(json_of(&output)["merged"], 1);
    assert!(note(&ws, "Knowledge/_global/unity/coyote.md").contains("manual: vu dans le run"));
    assert!(!ws
        .child("vault")
        .join("Knowledge/_global/unity/coyote-again.md")
        .exists());
}

#[test]
fn a_refused_file_exits_with_the_policy_code() {
    let (ws, project) = prepared();
    let file = proposals_file(&ws, vec![proposal(json!({ "slug": "Bad Slug" }))]);
    let output = run(&ws, &project, &["ingest-knowledge", &file]);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(json_of(&output)["rejected"].as_array().unwrap().len(), 1);
}

#[test]
fn the_inbox_lists_approves_and_rejects() {
    let (ws, project) = prepared();
    let file = proposals_file(
        &ws,
        vec![
            proposal(
                json!({ "domain": "lore", "slug": "the-forest", "title": "La foret", "body": "La foret garde un esprit ancien." }),
            ),
            proposal(
                json!({ "domain": "rooms", "slug": "hub-room", "title": "Salle hub", "body": "La salle hub relie trois couloirs." }),
            ),
        ],
    );
    let output = run(&ws, &project, &["ingest-knowledge", &file]);
    assert_eq!(json_of(&output)["inboxed"], 2, "{output:?}");
    assert!(search_paths(&ws, &project, "esprit ancien").is_empty());

    let listed = json_of(&run(&ws, &project, &["inbox", "list"]));
    let paths: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "Knowledge/_inbox/hub-room.md",
            "Knowledge/_inbox/the-forest.md"
        ]
    );

    let approved = run(&ws, &project, &["inbox", "approve", "the-forest"]);
    assert_eq!(approved.status.code(), Some(0), "{approved:?}");
    assert_eq!(
        json_of(&approved)["path"],
        "Knowledge/game/notes/lore/the-forest.md"
    );
    assert_eq!(
        search_paths(&ws, &project, "esprit ancien"),
        ["Knowledge/game/notes/lore/the-forest.md"]
    );
    assert!(!ws
        .child("vault")
        .join("Knowledge/_inbox/the-forest.md")
        .exists());

    let rejected = run(
        &ws,
        &project,
        &["inbox", "reject", "Knowledge/_inbox/hub-room.md"],
    );
    assert_eq!(rejected.status.code(), Some(0), "{rejected:?}");
    assert!(ws
        .child("vault")
        .join("Knowledge/_inbox/_rejected/hub-room.md")
        .is_file());
    assert_eq!(json_of(&run(&ws, &project, &["inbox", "list"])), json!([]));
}

#[test]
fn approving_an_unknown_note_is_not_found() {
    let (ws, project) = prepared();
    let output = run(&ws, &project, &["inbox", "approve", "missing"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
}

#[test]
fn a_superseded_note_stops_being_retrieved() {
    let (ws, project) = prepared();
    let file = proposals_file(
        &ws,
        vec![proposal(json!({
            "action": "supersede", "scope": "global", "slug": "coyote-v2", "title": "Coyote v2",
            "body": "Nouvelle regle de saut tolerant sans rapport avec le bord.",
            "target": "_global/unity/coyote.md"
        }))],
    );
    let output = run(&ws, &project, &["ingest-knowledge", &file]);
    assert_eq!(json_of(&output)["superseded"], 1, "{output:?}");
    assert!(note(&ws, "Knowledge/_global/unity/coyote.md").contains("status: superseded\n"));
    let found = search_paths(&ws, &project, "coyote");
    assert!(
        found
            .iter()
            .all(|path| path != "Knowledge/_global/unity/coyote.md"),
        "{found:?}"
    );
}
