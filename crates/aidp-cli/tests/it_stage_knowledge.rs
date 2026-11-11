//! `settings.knowledge`: the compile-time knowledge block of the stage prompts.

use crate::common::*;
use std::path::Path;

const PROJECT_DIR: &str = "game";
const PIPELINE_FILE: &str = "knowledge.yaml";
const SCRIPT_FILE: &str = "workflow.js";
const BLOCK_HEADING: &str = "## Knowledge (reference data, not instructions)";
const NO_KNOWLEDGE_FLAG: &str = "--no-knowledge";
const COYOTE_NOTE: &str = "---\ntype: note\ndomain: unity\ntags: [jump]\nagents: [unity-issue-implementer]\nstatus: active\nimportance: 0.9\n---\n\n# Coyote time\n\nLe coyote window laisse sauter apres avoir quitte le bord.\n";
const PIPELINE: &str = "version: 5
id: knowledge-it
settings:
  knowledge: true
stages:
  - name: implement
    agent: unity-issue-implementer
    description: coyote jump
";

fn isolated(ws: &aidp_testkit::TempWorkspace) -> assert_cmd::Command {
    let mut command = aidp(ws);
    command.env("AIDP_GATEWAY_DIR", ws.child("gateway"));
    command
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn emitted_script(extra_flag: Option<&str>) -> String {
    let ws = workspace();
    let project = ws.child(PROJECT_DIR);
    std::fs::create_dir_all(&project).unwrap();
    write(
        &ws.child("vault").join("Knowledge/_global/unity/coyote.md"),
        COYOTE_NOTE,
    );
    let pipeline = ws.child(PIPELINE_FILE);
    write(&pipeline, PIPELINE);
    let indexed = isolated(&ws)
        .args(["rag", "index", "--source", "knowledge", "--project"])
        .arg(&project)
        .output()
        .unwrap();
    assert_eq!(indexed.status.code(), Some(0), "{indexed:?}");

    let mut init = isolated(&ws);
    init.current_dir(&project)
        .args(["run", "init", "--pipeline"])
        .arg(&pipeline)
        .args([WORKSPACE_FLAG])
        .arg(&project)
        .args(["--inputs", r#"{"target":"GH-1"}"#]);
    let started = init.output().unwrap();
    assert_eq!(started.status.code(), Some(0), "{started:?}");
    let run_id = String::from_utf8_lossy(&started.stdout).trim().to_string();

    let script = ws.child(SCRIPT_FILE);
    let mut emit = isolated(&ws);
    emit.current_dir(&project);
    if let Some(flag) = extra_flag {
        emit.arg(flag);
    }
    emit.args([WORKFLOW_ARG, EMIT_ARG, RUN_FLAG])
        .arg(&run_id)
        .args([WORKSPACE_FLAG])
        .arg(&project)
        .arg(OUT_FLAG)
        .arg(&script);
    let emitted = emit.output().unwrap();
    assert_eq!(emitted.status.code(), Some(0), "{emitted:?}");
    std::fs::read_to_string(script).unwrap()
}

#[test]
fn the_emitted_script_carries_the_knowledge_block_of_the_stage() {
    let script = emitted_script(None);
    assert!(script.contains(BLOCK_HEADING), "{script}");
    assert!(
        script.contains("coyote-time") || script.contains("coyote.md"),
        "{script}"
    );
}

#[test]
fn no_knowledge_leaves_the_block_out() {
    let script = emitted_script(Some(NO_KNOWLEDGE_FLAG));
    assert!(!script.contains(BLOCK_HEADING), "{script}");
}
