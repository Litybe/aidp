//! Names and limits of the Pixellab usage aggregation.

/// File an art stage writes in its stage directory.
pub const USAGE_FILE: &str = "pixellab-usage.json";
/// Directory of the pipeline runs inside the vault.
pub const PIPELINES_DIR: &str = "Pipelines";
/// Run directories read per ticket, newest first.
pub const MAX_RUNS_PER_TICKET: usize = 100;
/// Directory levels searched below a run directory.
pub const MAX_STAGE_DEPTH: usize = 3;
/// Keys of a raw `get_balance` answer that hold the credits, in order of preference.
pub const BALANCE_KEYS: [&str; 7] = [
    "credits",
    "credit",
    "balance",
    "remaining",
    "usd",
    "amount",
    "total",
];
/// Object levels searched for a credit amount.
pub const BALANCE_MAX_DEPTH: usize = 3;
/// Label of the subscription generations left in a text `get_balance` answer.
pub const GENERATIONS_REMAINING_LABEL: &str = "generations_remaining";
