//! C10 (Lot 3, Capitalisation): per-stage tool restriction via a specialized
//! `agentType`, OFF by default.
//!
//! ## Why this exists
//!
//! `scope:` on a stage exists, at the moment a sub-agent actually works, only
//! as **text** — `prompt.rs` serializes `mode/allowed/denied/writes` into
//! markdown bullets in the prompt. Nothing stops the sub-agent from ignoring
//! those bullets and calling `Edit`/`Write` outside its lane; the only real
//! enforcement is `aidp scope verify` (C7), which checks what was actually
//! written **after the fact**. This module adds a second, narrower layer:
//! resolve a *different* Claude Code `agentType` — one whose own `.md`
//! persona in the `~/.claude/agents/` registry declares a restricted
//! `tools:` allowlist — for roles that never need to edit at all
//! (`analyzer`/`planner`/`reviewer`). It cannot replace C7; it can make a
//! whole class of accidental edits impossible for the model to even attempt.
//!
//! ## Two non-negotiable guardrails
//!
//! 1. **`executor` is never restricted**, at any scope, under any
//!    enforcement value. `~/.claude/agents/executor.md` is untouched by this
//!    lot. Restricting the one role every template actually uses to make
//!    changes would break every run in flight.
//! 2. **The default is `off`.** [`agent_type_for`] returns the bare role
//!    unchanged unless `enforcement == "strict"` — an explicit, per-pipeline
//!    opt-in (`settings.tool_enforcement`) a human makes only after a green
//!    `aidp doctor` on the target machine. An `agentType` the harness cannot
//!    resolve fails dispatch at the very first stage that uses it, so this
//!    is deliberately conservative.
//!
//! ## The registry that actually matters
//!
//! `opts.agentType` resolves from the SAME registry the Claude Code `Agent`
//! tool itself reads personas from: `~/.claude/agents/*.md` ([`registry_dir`]).
//! It is **not** `<skill_root>/agents/*.md` — that worktree directory holds
//! four files with the *same four names* (analyzer/executor/planner/reviewer)
//! but is a *different* set of files, injected as prompt text and read only
//! by `aidp agents list`. This module writes to the real registry only via an
//! explicit human command (`aidp setup agents`, in `main.rs`) — never as a
//! side effect of emitting or running a pipeline.
//!
//! ## Honest caveat: `Bash` weakens the "read-only" allowlist
//!
//! [`tools_for`] keeps `Bash` in the read-only roles' allowlist because an
//! `analyzer` genuinely needs it (`git diff`, `dotnet build`, …) to do its
//! job at all. That **weakens** the restriction: `Bash` can itself write
//! files (`echo > foo`, `rm`, …), so this allowlist stops a restricted role
//! from reaching the dedicated *editing* tools (`Edit`/`Write`/`MultiEdit`/
//! `NotebookEdit`) — it does **not** stop writes executed through a shell.
//! The real write boundary stays C7's after-the-fact `aidp scope verify`,
//! already merged on this branch: this module narrows which *tools* a role
//! can reach for; C7 checks what actually landed on disk, regardless of
//! which tool put it there.
//!
//! ## Explicit fallback
//!
//! If `tools:` restriction turns out not to be honored by the harness, or
//! its frontmatter shape changes underneath this module: `tool_enforcement`
//! simply stays `"off"` (the shipped default), this file stays a few dozen
//! inert lines, and C7's scope verification remains the only real guarantee
//! — exactly as it is today. Nothing about the rest of the engine depends on
//! `strict` ever being turned on. The frontmatter shape itself
//! ([`render_registry_md`]) is a genuine external Claude Code contract that
//! can change independently of this codebase; it is confined to this one
//! file on purpose, so a format change has exactly one place to be fixed.

use crate::models::enums::{AgentRole, ToolEnforcement};
use crate::pipeline::{self, Pipeline};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// An allow/deny pair for a restricted agentType's Claude Code `tools:`
/// frontmatter. `allow` is what actually narrows the toolbox; `deny` is kept
/// alongside it purely so a human reading the matrix (or the generated
/// frontmatter's `description:`) sees at a glance what a restricted role
/// loses, without having to diff it against the full built-in tool list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSet {
    pub allow: Vec<&'static str>,
    pub deny: Vec<&'static str>,
}

