//! Stable family slugs an `ExitStatus` reports through `family()`.

/// Family slug for `ExitStatus::Success`.
pub const SUCCESS: &str = "success";
/// Family slug for `ExitStatus::Internal`.
pub const INTERNAL: &str = "internal";
/// Family slug for `ExitStatus::HookWake`.
pub const HOOK_WAKE: &str = "hook_wake";
/// Family slug for `ExitStatus::NotFound`.
pub const NOT_FOUND: &str = "not_found";
/// Family slug for `ExitStatus::PolicyRefusal`.
pub const POLICY_REFUSAL: &str = "policy_refusal";
/// Family slug for `ExitStatus::UpstreamFailure`.
pub const UPSTREAM_FAILURE: &str = "upstream_failure";
/// Family slug for `ExitStatus::NegativeVerdict`.
pub const NEGATIVE_VERDICT: &str = "negative_verdict";
/// Family slug for `ExitStatus::Usage`.
pub const USAGE: &str = "usage";
