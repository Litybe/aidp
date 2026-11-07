//! Model policy: `models: { <route>: { <stageId>: haiku|sonnet|opus } }` in `TODO/config.yaml`,
//! frozen into the run at init so every re-emit compiles the same models.
//! Precedence per stage: CLI override, then the policy, then the template's `model:`.

use crate::models::enums::ModelTier;
use crate::pipeline::Pipeline;
use crate::run::RunState;
use std::collections::BTreeMap;
use std::path::Path;

/// Route, then stage id, then the tier that stage runs on.
pub type ModelPolicy = BTreeMap<String, BTreeMap<String, ModelTier>>;

/// Config file the policy lives in, relative to the vault root.
pub const CONFIG_RELATIVE_PATH: [&str; 2] = ["TODO", "config.yaml"];
/// Top-level key of the policy in the config file.
pub const MODELS_KEY: &str = "models";
/// Run input that names the ticket route (`code`, `art`, `map`, `doc`).
pub const INPUT_ROUTE: &str = "route";
const CLI_SEPARATOR: char = '=';

/// The policy of `<vault>/TODO/config.yaml`; a missing or invalid file yields an empty policy.
pub fn load_policy(vault_root: &Path) -> ModelPolicy {
    let file = CONFIG_RELATIVE_PATH
        .iter()
        .fold(vault_root.to_path_buf(), |path, part| path.join(part));
    let Ok(text) = std::fs::read_to_string(&file) else {
        return ModelPolicy::new();
    };
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return ModelPolicy::new();
    };
    let Some(models) = document.get(MODELS_KEY).cloned() else {
        return ModelPolicy::new();
    };
    serde_yaml::from_value(models).unwrap_or_else(|error| {
        tracing::warn!(%error, "invalid `models` policy ignored");
        ModelPolicy::new()
    })
}

/// Parses repeated `stage=tier` command-line overrides.
pub fn parse_cli_overrides(specs: &[String]) -> Result<BTreeMap<String, ModelTier>, String> {
    let mut overrides = BTreeMap::new();
    for spec in specs {
        let (stage, tier) = spec
            .split_once(CLI_SEPARATOR)
            .filter(|(stage, _)| !stage.trim().is_empty())
            .ok_or_else(|| format!("`{spec}` must be stage=tier"))?;
        let tier: ModelTier = serde_yaml::from_str(tier.trim())
            .map_err(|_| format!("`{tier}` is not a model tier (haiku, sonnet, opus)"))?;
        overrides.insert(stage.trim().to_string(), tier);
    }
    Ok(overrides)
}

/// The route a run input names, lowercased.
pub fn route_of_inputs(inputs: &BTreeMap<String, serde_json::Value>) -> Option<String> {
    let route = inputs
        .get(INPUT_ROUTE)?
        .as_str()?
        .trim()
        .to_ascii_lowercase();
    (!route.is_empty()).then_some(route)
}

/// The stage overrides the policy sets for `route`; empty when the route is unknown or absent.
pub fn overrides_for(policy: &ModelPolicy, route: Option<&str>) -> BTreeMap<String, ModelTier> {
    route
        .and_then(|route| policy.get(route))
        .cloned()
        .unwrap_or_default()
}

/// Freezes the overrides of `state` (policy of the vault, then `cli`) and returns how many stages they pin.
pub fn apply(
    state: &mut RunState,
    vault_root: Option<&Path>,
    cli: &BTreeMap<String, ModelTier>,
) -> usize {
    let route = route_of_inputs(&state.inputs);
    let mut overrides = vault_root
        .map(|root| overrides_for(&load_policy(root), route.as_deref()))
        .unwrap_or_default();
    overrides.extend(cli.iter().map(|(stage, tier)| (stage.clone(), *tier)));
    state.model_overrides = overrides;
    state.model_overrides.len()
}

/// The tier each top-level stage of `pipe` runs on once `overrides` are applied.
pub fn effective_tiers(
    pipe: &Pipeline,
    overrides: &BTreeMap<String, ModelTier>,
) -> Vec<(String, ModelTier)> {
    pipe.stages
        .iter()
        .map(|stage| {
            let tier = overrides
                .get(&stage.name)
                .copied()
                .or(stage.model)
                .unwrap_or_else(|| stage.agent_role().default_model());
            (stage.name.clone(), tier)
        })
        .collect()
}

/// True when at least one top-level stage of `pipe` runs on opus.
pub fn is_heavy(pipe: &Pipeline, overrides: &BTreeMap<String, ModelTier>) -> bool {
    effective_tiers(pipe, overrides)
        .iter()
        .any(|(_, tier)| *tier == ModelTier::Opus)
}

#[cfg(test)]
#[path = "../tests/unit/model_policy_tests.rs"]
mod tests;
