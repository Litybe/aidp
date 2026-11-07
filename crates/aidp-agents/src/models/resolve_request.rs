use std::collections::BTreeMap;

/// Inputs of provider resolution for one stage.
#[derive(Debug, Clone, Default)]
pub struct ResolveRequest {
    pub stage_provider: Option<String>,
    pub role: String,
    pub template_role_providers: BTreeMap<String, String>,
    pub launch_override: Option<String>,
    pub tier: Option<String>,
}
