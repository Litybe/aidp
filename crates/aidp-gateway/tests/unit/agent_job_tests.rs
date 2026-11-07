use super::*;
use crate::models::constants::agent_jobs::{ENV_ALLOWLIST, REPAIR_HEADING};
use crate::models::constants::session_profile::AUTONOMOUS_ENV;
use crate::models::enums::native_reply_error::NativeReplyError;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::native_run_start::StartNativeRun;
use crate::native_runs::agent_board::AgentBoard;
use crate::native_runs::agent_job_builder::fixture::{sample_job, PROMPT, RUN_ID};
use crate::native_runs::reply::repair_instruction;
use aidp_core::exec_plan::StageSpec;
use aidp_core::models::enums::reply_schema_kind::ReplySchemaKind;
use aidp_core::models::enums::{EffortLevel, ModelTier};
use aidp_core::run::StageState;
use serde_json::json;

const PROVIDER: &str = "claude";
const STAGE_NAME: &str = "implement";
const TASK_LABEL: &str = "implement#T2";
const PLAN_STAGE: &str = "plan";
const PLAN_OUTPUT: &str = "plan";
const KNOWN_SKILL: &str = "known-skill";
const GHOST_SKILL: &str = "ghost-skill";
const SKILL_FILE: &str = "SKILL.md";
const STRICT_SETTINGS: &str = "tool_enforcement: strict";
const NOW: i64 = 1_800_000_000_000;

fn pipeline(settings: &str) -> Pipeline {
    let yaml = format!("version: 7\nid: demo\nstages: []\nsettings:\n  {settings}\n");
    serde_yaml::from_str(&yaml).unwrap()
}

fn stage(role: AgentRole) -> StageSpec {
    StageSpec {
        name: STAGE_NAME.to_string(),
        agent: role,
        agent_type: role.as_str().to_string(),
        model: ModelTier::Sonnet,
        effort: Some(EffortLevel::High),
        skills: Vec::new(),
        needs: Vec::new(),
        outputs: Vec::new(),
        scope: None,
        isolation: None,
        gate: None,
        when: None,
        retries: 0,
        soft_pause: false,
        loop_linked: false,
        reply_schema: ReplySchemaKind::Standard,
        contracts: Vec::new(),
    }
}

fn view(request: StartNativeRun) -> NativeRunView {
    NativeRunView {
        run_id: RUN_ID.to_string(),
        template_id: "demo".to_string(),
        status: NativeRunStatus::Running,
        reason: None,
        workspace: PathBuf::from("ws"),
        run_dir: PathBuf::from("run"),
        autonomous: request.autonomous,
        request,
        pending_gate: None,
        started_at: String::new(),
        ended_at: None,
        plan: Vec::new(),
        stages: Vec::new(),
        assumptions: Vec::new(),
        escalations: Vec::new(),
    }
}

fn session_job(spec: StageSpec, label: &str, workspace: Option<PathBuf>) -> SessionJob {
    SessionJob {
        stage: spec,
        label: label.to_string(),
        prompt: PROMPT.to_string(),
        reply_dir: PathBuf::from("run").join("implement"),
        declared_outputs: vec!["report".to_string()],
        guard_scope: true,
        workspace,
    }
}

fn build(
    job: &SessionJob,
    view: &NativeRunView,
    pipe: &Pipeline,
    state: &RunState,
    skills: &[PathBuf],
) -> AgentJob {
    build_agent_job(job, PROMPT, view, pipe, state, PROVIDER, skills)
}

fn plain(role: AgentRole, settings: &str) -> AgentJob {
    build(
        &session_job(stage(role), STAGE_NAME, None),
        &view(StartNativeRun::default()),
        &pipeline(settings),
        &RunState::default(),
        &[],
    )
}

