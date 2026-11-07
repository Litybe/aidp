use serde::{Deserialize, Serialize};

/// What a stack frame iterates over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    Root,
    Block,
    LoopBody,
}
