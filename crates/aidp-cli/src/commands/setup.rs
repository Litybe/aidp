use super::CommandOutput;
use crate::args::SetupOp;
use crate::ctx::{Ctx, AGENTS_DIR, CLAUDE_DIR, SKILL_ROOT_ENV};
use crate::models::envelopes::{MergeHooksDone, MergeHooksDryRun, SetupAgentsDone};
use aidp_core::hooks::{merge_hooks, HookSettings};
use aidp_core::models::enums::AgentRole;
use std::path::{Path, PathBuf};

const WINDOWS_COMMAND_KEY: &str = "_windows_command";
const COMMAND_KEY: &str = "command";
const HOOK_TYPE_KEY: &str = "type";
const HOOK_TYPE_COMMAND: &str = "command";
const COMMENT_KEY_PREFIX: char = '_';
const BACKUP_EXTENSION: &str = "bak";
const RESTRICTED_ROLES: [AgentRole; 3] =
    [AgentRole::Analyzer, AgentRole::Planner, AgentRole::Reviewer];

pub fn run(ctx: &Ctx, op: SetupOp) -> anyhow::Result<CommandOutput> {
    match op {
        SetupOp::MergeHooks {
            example,
            target,
            dry_run,
        } => merge_settings_hooks(&example, &target, dry_run),
        SetupOp::Agents { dest, force } => setup_agents(ctx, dest, force),
    }
}

/// The example hooks file, with the OS-specific commands resolved and `_comment` keys dropped.
fn load_example(example: &Path) -> anyhow::Result<HookSettings> {
    let example_text = std::fs::read_to_string(example)
        .map_err(|e| anyhow::anyhow!("read example {}: {e}", example.display()))?;
    let mut example_json: serde_json::Value = serde_json::from_str(&example_text)
        .map_err(|e| anyhow::anyhow!("parse example JSON: {e}"))?;
    resolve_platform_commands(&mut example_json);
    if let Some(obj) = example_json.as_object_mut() {
        obj.retain(|k, _| !k.starts_with(COMMENT_KEY_PREFIX));
    }
    serde_json::from_value(example_json).map_err(|e| anyhow::anyhow!("parse example JSON: {e}"))
}

/// The target settings file, or empty settings when it is absent or blank.
fn load_target(target: &Path) -> anyhow::Result<HookSettings> {
    if !target.exists() {
        return Ok(HookSettings::default());
    }
    let text = std::fs::read_to_string(target)
        .map_err(|e| anyhow::anyhow!("read target {}: {e}", target.display()))?;
    if text.trim().is_empty() {
        return Ok(HookSettings::default());
    }
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("parse target JSON: {e}"))
}

fn backup_path(target: &Path) -> PathBuf {
    target.with_extension(
        target
            .extension()
            .map(|e| format!("{}.{BACKUP_EXTENSION}", e.to_string_lossy()))
            .unwrap_or_else(|| BACKUP_EXTENSION.to_string()),
    )
}

