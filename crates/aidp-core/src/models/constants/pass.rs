//! Limits on how much one Workflow pass compiles.

/// Stages one pass may accumulate before a not-yet-crossed composite entry must stop it.
pub const DEFAULT_MAX_STAGES_PER_PASS: usize = 10;
