//! Names, files and limits of the pipeline doctor.

use std::time::Duration;

/// Pipeline template of an investigation.
pub const DOCTOR_TEMPLATE: &str = "flow-doctor";
/// Actor the doctor's actions are journaled under.
pub const DOCTOR_ACTOR: &str = "doctor";
/// File the diagnose stage writes.
pub const DIAGNOSIS_FILE: &str = "diagnosis.json";
/// File the report stage writes: the body of the issue comment.
pub const REPORT_FILE: &str = "report.md";
/// Folder of the state dir holding the incidents and the evidence bundles.
pub const DOCTOR_DIR: &str = "doctor";
/// File of the state dir listing every incident.
pub const INCIDENTS_FILE: &str = "incidents.json";
/// Folder of one incident's bundle holding the copied evidence.
pub const EVIDENCE_DIR: &str = "evidence";
/// Index of the evidence bundle, read first by the prepare stage.
pub const EVIDENCE_INDEX_FILE: &str = "incident.json";
/// File of the bundle holding the issue comment the daemon posts.
pub const COMMENT_FILE: &str = "comment.md";
/// The daemon's error log, under the state dir.
pub const DAEMON_ERR_LOG: &str = "daemon.err.log";
/// Bytes read from the end of the daemon's error log.
pub const LOG_TAIL_BYTES: u64 = 256 * 1024;
/// Lines of the filtered log kept in the bundle.
pub const LOG_TAIL_LINES: usize = 200;
/// Log levels kept from the daemon's log even when a line names no key.
pub const LOG_LEVELS: [&str; 2] = ["ERROR", "WARN"];
/// Bus event published on every change of an incident.
pub const EVENT_DOCTOR_INCIDENT: &str = "doctor.incident";
/// Below this confidence an action that is not reversible goes to a human and does not run.
pub const MIN_CONFIDENCE: f64 = 0.7;
/// Below this confidence a reversible action (`retry_step`, a requeue) goes to a human.
pub const MIN_CONFIDENCE_REVERSIBLE: f64 = 0.5;
/// Actions the doctor takes on one incident, at most, whoever proposed them.
pub const MAX_ACTIONS_PER_INCIDENT: usize = 3;
/// Deterministic resumptions of one incident, at most (they count in the actions above).
pub const MAX_TRANSIENT_RETRIES: usize = 2;
/// Deterministic resumptions of one ticket over the sliding 24 hours, across its incidents.
pub const MAX_TRANSIENT_RETRIES_PER_TICKET: usize = 6;
/// Wait before the second resumption; it doubles at each further one.
pub const RETRY_BACKOFF_BASE_MS: i64 = 2 * 60 * 1000;
/// Minutes the doctor waits after a run ended before a step still `running` is stuck, so the
/// flow engine has time to reconcile.
pub const RUN_ENDED_GRACE_MINUTES: i64 = 3;
/// Hours a paused run outside any flow stays an incident.
pub const FRESH_RUN_HOURS: i64 = 24;
/// Templates whose runs are the art pipeline's: the doctor never treats their art findings.
pub const ART_TEMPLATE_PREFIXES: [&str; 2] = ["art-", "pixelart"];
/// Note of an incident closed without investigation because it is no longer a live failure.
pub const OBSOLETE_NOTE: &str =
    "obsolete: no longer a live failure (run replaced, flow over, or run too old)";
