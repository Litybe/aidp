//! End-to-end native plan tasks: parallel scheduling, per-task scope checks and the merge session.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::native_runs::fake_provider::{
    Behaviour, Call, ScriptedProvider, BLOCKING_SIGNAL, PROVIDER_ID,
};
use aidp_agents::ProvidersConfig;
use aidp_core::models::enums::ItemFailurePolicy;
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_core::run;
use aidp_core::workflow_emit;
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::registry::AlwaysAllow;
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::enums::native_stage_status::NativeStageStatus;
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::models::native_run_view::NativeRunView;
use aidp_gateway::models::LocalLock;
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde::Serialize;
use serde_json::Value;

const PLAN_FOUR_TASKS: &str = include_str!("fixtures/native_plan_tasks/plan_four_tasks.json");
const PLAN_OUT_OF_SCOPE: &str = include_str!("fixtures/native_plan_tasks/plan_out_of_scope.json");
const PLAN_SECOND_ITERATION: &str =
    include_str!("fixtures/native_plan_tasks/plan_second_iteration.json");
const PLAN_TEMPLATE: &str = include_str!("fixtures/native_plan_tasks/plan_tasks_pipeline.yaml");
const LOOP_TEMPLATE: &str =
    include_str!("fixtures/native_plan_tasks/plan_tasks_loop_pipeline.yaml");
