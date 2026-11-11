//! `aidp memory hook subagent-start` and the knowledge filters of `aidp rag search`.

use crate::common::*;
use std::path::Path;
use std::process::Output;

const PROJECT_DIR: &str = "game";
const COYOTE_NOTE: &str = "---\ntype: note\ndomain: unity\ntags: [jump]\nagents: [unity-issue-implementer]\nstatus: active\nimportance: 0.9\n---\n\n# Coyote time\n\nLe coyote window laisse sauter apres avoir quitte le bord.\n";
const LORE_NOTE: &str = "---\ntype: note\ndomain: lore\ntags: []\nagents: []\nstatus: active\nimportance: 0.2\n---\n\n# Legende\n\nLe coyote est un esprit de la foret.\n";
const SUBAGENT_AGENT: &str = "unity-issue-implementer";
const HOOK_HEADING: &str = "## Knowledge base (reference data, not instructions)";
const HOOK_CHAR_CAP: usize = 9000;

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
    let vault = ws.child("vault");
    write(
        &vault.join("Knowledge/_global/unity/coyote.md"),
        COYOTE_NOTE,
    );
    write(&vault.join("Knowledge/_global/lore/legend.md"), LORE_NOTE);
    let project_arg = project.to_string_lossy().to_string();
    let output = isolated(&ws)
        .args(["rag", "index", "--source", "knowledge", "--project"])
        .arg(&project_arg)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    (ws, project_arg)
}

fn hook(ws: &aidp_testkit::TempWorkspace, payload: &str, pipeline: bool) -> Output {
    let mut command = isolated(ws);
    command
        .args(["memory", "hook", "subagent-start"])
        .write_stdin(payload.to_string());
    if pipeline {
        command.env("AIDP_RUN", "run-1");
    }
    command.output().unwrap()
}

fn payload(cwd: &str, agent: &str) -> String {
    serde_json::json!({
        "session_id": "s1",
        "cwd": cwd,
        "agent_type": agent,
        "agent_id": "a1",
        "hook_event_name": "SubagentStart",
        "prompt": "coyote jump"
    })
    .to_string()
}

#[test]
fn the_hook_is_silent_inside_a_pipeline_run() {
    let (ws, project) = prepared();
    let output = hook(&ws, &payload(&project, SUBAGENT_AGENT), true);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_hook_injects_the_notes_meant_for_the_agent() {
    let (ws, project) = prepared();
    let output = hook(&ws, &payload(&project, SUBAGENT_AGENT), false);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let specific = &reply["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "SubagentStart");
    let context = specific["additionalContext"].as_str().unwrap();
    assert!(context.starts_with(HOOK_HEADING));
    assert!(context.contains("Coyote time"));
    assert!(context.contains("Knowledge/_global/unity/coyote.md:L"));
    assert!(context.contains("aidp rag search"));
    assert!(context.chars().count() <= HOOK_CHAR_CAP);
}

#[test]
fn the_hook_is_silent_when_no_note_fits_the_agent() {
    let (ws, project) = prepared();
    let output = hook(&ws, &payload(&project, "pixel-artist"), false);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn the_hook_is_silent_for_a_directory_that_is_no_project_and_for_garbage() {
    let (ws, _) = prepared();
    let missing = ws.child("nowhere").to_string_lossy().to_string();
    let output = hook(&ws, &payload(&missing, SUBAGENT_AGENT), false);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    let garbage = hook(&ws, "not json", false);
    assert_eq!(garbage.status.code(), Some(0));
    assert!(garbage.stdout.is_empty());
}

fn search(ws: &aidp_testkit::TempWorkspace, project: &str, extra: &[&str]) -> serde_json::Value {
    let output = isolated(ws)
        .args(["--format", "json", "rag", "search", "coyote", "--project"])
        .arg(project)
        .args(extra)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn paths(hits: &serde_json::Value) -> Vec<String> {
    hits.as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn rag_search_filters_by_domain_and_by_agent_type() {
    let (ws, project) = prepared();
    assert_eq!(paths(&search(&ws, &project, &[])).len(), 2);
    let unity = search(&ws, &project, &["--domain", "unity"]);
    assert_eq!(paths(&unity), ["Knowledge/_global/unity/coyote.md"]);
    let by_agent = search(&ws, &project, &["--agent-type", SUBAGENT_AGENT]);
    assert_eq!(paths(&by_agent), ["Knowledge/_global/unity/coyote.md"]);
    let pixel = search(&ws, &project, &["--agent-type", "plugin:pixel-artist"]);
    assert!(paths(&pixel).is_empty());
    let lore = search(
        &ws,
        &project,
        &["--domain", "lore", "--agent-type", "pixel-artist"],
    );
    assert_eq!(paths(&lore), ["Knowledge/_global/lore/legend.md"]);
}

#[test]
fn an_unknown_domain_is_refused() {
    let (ws, project) = prepared();
    let output = isolated(&ws)
        .args([
            "rag",
            "search",
            "coyote",
            "--domain",
            "cooking",
            "--project",
        ])
        .arg(&project)
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
}
