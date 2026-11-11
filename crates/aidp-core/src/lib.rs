//! aidp-core — types, schemas, scope, render helpers.
//!
//! Most of v3's logic in `aidp-render-task-prompt`, `aidp-parse-agent-reply`,
//! `aidp-scope-check`, `aidp-resolve-skills`, `aidp-validate-pipeline`,
//! `aidp-topo-sort`, `aidp-resolve-with`, `aidp-evaluate-when`,
//! `aidp-discover-pipelines` is ported here as library functions and exposed
//! through `aidp-cli` sub-commands.

pub mod aidp_binary;
pub mod clock;
pub mod compose;
pub mod config;
pub mod context_provider;
pub mod cost;
pub mod errors;
pub mod exec_plan;
pub mod exit_status;
pub mod expr;
pub mod findings;
pub mod git;
pub mod gitlog;
pub mod guard;
pub mod hooks;
pub mod id;
pub mod kg;
pub mod knowledge;
pub mod knowledge_gc;
pub mod knowledge_stale;
pub mod model_policy;
pub mod models;
pub mod obsidian;
pub mod pipeline;
pub mod plan;
pub mod process_runner;
pub mod program;
pub mod prompt;
pub mod render;
pub mod run;
pub mod safe_name;
pub mod scope;
pub mod skill_compat;
pub mod skill_rules;
pub mod skills;
pub mod stack;
pub mod text;
pub mod tokens;
pub mod toolset;
pub mod vault;
pub mod vault_leak;
pub mod verify;
pub mod workflow_emit;

pub use errors::Error;
pub use exit_status::{ExitStatus, TypedExit};
pub use render::render_value;

/// Output format shared by every command of the binary.
/// `Text` is the human rendering; a command with none falls back to pretty JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    #[default]
    JsonCompact,
    JsonPretty,
    Ndjson,
    Text,
}

impl Format {
    const JSON_COMPACT: &'static str = "json-compact";
    const JSON_PRETTY: &'static str = "json-pretty";
    const NDJSON: &'static str = "ndjson";
    const TEXT: &'static str = "text";

    /// The command-line spelling of the format.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonCompact => Self::JSON_COMPACT,
            Self::JsonPretty => Self::JSON_PRETTY,
            Self::Ndjson => Self::NDJSON,
            Self::Text => Self::TEXT,
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for Format {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            Self::JSON_COMPACT => Ok(Self::JsonCompact),
            Self::JSON_PRETTY => Ok(Self::JsonPretty),
            Self::NDJSON => Ok(Self::Ndjson),
            Self::TEXT => Ok(Self::Text),
            other => Err(format!(
                "invalid format: {other}; expected json-compact, json-pretty, ndjson, or text"
            )),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/format_tests.rs"]
mod format_tests;

#[cfg(test)]
#[path = "../tests/unit/pr_comments_template_tests.rs"]
mod pr_comments_template_tests;