const TEMPLATE_FILE: &str = "template.yaml";
const MAX_PARALLEL_TOKEN: &str = "@MAX_PARALLEL@";
const STAGE_MAX_TOKEN: &str = "@STAGE_MAX@";
const POLICY_TOKEN: &str = "@POLICY@";
const IMPLEMENTATION: &str = "implementation";
const REIMPLEMENTATION: &str = "reimplementation";
const REVIEW: &str = "review";
const REREVIEW: &str = "rereview";
const REPLAN: &str = "replan";
const ROW_SEPARATOR: &str = "#";
const FEATURE_INPUT: &str = "feature";
const FEATURE_VALUE: &str = "native-plan-tasks";
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USER_PROFILE_ENV: &str = "USERPROFILE";
const RUN_STATE_FILE: &str = "run-state.json";
const SCHEDULES_KEY: &str = "plan_schedules";
const SERIALIZATIONS_KEY: &str = "serializations";
const TASKS_KEY: &str = "tasks";
const ID_KEY: &str = "id";
const FILES_KEY: &str = "files";
const BEFORE_KEY: &str = "before";
const AFTER_KEY: &str = "after";
const CHANGED_FILES: &str = "changed-files.json";
const README: &str = "README.md";
const ORIGINAL_README: &str = "original\n";
const KEEP_FILE: &str = "src/keep.txt";
const STRAY_FILE: &str = "stray/outside.txt";
const REGISTRY_FILE: &str = "src/registry.txt";
const SHARED_FILE: &str = "src/shared_one.txt";
const MERGE_REGISTRY_CONTENT: &str = "registered by the merge\n";
const SERIALIZED_HEADING: &str = "Serialized with";
const SERIALIZED_LINE: &str = "`T3` runs before `T4`";
const SHARED_SECTION: &str = "Shared files to update now";
const FAILED_PAUSE_REASON: &str = "task `implementation#T2` failed";
const SRC_DIR: &str = "src";
const KEEP_CONTENT: &str = "keep\n";
const GIT: &str = "git";
const GIT_INIT: [&str; 2] = ["init", "-q"];
const GIT_CONFIG: &str = "config";
const GIT_COMMIT: [&str; 4] = ["commit", "-q", "-m", "initial"];
const GIT_ADD: [&str; 2] = ["add", "."];
const GIT_STATUS: [&str; 3] = ["status", "--porcelain", "-uall"];
const GIT_EMAIL: [&str; 2] = ["user.email", "native@example.test"];
const GIT_NAME: [&str; 2] = ["user.name", "native"];
const GIT_AUTOCRLF: [&str; 2] = ["core.autocrlf", "false"];
const VAULT_DIR: &str = "vault";
const HOME_DIR: &str = "home";
const STATE_DIR: &str = "state";
const WORKSPACE_DIR: &str = "workspace";
const TASK_ONE: &str = "T1";
const TASK_TWO: &str = "T2";
const TASK_THREE: &str = "T3";
const TASK_FOUR: &str = "T4";
const FIX_ONE: &str = "R1";
const FIX_TWO: &str = "R2";
const FILE_A: &str = "src/a.txt";
const FILE_B: &str = "src/b.txt";
const FILE_C: &str = "src/c.txt";
const FIX_ONE_FILE: &str = "src/fix_one.txt";
const TAMPERED_CONTENT: &str = "tampered\n";
const OUTSIDE_CONTENT: &str = "outside\n";
const CLAIMED_CONTENT: &str = "claimed by a task\n";
const TAG_PARALLEL: &str = "plan-tasks-parallel";
const TAG_ISOLATION: &str = "plan-tasks-isolation";
const TAG_SCOPE: &str = "plan-tasks-scope";
const TAG_CONTINUE: &str = "plan-tasks-continue";
const TAG_PAUSE: &str = "plan-tasks-pause";
const TAG_CAP_PREFIX: &str = "plan-tasks-cap";
const TAG_SHARED: &str = "plan-tasks-shared";
const TAG_LOOP: &str = "plan-tasks-loop";
const TAG_SEQUENTIAL: &str = "plan-tasks-sequential";
const TAG_CONCURRENT: &str = "plan-tasks-concurrent";
const TAG_PARITY: &str = "plan-tasks-parity";
const LINE_END: &str = "\n";
const FOUR_TASK_FILE_COUNT: usize = 3;
const RUN_ARTIFACT_DIR: &str = ".aidp";
const ISOLATION_FOLDER: &str = "iso";
const NODE: &str = "node";
const WORKFLOW_RUNNER: &str = include_str!("fixtures/native_plan_tasks/workflow_runner.js");
const WORKFLOW_RUNNER_FILE: &str = "workflow_runner.js";
const WORKFLOW_SCRIPT_FILE: &str = "workflow.js";
const WORKFLOW_PIPELINE_FILE: &str = "pipeline.yaml";
const WORKFLOW_PLAN_DIR: &str = "plan";
const WORKFLOW_PLAN_FILE: &str = "plan.json";
const WORKFLOW_WORKSPACE_DIR: &str = "workflow_workspace";
const WORKFLOW_SCRATCH_DIR: &str = "workflow_scratch";
const WORKFLOW_TAG: &str = "plan-tasks-workflow";
const WORKFLOW_DONE_STATUS: &str = "done";
const STATUS_KEY: &str = "status";
const PORCELAIN_PATH_OFFSET: usize = 3;
const FOUR_TASK_IDS: [&str; 4] = [TASK_ONE, TASK_TWO, TASK_THREE, TASK_FOUR];
const FIX_IDS: [&str; 2] = [FIX_ONE, FIX_TWO];
const HOLD: Duration = Duration::from_millis(300);
const SETTLE: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(5);
const RUN_POLL: Duration = Duration::from_millis(10);
const DEFAULT_CAP: u32 = 3;
/// One task at a time: no isolated worktree, so a write outside a task's files fails the task.
const SEQUENTIAL_CAP: u32 = 1;
const STAGE_MAX: u32 = 4;

struct Harness {
    provider: Arc<ScriptedProvider>,
    manager: Arc<NativeRunManager>,
    project_id: String,
    state_dir: PathBuf,
    workspace: PathBuf,
    _projects: Arc<Projects>,
    _root: TempWorkspace,
    _env: EnvGuard,
}

/// What the run view showed while the tasks of one stage were live.
struct Observation {
    peak: usize,
    snapshots: BTreeSet<BTreeSet<String>>,
}

