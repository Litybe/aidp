use std::collections::BTreeMap;

use crate::models::enums::agent_error::AgentError;
use crate::models::providers_config::ProvidersConfig;
use crate::models::resolve_request::ResolveRequest;
use crate::resolve::resolve_provider;

const FIXTURE: &str = include_str!("../fixtures/providers_config.json");
const RETIRED_PROVIDERS: &[&str] = &["codex", "copilot"];

fn config() -> ProvidersConfig {
    let json: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    ProvidersConfig::from_config_json(&json).unwrap()
}

fn request(stage: Option<&str>, settings: Option<&str>, launch: Option<&str>) -> ResolveRequest {
    let mut template = BTreeMap::new();
    if let Some(name) = settings {
        template.insert("executor".to_string(), name.to_string());
    }
    ResolveRequest {
        stage_provider: stage.map(str::to_string),
        role: "executor".to_string(),
        template_role_providers: template,
        launch_override: launch.map(str::to_string),
        tier: None,
    }
}

#[test]
fn resolution_order_table() {
    let cases = [
        (
            request(Some("other"), Some("fake"), Some("claude")),
            "other",
        ),
        (request(None, Some("fake"), Some("other")), "fake"),
        (request(None, None, Some("other")), "other"),
        (request(None, None, None), "claude"),
    ];
    for (req, expected) in cases {
        assert_eq!(
            resolve_provider(&req, &config()).unwrap().provider,
            expected
        );
    }
}

#[test]
fn unknown_provider_names_it_and_origin() {
    let error = resolve_provider(&request(Some("ghost"), None, None), &config()).unwrap_err();
    assert!(matches!(error, AgentError::UnknownProvider { .. }));
    let message = error.to_string();
    assert!(message.contains("ghost") && message.contains("stage"));
}

/// Codex and Copilot are no longer built in: a launch naming them is refused like any unknown id.
#[test]
fn retired_providers_are_refused_as_unknown() {
    for name in RETIRED_PROVIDERS {
        let error = resolve_provider(&request(None, None, Some(name)), &config()).unwrap_err();
        assert!(matches!(error, AgentError::UnknownProvider { .. }));
        assert!(error.to_string().contains(name));
    }
}

#[test]
fn opus_maps_to_configured_model() {
    let mut req = request(None, None, None);
    req.tier = Some("opus".to_string());
    let resolved = resolve_provider(&req, &config()).unwrap();
    assert_eq!(resolved.model.as_deref(), Some("claude-opus-5-5"));
}

#[test]
fn unmapped_tier_passes_through() {
    let mut req = request(None, None, None);
    req.tier = Some("haiku".to_string());
    let resolved = resolve_provider(&req, &config()).unwrap();
    assert_eq!(resolved.model.as_deref(), Some("haiku"));
}