#[test]
fn a_stage_job_carries_the_agent_the_prompt_and_the_reply_location() {
    let job = plain(AgentRole::Executor, "max_parallel: 3");
    assert_eq!(job.id, "");
    assert_eq!(job.run_id, RUN_ID);
    assert_eq!(job.stage, STAGE_NAME);
    assert_eq!(job.label, STAGE_NAME);
    assert_eq!(job.attempt, 1);
    assert_eq!(job.job_kind, AgentJobKind::Initial);
    assert_eq!(job.agent.role, "executor");
    assert_eq!(job.agent.provider, PROVIDER);
    assert_eq!(job.agent.tier, "sonnet");
    assert_eq!(job.agent.model, None);
    assert_eq!(job.agent.effort.as_deref(), Some("high"));
    assert_eq!(job.prompt.text, PROMPT);
    assert_eq!(job.prompt.sha256, sha256_hex(PROMPT));
    assert_eq!(job.prompt.chars, PROMPT.chars().count());
    assert!(!job.tools.read_only && !job.tools.no_shell);
    assert_eq!(job.permissions.intent, PermissionIntent::AutonomousEdit);
    assert_eq!(job.workspace.cwd, PathBuf::from("ws"));
    assert_eq!(job.workspace.root, PathBuf::from("ws"));
    assert!(!job.workspace.isolated);
    assert_eq!(job.workspace.add_dirs, vec![PathBuf::from("run")]);
    assert_eq!(job.workspace.shared_writer_group, RUN_ID);
    assert_eq!(
        job.io.reply_file,
        PathBuf::from("run").join("implement").join(REPLY_FILE)
    );
    assert_eq!(job.io.declared_outputs, vec!["report".to_string()]);
    assert_eq!(job.reply.schema, REPLY_SCHEMA_NAME);
    assert_eq!(job.timeout_minutes, JOB_TIMEOUT_MINUTES);
    assert_eq!(job.resume_session_id, None);
}