impl Observation {
    fn ever_together(&self, ids: &[&str]) -> bool {
        self.snapshots
            .iter()
            .any(|live| ids.iter().all(|id| live.contains(&row(IMPLEMENTATION, id))))
    }

    fn ever_live(&self, id: &str) -> bool {
        self.ever_together(&[id])
    }
}

fn task_content(id: &str) -> String {
    format!("{id}{LINE_END}")
}

fn row(stage: &str, id: &str) -> String {
    format!("{stage}{ROW_SEPARATOR}{id}")
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new(GIT)
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_workspace(directory: &Path) {
    std::fs::create_dir_all(directory.join(SRC_DIR)).unwrap();
    std::fs::write(directory.join(README), ORIGINAL_README).unwrap();
    std::fs::write(directory.join(KEEP_FILE), KEEP_CONTENT).unwrap();
    run_git(directory, &GIT_INIT);
    for [key, value] in [GIT_EMAIL, GIT_NAME, GIT_AUTOCRLF] {
        run_git(directory, &[GIT_CONFIG, key, value]);
    }
    run_git(directory, &GIT_ADD);
    run_git(directory, &GIT_COMMIT);
}

impl Harness {
    fn new(tag: &str, provider: Arc<ScriptedProvider>) -> Harness {
        let root = TempWorkspace::new(tag);
        let vault = root.child(VAULT_DIR);
        let home = root.child(HOME_DIR);
        let workspace = root.child(WORKSPACE_DIR);
        let state_dir = root.child(STATE_DIR);
        for directory in [&vault, &home, &state_dir] {
            std::fs::create_dir_all(directory).unwrap();
        }
        init_workspace(&workspace);
        let env = EnvGuard::with(&[
            (VAULT_ENV, Some(vault.to_str().unwrap())),
            (HOME_ENV, Some(home.to_str().unwrap())),
            (USER_PROFILE_ENV, Some(home.to_str().unwrap())),
        ]);
        let projects = Arc::new(Projects::load(&state_dir));
        let project_id = projects.add(workspace.to_str().unwrap()).id;
        let bus = Arc::new(Bus::new());
        let runner: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
        let sessions =
            SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), runner)
                .with_provider(PROVIDER_ID, provider.clone())
                .with_providers_config(ProvidersConfig::default());
        let mut deps = NativeDeps::new(
            sessions,
            projects.clone(),
            bus,
            Arc::new(AlwaysAllow),
            Arc::new(LocalLock::default()),
            &state_dir,
            None,
        );
        deps.poll_interval = RUN_POLL;
        Harness {
            provider,
            manager: NativeRunManager::new(deps),
            project_id,
            state_dir,
            workspace,
            _projects: projects,
            _root: root,
            _env: env,
        }
    }

    fn write_template(&self, body: &str) -> PathBuf {
        let path = self.state_dir.join(TEMPLATE_FILE);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn start(&self, template: &Path) -> String {
        let request = StartNativeRun {
            project_id: self.project_id.clone(),
            template_id: template.to_string_lossy().to_string(),
            inputs: BTreeMap::from([(FEATURE_INPUT.to_string(), Value::from(FEATURE_VALUE))]),
            provider: Some(PROVIDER_ID.to_string()),
            ..StartNativeRun::default()
        };
        self.manager.start(request).expect("the run starts")
    }

    fn view(&self, run_id: &str) -> NativeRunView {
        self.manager.get(run_id).expect("the run exists")
    }

    fn finish(&self, run_id: &str) -> NativeRunView {
        self.manager
            .wait_for(run_id, SETTLE)
            .expect("the run exists")
    }

    fn state_json(&self, run_id: &str) -> Value {
        let file = self.view(run_id).run_dir.join(RUN_STATE_FILE);
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
    }

    fn read_workspace(&self, relative_path: &str) -> String {
        std::fs::read_to_string(self.workspace.join(relative_path))
            .unwrap()
            .replace("\r\n", "\n")
    }

    fn workspace_has(&self, relative_path: &str) -> bool {
        self.workspace.join(relative_path).exists()
    }

    fn prompt_of(&self, label: &str) -> String {
        self.provider
            .calls()
            .into_iter()
            .find(|call| call.label == label)
            .unwrap_or_else(|| panic!("no session named {label}"))
            .prompt
    }

    fn observe(&self, run_id: &str, stage: &str) -> (Observation, NativeRunView) {
        let prefix = format!("{stage}{ROW_SEPARATOR}");
        let mut observation = Observation {
            peak: 0,
            snapshots: BTreeSet::new(),
        };
        let deadline = Instant::now() + SETTLE;
        loop {
            let view = self.view(run_id);
            let live: BTreeSet<String> = view
                .stages
                .iter()
                .filter(|stage_row| {
                    stage_row.name.starts_with(&prefix)
                        && stage_row.status == NativeStageStatus::Running
                })
                .map(|stage_row| stage_row.name.clone())
                .collect();
            observation.peak = observation.peak.max(live.len());
            if !live.is_empty() {
                observation.snapshots.insert(live);
            }
            let settled = !matches!(
                view.status,
                NativeRunStatus::Pending | NativeRunStatus::Running
            );
            if settled || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
        (observation, self.finish(run_id))
    }
}

fn task_files(plan_json: &str) -> BTreeMap<String, Vec<String>> {
    let plan: Value = serde_json::from_str(plan_json).unwrap();
    plan[TASKS_KEY]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            let files = task[FILES_KEY]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file.as_str().unwrap().to_string())
                .collect();
            (task[ID_KEY].as_str().unwrap().to_string(), files)
        })
        .collect()
}

