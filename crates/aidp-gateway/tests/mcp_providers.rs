use std::sync::Arc;

use aidp_agents::{AgentProvider, AgentSpec, ExitReason, McpConfiguration};
use aidp_gateway::claude_provider::ClaudeCodeProvider;

use crate::common_scripted::scripted_runner::ScriptedRunner;

const CONFIGURATION: &str = include_str!("fixtures/mcp/configuration.json");
const CLAUDE_COMPLETED: &str = include_str!("fixtures/claude_session.jsonl");
const SECRET: &str = "confidential-fixture-value";
const MCP_FLAG: &str = "--mcp-config";
const STRICT_FLAG: &str = "--strict-mcp-config";

fn spec() -> AgentSpec {
    AgentSpec {
        mcp_configuration: Some(Arc::new(
            serde_json::from_str::<McpConfiguration>(CONFIGURATION).unwrap(),
        )),
        ..AgentSpec::default()
    }
}

#[test]
fn claude_handoff_excludes_secret_values_from_command_arguments() {
    let runner = Arc::new(ScriptedRunner::new(CLAUDE_COMPLETED));
    let provider = ClaudeCodeProvider::new(runner.clone());
    let handle = provider.start(spec()).unwrap();
    assert_eq!(handle.wait().unwrap(), ExitReason::Completed);
    let processes = runner.specs.lock().unwrap();
    assert!(processes[0].args.iter().any(|value| value == MCP_FLAG));
    assert!(processes[0].args.iter().any(|value| value == STRICT_FLAG));
    assert!(!processes[0].args.join(" ").contains(SECRET));
    assert!(processes[0].envs.values().any(|value| value == SECRET));
    assert!(processes[0].piped_stderr);
}