/// The read-only allowlist shared by every restricted role today. See the
/// module doc's "Honest caveat" section for why `Bash` stays in it.
const READ_ONLY_ALLOW: [&str; 5] = ["Read", "Grep", "Glob", "Bash", "TodoWrite"];
/// The editing tools a read-only role loses. Documentation only (`deny` is
/// not itself sent to Claude Code — an allowlist is closed by construction),
/// kept so the matrix names what it removes, not just what it keeps.
const READ_ONLY_DENY: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// The tool-restriction matrix — **deliberately partial on this first pass**.
/// `analyzer` / `planner` / `reviewer`, at ANY `scope_mode`, resolve to the
/// same read-only allowlist: none of the three needs to edit the working
/// tree to do its job. `executor` (and any role this lot does not name, e.g.
/// a template author's own custom `agent:`) gets `None` — no restriction is
/// defined for it, so [`agent_type_for`] leaves it as the bare role.
///
/// `scope_mode` is accepted (not merely ignored) because a future, less
/// partial matrix is expected to vary the allowlist by scope (e.g. a
/// `tests`-scoped executor might one day get its own restricted variant);
/// nothing in this first jet does that yet.
pub fn tools_for(role: &str, scope_mode: &str) -> Option<ToolSet> {
    let _ = scope_mode; // reserved for a future, scope-varying matrix.
    match AgentRole::parse(role) {
        AgentRole::Analyzer | AgentRole::Planner | AgentRole::Reviewer => Some(ToolSet {
            allow: READ_ONLY_ALLOW.to_vec(),
            deny: READ_ONLY_DENY.to_vec(),
        }),
        _ => None,
    }
}

/// The restricted agentType name for `role`, e.g. `analyzer` → `aidp-analyzer-ro`.
/// Factored out (rather than inlined at each call site) because it is the
/// naming half of the same external contract [`render_registry_md`] confines:
/// `agent_type_for`, `render_registry_md`, and the doctor/emit-time registry
/// checks below must all agree on this exact string.
pub fn restricted_type_name(role: &str) -> String {
    format!("aidp-{role}-ro")
}

/// Resolve the `agentType` string `workflow_emit::agent_opts_with_label`
/// emits for a stage's role.
///
/// `enforcement != Strict` (the default, `Off`) ALWAYS returns `role`
/// unchanged — no run in flight changes shape until a human opts a pipeline
/// into `strict` (guardrail 2). `role == "executor"` ALWAYS returns
/// `"executor"` unchanged, regardless of `scope_mode` or `enforcement`
/// (guardrail 1) — `~/.claude/agents/executor.md` is never substituted.
/// Any other role with no entry in [`tools_for`] (a template's own custom
/// `agent:`) also passes through unchanged: this module does not invent a
/// restriction for a role it does not know.
pub fn agent_type_for(role: &str, scope_mode: &str, enforcement: ToolEnforcement) -> String {
    if enforcement != ToolEnforcement::Strict || AgentRole::parse(role) == AgentRole::Executor {
        return role.to_string();
    }
    match tools_for(role, scope_mode) {
        Some(_) => restricted_type_name(role),
        None => role.to_string(),
    }
}

/// Render the generated frontmatter + persona body for a restricted agent
/// `.md` file: `name` / `description` / `model` / `tools:`, followed by the
/// SOURCE persona's own body (`<skill_root>/agents/<role>.md`'s content)
/// verbatim and unmodified. `None` when `role` has no entry in
/// [`tools_for`] — there is nothing to generate for a role this lot does not
/// restrict.
///
/// This is the one function that would need to change if Claude Code's own
/// agent-frontmatter shape changes — see the module doc's "Explicit
/// fallback" section.
pub fn render_registry_md(role: &str, model: &str, source_body: &str) -> Option<String> {
    let ts = tools_for(role, "")?;
    let name = restricted_type_name(role);
    let tools_line = ts.allow.join(", ");
    Some(format!(
        "---\nname: {name}\ndescription: Read-only variant of `{role}`, generated by `aidp setup agents` for settings.tool_enforcement: strict (allow [{allow}], deny [{deny}] — deny is documentation only, the allowlist is closed by construction).\nmodel: {model}\ntools: {tools_line}\n---\n\n{body}",
        allow = tools_line,
        deny = ts.deny.join(", "),
        body = source_body.trim_start(),
    ))
}

