use super::*;
use crate::models::native_run_start::StartNativeRun;
use aidp_core::process_runner::{OsProcessRunner, ProcessSpec};
use aidp_core::run::{RunState, StageState};
use serde_json::json;

const BLOCKED_REPLY_WITH_ASSUMPTION: &str =
    include_str!("../fixtures/autonomy/blocked_reply_with_assumption.json");
const WORKER_ERROR_TEXT: &str = "no session";
const WORKER_PANIC_TEXT: &str = "worker exploded";
const START_PREFIX: &str = "start-";
const END_PREFIX: &str = "end-";
const SERIALIZATION_LINE: &str = "`A` runs before `B`: both write src/lib.rs";
const STAGE_REVIEW: &str = "review";
const STAGE_PLAN: &str = "plan";
const CONDITION_TEXT: &str = "stages.collect.outputs.report.ok";
const EMPTY_SETTINGS_YAML: &str = "max_correction_iterations: 2";
const PARALLEL_SETTINGS_YAML: &str = "max_parallel: 5";
const ZERO_PARALLEL_SETTINGS_YAML: &str = "max_parallel: 0";

fn gates(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn items(count: usize) -> Vec<String> {
    (1..=count).map(|index| format!("item-{index}")).collect()
}

fn settings(yaml: &str) -> Settings {
    serde_yaml::from_str(yaml).unwrap()
}

fn store_with_stage(dir: &std::path::Path, stage: &str) -> RunStore {
    let state = RunState {
        run_id: "run-1".to_string(),
        run_dir: dir.to_path_buf(),
        workspace_root: dir.to_path_buf(),
        stages: vec![StageState {
            name: stage.to_string(),
            stage_dir: dir.join(stage),
            ..StageState::default()
        }],
        ..RunState::default()
    };
    RunStore::create(state, &StartNativeRun::default(), dir.to_path_buf()).unwrap()
}

#[test]
fn false_condition_skips_the_stage_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_stage(dir.path(), STAGE_REVIEW);
    let reason = when_skip_reason(CONDITION_TEXT);
    store.skip(STAGE_REVIEW, &reason).unwrap();
    let row = store.row(STAGE_REVIEW).unwrap();
    assert_eq!(row.status, NativeStageStatus::Skipped);
    assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
    assert!(reason.contains(CONDITION_TEXT));
}

#[test]
fn a_settled_row_is_not_run_again() {
    assert!(NativeStageStatus::Done.is_settled());
    assert!(NativeStageStatus::Skipped.is_settled());
    assert!(!NativeStageStatus::Failed.is_settled());
    assert!(!NativeStageStatus::Running.is_settled());
}

#[test]
fn loop_stops_when_its_condition_clears() {
    assert!(loop_continues(true, 0, 2));
    assert!(!loop_continues(false, 0, 2));
}

#[test]
fn loop_stops_at_max_iterations_even_when_the_condition_holds() {
    assert!(loop_continues(true, 1, 2));
    assert!(!loop_continues(true, 2, 2));
}

#[test]
fn reviewer_needs_review_does_not_pause() {
    assert!(!pauses_run(EnvelopeStatus::NeedsReview, true));
}

#[test]
fn needs_review_pauses_a_stage_that_is_not_soft() {
    assert!(pauses_run(EnvelopeStatus::NeedsReview, false));
}

#[test]
fn blocked_and_failed_replies_pause_even_a_soft_stage() {
    assert!(pauses_run(EnvelopeStatus::Blocked, true));
    assert!(pauses_run(EnvelopeStatus::Failed, true));
    assert!(!pauses_run(EnvelopeStatus::Completed, false));
}

#[test]
fn gate_pauses_unless_approved_or_auto_approved() {
    assert!(gate_blocks(true, false, STAGE_PLAN, &[]));
    assert!(!gate_blocks(true, true, STAGE_PLAN, &[]));
    assert!(!gate_blocks(true, false, STAGE_PLAN, &gates(&[STAGE_PLAN])));
    assert!(!gate_blocks(true, false, STAGE_PLAN, &gates(&["*"])));
    assert!(gate_blocks(
        true,
        false,
        STAGE_PLAN,
        &gates(&[STAGE_REVIEW])
    ));
    assert!(!gate_blocks(false, false, STAGE_PLAN, &[]));
}