fn merge_settings_hooks(
    example: &Path,
    target: &Path,
    dry_run: bool,
) -> anyhow::Result<CommandOutput> {
    let example_settings = load_example(example)?;
    let mut target_settings = load_target(target)?;
    let added = merge_hooks(&mut target_settings, &example_settings);
    let merged = serde_json::to_value(&target_settings)?;

    if dry_run {
        let report = serde_json::to_value(MergeHooksDryRun {
            dry_run: true,
            hooks_added: added,
            preview: &merged,
            target: target.display().to_string(),
        })?;
        return Ok(CommandOutput::text(serde_json::to_string_pretty(&report)?));
    }

    if target.exists() {
        let _ = std::fs::copy(target, backup_path(target));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(target, serde_json::to_string_pretty(&merged)?)
        .map_err(|e| anyhow::anyhow!("write target {}: {e}", target.display()))?;
    let done = serde_json::to_value(MergeHooksDone {
        hooks_added: added,
        ok: true,
        target: target.display().to_string(),
    })?;
    Ok(CommandOutput::text(serde_json::to_string(&done)?))
}

/// `aidp setup agents`: writes the restricted `aidp-<role>-ro.md` personas to the registry.
/// An existing target refuses the command unless `force` is set; `executor` has no entry.
fn setup_agents(ctx: &Ctx, dest: Option<PathBuf>, force: bool) -> anyhow::Result<CommandOutput> {
    let root = ctx.skill_root.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "skill root not found — set {SKILL_ROOT_ENV} or run aidp from the skill directory"
        )
    })?;
    let agents_dir = root.join(AGENTS_DIR);
    let dest_dir = dest.unwrap_or_else(|| {
        ctx.home
            .as_ref()
            .map(|h| h.join(CLAUDE_DIR).join(AGENTS_DIR))
            .unwrap_or_else(|| PathBuf::from(CLAUDE_DIR).join(AGENTS_DIR))
    });
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| anyhow::anyhow!("create {}: {e}", dest_dir.display()))?;

    let mut rendered: Vec<(PathBuf, String)> = Vec::new();
    for role in RESTRICTED_ROLES {
        let role_name = role.as_str();
        let src = agents_dir.join(format!("{role_name}.md"));
        let body = std::fs::read_to_string(&src)
            .map_err(|e| anyhow::anyhow!("read source persona {}: {e}", src.display()))?;
        let model = role.default_model().as_str();
        let text =
            aidp_core::toolset::render_registry_md(role_name, model, &body).ok_or_else(|| {
                anyhow::anyhow!("aidp_core::toolset has no matrix entry for role `{role_name}`")
            })?;
        let name = aidp_core::toolset::restricted_type_name(role_name);
        rendered.push((dest_dir.join(format!("{name}.md")), text));
    }

    if !force {
        let existing: Vec<String> = rendered
            .iter()
            .filter(|(p, _)| p.exists())
            .map(|(p, _)| p.display().to_string())
            .collect();
        if !existing.is_empty() {
            anyhow::bail!(
                "refusing to overwrite {} existing registry file(s) without --force: {}",
                existing.len(),
                existing.join(", ")
            );
        }
    }

    let mut written: Vec<String> = Vec::new();
    for (path, text) in &rendered {
        std::fs::write(path, text).map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
        written.push(path.display().to_string());
    }

    let done = serde_json::to_value(SetupAgentsDone {
        dest: dest_dir.display().to_string(),
        force,
        ok: true,
        written,
    })?;
    Ok(CommandOutput::text(serde_json::to_string(&done)?))
}

/// In each hook object, swap `_windows_command` <-> `command` based on host OS,
/// then drop the leftover underscore field.
fn resolve_platform_commands(value: &mut serde_json::Value) {
    let is_windows = cfg!(windows);
    walk_hook_objects(value, &mut |obj| {
        if let Some(win) = obj.get(WINDOWS_COMMAND_KEY).cloned() {
            if is_windows {
                obj.insert(COMMAND_KEY.to_string(), win);
            }
            obj.remove(WINDOWS_COMMAND_KEY);
        }
        if !is_windows {
            obj.remove(WINDOWS_COMMAND_KEY);
        }
    });
}

/// Walk every JSON object that looks like a hook entry (has `"type": "command"`),
/// calling `cb` on it.
fn walk_hook_objects<F: FnMut(&mut serde_json::Map<String, serde_json::Value>)>(
    value: &mut serde_json::Value,
    cb: &mut F,
) {
    match value {
        serde_json::Value::Object(map) => {
            let is_hook =
                map.get(HOOK_TYPE_KEY).and_then(|t| t.as_str()) == Some(HOOK_TYPE_COMMAND);
            if is_hook {
                cb(map);
            }
            for (_, child) in map.iter_mut() {
                walk_hook_objects(child, cb);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                walk_hook_objects(child, cb);
            }
        }
        _ => {}
    }
}
