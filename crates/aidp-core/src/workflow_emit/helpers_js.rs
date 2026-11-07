/// JS emitted into every script. `safeAgent` holds a dispatch carrying `opts.skills` to its
/// `skills_loaded`.
pub(super) const HELPERS: &str = include_str!("../../assets/js/helpers.js");

/// Emitted only when a stage may retry (`settings.max_stage_retries > 0`).
pub(super) const RETRY_HELPERS: &str = include_str!("../../assets/js/retry_helpers.js");

/// Emitted only when some stage declares `fan_out:`.
pub(super) const FANOUT_HELPERS: &str = include_str!("../../assets/js/fanout_helpers.js");

/// Emitted only when some stage declares `plan_tasks:` — the runtime half of the plan scheduler.
/// `aidp` already decided the graph; these helpers only enforce it.
pub(super) const PLAN_HELPERS: &str = include_str!("../../assets/js/plan_helpers.js");

/// Emitted only when a `pool(...)` consumer exists.
pub(super) const POOL_HELPERS: &str = include_str!("../../assets/js/pool_helpers.js");