#[test]
fn signal_is_set_reads_booleans_numbers_and_text() {
    assert!(signal_is_set(Some(&json!(true))));
    assert!(signal_is_set(Some(&json!(2))));
    assert!(!signal_is_set(Some(&json!(0))));
    assert!(!signal_is_set(Some(&json!(false))));
    assert!(!signal_is_set(None));
}

#[test]
fn shards_fold_the_overflow_into_the_last_one() {
    let shards = shard_labels(items(5), 3, true);
    assert_eq!(shards.len(), 3);
    assert_eq!(shards[2], gates(&["item-3", "item-4", "item-5"]));
}

#[test]
fn shards_drop_the_overflow_when_not_folding() {
    let shards = shard_labels(items(5), 3, false);
    assert_eq!(shards.len(), 3);
    assert_eq!(shards[2], gates(&["item-3"]));
}

#[test]
fn few_items_make_one_shard_each() {
    assert_eq!(shard_labels(items(2), 4, true).len(), 2);
    assert!(shard_labels(Vec::new(), 4, true).is_empty());
}

#[test]
fn parallel_cap_defaults_to_three_and_is_at_least_one() {
    assert_eq!(max_parallel(&settings(EMPTY_SETTINGS_YAML)), 3);
    assert_eq!(max_parallel(&settings(PARALLEL_SETTINGS_YAML)), 5);
    assert_eq!(max_parallel(&settings(ZERO_PARALLEL_SETTINGS_YAML)), 1);
}

#[test]
fn sub_rows_are_named_after_their_stage() {
    assert_eq!(row_name("implement", "T1"), "implement#T1");
}

#[test]
fn an_interrupted_row_restarts_once_then_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_stage(dir.path(), STAGE_PLAN);
    store.begin(STAGE_PLAN).unwrap();
    assert_eq!(
        store.prepare_resume().unwrap(),
        vec![STAGE_PLAN.to_string()]
    );
    let row = store.row(STAGE_PLAN).unwrap();
    assert!(row.resumed);
    assert_eq!(row.status, NativeStageStatus::Pending);
    store.begin(STAGE_PLAN).unwrap();
    assert!(store.prepare_resume().unwrap().is_empty());
    assert_eq!(
        store.row_status(STAGE_PLAN),
        Some(NativeStageStatus::Failed)
    );
}

const TASK_A: &str = "A";
const TASK_B: &str = "B";
const TASK_C: &str = "C";
const TASK_D: &str = "D";
const LIB_PATH: &str = "src/lib.rs";
const OTHER_PATH: &str = "src/other.rs";
const TASK_HOLD: Duration = Duration::from_millis(25);
const DEADLOCK_GUARD: Duration = Duration::from_secs(20);

fn scheduled(id: &str, deps: &[&str], mutex_after: &[&str]) -> ScheduledTask {
    ScheduledTask {
        task: aidp_core::plan::Task {
            id: id.to_string(),
            ..aidp_core::plan::Task::default()
        },
        deps: deps.iter().map(|dep| dep.to_string()).collect(),
        mutex_after: mutex_after.iter().map(|dep| dep.to_string()).collect(),
        wave: 0,
    }
}

fn never_cancelled() -> bool {
    false
}

fn clean_outcome(_: &ScheduledTask) -> anyhow::Result<TaskOutcome> {
    Ok(TaskOutcome::succeeded(None, Vec::new()))
}

fn outcome_ids(tasks: &[ScheduledTask], outcomes: &[(usize, TaskOutcome)]) -> Vec<String> {
    outcomes
        .iter()
        .map(|(index, _)| tasks[*index].task.id.clone())
        .collect()
}