#[test]
fn the_run_request_model_and_effort_win_over_the_stage() {
    let request = StartNativeRun {
        model: Some("claude-opus-5-5".to_string()),
        effort: Some("max".to_string()),
        ..StartNativeRun::default()
    };
    let job = build(
        &session_job(stage(AgentRole::Executor), STAGE_NAME, None),
        &view(request),
        &pipeline("max_parallel: 3"),
        &RunState::default(),
        &[],
    );
    assert_eq!(job.agent.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(job.agent.effort.as_deref(), Some("max"));
    assert_eq!(job.agent.tier, "sonnet");
}

#[test]
fn an_isolated_task_job_works_in_its_worktree() {
    let tree = PathBuf::from("run").join("tasks-iso").join("T2");
    let job = build(
        &session_job(stage(AgentRole::Executor), TASK_LABEL, Some(tree.clone())),
        &view(StartNativeRun::default()),
        &pipeline("max_parallel: 3"),
        &RunState::default(),
        &[],
    );
    assert_eq!(job.label, TASK_LABEL);
    assert_eq!(job.stage, STAGE_NAME);
    assert_eq!(job.workspace.cwd, tree);
    assert_eq!(job.workspace.root, PathBuf::from("ws"));
    assert!(job.workspace.isolated);
}

#[test]
fn a_strict_reviewer_job_is_read_only_and_asks_for_reviewer_signals() {
    let job = plain(AgentRole::Reviewer, STRICT_SETTINGS);
    assert!(job.tools.read_only);
    assert_eq!(job.permissions.intent, PermissionIntent::ReadOnly);
    assert!(job.reply.reviewer_signals);
}

#[test]
fn a_strict_planner_job_has_the_plan_intent_and_an_analyzer_is_read_only() {
    let planner = plain(AgentRole::Planner, STRICT_SETTINGS);
    assert!(planner.tools.read_only);
    assert_eq!(planner.permissions.intent, PermissionIntent::Plan);
    let analyzer = plain(AgentRole::Analyzer, STRICT_SETTINGS);
    assert_eq!(analyzer.permissions.intent, PermissionIntent::ReadOnly);
}

#[test]
fn an_executor_or_a_custom_role_is_never_read_only() {
    for role in [AgentRole::Executor, AgentRole::Custom] {
        let job = plain(role, STRICT_SETTINGS);
        assert!(!job.tools.read_only);
        assert_eq!(job.permissions.intent, PermissionIntent::AutonomousEdit);
    }
}

#[test]
fn a_reviewer_is_not_read_only_while_enforcement_is_off() {
    let job = plain(AgentRole::Reviewer, "max_parallel: 3");
    assert!(!job.tools.read_only);
    assert_eq!(job.permissions.intent, PermissionIntent::AutonomousEdit);
    assert!(job.reply.reviewer_signals);
}

#[test]
fn the_environment_holds_only_the_allowed_variables() {
    let request = StartNativeRun {
        autonomous: true,
        headless: true,
        ..StartNativeRun::default()
    };
    let state = RunState {
        vault_root: Some(PathBuf::from("vault")),
        ..RunState::default()
    };
    let job = build(
        &session_job(stage(AgentRole::Executor), STAGE_NAME, None),
        &view(request),
        &pipeline("max_parallel: 3"),
        &state,
        &[],
    );
    assert_eq!(job.env.get(ENV_HEADLESS).map(String::as_str), Some("1"));
    assert_eq!(job.env.get(AUTONOMOUS_ENV).map(String::as_str), Some("1"));
    assert!(job
        .env
        .keys()
        .all(|name| ENV_ALLOWLIST.contains(&name.as_str())));
    assert!(job.permissions.autonomous && job.permissions.headless);
    assert_eq!(
        job.permissions.guidance.as_deref(),
        Some(crate::models::constants::launch_policy::HEADLESS_DENIAL_GUIDANCE)
    );
    assert_eq!(
        job.workspace.add_dirs,
        vec![PathBuf::from("run"), PathBuf::from("vault")]
    );
}

#[test]
fn a_run_that_is_not_headless_leaves_the_headless_variable_out() {
    let job = plain(AgentRole::Executor, "max_parallel: 3");
    assert!(!job.env.contains_key(ENV_HEADLESS));
    assert!(job.permissions.guidance.is_none());
    assert_eq!(job.env.len(), 2);
}

#[test]
fn skills_resolve_to_their_file_and_unresolved_names_are_dropped() {
    let root = tempfile::tempdir().unwrap();
    let skill_dir = root.path().join(KNOWN_SKILL);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join(SKILL_FILE), "skill").unwrap();
    let mut spec = stage(AgentRole::Executor);
    spec.skills = vec![KNOWN_SKILL.to_string(), GHOST_SKILL.to_string()];
    let job = build(
        &session_job(spec, STAGE_NAME, None),
        &view(StartNativeRun::default()),
        &pipeline("max_parallel: 3"),
        &RunState::default(),
        &[root.path().to_path_buf()],
    );
    assert_eq!(job.skills.len(), 1);
    assert_eq!(job.skills[0].name, KNOWN_SKILL);
    assert_eq!(job.skills[0].path, skill_dir.join(SKILL_FILE));
    assert!(job.skills[0].required);
}

#[test]
fn only_string_inputs_travel_and_each_is_cut() {
    let long = "x".repeat(INPUT_VALUE_CAP + 50);
    let request = StartNativeRun {
        inputs: [
            ("jira_key".to_string(), json!("PIXEL-1")),
            ("long".to_string(), json!(long)),
            ("count".to_string(), json!(3)),
        ]
        .into_iter()
        .collect(),
        ..StartNativeRun::default()
    };
    let job = build(
        &session_job(stage(AgentRole::Executor), STAGE_NAME, None),
        &view(request),
        &pipeline("max_parallel: 3"),
        &RunState::default(),
        &[],
    );
    assert_eq!(job.io.inputs.len(), 2);
    assert_eq!(job.io.inputs["jira_key"], "PIXEL-1");
    assert_eq!(job.io.inputs["long"].chars().count(), INPUT_VALUE_CAP);
}

