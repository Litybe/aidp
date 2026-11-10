//! The doctor on bugs that do not converge: a ticket escalated on `bug_capped_stop`, the
//! evidence it computes from real git repositories, and the actions it takes. The git repos are
//! temporary local ones and `aidp`/`gh` are simulated.

use std::path::Path;

use crate::doctor_flow::{run_git, world_full, Investigation, World, KEY, MALFORMED_REPLY};
use aidp_flows::FlowStatus;
use aidp_gateway::models::enums::{DoctorAction, DoctorIncidentStatus, DoctorTrigger};
use serde_json::{json, Value};

const CAPPED: &str = include_str!("fixtures/doctor/capped.yaml");
const SCENE: &str = "Assets/Tests/Playtest/Scenes/PlaytestArena.unity";
const SCRIPT: &str = "Assets/Scripts/Player.cs";
const SCENE_BYTES: usize = 27_000;
const BROKEN_SCENE_BYTES: usize = 12_000;
const BRANCH: &str = "feat/GH-1";

fn write(workspace: &Path, path: &str, bytes: usize) {
    let target = workspace.join(path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, "x".repeat(bytes)).unwrap();
}

fn commit(workspace: &Path, message: &str) {
    run_git(workspace, &["add", "."]);
    run_git(workspace, &["commit", "-q", "-m", message]);
}

/// The GH-227 case: main holds the test scene, the feature branch merged three bugs and the first
/// one re-saved the scene without its Player.
fn broken_branch(world: &World) {
    let workspace = world.workspace.as_path();
    write(workspace, SCENE, SCENE_BYTES);
    write(workspace, SCRIPT, 10);
    commit(workspace, "test scene");
    run_git(workspace, &["push", "-q", "origin", "main"]);
    run_git(workspace, &["checkout", "-q", "-b", BRANCH]);
    for (number, scene) in [(2, BROKEN_SCENE_BYTES), (3, SCENE_BYTES), (4, SCENE_BYTES)] {
        let bug = format!("fix/GH-{number}");
        run_git(workspace, &["checkout", "-q", "-b", &bug, BRANCH]);
        write(workspace, SCRIPT, 10 + number);
        if number == 2 {
            write(workspace, SCENE, scene);
        }
        commit(workspace, &format!("fix(GH-{number}): le joueur manque"));
        run_git(workspace, &["checkout", "-q", BRANCH]);
        let message = format!("Merge branch '{bug}' into {BRANCH}");
        run_git(workspace, &["merge", "--no-ff", "-q", "-m", &message, &bug]);
    }
    run_git(workspace, &["push", "-q", "origin", BRANCH]);
    run_git(workspace, &["checkout", "-q", "main"]);
}

fn diagnosis(category: &str, action: DoctorAction, params: Value) -> Value {
    json!({
        "category": category,
        "cause": "la scène de test a perdu son Player",
        "evidence": [{ "file": "evidence/test-infra.json", "line": SCENE }],
        "confidence": 0.9,
        "action": action.as_str(),
        "params": params,
        "needs_human": false,
        "proposed_patch": null,
        "summary": "La scène de test est cassée par la branche."
    })
}

fn capped_world(tag: &str, diagnosis: Value) -> World {
    let world = world_full(
        tag,
        Investigation::Diagnosis(diagnosis),
        0,
        MALFORMED_REPLY,
        &[],
    );
    broken_branch(&world);
    world.start_def(KEY, CAPPED);
    world.wait_flow(FlowStatus::Escalated);
    world
}

fn calls(world: &World) -> Vec<Vec<String>> {
    world.cli.calls.lock().unwrap().clone()
}

fn called(world: &World, word: &str) -> bool {
    calls(world)
        .iter()
        .any(|args| args.iter().any(|arg| arg == word))
}

fn show(world: &World, spec: &str) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .args(["--git-dir", world.origin.to_str().unwrap(), "show", spec])
        .output()
        .unwrap();
    assert!(output.status.success(), "git show {spec}");
    output.stdout
}

