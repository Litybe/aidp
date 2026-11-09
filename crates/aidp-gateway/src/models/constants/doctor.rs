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
/// Below this confidence the diagnosis goes to a human and no action runs.
pub const MIN_CONFIDENCE: f64 = 0.7;
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
/// Suffix of the idempotency key of an incident's action.
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
