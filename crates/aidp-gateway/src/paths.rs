use aidp_core::vault::home_dir;
use anyhow::anyhow;
use std::path::PathBuf;

/// Where the daemon persists everything: `$AIDP_GATEWAY_DIR` or `~/.aidp/gateway`.
/// Fails when the variable is unset and no home directory resolves.
pub fn state_dir() -> anyhow::Result<PathBuf> {
    if let Ok(d) = std::env::var("AIDP_GATEWAY_DIR") {
        if !d.is_empty() {
            return Ok(PathBuf::from(d));
        }
    }
    let home = home_dir().ok_or_else(|| {
        anyhow!("cannot resolve the gateway state directory: set AIDP_GATEWAY_DIR or a home directory (USERPROFILE / HOME)")
    })?;
    Ok(home.join(".aidp").join("gateway"))
}

/// The ai-dev-pipeline skill root: `$AIDP_SKILL_ROOT`, else the newest generation installed.
pub fn skill_root() -> Option<PathBuf> {
    if let Ok(r) = std::env::var("AIDP_SKILL_ROOT") {
        if !r.is_empty() {
            return Some(PathBuf::from(r));
        }
    }
    let home = home_dir()?;
    for gen in ["v7", "v6", "v5", "v4"] {
        let p = home
            .join(".claude")
            .join("skills")
            .join(format!("ai-dev-pipeline-{gen}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// "v7" for `.../ai-dev-pipeline-v7` — names the `/run-pipeline-<gen>` slash command.
pub fn skill_generation() -> String {
    skill_root()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .map(|n| n.trim_start_matches("ai-dev-pipeline-").to_string())
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| "v7".to_string())
}

/// Resolves the bundled command even when the skill directory is named by its content hash.
pub(crate) fn configured_skill_generation(root: &std::path::Path) -> String {
    use crate::models::constants::skill_paths::*;
    if let Some(commands) = aidp_core::toolset::commands_dir() {
        for generation in GENERATIONS {
            if commands
                .join(format!(
                    "{RUN_COMMAND_PREFIX}{generation}{COMMAND_EXTENSION}"
                ))
                .is_file()
            {
                return generation.to_string();
            }
        }
    }
    root.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(SKILL_DIRECTORY_PREFIX))
        .filter(|generation| GENERATIONS.contains(generation))
        .unwrap_or(crate::models::constants::engine::ENGINE_V7_ALIAS)
        .to_string()
}
