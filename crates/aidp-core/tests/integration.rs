//! Integration tests, linked into one binary: one test binary per file cost
//! a full link of the crate's dependency graph each.
mod common;
mod cost_price_table;
mod dev_flow_template;
mod knowledge_stale_ports;
mod plan_schedule_props;
mod ports_tests;
mod prop_compile_invariants;
mod repo_quality_gates;
mod repository_source_gates;
mod review_schemas;
mod run_facade;
mod run_fs_gates;
mod run_fs_init;
mod run_fs_layout;
mod run_fs_reconcile;
mod run_git;
mod run_ports;
mod run_render_snapshots;
mod vault_artifact_leak;
mod workspace_boundary_gate;