/// Daemon repository: its local clone is named by `AIDP_SOURCE_ROOT`.
pub const HARNESS_REPO_AIDP: &str = "Litybe/aidp";
/// Skill repository: its local clone is the skill root.
pub const HARNESS_REPO_SKILL: &str = "Litybe/aidp-skill";
/// Studio repository (web front); its clone comes from `doctor.harness_repos`.
pub const HARNESS_REPO_STUDIO: &str = "Litybe/aidp-studio";
/// Unity package repository; its clone comes from `doctor.harness_repos`.
pub const HARNESS_REPO_UNITY: &str = "Litybe/aidp-unity";
/// Repositories a harness patch may target, besides those of `doctor.harness_repos`.
pub const HARNESS_REPOS: [&str; 4] = [
    HARNESS_REPO_AIDP,
    HARNESS_REPO_SKILL,
    HARNESS_REPO_STUDIO,
    HARNESS_REPO_UNITY,
];
/// Environment variable naming the local clone of the daemon repository.
pub const ENV_SOURCE_ROOT: &str = "AIDP_SOURCE_ROOT";
/// Name of the remote whose URL must name the repository of a clone.
pub const CLONE_REMOTE: &str = "origin";
/// Host of the remote URLs a clone may have.
pub const GITHUB_HOST: &str = "github.com";
/// Parameter of `open_harness_pr` naming the `owner/name` repository.
pub const PARAM_HARNESS_REPO: &str = "repo";
/// Prefix of the branch a harness patch is pushed to.
pub const HARNESS_BRANCH_PREFIX: &str = "doctor/";
/// Branch the harness pull request is cut from and aimed at.
pub const HARNESS_BASE: &str = "main";
/// Remote the harness branch is fetched from and pushed to.
pub const HARNESS_REMOTE: &str = "origin";
/// Folder of the state dir holding the temporary worktrees of harness patches.
pub const WORKTREES_DIR: &str = "worktrees";
/// Author of the commit of a harness patch.
pub const HARNESS_AUTHOR: [&str; 4] = [
    "-c",
    "user.name=aidp-doctor",
    "-c",
    "user.email=doctor@aidp.invalid",
];
/// Branch prefix the doctor never reads from, builds on or pushes to.
pub const FORBIDDEN_BRANCH_PREFIX: &str = "gamedev/";
/// Parameter of `create_bug_ticket` carrying a one-line summary.
pub const PARAM_SUMMARY: &str = "summary";
/// Minutes a step may wait for something external before it is an incident.
pub const WAITING_EXTERNAL_MINUTES: i64 = 30;
/// Window over which the per-ticket investigation cap counts.
pub const ATTEMPT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// Interval between two passes of the doctor's thread.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);
/// Delay before the first pass, so the daemon finishes starting first.
pub const FIRST_TICK_DELAY: Duration = Duration::from_secs(30);
/// Sleep slice of the doctor's thread, so a shutdown is seen quickly.
pub const SLICE: Duration = Duration::from_millis(200);
/// Name of the doctor's thread for the thread guard.
pub const DOCTOR_LOOP: &str = "doctor";
/// Separator of an incident id: `<key>/<cause>`.
pub const INCIDENT_SEPARATOR: &str = "/";
/// Prefix of the cause of an incident on a native run outside any flow.
pub const RUN_CAUSE_PREFIX: &str = "run:";
/// Suffix of the idempotency key of an incident's action; the attempt number follows.
pub const ACTION_KEY_SUFFIX: &str = ":action";
/// Parameter of `retry_step` naming the step.
pub const PARAM_STEP: &str = "step";
/// Parameter of `update_pr_branch` naming the PR number.
pub const PARAM_PR: &str = "pr";
/// Parameter of `update_pr_branch` naming the `owner/name` repository.
pub const PARAM_REPO: &str = "repo";
/// Parameter of `release_unity_lock` naming the resource.
pub const PARAM_RESOURCE: &str = "resource";
/// Parameters that would turn a branch update into a rebase; refused.
pub const REBASE_PARAMS: [&str; 2] = ["rebase", "force"];
/// The `aidp` verb that labels a ticket.
pub const AIDP_TICKET: &str = "ticket";
/// The `aidp ticket` verb that changes the state label.
pub const AIDP_LABEL: &str = "label";
/// The `aidp ticket` verb that posts a comment.
pub const AIDP_COMMENT: &str = "comment";
/// Flag of `aidp ticket comment` naming the body file.
pub const BODY_FILE_FLAG: &str = "--body-file";
/// Machine output flag of every `aidp` verb.
pub const FORMAT_FLAG: &str = "--format";
/// Machine output format of every `aidp` verb.
pub const FORMAT_JSON: &str = "json-compact";
/// Label state that puts a ticket back into the test lane.
pub const STATE_TO_TEST: &str = "to-test";
/// Label state that puts a ticket back into the dev queue.
pub const STATE_READY: &str = "ready";
/// The GitHub CLI.
pub const GH_PROGRAM: &str = "gh";
/// `gh pr update-branch` arguments before the number; merge is the default method.
pub const GH_UPDATE_BRANCH: [&str; 2] = ["pr", "update-branch"];
/// `gh` flag naming the repository.
pub const GH_REPO_FLAG: &str = "--repo";
/// Input of an investigation naming the incident id.
pub const INPUT_INCIDENT: &str = "incident";
/// Input of an investigation naming the evidence bundle directory.
pub const INPUT_BUNDLE: &str = "bundle";
/// Input of an investigation naming the vault run folder.
pub const INPUT_FEATURE: &str = "feature";
/// Key of the effect id that holds a flow step's pipeline run: `<step>:<iteration>:start_pipeline`.
pub const START_PIPELINE_SUFFIX: &str = "start_pipeline";
/// Separator of the parts of an effect id.
pub const EFFECT_SEPARATOR: char = ':';
/// Step id of the test flows that stops on the cap of bugs.
pub const BUG_CAPPED_STEP: &str = "bug_capped_stop";
/// Separator between the iteration and the flow start time in the id of a cap incident, so a new cap of the same parent is a new incident.
pub const CAP_INCIDENT_SEPARATOR: &str = ".";
/// Prefix of the branch a feature ticket works on.
pub const PARENT_BRANCH_PREFIX: &str = "feat/";
/// Folders that hold the test infrastructure: a branch that changes them can break every test.
pub const TEST_INFRA_PATHS: [&str; 3] = ["Assets/Tests/Playtest", "Packages", "ProjectSettings"];
/// A file that shrank to under this percentage of its size on `main` is suspect.
pub const SUSPECT_SIZE_PERCENT: u64 = 70;
/// Characters of a `git diff --stat` kept in the bundle.
pub const STAT_MAX_CHARS: usize = 4000;
/// Commits listed for one bug.
pub const BUG_COMMITS_MAX: usize = 20;
/// Tickets that must carry an error for it to be a shared pattern.
pub const SHARED_PATTERN_MIN_TICKETS: usize = 2;
/// Shared patterns kept in the bundle.
pub const SHARED_PATTERNS_MAX: usize = 10;
/// Characters of a normalized error line.
pub const SIGNATURE_MAX_CHARS: usize = 120;
/// Characters under which a line is too short to be an error signature.
pub const SIGNATURE_MIN_CHARS: usize = 12;
/// Largest report read when looking for a shared pattern.
pub const REPORT_MAX_BYTES: u64 = 256 * 1024;
/// Fragment of the id of the templates that test a ticket.
pub const TEST_TEMPLATE_FRAGMENT: &str = "test";
/// Evidence file: what each bug of the parent changed.
pub const BUG_HISTORY_FILE: &str = "bug-history.json";
/// Evidence file: test-infrastructure files the branch changed against `origin/main`.
pub const TEST_INFRA_FILE: &str = "test-infra.json";
/// Evidence file: how far the branch lags behind `main` and whether it still merges.
pub const PR_LAG_FILE: &str = "pr-lag.json";
/// Evidence file: errors shared by several tickets over the last 24 hours.
pub const SHARED_PATTERNS_FILE: &str = "shared-patterns.json";
/// Parameter of `restore_test_infra` listing the files to put back.
pub const PARAM_FILES: &str = "files";
/// Parameter of `open_harness_pr` and `create_bug_ticket` naming the shared pattern.
pub const PARAM_PATTERN: &str = "pattern";
/// Parameter of a `real_defect` diagnosis naming the defect.
pub const PARAM_DEFECT: &str = "defect";
/// Parameter of a `real_defect` diagnosis naming the file the defect is in.
pub const PARAM_DEFECT_FILE: &str = "file";
/// Parameter of a `real_defect` diagnosis naming the acceptance criterion it breaks.
pub const PARAM_CRITERION: &str = "criterion";
/// Parameter of a shared failure diagnosis listing the tickets it touches.
pub const PARAM_TICKETS: &str = "tickets";
/// Commit title of a test-infrastructure restoration.
pub const RESTORE_COMMIT_PREFIX: &str =
    "fix(doctor): restaure l'infrastructure de test depuis origin/main";