fn task_id(call: &Call) -> Option<&str> {
    call.sub_session
        .then(|| call.label.split_once(ROW_SEPARATOR).map(|(_, id)| id))
        .flatten()
}

type Adjust = dyn Fn(&Call, Behaviour) -> Behaviour + Send + Sync;

/// Each task writes its declared files and stays live for [`HOLD`]; `adjust` may change a behaviour.
fn task_behaviour(
    plans: &[&str],
    adjust: impl Fn(&Call, Behaviour) -> Behaviour + Send + Sync + 'static,
) -> impl Fn(&Call) -> Behaviour + Send + Sync + 'static {
    let files: BTreeMap<String, Vec<String>> =
        plans.iter().flat_map(|plan| task_files(plan)).collect();
    let adjust: Box<Adjust> = Box::new(adjust);
    move |call| {
        let mut behaviour = Behaviour::ok();
        if let Some(id) = task_id(call) {
            for file in files.get(id).into_iter().flatten() {
                behaviour = behaviour.write(file, &task_content(id));
            }
            behaviour = behaviour.hold(HOLD);
        }
        adjust(call, behaviour)
    }
}

fn keep_behaviour(_call: &Call, behaviour: Behaviour) -> Behaviour {
    behaviour
}

fn plan_harness(
    tag: &str,
    plan: &str,
    adjust: impl Fn(&Call, Behaviour) -> Behaviour + Send + Sync + 'static,
) -> Harness {
    Harness::new(
        tag,
        ScriptedProvider::new(Some(plan), task_behaviour(&[plan], adjust)),
    )
}

