//! aidp gateway — the resident control plane of the AI dev pipeline.
//!
//! Every business rule lives here (Rust), the viewer only renders:
//! - **sessions**: each pipeline run is a headless `claude -p` process (stream-json), persisted, resumable;
//! - **automations**: cron / interval / one-shot / inbound webhook / internal event hooks / CI heartbeat;
//! - **guards**: subscription rate-limit window, per-session budget, API-key stripping (Max subscription only);
//! - **bus**: every fact is an event, exposed over SSE for the UI and consumed by event-triggered automations.
//!
//! `aidp gateway serve` runs the daemon; [`cli_api`] holds the one-shot HTTP client verbs.

pub mod aidp_bin;
pub mod art_assets;
pub mod attachments;
pub mod attention;
pub mod automations;
pub mod bus;
pub mod catalog;
pub mod changes;
pub mod claude_provider;
pub mod claude_runner;
pub mod claude_sessions;
pub mod cli_api;
pub mod client;
pub mod cost_routes;
pub mod cron;
pub mod crypto;
pub mod dispatcher;
pub mod doctor;
pub mod flows;
pub mod github_rate_limit;
pub mod health;
pub mod http;
pub mod ids;
pub mod inbox;
pub mod issue_cost;
pub mod launch_gates;
pub mod launcher;
pub mod manual_run;
pub mod mcp_configurations;
pub mod memory_ingest;
pub mod memory_routes;
pub mod model_routes;
pub mod model_service;
pub mod models;
pub mod native_runs;
pub mod net;
pub mod observability;
pub mod path_segments;
pub mod paths;
pub mod pixellab_usage;
pub mod pr_actions;
pub mod pr_adoption;
pub mod pr_attention;
pub mod pr_comment_state;
pub mod pr_detail_routes;
pub mod pr_drafts;
pub mod pr_fix;
pub mod pr_fix_recipe;
pub mod pr_marks;
pub mod pr_viewed;
pub mod pr_workspace_holder;
pub mod presets;
pub mod projects;
pub mod prs;
pub mod queue;
pub mod routes;
pub mod run_files;
pub mod runs;
pub mod session_error;
pub mod sessions;
pub mod settings_routes;
pub mod settings_yaml;
pub mod shutdown;
pub mod squad_prs;
pub mod static_ui;
pub mod store;
pub mod sync;
pub mod template;
pub mod threads;
pub mod ticket_ceiling;
pub mod ticket_cost;
pub mod ticket_flow_summary;
pub mod ticket_sync;
pub mod time;
pub mod unity_lock;
pub mod unity_routes;
pub mod unity_status;
pub mod usage;
pub mod workspace_resolver;
pub mod worktrees;

pub mod workflow_progress;

mod mcp_provider;
#[cfg(test)]
#[path = "../tests/common_scripted/recording_writer.rs"]
mod recording_writer;