/// Branches the doctor never pushes to.
pub const PROTECTED_BRANCHES: [&str; 3] = ["main", "master", "develop"];
/// Words shorter than this do not count when two causes are compared.
pub const CAUSE_WORD_MIN_CHARS: usize = 4;
/// Share of words two causes must have in common to be the same cause, in percent.
pub const SAME_CAUSE_PERCENT: usize = 50;
/// Placeholder when a `real_defect` summary lacks a field.
pub const UNKNOWN_FIELD: &str = "inconnu";
/// Times the Unity compile gate wait and its heartbeat tolerance are multiplied by after `extend_unity_wait`.
pub const UNITY_WAIT_MULTIPLIER: u32 = 3;
/// Heartbeat silence, in seconds, past which a Unity editor counts as blocked when its status gives no limit.
pub const UNITY_HEARTBEAT_STALE_SECONDS: i64 = 300;
/// Key of the editor object in the Unity status document.
pub const UNITY_KEY_EDITOR: &str = "editor";
/// Key of the running flag in the editor object.
pub const UNITY_KEY_RUNNING: &str = "running";
/// Key of the heartbeat in the editor object.
pub const UNITY_KEY_HEARTBEAT: &str = "heartbeat";
/// Key of the heartbeat time in the heartbeat object.
pub const UNITY_KEY_HEARTBEAT_AT: &str = "at";
/// Key of the silence limit the status gives, in seconds.
pub const UNITY_KEY_STALE_AFTER: &str = "staleAfterSeconds";
/// Fragments of a step id or action name that make a step a report or verdict step.
pub const VERDICT_STEP_FRAGMENTS: [&str; 2] = ["report", "verdict"];
/// Parameter of `mark_step_passed` telling the recheck of the evidence is clean.
pub const PARAM_RECHECK: &str = "recheck";
/// Value of `params.recheck` for a clean recheck.
pub const RECHECK_CLEAN: &str = "clean";
/// Parameter of `retry_from_stage` naming the stage.
pub const PARAM_STAGE: &str = "stage";
/// Value of the live PR status `mergeable` field for a pull request that merges.
pub const PR_MERGEABLE: &str = "mergeable";
/// Key of the mergeable state in the live PR status.
pub const PR_KEY_MERGEABLE: &str = "mergeable";
/// Note a `nudge_budget` detail carries when it lifted the ceiling of the ticket.
pub const BUDGET_RAISED_NOTE: &str = "ceiling raised";
/// Extension of a Unity metadata file.
pub const META_EXTENSION: &str = ".meta";
/// Prefix of the branch `rebuild_branch` pushes to.
pub const REBUILD_BRANCH_PREFIX: &str = "doctor/rebuild/";
/// Commit title of a `.meta` conflict resolution.
pub const META_MERGE_COMMIT_PREFIX: &str =
    "fix(doctor): fusionne origin/main, .meta de main gardes";
/// Milliseconds in an hour.
pub const MILLIS_PER_HOUR: i64 = 60 * 60 * 1000;
/// Wait before the second investigation of an incident whose first one failed.
pub const INVESTIGATION_RETRY_DELAY_MS: i64 = 15 * 60 * 1000;
/// Investigations that may fail for one incident before it is final.
pub const MAX_INVESTIGATION_FAILURES: u32 = 2;
/// Status code letter of a path both sides added, in `git status --porcelain`.
pub const GIT_UNMERGED_BOTH_ADDED: &str = "AA";
/// Prefix of a `git status --porcelain` line for any unmerged path.
pub const GIT_UNMERGED_MARKERS: [&str; 7] = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"];
/// Why a modal window cannot be closed: no channel reaches the editor's main thread.
pub const NO_MODAL_CHANNEL: &str =
    "no channel closes a Unity modal window: the playtest package cannot run while the editor's main thread is blocked";
/// Commits `rebuild_branch` replays at most.
pub const REBUILD_MAX_COMMITS: usize = 50;
