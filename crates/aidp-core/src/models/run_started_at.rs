use serde::Deserialize;

/// Just enough of a `run-state.json` to count and order runs.
#[derive(Deserialize)]
pub(crate) struct RunStartedAt {
    pub started_at: String,
}
