//! Shared user-facing message fragments, kept apart from the data models.

/// Prefix of the error raised when a run's `run-state.json` cannot be read.
pub const FAILED_TO_READ_RUN_STATE: &str = "failed to read run state";

/// Prefix of the error raised when a stage name has no matching stage in the pipeline.
pub const UNKNOWN_STAGE: &str = "unknown stage:";

/// Error text when the resolved stack is `unknown`.
pub const UNKNOWN_REFUSAL: &str = "stack: unknown - no project marker found (sln/csproj, serverless or template.yaml with package.json, angular.json, Cargo.toml, package.json); pass an explicit stack";

/// Default scope mode a stage runs under when it declares no `scope:`.
pub const DEFAULT_SCOPE_MODE: &str = "stage_dir_only";
