//! Keys read from a Workflow result object and its stage envelopes.

/// Result key holding the pass status.
pub const STATUS_KEY: &str = "status";
/// Result key mapping stage names to their envelopes.
pub const RESULTS_KEY: &str = "results";
/// Result key listing the guard findings.
pub const GUARD_FINDINGS_KEY: &str = "guard_findings";
/// Envelope key holding the metrics object.
pub const METRICS_KEY: &str = "metrics";
/// Metrics key holding the token count.
pub const TOKENS_USED_KEY: &str = "tokens_used";
/// Metrics key holding the duration in milliseconds.
pub const DURATION_MS_KEY: &str = "duration_ms";
/// Envelope key mapping output names to paths.
pub const OUTPUTS_KEY: &str = "outputs";
/// Envelope key holding the summary text.
pub const SUMMARY_KEY: &str = "summary";
/// Envelope key listing the follow-ups.
pub const FOLLOW_UPS_KEY: &str = "follow_ups";
