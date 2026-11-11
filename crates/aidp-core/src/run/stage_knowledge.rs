//! Freezes the knowledge block of each agent stage, so every re-emit of a run gives the same prompt.

use super::layout::save;
use super::RunState;
use crate::context_provider::ContextProvider;
use crate::models::constants::knowledge_context::{
    DEFAULT_KNOWLEDGE_AGENT, ISSUE_INPUT_KEYS, ISSUE_TEXT_INPUT_KEYS, ISSUE_TEXT_MAX_CHARS,
};
use crate::models::knowledge_settings::KnowledgeSettings;
use crate::models::stage_context_request::StageContextRequest;
use crate::pipeline::{Pipeline, Stage};

fn input_text(state: &RunState, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        state
            .inputs
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

/// The request for `stage`, from the run inputs and the pipeline `settings.knowledge`.
pub fn stage_context_request(
    state: &RunState,
    settings: &KnowledgeSettings,
    stage: &Stage,
) -> StageContextRequest {
    StageContextRequest {
        project_root: state.workspace_root.clone(),
        stage_id: stage.name.clone(),
        stage_description: stage.description.trim().to_string(),
        agent_type: stage
            .agent
            .clone()
            .unwrap_or_else(|| DEFAULT_KNOWLEDGE_AGENT.to_string()),
        route: settings.route,
        issue: input_text(state, &ISSUE_INPUT_KEYS),
        issue_text: input_text(state, &ISSUE_TEXT_INPUT_KEYS)
            .map(|text| text.chars().take(ISSUE_TEXT_MAX_CHARS).collect()),
        budget_tokens: settings.budget,
    }
}

fn agent_stages(pipe: &Pipeline) -> impl Iterator<Item = &Stage> {
    pipe.stages
        .iter()
        .flat_map(|stage| std::iter::once(stage).chain(stage.body.iter()))
        .filter(|stage| !stage.is_loop() && !stage.is_composite())
}

/// Fills `RunState::stage_knowledge` for the stages `settings.knowledge` covers and that are not
/// frozen yet, and persists it. Does nothing without a provider or when the setting is off.
pub fn ensure_stage_knowledge(
    state: &mut RunState,
    pipe: &Pipeline,
    provider: Option<&dyn ContextProvider>,
) -> anyhow::Result<()> {
    let settings = KnowledgeSettings::from_settings(&pipe.settings);
    let Some(provider) = provider.filter(|_| settings.enabled) else {
        return Ok(());
    };
    let mut changed = false;
    for stage in agent_stages(pipe) {
        if !settings.covers(&stage.name) || state.stage_knowledge.contains_key(&stage.name) {
            continue;
        }
        let request = stage_context_request(state, &settings, stage);
        let block = provider.stage_context(&request).unwrap_or_default();
        state.stage_knowledge.insert(stage.name.clone(), block);
        changed = true;
    }
    if changed {
        save(state)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/stage_knowledge_tests.rs"]
mod tests;
