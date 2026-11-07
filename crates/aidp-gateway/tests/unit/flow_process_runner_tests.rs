use super::*;
use crate::models::constants::flow_execution::{GATEWAY_URL_ENV, UI_TOKEN_ENV, VAULT_ENV};
use aidp_core::process_runner::ScriptedProcessRunner;

const FIRST_URL: &str = "http://127.0.0.1:4101";
const SECOND_URL: &str = "http://127.0.0.1:4102";
const FIRST_TOKEN: &str = "first-test-token";
const SECOND_TOKEN: &str = "second-test-token";
const FIRST_VAULT: &str = "first-test-vault";
const SECOND_VAULT: &str = "second-test-vault";
const CURRENT_BINARY: &str = "current-aidp";

#[test]
fn concurrent_runtime_children_keep_their_own_gateway_and_storage() {
    let runner = Arc::new(ScriptedProcessRunner::new());
    let first = FlowProcessRunner::new(
        runner.clone(),
        BTreeMap::from([
            (GATEWAY_URL_ENV.to_string(), FIRST_URL.to_string()),
            (UI_TOKEN_ENV.to_string(), FIRST_TOKEN.to_string()),
            (VAULT_ENV.to_string(), FIRST_VAULT.to_string()),
        ]),
        Some(CURRENT_BINARY.to_string()),
    );
    let second = FlowProcessRunner::new(
        runner,
        BTreeMap::from([
            (GATEWAY_URL_ENV.to_string(), SECOND_URL.to_string()),
            (UI_TOKEN_ENV.to_string(), SECOND_TOKEN.to_string()),
            (VAULT_ENV.to_string(), SECOND_VAULT.to_string()),
        ]),
        None,
    );
    let spec = ProcessSpec::new(AIDP_PROGRAM);
    let first_spec = first.contextual_spec(&spec);
    let second_spec = second.contextual_spec(&spec);
    assert_eq!(first_spec.envs[GATEWAY_URL_ENV], FIRST_URL);
    assert_eq!(second_spec.envs[GATEWAY_URL_ENV], SECOND_URL);
    assert_eq!(first_spec.envs[UI_TOKEN_ENV], FIRST_TOKEN);
    assert_eq!(second_spec.envs[UI_TOKEN_ENV], SECOND_TOKEN);
    assert_eq!(first_spec.envs[VAULT_ENV], FIRST_VAULT);
    assert_eq!(second_spec.envs[VAULT_ENV], SECOND_VAULT);
    assert_eq!(first_spec.program, CURRENT_BINARY);
    assert_eq!(second_spec.program, AIDP_PROGRAM);
    assert!(spec.envs.is_empty());
}