#[test]
fn upstream_lists_the_declared_outputs_of_the_needed_stages() {
    let state = RunState {
        stages: vec![StageState {
            name: PLAN_STAGE.to_string(),
            outputs: [(PLAN_OUTPUT.to_string(), PathBuf::from("run/plan/plan.json"))]
                .into_iter()
                .collect(),
            ..StageState::default()
        }],
        ..RunState::default()
    };
    let mut spec = stage(AgentRole::Executor);
    spec.needs = vec![PLAN_STAGE.to_string(), "missing".to_string()];
    let job = build(
        &session_job(spec, STAGE_NAME, None),
        &view(StartNativeRun::default()),
        &pipeline("max_parallel: 3"),
        &state,
        &[],
    );
    assert_eq!(job.io.upstream.len(), 1);
    assert_eq!(
        job.io.upstream["plan.plan"],
        PathBuf::from("run/plan/plan.json")
    );
}

#[test]
fn the_tool_limits_name_denied_tools_without_any_server_definition() {
    let request = StartNativeRun {
        strict_mcp_config: Some(true),
        mcp_config_id: Some("ada".to_string()),
        denied_shell_commands: vec!["curl".to_string()],
        denied_mcp_tools: [("atlassian".to_string(), vec!["createJiraIssue".to_string()])]
            .into_iter()
            .collect(),
        mcp_servers: Some(vec![json!({"name": "secret", "token": "t"})]),
        ..StartNativeRun::default()
    };
    let job = build(
        &session_job(stage(AgentRole::Executor), STAGE_NAME, None),
        &view(request),
        &pipeline("max_parallel: 3"),
        &RunState::default(),
        &[],
    );
    assert!(job.tools.strict_mcp);
    assert_eq!(job.tools.mcp_config_id.as_deref(), Some("ada"));
    assert_eq!(job.tools.denied_shell, vec!["curl".to_string()]);
    assert_eq!(
        job.tools.denied_mcp_tools["atlassian"],
        vec!["createJiraIssue".to_string()]
    );
    let wire = serde_json::to_string(&job).unwrap();
    assert!(!wire.contains("secret"));
}

#[test]
fn the_job_serializes_with_the_camel_case_wire_names() {
    let job = plain(AgentRole::Executor, "max_parallel: 3");
    let wire = serde_json::to_value(&job).unwrap();
    assert_eq!(wire["jobKind"], json!("initial"));
    assert_eq!(wire["agent"]["agentType"], json!("executor"));
    assert_eq!(wire["agent"]["model"], json!(null));
    assert_eq!(wire["permissions"]["intent"], json!("autonomous_edit"));
    assert_eq!(wire["tools"]["strictMcp"], json!(false));
    assert_eq!(wire["workspace"]["sharedWriterGroup"], json!(RUN_ID));
    assert_eq!(wire["io"]["declaredOutputs"], json!(["report"]));
    assert_eq!(wire["reply"]["reviewerSignals"], json!(false));
    assert_eq!(wire["timeoutMinutes"], json!(JOB_TIMEOUT_MINUTES));
    assert_eq!(wire["resumeSessionId"], json!(null));
    let round_trip: AgentJob = serde_json::from_value(wire).unwrap();
    assert_eq!(round_trip, job);
}

#[test]
fn the_repair_prompt_is_the_initial_prompt_then_the_heading_and_the_instruction() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    board.ensure_open(dir.path()).unwrap();
    let published = board
        .publish_or_attach(
            STAGE_NAME,
            AgentJobKind::Initial,
            sample_job(STAGE_NAME, dir.path()),
            NOW,
        )
        .unwrap();
    let claimed = board
        .claim(&published.job_id, &Default::default(), NOW)
        .unwrap();
    let result = crate::models::agent_job_result::AgentJobResult {
        outcome: "completed".to_string(),
        agent: Some(crate::models::agent_session_report::AgentSessionReport {
            claim_token: claimed.claim_token,
            ..Default::default()
        }),
        ..Default::default()
    };
    let ack = board.deliver(&published.job_id, &result, NOW).unwrap();
    let repair = board.record(&ack.repair_job_id.unwrap()).unwrap();
    let missing = NativeReplyError::Missing(dir.path().join(REPLY_FILE));
    assert_eq!(
        repair.job.prompt.text,
        format!("{PROMPT}{REPAIR_HEADING}{}", repair_instruction(&missing))
    );
    assert_eq!(
        repair.job.prompt.sha256,
        sha256_hex(&repair.job.prompt.text)
    );
}