#[test]
fn plan_cap_is_the_smaller_of_stage_max_and_max_parallel() {
    assert_eq!(plan_cap(5, 2), 2);
    assert_eq!(plan_cap(2, 5), 2);
    assert_eq!(plan_cap(0, 3), 1);
    assert_eq!(plan_cap(4, 0), 1);
}

#[test]
fn plan_slots_never_exceed_the_cap_under_contention() {
    let ids: Vec<String> = (0..12).map(|index| format!("T{index}")).collect();
    let tasks: Vec<ScheduledTask> = ids.iter().map(|id| scheduled(id, &[], &[])).collect();
    let slots = PlanSlots::new(3, Vec::new());
    let running = std::sync::atomic::AtomicUsize::new(0);
    let peak = std::sync::atomic::AtomicUsize::new(0);
    let worker = |_: &ScheduledTask| {
        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(TASK_HOLD);
        running.fetch_sub(1, Ordering::SeqCst);
        Ok(TaskOutcome::succeeded(None, Vec::new()))
    };
    let (outcomes, error) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert!(error.is_none());
    assert_eq!(outcomes.len(), 12);
    assert!(outcomes.iter().all(|(_, outcome)| outcome.is_success()));
    let observed = peak.load(Ordering::SeqCst);
    assert!(observed <= 3, "peak {observed} above the cap");
    assert!(observed >= 2, "tasks never ran together");
}

#[test]
fn a_failed_dependency_skips_the_chain_without_deadlock() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[TASK_A], &[]),
        scheduled(TASK_C, &[TASK_B], &[]),
        scheduled(TASK_D, &[], &[]),
    ];
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let slots = PlanSlots::new(2, Vec::new());
        let worker = |scheduled: &ScheduledTask| {
            if scheduled.task.id == TASK_A {
                Ok(TaskOutcome::failed(NativeReason::TaskFailed))
            } else {
                clean_outcome(scheduled)
            }
        };
        let (outcomes, error) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
        let ran = outcome_ids(&tasks, &outcomes);
        let _ = sender.send((ran, slots.skipped_ids(), error.is_none()));
    });
    let (ran, skipped, clean) = receiver
        .recv_timeout(DEADLOCK_GUARD)
        .expect("scheduler hung");
    assert!(clean);
    assert_eq!(ran, vec![TASK_A.to_string(), TASK_D.to_string()]);
    assert_eq!(skipped, vec![TASK_B.to_string(), TASK_C.to_string()]);
}

#[test]
fn a_worker_panic_becomes_a_failed_task() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[TASK_A], &[]),
    ];
    let slots = PlanSlots::new(2, Vec::new());
    let worker =
        |_: &ScheduledTask| -> anyhow::Result<TaskOutcome> { panic!("{WORKER_PANIC_TEXT}") };
    let (outcomes, error) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert!(error.is_none());
    assert_eq!(outcomes.len(), 1);
    let outcome = &outcomes[0].1;
    assert_eq!(outcome.status, TaskStatus::Failed);
    assert_eq!(outcome.reason, Some(NativeReason::TaskPanicked));
    assert_eq!(slots.skipped_ids(), vec![TASK_B.to_string()]);
}

#[test]
fn a_worker_error_fails_the_task_and_is_reported() {
    let tasks = vec![scheduled(TASK_A, &[], &[])];
    let slots = PlanSlots::new(1, Vec::new());
    let worker =
        |_: &ScheduledTask| -> anyhow::Result<TaskOutcome> { anyhow::bail!("{WORKER_ERROR_TEXT}") };
    let (outcomes, error) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert_eq!(outcomes[0].1.status, TaskStatus::Failed);
    assert_eq!(error.unwrap().to_string(), WORKER_ERROR_TEXT);
}

