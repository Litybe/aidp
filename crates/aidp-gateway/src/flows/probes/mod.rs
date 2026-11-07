//! Wait probes reading the PR status: review threads and review activity.

pub mod pr_reviews;
pub mod pr_threads;

use super::registry::Registry;
use crate::models::enums::BuiltinProbe;
use std::sync::Arc;

#[cfg(test)]
#[path = "../../../tests/unit/flow_probes_tests.rs"]
mod tests;

/// Registers the PR wait probes under their builtin names.
pub fn register(registry: &mut Registry) {
    registry.register_probe(
        BuiltinProbe::PrThreads.as_str(),
        Arc::new(pr_threads::PrThreadsProbe),
    );
    registry.register_probe(
        BuiltinProbe::PrReviews.as_str(),
        Arc::new(pr_reviews::PrReviewsProbe),
    );
}
