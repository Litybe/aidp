use super::*;
use aidp_testkit::TempWorkspace;

const WORKSPACE_TAG: &str = "obsidian";
const NOTE_FILE: &str = "note.md";
const NOTE_CONTENT: &str = "content";

/// A directory standing where the note should go makes the write fail on
/// every OS, and the error names the path it could not write.
#[test]
fn write_if_changed_reports_a_path_it_cannot_write() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let blocked_note = workspace.child(NOTE_FILE);
    std::fs::create_dir_all(&blocked_note).unwrap();

    let error = write_if_changed(&blocked_note, NOTE_CONTENT).unwrap_err();

    assert!(error.contains(NOTE_FILE), "unexpected error: {error}");
}

/// The first write reports a change, the identical second one does not.
#[test]
fn write_if_changed_reports_written_then_unchanged() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let note = workspace.child(NOTE_FILE);

    assert_eq!(write_if_changed(&note, NOTE_CONTENT), Ok(true));
    assert_eq!(write_if_changed(&note, NOTE_CONTENT), Ok(false));
    assert_eq!(std::fs::read_to_string(&note).unwrap(), NOTE_CONTENT);
}

const RUN_ID: &str = "20260929-000000-abcd";
const GATE_STAGE: &str = "pr";
const PERSISTED_WORKFLOW_RUN_ID: &str = "wf-1";
const ANCHOR_PERSISTED_KEY: &str = "anchor_persisted";

fn run_dir_awaiting_gate(workspace: &TempWorkspace, with_result: bool) -> PathBuf {
    let run_dir = workspace.child(RUN_ID);
    std::fs::create_dir_all(&run_dir).unwrap();
    if with_result {
        let result = serde_json::json!({
            "status": "awaiting_gate",
            "gate": GATE_STAGE,
            "runId": PERSISTED_WORKFLOW_RUN_ID,
        });
        std::fs::write(
            run_dir.join(crate::run::WORKFLOW_RESULT_FILE),
            result.to_string(),
        )
        .unwrap();
    }
    run_dir
}

fn render_run_note(workspace: &TempWorkspace, state: &RunState) -> String {
    write_run_note(workspace.path(), state, None).unwrap();
    std::fs::read_to_string(vault::index_note(&state.run_dir)).unwrap()
}

fn state_in(run_dir: &Path) -> RunState {
    RunState {
        run_id: RUN_ID.into(),
        run_dir: run_dir.to_path_buf(),
        started_at: "2026-09-29T00:00:00Z".into(),
        ..Default::default()
    }
}

#[test]
fn run_note_flags_a_gate_whose_anchor_was_not_persisted() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let run_dir = run_dir_awaiting_gate(&workspace, true);

    let note = render_run_note(&workspace, &state_in(&run_dir));

    assert!(note.contains("anchor_persisted: false"), "{note}");
    assert!(
        note.contains(&format!("pending_gate: {GATE_STAGE}")),
        "{note}"
    );
    assert_eq!(note.matches("pending_gate:").count(), 1, "{note}");
    assert!(
        note.contains(&format!(
            "aidp run record-resume --run {RUN_ID} --from-result"
        )),
        "{note}"
    );
    assert!(
        note.contains(&format!("/pipeline-resume-v7 {RUN_ID}")),
        "{note}"
    );
}

#[test]
fn run_note_has_no_anchor_warning_without_a_result_file() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let run_dir = run_dir_awaiting_gate(&workspace, false);

    let note = render_run_note(&workspace, &state_in(&run_dir));

    assert!(!note.contains(ANCHOR_PERSISTED_KEY), "{note}");
}

#[test]
fn run_note_has_no_anchor_warning_when_the_anchor_is_persisted() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let run_dir = run_dir_awaiting_gate(&workspace, true);
    let state = RunState {
        workflow_run_id: Some(PERSISTED_WORKFLOW_RUN_ID.into()),
        ..state_in(&run_dir)
    };

    let note = render_run_note(&workspace, &state);

    assert!(!note.contains(ANCHOR_PERSISTED_KEY), "{note}");
}
