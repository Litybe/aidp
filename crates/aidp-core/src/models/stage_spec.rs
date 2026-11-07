use crate::models::enums::contract_flag::ContractFlag;
use crate::models::enums::reply_schema_kind::ReplySchemaKind;
use crate::models::enums::{AgentRole, EffortLevel, Isolation, ModelTier};
use crate::models::gate_spec::GateSpec;
use crate::models::pipeline::ScopeSpec;
use serde::{Deserialize, Serialize};

/// Everything needed to dispatch one stage's sub-agent, with defaults already resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSpec {
    pub name: String,
    pub agent: AgentRole,
    /// The registry name the dispatch resolves: the bare role, a restricted variant or a custom name.
    pub agent_type: String,
    pub model: ModelTier,
    pub effort: Option<EffortLevel>,
    pub skills: Vec<String>,
    pub needs: Vec<String>,
    pub outputs: Vec<String>,
    pub scope: Option<ScopeSpec>,
    pub isolation: Option<Isolation>,
    pub gate: Option<GateSpec>,
    /// The raw `when:` expression, kept only when it reads a stage output (decided at run time).
    pub when: Option<String>,
    pub retries: u32,
    pub soft_pause: bool,
    pub loop_linked: bool,
    pub reply_schema: ReplySchemaKind,
    pub contracts: Vec<ContractFlag>,
}