#[test]
fn a_serialized_task_waits_for_its_predecessor_even_when_it_failed() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[], &[TASK_A]),
    ];
    let slots = PlanSlots::new(3, Vec::new());
    let order = Mutex::new(Vec::new());
    let worker = |scheduled: &ScheduledTask| {
        order
            .lock()
            .unwrap()
            .push(format!("{START_PREFIX}{}", scheduled.task.id));
        std::thread::sleep(TASK_HOLD);
        order
            .lock()
            .unwrap()
            .push(format!("{END_PREFIX}{}", scheduled.task.id));
        if scheduled.task.id == TASK_A {
            Ok(TaskOutcome::failed(NativeReason::TaskFailed))
        } else {
            clean_outcome(scheduled)
        }
    };
    let (outcomes, _) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert_eq!(outcomes.len(), 2);
    let order = order.into_inner().unwrap();
    let expected: Vec<String> = [
        (START_PREFIX, TASK_A),
        (END_PREFIX, TASK_A),
        (START_PREFIX, TASK_B),
        (END_PREFIX, TASK_B),
    ]
    .iter()
    .map(|(prefix, task)| format!("{prefix}{task}"))
    .collect();
    assert_eq!(order, expected);
}

#[test]
fn halting_on_failure_stops_launching_new_tasks() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[], &[]),
        scheduled(TASK_C, &[], &[]),
    ];
    let slots = PlanSlots::new(1, Vec::new());
    let worker = |_: &ScheduledTask| Ok(TaskOutcome::failed(NativeReason::TaskFailed));
    let (outcomes, _) = drive_tasks(&slots, &tasks, &never_cancelled, true, &worker);
    assert_eq!(outcome_ids(&tasks, &outcomes), vec![TASK_A.to_string()]);
}

#[test]
fn a_cancel_stops_launching_new_tasks() {
    let tasks = vec![scheduled(TASK_A, &[], &[]), scheduled(TASK_B, &[], &[])];
    let slots = PlanSlots::new(1, Vec::new());
    let (outcomes, _) = drive_tasks(&slots, &tasks, &|| true, false, &clean_outcome);
    assert!(outcomes.is_empty());
}

#[test]
fn rows_done_before_a_resume_are_not_run_again() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[TASK_A], &[]),
    ];
    let slots = PlanSlots::new(2, vec![(TASK_A.to_string(), TaskStatus::Success)]);
    let (outcomes, _) = drive_tasks(&slots, &tasks, &never_cancelled, false, &clean_outcome);
    assert_eq!(outcome_ids(&tasks, &outcomes), vec![TASK_B.to_string()]);
}

#[test]
fn tasks_that_overlapped_and_wrote_the_same_path_both_fail() {
    let tasks = vec![scheduled(TASK_A, &[], &[]), scheduled(TASK_B, &[], &[])];
    let slots = PlanSlots::new(2, Vec::new());
    let barrier = std::sync::Barrier::new(2);
    let worker = |_: &ScheduledTask| {
        barrier.wait();
        Ok(TaskOutcome::succeeded(None, vec![LIB_PATH.to_string()]))
    };
    let (mut outcomes, _) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    let restore = flag_overlaps(&slots, &tasks, &mut outcomes);
    assert_eq!(restore, vec![LIB_PATH.to_string()]);
    assert!(outcomes.iter().all(|(_, outcome)| {
        outcome.status == TaskStatus::Failed
            && outcome.reason == Some(NativeReason::OverlappingWrites)
            && outcome.violations == vec![LIB_PATH.to_string()]
    }));
}

#[test]
fn tasks_that_never_overlapped_keep_their_success() {
    let tasks = vec![
        scheduled(TASK_A, &[], &[]),
        scheduled(TASK_B, &[TASK_A], &[]),
    ];
    let slots = PlanSlots::new(2, Vec::new());
    let worker = |_: &ScheduledTask| Ok(TaskOutcome::succeeded(None, vec![LIB_PATH.to_string()]));
    let (mut outcomes, _) = drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert!(flag_overlaps(&slots, &tasks, &mut outcomes).is_empty());
    assert!(outcomes.iter().all(|(_, outcome)| outcome.is_success()));
}

