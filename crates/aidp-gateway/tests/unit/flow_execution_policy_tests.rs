use super::*;
use crate::models::{Runtime, TicketFlowRequest};
use serde_json::{Map, Value};

const HEADLESS_REQUEST: &str = include_str!("../fixtures/flows/headless_flow_request.json");
const HEADLESS_FIELD: &str = "headless";

/// A headless host policy keeps `headless` through the flow runtime the gateway persists.
#[test]
fn a_headless_policy_survives_the_persisted_flow_runtime() {
    let request: TicketFlowRequest = serde_json::from_str(HEADLESS_REQUEST).unwrap();
    let runtime = Runtime {
        start_request: Some(request),
        ..Runtime::default()
    };

    let stored: Runtime = serde_json::from_str(&serde_json::to_string(&runtime).unwrap()).unwrap();

    assert_eq!(stored, runtime);
    let policy = stored.start_request.and_then(|request| request.policy);
    assert!(policy.is_some_and(|policy| policy.headless));
}

/// A policy that does not ask for it is not headless and writes no `headless` key.
#[test]
fn a_policy_is_not_headless_unless_asked() {
    let omitted: FlowExecutionPolicy = serde_json::from_value(Value::Object(Map::new())).unwrap();
    assert!(!omitted.headless);
    let written = serde_json::to_value(FlowExecutionPolicy::default()).unwrap();
    assert!(written.get(HEADLESS_FIELD).is_none(), "{written}");
}

const EXECUTOR_FIELD: &str = "executor";

/// A policy that names no executor is internal and writes no `executor` key, so stored requests keep their shape.
#[test]
fn a_policy_runs_its_actions_in_the_gateway_unless_asked() {
    let omitted: FlowExecutionPolicy = serde_json::from_value(Value::Object(Map::new())).unwrap();
    assert_eq!(omitted.executor, FlowExecutorMode::Internal);
    let written = serde_json::to_value(FlowExecutionPolicy::default()).unwrap();
    assert!(written.get(EXECUTOR_FIELD).is_none(), "{written}");
}

/// `external` survives the persisted flow runtime, so a retried start can be compared with it.
#[test]
fn an_external_policy_survives_the_persisted_flow_runtime() {
    let policy: FlowExecutionPolicy =
        serde_json::from_value(serde_json::json!({ EXECUTOR_FIELD: "external" })).unwrap();
    assert_eq!(policy.executor, FlowExecutorMode::External);
    let request = TicketFlowRequest {
        policy: Some(policy),
        ..TicketFlowRequest::default()
    };
    let runtime = Runtime {
        start_request: Some(request),
        ..Runtime::default()
    };

    let stored: Runtime = serde_json::from_str(&serde_json::to_string(&runtime).unwrap()).unwrap();

    assert_eq!(stored, runtime);
    let written = serde_json::to_value(&stored).unwrap();
    assert_eq!(
        written["start_request"]["policy"][EXECUTOR_FIELD],
        serde_json::json!("external")
    );
}

const EXTERNAL_AGENTS_FIELD: &str = "externalAgents";

/// A policy keeps the agent sessions in the gateway unless asked, and writes no key for it.
#[test]
fn a_policy_keeps_agent_sessions_in_the_gateway_unless_asked() {
    let omitted: FlowExecutionPolicy = serde_json::from_value(Value::Object(Map::new())).unwrap();
    assert!(!omitted.external_agents);
    let written = serde_json::to_value(FlowExecutionPolicy::default()).unwrap();
    assert!(written.get(EXTERNAL_AGENTS_FIELD).is_none(), "{written}");
}

/// `externalAgents` survives the persisted flow runtime, so a retried start can be compared with it.
#[test]
fn external_agents_survive_the_persisted_flow_runtime() {
    let policy: FlowExecutionPolicy = serde_json::from_value(serde_json::json!({
        EXECUTOR_FIELD: "external",
        EXTERNAL_AGENTS_FIELD: true,
    }))
    .unwrap();
    let request = TicketFlowRequest {
        policy: Some(policy),
        ..TicketFlowRequest::default()
    };
    let runtime = Runtime {
        start_request: Some(request),
        ..Runtime::default()
    };

    let stored: Runtime = serde_json::from_str(&serde_json::to_string(&runtime).unwrap()).unwrap();

    assert_eq!(stored, runtime);
    let written = serde_json::to_value(&stored).unwrap();
    assert_eq!(
        written["start_request"]["policy"][EXTERNAL_AGENTS_FIELD],
        serde_json::json!(true)
    );
}

/// Host-run agent sessions need the external executor; the gateway refuses the other combinations only when asked.
#[test]
fn external_agents_need_the_external_executor() {
    let agents_only = FlowExecutionPolicy {
        external_agents: true,
        ..FlowExecutionPolicy::default()
    };
    assert!(validate_external_agents(Some(&agents_only)).is_err());
    let both = FlowExecutionPolicy {
        executor: FlowExecutorMode::External,
        ..agents_only
    };
    assert!(validate_external_agents(Some(&both)).is_ok());
    assert!(validate_external_agents(Some(&FlowExecutionPolicy::default())).is_ok());
    assert!(validate_external_agents(None).is_ok());
}
