//! The workspace's shared test fixtures. Tests compile the frozen skill under `fixtures/skill`,
//! never the real one: the skill lives in its own repository and is covered by the e2e suite.

use std::path::{Path, PathBuf};

/// Workspace directory holding every shared fixture, from this crate's manifest directory.
const FIXTURES_FROM_CRATE: &str = "../../fixtures";
const SKILL_DIR: &str = "skill";
const TEMPLATES_DIR: &str = "templates";
const FLOWS_DIR: &str = "flows";
const RECIPES_DIR: &str = "recipes";
const HOOKS_DIR: &str = "hooks";
const MOCKS_DIR: &str = "mocks";
const TEMPLATE_INPUTS_FILE: &str = "template-inputs.json";
const TEMPLATE_EXTENSION: &str = "yaml";

/// The workspace `fixtures/` directory, absolute.
pub fn dir() -> PathBuf {
    let relative = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES_FROM_CRATE);
    relative.canonicalize().unwrap_or(relative)
}

/// The frozen skill: `SKILL.md`, `templates/`, `agents/`, `hooks/` and `commands/`, as the
/// tests' `AIDP_SKILL_ROOT`.
pub fn skill_root() -> PathBuf {
    dir().join(SKILL_DIR)
}

/// `templates/` of the frozen skill.
pub fn templates_dir() -> PathBuf {
    skill_root().join(TEMPLATES_DIR)
}

/// `templates/<id>.yaml` of the frozen skill.
pub fn template(id: &str) -> PathBuf {
    templates_dir().join(format!("{id}.{TEMPLATE_EXTENSION}"))
}

/// `templates/flows/` of the frozen skill.
pub fn flows_dir() -> PathBuf {
    templates_dir().join(FLOWS_DIR)
}

/// `templates/recipes/<name>` of the frozen skill.
pub fn recipe(name: &str) -> PathBuf {
    templates_dir().join(RECIPES_DIR).join(name)
}

/// `hooks/<name>` of the frozen skill.
pub fn hook_example(name: &str) -> PathBuf {
    skill_root().join(HOOKS_DIR).join(name)
}

/// Data directory of the mock Jira, GitHub and Sonar servers.
pub fn mocks_dir() -> PathBuf {
    dir().join(MOCKS_DIR)
}

/// Inputs that satisfy every required input of every frozen template.
pub fn template_inputs() -> PathBuf {
    dir().join(TEMPLATE_INPUTS_FILE)
}