/// Serde text of a closed-set value, the form a template or a state file carries.
fn wire_text(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

fn template(max_parallel: u32, stage_max: u32, policy: ItemFailurePolicy) -> String {
    PLAN_TEMPLATE
        .replace(MAX_PARALLEL_TOKEN, &max_parallel.to_string())
        .replace(STAGE_MAX_TOKEN, &stage_max.to_string())
        .replace(POLICY_TOKEN, &wire_text(&policy))
}

fn describe(view: &NativeRunView) -> String {
    let rows: Vec<String> = view
        .stages
        .iter()
        .map(|stage_row| {
            format!(
                "{}={:?}({:?})",
                stage_row.name, stage_row.status, stage_row.reason
            )
        })
        .collect();
    format!(
        "{:?} {:?} rows: {}",
        view.status,
        view.reason,
        rows.join(", ")
    )
}

fn assert_done(view: &NativeRunView) {
    assert_eq!(view.status, NativeRunStatus::Done, "{}", describe(view));
}

fn assert_row(view: &NativeRunView, name: &str, expected: NativeStageStatus) {
    let found = view
        .stage(name)
        .unwrap_or_else(|| panic!("no row {name}: {}", describe(view)));
    assert_eq!(found.status, expected, "row {name}: {}", describe(view));
}

fn run_plan(harness: &Harness, template_text: &str) -> (Observation, NativeRunView, String) {
    let template_path = harness.write_template(template_text);
    let run_id = harness.start(&template_path);
    let (observation, view) = harness.observe(&run_id, IMPLEMENTATION);
    (observation, view, run_id)
}

fn stray_writes(call: &Call, behaviour: Behaviour) -> Behaviour {
    if task_id(call) == Some(TASK_TWO) {
        behaviour
            .write(STRAY_FILE, OUTSIDE_CONTENT)
            .write(README, TAMPERED_CONTENT)
    } else {
        behaviour
    }
}

#[test]
fn disjoint_tasks_run_together_and_conflicting_tasks_run_in_plan_order() {
    let harness = plan_harness(TAG_PARALLEL, PLAN_FOUR_TASKS, keep_behaviour);
    let (observation, view, run_id) = run_plan(
        &harness,
        &template(DEFAULT_CAP, STAGE_MAX, ItemFailurePolicy::Pause),
    );

    assert_done(&view);
    for id in FOUR_TASK_IDS {
        assert_row(&view, &row(IMPLEMENTATION, id), NativeStageStatus::Done);
    }
    assert!(observation.ever_together(&[TASK_ONE, TASK_TWO]));
    assert!(observation.ever_live(TASK_THREE) && observation.ever_live(TASK_FOUR));
    assert!(!observation.ever_together(&[TASK_THREE, TASK_FOUR]));
    let labels = harness.provider.labels();
    let position = |id: &str| {
        labels
            .iter()
            .position(|label| *label == row(IMPLEMENTATION, id))
    };
    assert!(position(TASK_THREE) < position(TASK_FOUR), "{labels:?}");
    assert_eq!(harness.read_workspace(SHARED_FILE), task_content(TASK_FOUR));

    let schedule = &harness.state_json(&run_id)[SCHEDULES_KEY][IMPLEMENTATION];
    let edges = schedule[SERIALIZATIONS_KEY].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0][BEFORE_KEY], TASK_THREE);
    assert_eq!(edges[0][AFTER_KEY], TASK_FOUR);
    assert_eq!(edges[0][FILES_KEY][0], SHARED_FILE);
    let prompt = harness.prompt_of(&row(IMPLEMENTATION, TASK_FOUR));
    assert!(prompt.contains(SERIALIZED_HEADING) && prompt.contains(SERIALIZED_LINE));
    assert!(!harness
        .prompt_of(&row(IMPLEMENTATION, TASK_ONE))
        .contains(SERIALIZED_HEADING));
}

#[test]
fn parallel_tasks_on_distinct_files_both_land_in_the_ticket_tree_and_leave_no_isolated_tree() {
    let harness = plan_harness(TAG_ISOLATION, PLAN_FOUR_TASKS, keep_behaviour);
    let (observation, view, _) = run_plan(
        &harness,
        &template(DEFAULT_CAP, STAGE_MAX, ItemFailurePolicy::Pause),
    );

    assert_done(&view);
    assert!(observation.ever_together(&[TASK_ONE, TASK_TWO]));
    assert_eq!(harness.read_workspace(FILE_A), task_content(TASK_ONE));
    assert_eq!(harness.read_workspace(FILE_B), task_content(TASK_TWO));
    let leftovers = std::fs::read_dir(view.run_dir.join(ISOLATION_FOLDER))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}

