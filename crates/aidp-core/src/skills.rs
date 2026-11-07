//! Skill name -> SKILL.md path resolution. Port from v3 `aidp-resolve-skills`.

use crate::models::constants::skills::{
    GRAPHIFY_CONTEXT_SKILL, SKILL_DROPPED_WARNING_PREFIX, SKILL_DROPPED_WARNING_SEPARATOR,
    SKILL_DROPPED_WARNING_STAGE_TEXT,
};
use crate::pipeline::{Pipeline, Stage};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub fn resolve(name: &str, search_paths: &[&Path]) -> Option<PathBuf> {
    for root in search_paths {
        let candidate = root.join(name).join("SKILL.md");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

pub fn default_search_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = dirs_home() {
        v.push(home.join(".claude/skills"));
    }
    v
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// One declared skill `resolve` cannot find, and the stage declaring it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub stage: String,
    pub skill: String,
}

/// Every skill a stage of `pipe` may hand its sub-agent that `resolve` does
/// not find under `search_paths`, i.e. that its prompt would list as
/// `(not found)`. Covers `skills`, every `skills_by` bucket (the input value
/// is unknown before run init) and every `plan_tasks.skills_by_kind` list, at
/// any loop depth.
pub fn unresolved(pipe: &Pipeline, search_paths: &[&Path]) -> Vec<Unresolved> {
    fn walk(stages: &[Stage], search: &[&Path], out: &mut Vec<Unresolved>) {
        for st in stages {
            let mut names: Vec<&String> = st.skills.iter().collect();
            if let Some(by) = &st.skills_by {
                names.extend(by.map.values().flatten());
                names.extend(by.default.iter());
            }
            if let Some(pt) = &st.plan_tasks {
                names.extend(pt.skills_by_kind.values().flatten());
            }
            for name in names {
                let miss = Unresolved {
                    stage: st.name.clone(),
                    skill: name.clone(),
                };
                if resolve(name, search).is_none() && !out.contains(&miss) {
                    out.push(miss);
                }
            }
            walk(&st.body, search, out);
        }
    }
    let mut out = Vec::new();
    walk(&pipe.stages, search_paths, &mut out);
    out
}

/// Drops `graphify-context` from every stage when the run has no knowledge
/// graph, returning one warning per stage that lost it. Borrows when nothing changes.
pub fn without_unavailable(
    pipe: &Pipeline,
    graph_available: bool,
) -> (Cow<'_, Pipeline>, Vec<String>) {
    if graph_available {
        return (Cow::Borrowed(pipe), Vec::new());
    }
    fn strip(stages: &mut [Stage], warnings: &mut Vec<String>) {
        for st in stages {
            let drop_skill = |list: &mut Vec<String>| {
                let before = list.len();
                list.retain(|name| name != GRAPHIFY_CONTEXT_SKILL);
                list.len() != before
            };
            let mut dropped = drop_skill(&mut st.skills);
            if let Some(by) = st.skills_by.as_mut() {
                for list in by.map.values_mut() {
                    dropped |= drop_skill(list);
                }
                dropped |= drop_skill(&mut by.default);
            }
            if let Some(pt) = st.plan_tasks.as_mut() {
                for list in pt.skills_by_kind.values_mut() {
                    dropped |= drop_skill(list);
                }
            }
            if dropped {
                let warning = format!(
                    "{SKILL_DROPPED_WARNING_PREFIX}{SKILL_DROPPED_WARNING_SEPARATOR}{GRAPHIFY_CONTEXT_SKILL}{SKILL_DROPPED_WARNING_STAGE_TEXT}{}",
                    st.name
                );
                tracing::info!("{warning}");
                warnings.push(warning);
            }
            strip(&mut st.body, warnings);
        }
    }
    let mut owned = pipe.clone();
    let mut warnings = Vec::new();
    strip(&mut owned.stages, &mut warnings);
    (Cow::Owned(owned), warnings)
}

#[cfg(test)]
#[path = "../tests/unit/skills_tests.rs"]
mod tests;