#[test]
fn the_task_section_names_the_serializations_that_concern_it() {
    let task = scheduled(TASK_B, &[], &[TASK_A]);
    let edges = vec![
        Serialization {
            before: TASK_A.to_string(),
            after: TASK_B.to_string(),
            files: vec![LIB_PATH.to_string()],
        },
        Serialization {
            before: TASK_C.to_string(),
            after: TASK_D.to_string(),
            files: vec![OTHER_PATH.to_string()],
        },
    ];
    let text = task_section(&task, &[], &edges, false);
    assert!(text.contains(SERIALIZATION_LINE));
    assert!(!text.contains(OTHER_PATH));
}

const BLOCKED_TWICE_VALUE: &str = "blocked-twice";

/// A stage blocked twice is failed with the stable blocked-twice reason on its row.
#[test]
fn blocked_twice_fails_the_stage_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_stage(dir.path(), STAGE_PLAN);
    store.begin(STAGE_PLAN).unwrap();
    store
        .fail(STAGE_PLAN, NativeReason::BlockedTwice.as_str())
        .unwrap();
    let row = store.row(STAGE_PLAN).unwrap();
    assert_eq!(row.status, NativeStageStatus::Failed);
    assert_eq!(row.reason.as_deref(), Some(BLOCKED_TWICE_VALUE));
}

/// The first blocked reply's own assumptions are journaled with the auto-answers.
#[test]
fn first_reply_assumptions_are_journaled_on_a_block() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_stage(dir.path(), STAGE_PLAN);
    let envelope: NativeReplyEnvelope =
        serde_json::from_str(BLOCKED_REPLY_WITH_ASSUMPTION).unwrap();
    let answers = auto_answers(&envelope.ask_user_questions);
    let journal = blocked_journal(STAGE_PLAN, &envelope, answers);
    store.add_assumptions(STAGE_PLAN, journal).unwrap();
    let recorded = store.view().assumptions;
    assert_eq!(recorded.len(), 2);
    assert!(!recorded[0].auto_answered);
    assert!(recorded[1].auto_answered);
}

fn owning(id: &str, file: &str) -> ScheduledTask {
    let mut task = scheduled(id, &[], &[]);
    task.task.files = vec![file.to_string()];
    task
}

#[test]
fn an_isolated_task_gets_no_peer_file_allowance() {
    let tasks = vec![owning(TASK_A, "src/a.rs"), owning(TASK_B, "src/b.rs")];
    let schedule = Schedule {
        tasks: tasks.clone(),
        waves: Vec::new(),
        serializations: Vec::new(),
        warnings: Vec::new(),
        max_parallel: 2,
        width: 2,
        shared_files: Vec::new(),
    };
    let slots = PlanSlots::new(2, Vec::new());
    let barrier = std::sync::Barrier::new(2);
    let worker = |_: &ScheduledTask| {
        barrier.wait();
        Ok(TaskOutcome::succeeded(None, Vec::new()))
    };
    drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    assert!(peer_files(&tasks[0], &schedule, &slots, true).is_empty());
    assert_eq!(
        peer_files(&tasks[0], &schedule, &slots, false),
        vec!["src/b.rs".to_string()]
    );
}

const CONFLICT_STDERR: &str = "error: src/a.rs: patch does not apply\n";

/// Answers `git diff` with a patch and fails `git apply` with `apply_stderr` when it is set.
struct ApplyRunner {
    apply_stderr: Option<&'static str>,
    fail_add: bool,
}