#[test]
fn an_isolated_task_writing_outside_its_files_lands_without_its_stray_writes() {
    let harness = plan_harness(TAG_SCOPE, PLAN_OUT_OF_SCOPE, stray_writes);
    let (_, view, _) = run_plan(
        &harness,
        &template(DEFAULT_CAP, STAGE_MAX, ItemFailurePolicy::Continue),
    );

    assert_done(&view);
    for task in [TASK_ONE, TASK_TWO, TASK_THREE] {
        assert_row(&view, &row(IMPLEMENTATION, task), NativeStageStatus::Done);
    }
    assert!(!harness.workspace_has(STRAY_FILE));
    assert_eq!(harness.read_workspace(README), ORIGINAL_README);
    assert_eq!(harness.read_workspace(FILE_A), task_content(TASK_ONE));
    assert_eq!(harness.read_workspace(FILE_C), task_content(TASK_THREE));
}

#[test]
fn on_item_failure_continue_lets_the_merge_see_each_task_state() {
    let harness = plan_harness(TAG_CONTINUE, PLAN_OUT_OF_SCOPE, stray_writes);
    let (_, view, _) = run_plan(
        &harness,
        &template(SEQUENTIAL_CAP, STAGE_MAX, ItemFailurePolicy::Continue),
    );

    assert_done(&view);
    assert_eq!(harness.provider.count(IMPLEMENTATION), 1);
    let merge_prompt = harness.prompt_of(IMPLEMENTATION);
    for (id, status) in [
        (TASK_ONE, NativeStageStatus::Done),
        (TASK_TWO, NativeStageStatus::Failed),
        (TASK_THREE, NativeStageStatus::Done),
    ] {
        let line = format!("`{}` {}", row(IMPLEMENTATION, id), wire_text(&status));
        assert!(merge_prompt.contains(&line), "missing {line}");
    }
}

#[test]
fn on_item_failure_pause_stops_the_run_after_the_running_tasks() {
    let harness = plan_harness(TAG_PAUSE, PLAN_OUT_OF_SCOPE, stray_writes);
    let (_, view, _) = run_plan(
        &harness,
        &template(SEQUENTIAL_CAP, STAGE_MAX, ItemFailurePolicy::Pause),
    );

    assert_eq!(view.status, NativeRunStatus::Paused, "{}", describe(&view));
    let reason = view.reason.clone().unwrap_or_default();
    assert!(reason.contains(FAILED_PAUSE_REASON), "reason: {reason}");
    assert_row(
        &view,
        &row(IMPLEMENTATION, TASK_ONE),
        NativeStageStatus::Done,
    );
    assert_row(
        &view,
        &row(IMPLEMENTATION, TASK_TWO),
        NativeStageStatus::Failed,
    );
    assert_row(
        &view,
        &row(IMPLEMENTATION, TASK_THREE),
        NativeStageStatus::Done,
    );
    assert_eq!(harness.provider.count(IMPLEMENTATION), 0);
    assert!(view
        .stages
        .iter()
        .filter(|stage_row| stage_row.name.contains(ROW_SEPARATOR))
        .all(|stage_row| stage_row.status != NativeStageStatus::Running));
}

#[test]
fn never_more_than_the_cap_of_task_sessions_are_live() {
    let cases = [
        (2, STAGE_MAX, 2),
        (DEFAULT_CAP, 1, 1),
        (DEFAULT_CAP, STAGE_MAX, 3),
    ];
    for (max_parallel, stage_max, expected_peak) in cases {
        let tag = format!("{TAG_CAP_PREFIX}-{max_parallel}-{stage_max}");
        let harness = plan_harness(&tag, PLAN_FOUR_TASKS, keep_behaviour);
        let (observation, view, _) = run_plan(
            &harness,
            &template(max_parallel, stage_max, ItemFailurePolicy::Pause),
        );

        assert_done(&view);
        assert_eq!(
            observation.peak, expected_peak,
            "max_parallel {max_parallel}, stage max {stage_max}"
        );
        assert_eq!(harness.provider.count(&row(IMPLEMENTATION, TASK_FOUR)), 1);
    }
}

