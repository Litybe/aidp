//! Names the pricing grid is read through.

/// Environment variable pointing at a pricing grid that replaces the embedded one.
pub const MODEL_PRICES_ENV: &str = "AIDP_MODEL_PRICES";

/// Pricing version reported when the grid failed to load.
pub const UNKNOWN_PRICING_VERSION: &str = "unknown";

/// Tier row used for a model the grid does not know (priced at zero).
pub const UNKNOWN_TIER: &str = "unknown";

/// Service tier billed at half the standard price.
pub const BATCH_SERVICE_TIER: &str = "batch";

/// Label used when a model name is missing.
pub const UNKNOWN_LABEL: &str = "unknown";

/// Claude configuration directory under the home directory.
pub const CLAUDE_DIR: &str = ".claude";

/// Projects directory under the Claude configuration directory.
pub const PROJECTS_DIR: &str = "projects";
