use super::*;
use crate::models::constants::knowledge_context::KNOWLEDGE_BLOCK_HEADING;
use crate::models::enums::KnowledgeRoute;
use crate::pipeline;
use crate::prompt::build_prompt_text;
use std::cell::RefCell;

const NOTE: &str = "Use `coyote` time \"always\" ${danger} \\ done";
const TARGET: &str = "GH-12";

const ENABLED: &str = "version: 5
id: p
settings:
  knowledge:
    enabled: true
    budget: 700
    route: code
stages:
  - name: plan
    agent: planner
    description: Plan the dash.
  - name: loop-review
    type: loop
    body:
      - name: review
        agent: reviewer
  - name: implement
";

const RESTRICTED: &str = "version: 5
id: p
settings:
  knowledge:
    enabled: true
    stages: [implement]
stages:
  - name: plan
    agent: planner
  - name: implement
";

const DISABLED: &str = "version: 5
id: p
stages:
  - name: plan
    agent: planner
  - name: implement
";

struct FakeProvider {
    requests: RefCell<Vec<StageContextRequest>>,
    reply: Option<String>,
}

impl FakeProvider {
    fn replying(reply: Option<&str>) -> Self {
        Self {
            requests: RefCell::new(Vec::new()),
            reply: reply.map(str::to_string),
        }
    }
}

impl ContextProvider for FakeProvider {
    fn stage_context(&self, request: &StageContextRequest) -> Option<String> {
        self.requests.borrow_mut().push(request.clone());
        self.reply.clone()
    }
}

fn state_in(dir: &std::path::Path) -> RunState {
    let mut state = RunState {
        run_id: "20260101-000000-0000".into(),
        pipeline_path: dir.join("p.yaml"),
        pipeline_id: "p".into(),
        workspace_root: dir.to_path_buf(),
        run_dir: dir.join("run"),
        started_at: "2026-01-01T00:00:00Z".into(),
        knowledge_root: Some(dir.join("knowledge-base")),
        ..Default::default()
    };
    state
        .inputs
        .insert("target".into(), serde_json::Value::String(TARGET.into()));
    state
}

#[test]
fn freezes_the_block_of_every_agent_stage_once() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = pipeline::parse(ENABLED).unwrap();
    let mut state = state_in(dir.path());
    let provider = FakeProvider::replying(Some(NOTE));

    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();
    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();

    let names: Vec<String> = state.stage_knowledge.keys().cloned().collect();
    assert_eq!(names, ["implement", "plan", "review"]);
    let requests = provider.requests.borrow();
    assert_eq!(
        requests.len(),
        3,
        "the second call reuses the frozen blocks"
    );
    let plan = &requests[0];
    assert_eq!(plan.stage_id, "plan");
    assert_eq!(plan.agent_type, "planner");
    assert_eq!(plan.stage_description, "Plan the dash.");
    assert_eq!(plan.budget_tokens, 700);
    assert_eq!(plan.route, Some(KnowledgeRoute::Code));
    assert_eq!(plan.issue.as_deref(), Some(TARGET));
    assert_eq!(requests[2].agent_type, "executor");
    assert!(dir.path().join("run/run-state.json").is_file());
}

#[test]
fn stages_setting_restricts_the_block() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = pipeline::parse(RESTRICTED).unwrap();
    let mut state = state_in(dir.path());
    let provider = FakeProvider::replying(Some(NOTE));

    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();

    assert_eq!(state.stage_knowledge.len(), 1);
    assert!(state.stage_knowledge.contains_key("implement"));
}

#[test]
fn disabled_or_providerless_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let off = pipeline::parse(DISABLED).unwrap();
    let on = pipeline::parse(ENABLED).unwrap();
    let mut state = state_in(dir.path());
    let provider = FakeProvider::replying(Some(NOTE));

    ensure_stage_knowledge(&mut state, &off, Some(&provider)).unwrap();
    ensure_stage_knowledge(&mut state, &on, None).unwrap();

    assert!(state.stage_knowledge.is_empty());
    assert!(provider.requests.borrow().is_empty());
    assert!(!dir.path().join("run").exists());
}

#[test]
fn an_empty_answer_is_frozen_and_adds_no_section() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = pipeline::parse(ENABLED).unwrap();
    let mut state = state_in(dir.path());
    let before = build_prompt_text(&pipe, &state, "implement").unwrap();
    let provider = FakeProvider::replying(None);

    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();
    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();

    assert_eq!(provider.requests.borrow().len(), 3);
    assert_eq!(
        build_prompt_text(&pipe, &state, "implement").unwrap(),
        before
    );
}

#[test]
fn the_prompt_carries_the_block_after_the_goal_and_only_where_frozen() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = pipeline::parse(RESTRICTED).unwrap();
    let mut state = state_in(dir.path());
    let before_plan = build_prompt_text(&pipe, &state, "plan").unwrap();
    let provider = FakeProvider::replying(Some(NOTE));

    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();

    let implement = build_prompt_text(&pipe, &state, "implement").unwrap();
    let heading = implement.find(KNOWLEDGE_BLOCK_HEADING).expect("heading");
    assert!(heading > implement.find("## Goal").unwrap());
    assert!(implement[heading..].contains(NOTE));
    assert_eq!(
        build_prompt_text(&pipe, &state, "plan").unwrap(),
        before_plan
    );
}

#[test]
fn the_script_escapes_the_block_for_javascript() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = pipeline::parse(RESTRICTED).unwrap();
    let mut state = state_in(dir.path());
    let provider = FakeProvider::replying(Some(NOTE));
    ensure_stage_knowledge(&mut state, &pipe, Some(&provider)).unwrap();

    let script = crate::workflow_emit::compile_with(&pipe, &state)
        .unwrap()
        .script;

    let escaped = serde_json::to_string(NOTE).unwrap();
    let inner = &escaped[1..escaped.len() - 1];
    assert!(script.contains(inner), "{script}");
}
