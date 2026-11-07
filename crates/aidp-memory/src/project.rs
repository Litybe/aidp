//! Project lookup: registered key or id, else a direct path.

use crate::error::{MemoryError, Result};
use crate::models::constants::paths::{BACKSLASH, PROJECTS_FILE, SEPARATOR};
use crate::models::project_ref::ProjectRef;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Registered {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    path: String,
}

fn registry(gateway_dir: Option<&Path>) -> Vec<Registered> {
    gateway_dir
        .and_then(|dir| std::fs::read_to_string(dir.join(PROJECTS_FILE)).ok())
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn comparable(path: &str) -> String {
    path.replace(BACKSLASH, &SEPARATOR.to_string())
        .trim_end_matches(SEPARATOR)
        .to_lowercase()
}

/// Resolves `--project`: a registered id or name, a directory, or the current directory.
///
/// `gateway_dir` holds the daemon `projects.json`; the key of a registered
/// project is its name, the key of any other directory is its folder name.
pub fn resolve_project(
    argument: Option<&str>,
    gateway_dir: Option<&Path>,
    cwd: &Path,
) -> Result<ProjectRef> {
    let projects = registry(gateway_dir);
    if let Some(name) = argument {
        if let Some(found) = projects
            .iter()
            .find(|project| project.id == name || project.name == name)
        {
            return Ok(ProjectRef {
                key: found.name.clone(),
                root: PathBuf::from(&found.path),
            });
        }
    }
    let directory = match argument {
        Some(raw) => {
            let given = PathBuf::from(raw);
            if given.is_absolute() {
                given
            } else {
                cwd.join(given)
            }
        }
        None => cwd.to_path_buf(),
    };
    if !directory.is_dir() {
        return Err(MemoryError::NotFound(format!(
            "project not found (not registered, not a directory): {}",
            argument.unwrap_or_default()
        )));
    }
    let wanted = comparable(&directory.to_string_lossy());
    let key = projects
        .iter()
        .find(|project| comparable(&project.path) == wanted)
        .map(|project| project.name.clone())
        .or_else(|| {
            directory
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| wanted.clone());
    Ok(ProjectRef {
        key,
        root: directory,
    })
}

#[cfg(test)]
#[path = "../tests/unit/project_tests.rs"]
mod tests;
