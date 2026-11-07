use serde::{Deserialize, Serialize};

/// Name of an effect, as it appears in the effect id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectName {
    RunAction,
    StartPipeline,
    CancelSession,
    CreateInbox,
    ResolveInbox,
    StartWait,
    Journal,
    Emit,
}

impl EffectName {
    /// The wire name used in effect ids.
    pub fn as_str(self) -> &'static str {
        match self {
            EffectName::RunAction => "run_action",
            EffectName::StartPipeline => "start_pipeline",
            EffectName::CancelSession => "cancel_session",
            EffectName::CreateInbox => "create_inbox",
            EffectName::ResolveInbox => "resolve_inbox",
            EffectName::StartWait => "start_wait",
            EffectName::Journal => "journal",
            EffectName::Emit => "emit",
        }
    }
}
