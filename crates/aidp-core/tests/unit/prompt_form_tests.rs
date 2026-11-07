use super::*;
use std::path::PathBuf;

const STAGE_NAME: &str = "implement";
const RUN_ID: &str = "20260101-000000-abcd";
const REPLY_FILE: &str = "reply.md";
const FORBIDDEN_FILE_FORM_PHRASE: &str = "do NOT write `reply.md`";

fn fixture_pipeline() -> Pipeline {
    serde_yaml::from_str(
        "version: 1\nid: fixture-pipe\nstages:\n  - name: implement\n    agent: executor\n    outputs: [changed-files.json]\n",
    )
    .unwrap()
}

fn fixture_state() -> RunState {
    RunState {
        run_id: RUN_ID.into(),
        run_dir: PathBuf::from("/runs/fixture"),
        ..Default::default()
    }
}

#[test]
fn workflow_form_equals_the_legacy_prompt() {
    let pipe = fixture_pipeline();
    let state = fixture_state();
    let legacy = build_prompt_text(&pipe, &state, STAGE_NAME).unwrap();
    let explicit =
        build_prompt_text_with_form(&pipe, &state, STAGE_NAME, PromptForm::Workflow).unwrap();
    assert_eq!(legacy, explicit);
    assert!(legacy.contains(FORBIDDEN_FILE_FORM_PHRASE));
}

#[test]
fn file_form_names_the_absolute_reply_path() {
    let pipe = fixture_pipeline();
    let state = fixture_state();
    let stage_dir = state.stage_dir(
        STAGE_NAME,
        pipeline::find_stage(&pipe, STAGE_NAME).unwrap().placement(),
    );
    let prompt = build_prompt_text_with_form(&pipe, &state, STAGE_NAME, PromptForm::File).unwrap();
    assert!(prompt.contains(&stage_dir.join(REPLY_FILE).display().to_string()));
    assert!(!prompt.contains(FORBIDDEN_FILE_FORM_PHRASE));
    assert!(!prompt.contains("no `reply.md` to write"));
    assert!(prompt.contains("agent-reply.v4"));
}

#[test]
fn run_state_mode_round_trips_and_defaults_to_none() {
    let mut state = fixture_state();
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("\"mode\""));
    state.mode = Some("native".into());
    let restored: RunState = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(restored.mode.as_deref(), Some("native"));
}
