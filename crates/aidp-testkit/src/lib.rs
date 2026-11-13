//! A test toolkit for AIDP crates.
//!
//! This crate provides utilities for writing isolated, reproducible tests:
//! - `TempWorkspace`: Temporary directory with automatic cleanup
//! - `EnvGuard`: Process-wide environment variable management
//! - `snapshot_tree`: Directory tree snapshots for dry-run verification
//! - `normalize`: Normalize command output for snapshot comparison
//! - `ProcShim`: Fake executables for intercepting process calls
//! - `FakeClaude`: Configurable fake `claude` executable for daemon tests

pub mod env_guard;
pub mod fake_claude;
pub mod fake_provider;
pub mod fake_provider_script;
pub mod fixtures;
pub mod git_fixture;
pub mod models;
pub mod normalize;
pub mod proc_shim;
pub mod snapshot_tree;
pub mod temp_workspace;

pub use env_guard::EnvGuard;
pub use fake_claude::{pid_alive, spawn_retrying_busy, FakeClaude, FakeClaudeConfig};
pub use fake_provider::FakeProvider;
pub use git_fixture::GitFixture;
pub use normalize::normalize;
pub use proc_shim::ProcShim;
pub use snapshot_tree::snapshot_tree;
pub use temp_workspace::TempWorkspace;