/// The registry `opts.agentType` resolves personas from at runtime — the
/// SAME directory the Claude Code `Agent` tool itself reads, `~/.claude/agents/`.
/// Resolved via `USERPROFILE`/`HOME` directly (this crate does not depend on
/// `aidp-cli`, so this intentionally duplicates — rather than imports —
/// `main.rs::home_dir`'s one-line rule; the two cannot disagree because
/// both read the same two env vars in the same order).
pub fn registry_dir() -> Option<PathBuf> {
    claude_subdir(AGENTS_SUBDIR)
}

/// The slash-command directory Claude Code reads, `~/.claude/commands/`: the one source of commands.
pub fn commands_dir() -> Option<PathBuf> {
    claude_subdir(COMMANDS_SUBDIR)
}

fn claude_subdir(subdir: &str) -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(CLAUDE_HOME_DIR).join(subdir))
}

const CLAUDE_HOME_DIR: &str = ".claude";
const AGENTS_SUBDIR: &str = "agents";
const COMMANDS_SUBDIR: &str = "commands";

/// True when `<dir>/<agent_type>.md` exists AND declares a `tools:` line.
/// This is the exact rule both `doctor`'s `agent_toolsets` check (main.rs)
/// and `workflow emit`'s strict-mode admission gate must agree on — factored
/// here so neither can drift from the other.
pub fn registry_has_toolset(dir: &Path, agent_type: &str) -> bool {
    std::fs::read_to_string(dir.join(format!("{agent_type}.md")))
        .map(|s| s.lines().any(|l| l.trim_start().starts_with("tools:")))
        .unwrap_or(false)
}

/// Every restricted agentType this pipeline's stages — via
/// `pipeline::flat_stages` (top-level stages plus loop bodies; composites are
/// already expanded into the effective pipeline before this runs) — would
/// resolve to under `enforcement: "strict"`. A stage whose own role is never
/// restricted (executor, or a custom `agent:` [`tools_for`] does not know)
/// contributes nothing.
pub fn strict_agent_types(pipe: &Pipeline) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for st in pipeline::flat_stages(pipe) {
        let role = st.agent.as_deref().unwrap_or(AgentRole::Executor.as_str());
        let scope_mode = st
            .scope
            .as_ref()
            .and_then(|s| s.mode.as_deref())
            .unwrap_or("stage_dir_only");
        let restricted = agent_type_for(role, scope_mode, ToolEnforcement::Strict);
        if restricted != role {
            out.insert(restricted);
        }
    }
    out
}

