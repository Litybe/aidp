//! Wire names, limits and messages of the external executor.

pub const OUTCOME_OK: &str = "ok";
pub const OUTCOME_SKIPPED: &str = "skipped";
pub const OUTCOME_WAITING_EXTERNAL: &str = "waiting_external";
pub const OUTCOME_TECHNICAL_ERROR: &str = "technical_error";
pub const OUTCOME_BUSINESS_ERROR: &str = "business_error";
pub const OUTCOME_SATISFIED: &str = "satisfied";
pub const OUTCOME_PENDING: &str = "pending";

/// Results kept to answer a replayed post.
pub const MAX_RESULT_DIGESTS: usize = 64;

pub const DEFAULT_WAITING_REASON: &str = "waiting for an external fact";
pub const DEFAULT_ERROR_REASON: &str = "external action failed";

pub const UNKNOWN_OUTCOME: &str = "unknown outcome";
pub const SKIPPED_NEEDS_REASON: &str = "outcome `skipped` needs a reason";
pub const OUTPUTS_NOT_OBJECT: &str = "outputs must be a JSON object";
pub const NOT_EXTERNAL: &str = "the run does not use the external executor";
pub const FLOW_NOT_ACTIVE: &str = "the run is paused, aborted or finished";
pub const KIND_MISMATCH: &str = "the outcome does not fit the kind of the job";
pub const NOT_PENDING: &str = "is not pending";
pub const UNKNOWN_JOB: &str = "no external job";
/// Prefix of the id of an agent job, which routes a result to the agent board.
pub const AGENT_ID_PREFIX: &str = "agent:";

/// `with` key a `jira.transition` job carries when the vault maps its target.
pub const WITH_STATUS: &str = "status";
pub const WITH_TO: &str = "to";

pub const DUPLICATE_KEY: &str = "duplicate";
