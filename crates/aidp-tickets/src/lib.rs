//! `aidp-tickets`: the ticket domain model and store for the backlog living
//! under `<vault>/TODO/`: local tickets and mirrors of GitHub issues.
//!
//! This crate never resolves the vault itself: every entry point takes an
//! already-resolved `vault_root: &Path` from its caller, and it reads no
//! process environment. The domain defines the `GithubPort` port and the
//! `GithubIssue` value; adapters (`aidp-integrations`) implement the port and
//! depend on this crate, never the reverse.

pub mod blockers;
pub mod budget;
pub mod budget_ticket;
pub mod cli;
pub mod config;
pub mod dispatcher_state;
pub mod github_sync;
pub mod index;
pub mod ledger;
pub mod lifecycle;
pub mod model;
pub mod models;
pub mod night_mode;
pub mod note;
pub mod order;
pub mod plan;
pub mod routing;
pub mod safe_name;
pub mod state_labels;
pub mod store;
pub mod ticket_error;
pub mod ticket_route;

pub use aidp_core::Format as Fmt;
pub use cli::{BodySource, ListQuery, NewTicket, PlanOutput, TicketEdit};
pub use models::enums::SkipReason;
pub use models::track::Track;
pub use ticket_error::TicketError;