/// Which of [`strict_agent_types`] are missing from `dir` — file absent, or
/// present without a `tools:` line. Empty ⇒ `strict` is safe to emit for
/// this pipeline; this is the condition `doctor`'s `agent_toolsets` check
/// reports and `workflow emit` refuses to bypass (see `main.rs`). `dir:
/// None` (HOME/USERPROFILE unresolved) reports every needed type as
/// missing rather than silently passing.
pub fn missing_agent_toolsets(pipe: &Pipeline, dir: Option<&Path>) -> Vec<String> {
    strict_agent_types(pipe)
        .into_iter()
        .filter(|t| match dir {
            Some(d) => !registry_has_toolset(d, t),
            None => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Stage;

    const SCOPE_MODES: [&str; 6] = [
        "stage_dir_only",
        "production",
        "tests",
        "repo_wide",
        "custom",
        "made-up-mode",
    ];
    const ENFORCEMENTS: [ToolEnforcement; 2] = [ToolEnforcement::Off, ToolEnforcement::Strict];

    /// The plan's own required assertion: an assertion on the matrix itself,
    /// for every scope and every enforcement value — executor must never
    /// come back restricted.
    #[test]
    fn executor_is_never_restricted() {
        assert!(
            tools_for("executor", "production").is_none(),
            "the matrix must define no ToolSet at all for executor"
        );
        for scope_mode in SCOPE_MODES {
            for enforcement in ENFORCEMENTS {
                assert_eq!(
                    agent_type_for("executor", scope_mode, enforcement),
                    "executor",
                    "executor must resolve to the bare role (scope_mode={scope_mode:?}, enforcement={enforcement:?})"
                );
            }
        }
    }

    #[test]
    fn off_always_emits_the_bare_role_for_every_known_role() {
        for role in ["analyzer", "planner", "reviewer", "executor", "custom-role"] {
            for scope_mode in SCOPE_MODES {
                assert_eq!(
                    agent_type_for(role, scope_mode, ToolEnforcement::Off),
                    role,
                    "enforcement=off must never substitute a role (role={role:?}, scope_mode={scope_mode:?})"
                );
            }
        }
    }

    #[test]
    fn strict_restricts_the_three_read_only_roles_at_every_scope() {
        for role in ["analyzer", "planner", "reviewer"] {
            assert!(tools_for(role, "stage_dir_only").is_some());
            for scope_mode in SCOPE_MODES {
                assert_eq!(
                    agent_type_for(role, scope_mode, ToolEnforcement::Strict),
                    format!("aidp-{role}-ro"),
                    "role={role:?} scope_mode={scope_mode:?}"
                );
            }
        }
    }

    #[test]
    fn strict_leaves_an_unknown_custom_role_unchanged() {
        assert_eq!(
            agent_type_for(
                "some-template-specific-role",
                "production",
                ToolEnforcement::Strict
            ),
            "some-template-specific-role"
        );
    }

    #[test]
    fn read_only_allowlist_keeps_bash_and_denies_the_editing_tools() {
        let ts = tools_for("reviewer", "stage_dir_only").expect("reviewer has a matrix entry");
        assert!(
            ts.allow.contains(&"Bash"),
            "see module doc: Bash stays, deliberately weakening this allowlist"
        );
        for t in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
            assert!(ts.deny.contains(&t));
            assert!(!ts.allow.contains(&t));
        }
    }

    #[test]
    fn render_registry_md_embeds_tools_line_and_source_body_verbatim() {
        let out = render_registry_md("reviewer", "sonnet", "You are the reviewer.\nBe careful.\n")
            .expect("reviewer has a matrix entry");
        assert!(out.starts_with("---\nname: aidp-reviewer-ro\n"));
        assert!(out.contains("model: sonnet\n"));
        assert!(out.contains("tools: Read, Grep, Glob, Bash, TodoWrite\n"));
        assert!(out.ends_with("You are the reviewer.\nBe careful.\n"));
    }

    #[test]
    fn render_registry_md_is_none_for_an_unrestricted_role() {
        assert!(render_registry_md("executor", "sonnet", "body").is_none());
    }

    #[test]
    fn missing_agent_toolsets_reports_every_needed_type_when_the_registry_is_absent() {
        let pipe = fixture_pipeline(vec![
            Stage {
                agent: Some("reviewer".into()),
                ..bare_stage("review")
            },
            Stage {
                agent: Some("executor".into()),
                ..bare_stage("code")
            },
        ]);
        let missing = missing_agent_toolsets(&pipe, None);
        assert_eq!(missing, vec!["aidp-reviewer-ro".to_string()]);
    }

    #[test]
    fn missing_agent_toolsets_is_empty_once_the_registry_has_the_file_and_tools_line() {
        let dir = std::env::temp_dir().join(format!(
            "aidp-toolset-test-{}-{}",
            std::process::id(),
            "registry-present"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("aidp-reviewer-ro.md"),
            "---\nname: aidp-reviewer-ro\ntools: Read, Grep, Glob, Bash, TodoWrite\n---\nbody\n",
        )
        .unwrap();
        let pipe = fixture_pipeline(vec![Stage {
            agent: Some("reviewer".into()),
            ..bare_stage("review")
        }]);
        let missing = missing_agent_toolsets(&pipe, Some(&dir));
        assert!(missing.is_empty(), "missing: {missing:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fixture_pipeline(stages: Vec<Stage>) -> Pipeline {
        Pipeline {
            version: 5,
            id: "toolset-test".to_string(),
            name: String::new(),
            description: String::new(),
            kind: "pipeline".to_string(),
            policy: Vec::new(),
            inputs: Default::default(),
            settings: Default::default(),
            stages,
        }
    }

    fn bare_stage(name: &str) -> Stage {
        Stage {
            name: name.to_string(),
            description: String::new(),
            policy: Vec::new(),
            skills_by: None,
            r#type: None,
            uses: None,
            composite_entry: None,
            composite_when: vec![],
            agent: None,
            model: None,
            effort: None,
            provider: None,
            skills: Vec::new(),
            needs: Vec::new(),
            inputs: Default::default(),
            outputs: Vec::new(),
            with: Default::default(),
            when: None,
            gate: None,
            scope: None,
            isolation: None,
            tools: None,
            fan_out: None,
            merge_findings: None,
            on_harness_gap: None,
            plan_tasks: None,
            max_iterations: None,
            on_exhausted: None,
            knowledge: None,
            body: Vec::new(),
        }
    }
}
