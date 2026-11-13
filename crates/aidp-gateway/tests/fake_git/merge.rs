//! Three-way merge of whole-tree snapshots and unified-diff application.

use std::collections::BTreeSet;
use std::path::Path;

use super::constants::*;
use super::state::Files;

/// The merged files and the paths both sides changed differently.
pub fn three_way(base: &Files, ours: &Files, theirs: &Files) -> (Files, Vec<String>) {
    let mut merged = Files::new();
    let mut conflicts = Vec::new();
    let paths: BTreeSet<&String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    for path in paths {
        let (before, mine, other) = (base.get(path), ours.get(path), theirs.get(path));
        let chosen = if mine == other || other == before {
            mine
        } else if mine == before {
            other
        } else {
            conflicts.push(path.clone());
            mine
        };
        if let Some(bytes) = chosen {
            merged.insert(path.clone(), bytes.clone());
        }
    }
    (merged, conflicts)
}

struct Hunk {
    removed: Vec<String>,
    added: Vec<String>,
}

fn side_path(line: &str, prefix: &str) -> Option<String> {
    let name = line.strip_prefix(prefix)?.split('\t').next()?.trim();
    if name == NO_PATCH_FILE {
        return None;
    }
    let name = name
        .strip_prefix(DIFF_PATH_LEFT)
        .or_else(|| name.strip_prefix(DIFF_PATH_RIGHT))
        .unwrap_or(name);
    Some(name.to_string())
}

fn parse(text: &str) -> Vec<(String, Vec<Hunk>)> {
    let mut files: Vec<(String, Vec<Hunk>)> = Vec::new();
    let mut old_name: Option<String> = None;
    for line in text.lines() {
        if line.starts_with(PATCH_OLD_PREFIX) {
            old_name = side_path(line, PATCH_OLD_PREFIX);
        } else if line.starts_with(PATCH_NEW_PREFIX) {
            let name = side_path(line, PATCH_NEW_PREFIX).or_else(|| old_name.take());
            if let Some(name) = name {
                files.push((name, Vec::new()));
            }
        } else if line.starts_with(PATCH_HUNK_PREFIX) {
            if let Some((_, hunks)) = files.last_mut() {
                hunks.push(Hunk {
                    removed: Vec::new(),
                    added: Vec::new(),
                });
            }
        } else if let Some(hunk) = files.last_mut().and_then(|(_, hunks)| hunks.last_mut()) {
            let (marker, content) = line.split_at(line.len().min(1));
            match marker {
                PATCH_REMOVED => hunk.removed.push(content.to_string()),
                PATCH_ADDED => hunk.added.push(content.to_string()),
                PATCH_CONTEXT => {
                    hunk.removed.push(content.to_string());
                    hunk.added.push(content.to_string());
                }
                _ => {}
            }
        }
    }
    files
}

/// Applies the patch under `root`; with `write` false it only checks that it applies.
pub fn apply(text: &str, root: &Path, write: bool) -> Result<(), String> {
    let mut results = Vec::new();
    for (path, hunks) in parse(text) {
        let current = std::fs::read_to_string(root.join(&path)).unwrap_or_default();
        let trailing_newline = current.is_empty() || current.ends_with('\n');
        let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
        let mut cursor = 0;
        for hunk in &hunks {
            let position = (cursor..=lines.len())
                .find(|start| {
                    lines.len() >= start + hunk.removed.len()
                        && lines[*start..start + hunk.removed.len()] == hunk.removed[..]
                })
                .ok_or_else(|| format!("{PATCH_DOES_NOT_APPLY}: {path}"))?;
            lines.splice(position..position + hunk.removed.len(), hunk.added.clone());
            cursor = position + hunk.added.len();
        }
        let mut output = lines.join("\n");
        if trailing_newline && !output.is_empty() {
            output.push('\n');
        }
        results.push((path, output));
    }
    if write {
        for (path, output) in results {
            let target = root.join(&path);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(target, output).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