fn evidence(world: &World, file: &str) -> Value {
    let incident = world.incident();
    let bundle = std::path::PathBuf::from(incident.bundle.unwrap());
    let text = std::fs::read_to_string(bundle.join("evidence").join(file)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn gh_227_the_scene_is_restored_the_test_rerun_and_the_cap_rearmed() {
    let world = capped_world(
        "converge-gh227",
        diagnosis(
            "test_infra_regression",
            DoctorAction::RestoreTestInfra,
            json!({ "files": [SCENE] }),
        ),
    );
    let main_before = show(&world, "main");
    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert_eq!(closed.candidate.trigger, DoctorTrigger::BugsNotConverging);
    assert_eq!(closed.actions[0].action, DoctorAction::RestoreTestInfra);
    assert!(closed.actions[0].ok, "{:?}", closed.actions);
    assert!(closed.rearmed_ms.is_some(), "{closed:?}");

    let history = evidence(&world, "bug-history.json");
    assert_eq!(history.as_array().unwrap().len(), 3, "{history}");
    let infra = evidence(&world, "test-infra.json");
    let scene = &infra.as_array().unwrap()[0];
    assert_eq!(scene["path"], SCENE);
    assert_eq!(scene["before_bytes"], SCENE_BYTES);
    assert_eq!(scene["after_bytes"], BROKEN_SCENE_BYTES);
    assert_eq!(scene["suspect"], true);

    assert_eq!(
        show(&world, &format!("{BRANCH}:{SCENE}")).len(),
        SCENE_BYTES
    );
    assert_eq!(show(&world, "main"), main_before);
    let pushes: Vec<String> = world
        .cli
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.iter().any(|arg| arg == "push"))
        .map(|line| line.join(" "))
        .collect();
    assert!(!pushes.is_empty(), "nothing was pushed");
    for push in &pushes {
        assert!(
            !push.contains("force") && !push.contains(" -f") && !push.contains(" +"),
            "{push}"
        );
        assert!(
            !push.ends_with(" main") && !push.contains("refs/heads/main"),
            "{push}"
        );
    }
    assert!(called(&world, "bug-rearm"), "{:?}", calls(&world));
    assert!(called(&world, "to-test"), "{:?}", calls(&world));
    assert_eq!(world.engine.get(KEY).unwrap().status, FlowStatus::Aborted);
}

#[test]
fn the_restore_refuses_a_file_outside_the_test_infrastructure() {
    let world = capped_world(
        "converge-restore-refused",
        diagnosis(
            "test_infra_regression",
            DoctorAction::RestoreTestInfra,
            json!({ "files": [SCRIPT] }),
        ),
    );
    let closed = world.investigate();
    assert_ne!(closed.status, DoctorIncidentStatus::Fixed, "{closed:?}");
    assert_eq!(
        show(&world, &format!("{BRANCH}:{SCENE}")).len(),
        BROKEN_SCENE_BYTES
    );
    assert!(!called(&world, "bug-rearm"));
}

#[test]
fn a_real_defect_without_a_proven_cause_asks_a_human_and_names_defect_file_criterion() {
    let world = capped_world(
        "converge-real-defect",
        diagnosis(
            "real_defect",
            DoctorAction::CreateBugTicket,
            json!({ "defect": "le joueur manque dans la scène", "file": SCENE }),
        ),
    );
    let closed = world.investigate();
    assert_eq!(
        closed.status,
        DoctorIncidentStatus::NeedsHuman,
        "{closed:?}"
    );
    let note = closed.note.unwrap();
    assert!(note.contains("défaut"), "{note}");
    assert!(note.contains(SCENE), "{note}");
    assert!(!called(&world, "bug-create"));
}

#[test]
fn a_shared_harness_failure_without_a_harness_ticket_asks_a_human() {
    let world = capped_world(
        "converge-shared-no-ticket",
        diagnosis(
            "shared_harness_failure",
            DoctorAction::CreateBugTicket,
            json!({ "pattern": "no_player_in_scene", "tickets": ["GH-1", "GH-5"] }),
        ),
    );
    let closed = world.investigate();
    assert_eq!(closed.status, DoctorIncidentStatus::Refused, "{closed:?}");
    assert!(!called(&world, "bug-create"));
}

#[test]
fn the_cap_incident_is_not_held_back_by_the_daily_cap() {
    let world = capped_world(
        "converge-uncapped",
        diagnosis("other", DoctorAction::None, json!({})),
    );
    let closed = world.investigate();
    assert_eq!(closed.candidate.trigger, DoctorTrigger::BugsNotConverging);
    assert_ne!(closed.status, DoctorIncidentStatus::Capped, "{closed:?}");
}
