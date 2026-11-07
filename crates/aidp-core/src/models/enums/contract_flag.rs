use crate::models::pipeline::KnowledgeLayer;
use serde::{Deserialize, Serialize};

/// A prompt contract or runtime guard that applies to a stage. Names only, never rendered text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractFlag {
    FanoutProducer,
    PlanProducer,
    MergeFindings,
    ReviewDimension,
    ReviewSynthesis,
    HarnessGapPause,
    Knowledge(KnowledgeLayer),
}
