//! Names and limits of the per-ticket cost report.

/// `groupBy` values served by the ticket-aware usage report, besides the plain ones.
pub const GROUP_TICKET: &str = "ticket";
pub const GROUP_MODEL: &str = "model";
pub const GROUP_STAGE: &str = "stage";
pub const GROUP_ROUTE: &str = "route";
/// Bucket key of a record the grouping cannot place.
pub const UNKNOWN_KEY: &str = "unknown";
/// Bucket key of the total row.
pub const TOTAL_KEY: &str = "total";
/// Length of the 5-hour rate-limit window, in seconds.
pub const FIVE_HOUR_SECONDS: i64 = 5 * 3600;
/// Length of the 7-day rate-limit window, in seconds.
pub const SEVEN_DAY_SECONDS: i64 = 7 * 24 * 3600;
/// Directory of the pipeline runs inside the vault.
pub const PIPELINES_DIR: &str = "Pipelines";
/// Run directories read per ticket, newest first.
pub const MAX_RUNS_PER_TICKET: usize = 100;
/// Tickets needed before an anomaly can be reported.
pub const ANOMALY_MIN_POINTS: usize = 3;
/// Anomaly report statuses.
pub const ANOMALY_OK: &str = "ok";
pub const ANOMALY_INSUFFICIENT: &str = "insufficient_data";
/// Footer label, unit words and separators (French, as the reports are).
pub const FOOTER_PREFIX: &str = "Coût : ";
pub const FOOTER_UNIT_MILLION: &str = " M";
pub const FOOTER_UNIT_THOUSAND: &str = " k";
pub const FOOTER_WINDOW_SUFFIX: &str = " % de la fenêtre 5 h";
/// Tokens in a million, in a thousand.
pub const MILLION: u64 = 1_000_000;
pub const THOUSAND: u64 = 1_000;