fn registry_writes(call: &Call, behaviour: Behaviour) -> Behaviour {
    match (task_id(call), call.label.as_str()) {
        (Some(TASK_TWO), _) => behaviour.write(REGISTRY_FILE, CLAIMED_CONTENT),
        (None, IMPLEMENTATION) => behaviour.write(REGISTRY_FILE, MERGE_REGISTRY_CONTENT),
        _ => behaviour,
    }
}

#[test]
fn shared_files_are_written_only_by_the_merge() {
    let harness = plan_harness(TAG_SHARED, PLAN_OUT_OF_SCOPE, registry_writes);
    let (_, view, _) = run_plan(
        &harness,
        &template(SEQUENTIAL_CAP, STAGE_MAX, ItemFailurePolicy::Continue),
    );

    assert_done(&view);
    let failed = view.stage(&row(IMPLEMENTATION, TASK_TWO)).unwrap();
    assert_eq!(failed.status, NativeStageStatus::Failed);
    assert!(failed
        .reason
        .clone()
        .unwrap_or_default()
        .contains(REGISTRY_FILE));
    for id in [TASK_ONE, TASK_THREE] {
        assert_row(&view, &row(IMPLEMENTATION, id), NativeStageStatus::Done);
    }
    let merge_prompt = harness.prompt_of(IMPLEMENTATION);
    assert!(merge_prompt.contains(SHARED_SECTION) && merge_prompt.contains(REGISTRY_FILE));
    assert_eq!(
        harness.read_workspace(REGISTRY_FILE),
        MERGE_REGISTRY_CONTENT
    );
}

