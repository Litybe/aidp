//! Reading the GitHub quotas through `gh api rate_limit`, a call GitHub does not count.

pub const GH_RATE_LIMIT_PATH: &str = "rate_limit";
pub const PTR_GRAPHQL_BUCKET: &str = "/resources/graphql";
pub const PTR_CORE_BUCKET: &str = "/resources/core";
pub const KEY_LIMIT: &str = "limit";
pub const KEY_REMAINING: &str = "remaining";
pub const KEY_USED: &str = "used";
pub const KEY_RESET: &str = "reset";
pub const MESSAGE_NO_RATE_LIMIT: &str = "gh api rate_limit returned no JSON";