impl ProcessRunner for ApplyRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        let line = spec.args.join(" ");
        let (code, stdout, stderr) = if line.contains(" add ") && self.fail_add {
            (1, "", "fatal: no index")
        } else if line.contains(" apply ") && self.apply_stderr.is_some() {
            (1, "", self.apply_stderr.unwrap_or_default())
        } else if line.contains(" diff ") {
            (0, "diff --git a/src/a.rs b/src/a.rs\n", "")
        } else {
            (0, "", "")
        };
        Ok(std::process::Output {
            status: exit_code(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn spawn(
        &self,
        _spec: &ProcessSpec,
    ) -> std::io::Result<Box<dyn aidp_core::process_runner::SpawnedChild>> {
        Err(std::io::Error::other("not scripted"))
    }
}

#[cfg(windows)]
fn exit_code(code: u32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

#[cfg(unix)]
fn exit_code(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

fn apply_outcome(runner: &ApplyRunner) -> TaskOutcome {
    let dir = tempfile::tempdir().unwrap();
    let tree = TaskWorktree {
        path: dir.path().join("iso"),
        base: "base".to_string(),
    };
    std::fs::create_dir_all(&tree.path).unwrap();
    let mut outcome = TaskOutcome::succeeded(Some(7), vec!["src/a.rs".to_string()]);
    apply_task(dir.path(), &tree, &mut outcome, runner);
    outcome
}

#[test]
fn an_apply_conflict_fails_the_task_with_the_conflicting_paths() {
    let outcome = apply_outcome(&ApplyRunner {
        apply_stderr: Some(CONFLICT_STDERR),
        fail_add: false,
    });
    assert_eq!(outcome.status, TaskStatus::Failed);
    assert_eq!(outcome.reason, Some(NativeReason::ApplyConflict));
    assert_eq!(outcome.violations, vec!["src/a.rs".to_string()]);
}

#[test]
fn a_clean_apply_keeps_the_task_successful() {
    let outcome = apply_outcome(&ApplyRunner {
        apply_stderr: None,
        fail_add: false,
    });
    assert!(outcome.is_success());
    assert_eq!(outcome.written, vec!["src/a.rs".to_string()]);
}

#[test]
fn a_git_failure_while_applying_back_fails_the_task() {
    let outcome = apply_outcome(&ApplyRunner {
        apply_stderr: None,
        fail_add: true,
    });
    assert_eq!(outcome.reason, Some(NativeReason::TaskFailed));
}

fn git_in(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn verified_peer_write(isolated: bool) -> (TaskOutcome, bool) {
    let dir = tempfile::tempdir().unwrap();
    git_in(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("seed.txt"), "seed").unwrap();
    git_in(dir.path(), &["add", "-A"]);
    git_in(dir.path(), &["commit", "-q", "-m", "seed"]);
    let tasks = vec![owning(TASK_A, "src/a.rs"), owning(TASK_B, "src/b.rs")];
    let schedule = Schedule {
        tasks: tasks.clone(),
        waves: Vec::new(),
        serializations: Vec::new(),
        warnings: Vec::new(),
        max_parallel: 2,
        width: 2,
        shared_files: Vec::new(),
    };
    let slots = PlanSlots::new(2, Vec::new());
    let barrier = std::sync::Barrier::new(2);
    let worker = |_: &ScheduledTask| {
        barrier.wait();
        Ok(TaskOutcome::succeeded(None, Vec::new()))
    };
    drive_tasks(&slots, &tasks, &never_cancelled, false, &worker);
    let baseline = scope_guard::capture_task_baseline(dir.path(), &OsProcessRunner, None);
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/b.rs"), "peer file").unwrap();
    let mut outcome = TaskOutcome::succeeded(None, Vec::new());
    verify_task_scope(
        &OsProcessRunner,
        dir.path(),
        &tasks[0],
        &schedule,
        &slots,
        &baseline,
        isolated,
        &mut outcome,
    )
    .unwrap();
    (outcome, dir.path().join("src/b.rs").exists())
}

#[test]
fn an_isolated_task_writing_outside_its_files_drops_the_write_and_succeeds() {
    let (outcome, still_there) = verified_peer_write(true);
    assert!(outcome.is_success());
    assert!(outcome.written.is_empty());
    assert!(!still_there);
}

#[test]
fn a_shared_tree_task_may_touch_a_peer_file() {
    let (outcome, _) = verified_peer_write(false);
    assert!(outcome.is_success());
    assert!(outcome.written.is_empty());
}