#[test]
fn a_correction_loop_with_two_successive_plans_gives_two_schedules() {
    let behaviour = task_behaviour(
        &[PLAN_FOUR_TASKS, PLAN_SECOND_ITERATION],
        |call, behaviour| match call.label.as_str() {
            REVIEW => behaviour.signal(BLOCKING_SIGNAL, Value::Bool(true)),
            REREVIEW => behaviour.signal(BLOCKING_SIGNAL, Value::Bool(false)),
            _ => behaviour,
        },
    );
    let provider = ScriptedProvider::with_plans(
        Some(PLAN_FOUR_TASKS),
        &[(REPLAN, PLAN_SECOND_ITERATION)],
        behaviour,
    );
    let harness = Harness::new(TAG_LOOP, provider);
    let template_path = harness.write_template(LOOP_TEMPLATE);
    let run_id = harness.start(&template_path);
    let view = harness.finish(&run_id);

    assert_done(&view);
    let schedules = harness.state_json(&run_id)[SCHEDULES_KEY].clone();
    let schedule_ids = |stage: &str| -> Vec<String> {
        schedules[stage][TASKS_KEY]
            .as_array()
            .unwrap_or_else(|| panic!("no schedule for {stage}: {schedules}"))
            .iter()
            .map(|task| task[ID_KEY].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(schedules.as_object().unwrap().len(), 2);
    assert_eq!(schedule_ids(IMPLEMENTATION), FOUR_TASK_IDS);
    assert_eq!(schedule_ids(REIMPLEMENTATION), FIX_IDS);
    for id in FIX_IDS {
        assert_row(&view, &row(REIMPLEMENTATION, id), NativeStageStatus::Done);
    }
    assert_eq!(harness.read_workspace(FIX_ONE_FILE), task_content(FIX_ONE));
    assert_eq!(harness.provider.count(REIMPLEMENTATION), 1);
}

fn stage_output(harness: &Harness, run_id: &str, file: &str) -> String {
    let path = harness.view(run_id).run_dir.join(IMPLEMENTATION).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn sorted_labels(harness: &Harness) -> Vec<String> {
    let mut labels = harness.provider.labels();
    labels.sort();
    labels
}

#[test]
fn a_parallel_run_matches_the_sequential_run_of_the_same_plan() {
    let outcome = |tag: &str, max_parallel: u32| {
        let harness = plan_harness(tag, PLAN_FOUR_TASKS, keep_behaviour);
        let (_, view, run_id) = run_plan(
            &harness,
            &template(max_parallel, STAGE_MAX, ItemFailurePolicy::Pause),
        );
        assert_done(&view);
        let files: Vec<String> = [FILE_A, FILE_B, SHARED_FILE]
            .iter()
            .map(|file| harness.read_workspace(file))
            .collect();
        (
            stage_output(&harness, &run_id, CHANGED_FILES),
            files,
            sorted_labels(&harness),
        )
    };
    let sequential = outcome(TAG_SEQUENTIAL, 1);
    let parallel = outcome(TAG_CONCURRENT, DEFAULT_CAP);

    assert_eq!(parallel.0, sequential.0);
    assert_eq!(parallel.1, sequential.1);
    assert_eq!(parallel.2, sequential.2);
}

/// Paths git reports as changed in `directory`, each with its content.
fn changed_files_in(directory: &Path) -> BTreeMap<String, String> {
    let output = Command::new(GIT)
        .args(GIT_STATUS)
        .current_dir(directory)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line[PORCELAIN_PATH_OFFSET..].to_string())
        .map(|path| {
            let content = std::fs::read_to_string(directory.join(&path)).unwrap();
            (
                path,
                content.replace(
                    "

", "
",
                ),
            )
        })
        .collect()
}

/// Emits the workflow script for the plan and runs it under node with a stub agent, returning the changed files.
fn workflow_changed_files(template_text: &str, plan_json: &str) -> BTreeMap<String, String> {
    let root = TempWorkspace::new(WORKFLOW_TAG);
    let workspace = root.child(WORKFLOW_WORKSPACE_DIR);
    let scratch = root.child(WORKFLOW_SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).unwrap();
    init_workspace(&workspace);
    let pipeline = scratch.join(WORKFLOW_PIPELINE_FILE);
    std::fs::write(&pipeline, template_text).unwrap();
    let state = run::init(
        &pipeline,
        &workspace,
        Value::Object(Default::default()),
        None,
    )
    .unwrap();
    let plan_dir = state.run_dir.join(WORKFLOW_PLAN_DIR);
    std::fs::create_dir_all(&plan_dir).unwrap();
    std::fs::write(plan_dir.join(WORKFLOW_PLAN_FILE), plan_json).unwrap();
    let script = scratch.join(WORKFLOW_SCRIPT_FILE);
    std::fs::write(&script, workflow_emit::emit(&state).unwrap()).unwrap();
    let runner = scratch.join(WORKFLOW_RUNNER_FILE);
    std::fs::write(&runner, WORKFLOW_RUNNER).unwrap();
    let output = Command::new(NODE)
        .arg(&runner)
        .arg(&script)
        .arg(&workspace)
        .arg(HOLD.as_millis().to_string())
        .output()
        .expect("node runs the emitted workflow script");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(outcome[STATUS_KEY], WORKFLOW_DONE_STATUS, "{outcome}");
    let mut changed = changed_files_in(&workspace);
    changed.retain(|path, _| !path.starts_with(RUN_ARTIFACT_DIR));
    changed
}

/// The native run leaves the same changed files as the emitted workflow script run for the same plan.
#[test]
fn the_native_changed_files_match_a_real_run_of_the_emitted_workflow() {
    let template_text = template(DEFAULT_CAP, STAGE_MAX, ItemFailurePolicy::Pause);
    let harness = plan_harness(TAG_PARITY, PLAN_FOUR_TASKS, keep_behaviour);
    let (_, view, _) = run_plan(&harness, &template_text);

    assert_done(&view);
    let native = changed_files_in(&harness.workspace);
    assert_eq!(native.len(), FOUR_TASK_FILE_COUNT);
    assert_eq!(
        native,
        workflow_changed_files(&template_text, PLAN_FOUR_TASKS)
    );
}
