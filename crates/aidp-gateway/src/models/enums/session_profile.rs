//! Interactive or autonomous session profile and its launch-time effects.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::constants::session_profile::{
    ALLOWED_TOOLS_FLAG, ALLOW_PUSH_HEAD_PREFIX, ALLOW_PUSH_ORIGIN_PREFIX, ALLOW_PUSH_TRACKING,
    ASK_USER_QUESTION_TOOL, AUTONOMOUS_ENABLED, AUTONOMOUS_ENV, BRANCH_DOUBLE_DOT,
    BRANCH_FORBIDDEN_CHARS, BRANCH_FORBIDDEN_LEADING, DENY_RULES, DISALLOWED_TOOLS_FLAG,
    INVALID_PUSH_BRANCH_MESSAGE, PERMISSIONS_FILE, PUSH_BRANCH_DENY_RULES, RULE_SUFFIX,
};
use crate::models::session_permissions::SessionPermissions;
use crate::store::write_json;

/// How a session talks to its human: it may ask questions, or it decides alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionProfile {
    #[default]
    Interactive,
    Autonomous,
}

impl SessionProfile {
    /// Autonomous when the session environment carries `AIDP_AUTONOMOUS=1`.
    pub fn from_env(env: &BTreeMap<String, String>) -> Self {
        match env.get(AUTONOMOUS_ENV).map(String::as_str) {
            Some(AUTONOMOUS_ENABLED) => Self::Autonomous,
            _ => Self::Interactive,
        }
    }

    /// Unchanged for interactive sessions; autonomous ones disallow `AskUserQuestion` and the deny rules.
    pub fn apply_to_launch_tail(self, mut tail: Vec<String>) -> Vec<String> {
        if self == Self::Autonomous {
            tail.push(DISALLOWED_TOOLS_FLAG.to_string());
            tail.push(ASK_USER_QUESTION_TOOL.to_string());
            tail.extend(SessionPermissions::autonomous().deny);
        }
        tail
    }

    /// Writes `permissions.json` (atomic) into `state_dir` for autonomous sessions; a no-op otherwise.
    pub fn write_permissions(self, state_dir: &Path) -> anyhow::Result<()> {
        if self != Self::Autonomous {
            return Ok(());
        }
        write_json(
            &state_dir.join(PERMISSIONS_FILE),
            &SessionPermissions::autonomous(),
        )
    }

    /// Rejects empty names, whitespace, `..`, a leading `-` and rule glob characters.
    pub fn validate_push_branch(branch: &str) -> anyhow::Result<()> {
        let invalid = branch.is_empty()
            || branch
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
            || branch.contains(BRANCH_DOUBLE_DOT)
            || branch.starts_with(BRANCH_FORBIDDEN_LEADING)
            || branch.contains(BRANCH_FORBIDDEN_CHARS);
        if invalid {
            anyhow::bail!("{INVALID_PUSH_BRANCH_MESSAGE}: {branch:?}");
        }
        Ok(())
    }

    /// Allow rules for `git push origin <branch>`, `HEAD:<branch>` and a plain `git push`.
    pub fn push_branch_allow_rules(branch: &str) -> anyhow::Result<Vec<String>> {
        Self::validate_push_branch(branch)?;
        Ok(vec![
            ALLOW_PUSH_TRACKING.to_string(),
            format!("{ALLOW_PUSH_ORIGIN_PREFIX}{branch}{RULE_SUFFIX}"),
            format!("{ALLOW_PUSH_HEAD_PREFIX}{branch}{RULE_SUFFIX}"),
        ])
    }

    /// Autonomous permissions; with a push branch the deny list gains the extra push rules.
    pub fn permissions_for_push_branch(
        self,
        branch: Option<&str>,
    ) -> anyhow::Result<SessionPermissions> {
        let mut permissions = SessionPermissions::autonomous();
        if let Some(branch) = branch {
            Self::validate_push_branch(branch)?;
            permissions
                .deny
                .extend(PUSH_BRANCH_DENY_RULES.iter().map(|rule| rule.to_string()));
        }
        Ok(permissions)
    }

    /// Like `apply_to_launch_tail`; with a push branch, the extra denies and push allow rules follow.
    pub fn apply_to_launch_tail_for_push_branch(
        self,
        tail: Vec<String>,
        branch: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        let mut tail = self.apply_to_launch_tail(tail);
        if let (Self::Autonomous, Some(branch)) = (self, branch) {
            let permissions = self.permissions_for_push_branch(Some(branch))?;
            tail.extend(permissions.deny.into_iter().skip(DENY_RULES.len()));
            tail.push(ALLOWED_TOOLS_FLAG.to_string());
            tail.extend(Self::push_branch_allow_rules(branch)?);
        }
        Ok(tail)
    }

    /// Writes `permissions.json` with the push-branch deny rules; a no-op unless autonomous.
    pub fn write_permissions_for_push_branch(
        self,
        state_dir: &Path,
        branch: Option<&str>,
    ) -> anyhow::Result<()> {
        if self != Self::Autonomous {
            return Ok(());
        }
        write_json(
            &state_dir.join(PERMISSIONS_FILE),
            &self.permissions_for_push_branch(branch)?,
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/session_profile_tests.rs"]
mod tests;
