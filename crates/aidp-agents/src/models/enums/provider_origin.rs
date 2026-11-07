use std::fmt;

use serde::Serialize;

const STAGE_LABEL: &str = "stage";
const TEMPLATE_SETTINGS_LABEL: &str = "template settings";
const LAUNCH_OVERRIDE_LABEL: &str = "launch override";
const CONFIG_DEFAULT_LABEL: &str = "config default";

/// Where a provider id came from during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOrigin {
    Stage,
    TemplateSettings,
    LaunchOverride,
    ConfigDefault,
}

impl fmt::Display for ProviderOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Stage => STAGE_LABEL,
            Self::TemplateSettings => TEMPLATE_SETTINGS_LABEL,
            Self::LaunchOverride => LAUNCH_OVERRIDE_LABEL,
            Self::ConfigDefault => CONFIG_DEFAULT_LABEL,
        };
        formatter.write_str(label)
    }
}
