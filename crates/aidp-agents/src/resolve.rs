use crate::config::ProvidersConfig;
use crate::models::enums::agent_error::AgentError;
use crate::models::enums::provider_origin::ProviderOrigin;
use crate::models::resolve_request::ResolveRequest;
pub use crate::models::resolved::Resolved;

/// Picks the provider: stage, then template settings, then launch override, then config default.
pub fn resolve_provider(
    request: &ResolveRequest,
    config: &ProvidersConfig,
) -> Result<Resolved, AgentError> {
    let (provider, origin) = pick_provider(request, config);
    if !config.knows(&provider) {
        return Err(AgentError::UnknownProvider {
            name: provider,
            origin,
        });
    }
    let model = request
        .tier
        .as_ref()
        .map(|tier| map_tier(config, &provider, tier));
    Ok(Resolved { provider, model })
}

fn pick_provider(request: &ResolveRequest, config: &ProvidersConfig) -> (String, ProviderOrigin) {
    if let Some(name) = &request.stage_provider {
        return (name.clone(), ProviderOrigin::Stage);
    }
    if let Some(name) = request.template_role_providers.get(&request.role) {
        return (name.clone(), ProviderOrigin::TemplateSettings);
    }
    if let Some(name) = &request.launch_override {
        return (name.clone(), ProviderOrigin::LaunchOverride);
    }
    (
        config.default_id().to_string(),
        ProviderOrigin::ConfigDefault,
    )
}

fn map_tier(config: &ProvidersConfig, provider: &str, tier: &str) -> String {
    config
        .entries
        .get(provider)
        .and_then(|entry| entry.models.get(tier))
        .cloned()
        .unwrap_or_else(|| tier.to_string())
}

#[cfg(test)]
#[path = "../tests/unit/resolve_tests.rs"]
mod resolve_tests;
