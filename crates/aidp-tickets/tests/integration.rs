//! Integration tests, linked into one binary: one test binary per file cost
//! a full link of the crate's dependency graph each.
mod budget_verdict;
mod lane_plan;
mod lifecycle_transitions;
mod plan_dispatch;
