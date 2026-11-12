use crate::models::flow_execution_policy::FlowExecutionPolicy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Body of a ticket flow start, as received and as handed to the flow start route.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TicketFlowRequest {
    #[serde(
        default,
        rename = "executionKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_key: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub ticket: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<FlowExecutionPolicy>,
    /// The entry of the flow definition to start from (a forced move of the ticket).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}
