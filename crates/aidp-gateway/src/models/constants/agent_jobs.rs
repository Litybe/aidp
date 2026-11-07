//! Limits, wire names and messages of the agent jobs a host runs for an external run.

use super::launch_policy::HEADLESS_SESSION_ENV;
use super::session_profile::AUTONOMOUS_ENV;
use std::time::Duration;

pub const ENV_RUN: &str = "AIDP_RUN";
pub const ENV_STAGE: &str = "AIDP_STAGE";
pub const ENV_HEADLESS: &str = HEADLESS_SESSION_ENV;
pub const ENV_HEADLESS_ENABLED: &str = "1";
/// The only variables a job may carry: tokens never travel in a job.
pub const ENV_ALLOWLIST: [&str; 4] = [ENV_RUN, ENV_STAGE, AUTONOMOUS_ENV, ENV_HEADLESS];

pub const AGENT_JOBS_DIR: &str = "agent-jobs";
pub const JOB_FILE_EXTENSION: &str = "json";
pub const ID_PREFIX: &str = "agent";
pub const ID_SEPARATOR: &str = ":";

pub const LEASE_SECONDS: u64 = 120;
pub const WAKE_SLICE: Duration = Duration::from_secs(5);
pub const CLAIM_TIMEOUT_MS: i64 = 30 * 60 * 1000;
pub const JOB_TIMEOUT_MINUTES: u32 = 90;
pub const SESSION_GRACE_MS: i64 = 10 * 60 * 1000;
pub const MILLIS_PER_MINUTE: i64 = 60 * 1000;
pub const MILLIS_PER_SECOND: i64 = 1000;

pub const INPUT_VALUE_CAP: usize = 1000;

pub const REPAIR_HEADING: &str = "\n\n## Reply repair\n\n";
pub const RESUME_HEADING: &str = "\n\n## Resumption\n\n";
pub const RESUME_INSTRUCTION: &str = "A previous session on this stage was interrupted. Inspect the stage directory and the workspace, keep what is already done and finish the rest. Do not start over.";

pub const REPLY_SCHEMA_NAME: &str = "agent-reply.v4";
pub const REPLY_LOCATION_FILE: &str = "file";
/// Folder a host writes its session log and transcript to, under the run workspace.
pub const HOST_SESSION_DIR: &str = ".ai-assistant";

pub const NEXT_SETTLED: &str = "settled";
pub const NEXT_REPAIR: &str = "repair";
pub const NEXT_IGNORED: &str = "ignored";

pub const REASON_SESSION_FAILED: &str = "session failed";
pub const REASON_SESSION_TIMED_OUT: &str = "session timed out";
pub const REASON_SESSION_RATE_LIMITED: &str = "session rate limited";
pub const REASON_REPLY_REJECTED: &str = "reply rejected after one repair";
pub const REASON_NO_HOST: &str = "no host claimed the job";
pub const REASON_HOST_TIMEOUT: &str = "host session exceeded its timeout";
pub const REASON_INTERRUPTED_TWICE: &str = "host session interrupted twice";
pub const REASON_RUN_CANCELLED: &str = "the run was cancelled";
pub const REASON_STALE_JOB: &str = "the job record lost its repair job";
pub const REASON_UNKNOWN_OUTCOME: &str = "unknown outcome";
pub const RETRY_AFTER_NOTE: &str = "retry after";
pub const RETRY_AFTER_UNIT: &str = "s";

pub const ERROR_UNKNOWN_JOB: &str = "no agent job";
pub const ERROR_ALREADY_CLAIMED: &str = "the agent job is already claimed";
pub const ERROR_NOT_PENDING: &str = "the agent job is not pending";
pub const ERROR_LEASE_LOST: &str = "the lease on the agent job is lost";
pub const ERROR_WRONG_TOKEN: &str = "wrong claim token for the agent job";
pub const ERROR_DIFFERENT_RESULT: &str = "a different result was already posted for the agent job";
pub const ERROR_NOT_EXTERNAL: &str = "the run does not use external agents";
pub const ERROR_FOREIGN_JOB: &str = "the agent job belongs to another run";
pub const ERROR_MALFORMED_ID: &str = "malformed agent job id";
