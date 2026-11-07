//! Ports the core reads through, so each can be substituted.

use crate::plan;
use std::path::Path;

/// A port a single instance of can be shared across threads and stages.
pub trait SharedPort: Send + Sync {}

impl<T: Send + Sync + ?Sized> SharedPort for T {}

/// Reads a plan artifact for a `plan_tasks:` stage.
pub trait PlanSource {
    /// `Ok(None)` while the plan does not exist yet, `Ok(Some(_))` once it does.
    fn load(&self, path: &Path) -> anyhow::Result<Option<plan::Plan>>;
}
