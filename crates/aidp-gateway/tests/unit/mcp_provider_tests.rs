use super::*;

const CONFIGURATION: &str = include_str!("../fixtures/mcp/configuration.json");
const SECRET: &str = "confidential-fixture-value";
const AUTHORIZATION: &str = "Authorization";
const TOKEN_NAME: &str = "TOKEN";
const MISSING_PLACEHOLDER: &str = "${MISSING_TOKEN}";
const CONFIGURATION_DEBUG: &str = "McpConfiguration { <redacted> }";
const EXPECTED_HEADER: &str = "Bearer confidential-fixture-value";
const LITERAL_HEADER: &str = "Bearer literal-confidential-header";
const LITERAL_TOKEN: &str = "literal-confidential-header";
const RAW_INIT: &str = include_str!("../fixtures/mcp/raw_init.json");
const INIT: &str = "init";
const FRAGMENT_ENV: &str = "FRAGMENT";
const FRAGMENT: &str = "confidential";
const FIELD_SUBTYPE: &str = "subtype";
const FIELD_SESSION: &str = "session_id";
const FIELD_MODEL: &str = "model";
const TEXT_POINTER: &str = "/message/content/0/text";
const LITERAL_QUERY_URL: &str = "https://example.invalid/?token=literal%2Dconfidential-header";

fn configuration() -> McpConfiguration {
    serde_json::from_str(CONFIGURATION).unwrap()
}

#[test]
fn claude_configuration_keeps_every_literal_out_of_argv() {
    let (json, environment) = claude_configuration(&configuration()).unwrap();
    assert!(!json.contains(SECRET));
    assert!(!json.contains(EXPECTED_HEADER));
    assert!(environment.values().any(|value| value == EXPECTED_HEADER));
    assert!(environment.values().any(|value| value == SECRET));
    assert!(!environment.contains_key(TOKEN_NAME));
}

#[test]
fn output_redaction_scrubs_nested_provider_diagnostics() {
    let mut value = serde_json::from_str::<serde_json::Value>(CONFIGURATION).unwrap();
    redact_value(Some(&configuration()), &mut value);
    assert!(!value.to_string().contains(SECRET));
    assert_eq!(format!("{:?}", configuration()), CONFIGURATION_DEBUG);
    assert!(!format!("{:?}", configuration().servers).contains(SECRET));
}

#[test]
fn literal_authentication_header_redacts_token_only_errors() {
    let mut configuration = configuration();
    configuration.servers[0]
        .headers
        .insert(AUTHORIZATION.to_string(), LITERAL_HEADER.to_string());
    assert!(!redact(Some(&configuration), LITERAL_TOKEN).contains(LITERAL_TOKEN));
}

#[test]
fn missing_variables_and_incoherent_fields_fail_without_values() {
    let mut configuration = configuration();
    configuration.servers[0].url = Some(MISSING_PLACEHOLDER.to_string());
    assert!(!configuration
        .validate()
        .unwrap_err()
        .to_string()
        .contains(MISSING_PLACEHOLDER));
    configuration.servers[0].url = Some(SECRET.to_string());
    assert!(!configuration
        .validate()
        .unwrap_err()
        .to_string()
        .contains(SECRET));
}

#[test]
fn redaction_preserves_control_identifiers() {
    let event = aidp_agents::AgentEvent::PermissionRequest {
        id: SECRET.to_string(),
        tool: SECRET.to_string(),
        input: serde_json::Value::String(EXPECTED_HEADER.to_string()),
    };
    let aidp_agents::AgentEvent::PermissionRequest { id, tool, input } =
        redact_event(Some(&configuration()), event)
    else {
        panic!()
    };
    assert_eq!(id, SECRET);
    assert_eq!(tool, SECRET);
    assert!(!input.to_string().contains(SECRET));
}

#[test]
fn raw_claude_init_preserves_protocol_metadata_with_short_secrets() {
    let mut configuration = configuration();
    configuration
        .environment
        .insert(TOKEN_NAME.to_string(), INIT.to_string());
    configuration
        .environment
        .insert(FRAGMENT_ENV.to_string(), FRAGMENT.to_string());
    let event = aidp_agents::AgentEvent::Raw(serde_json::from_str(RAW_INIT).unwrap());
    let aidp_agents::AgentEvent::Raw(message) = redact_event(Some(&configuration), event) else {
        panic!()
    };
    assert_eq!(message[FIELD_SUBTYPE], INIT);
    assert_eq!(message[FIELD_SESSION], SECRET);
    assert_eq!(message[FIELD_MODEL], INIT);
    let text = message.pointer(TEXT_POINTER).unwrap().as_str().unwrap();
    assert!(!text.contains(INIT));
    assert!(!text.contains(FRAGMENT));
}

#[test]
fn literal_url_query_credentials_redact_extracted_decoded_errors() {
    let mut configuration = configuration();
    configuration.servers[0].url = Some(LITERAL_QUERY_URL.to_string());
    assert!(!redact(Some(&configuration), LITERAL_TOKEN).contains(LITERAL_TOKEN));
}
