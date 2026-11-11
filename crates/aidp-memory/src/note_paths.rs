//! Where knowledge notes live in the vault, and the checks that keep a path inside `Knowledge/`.

use crate::error::{MemoryError, Result};
use crate::models::constants::knowledge::KNOWLEDGE_DIR;
use crate::models::constants::knowledge_proposals::{
    FOLDER_SEPARATOR, NAME_COLLISION_SEPARATOR, NOTES_SUBDIR, NOTE_EXTENSION, WIKILINK_CLOSE,
    WIKILINK_OPEN,
};
use crate::models::constants::paths::INBOX_DIR;
use crate::models::constants::schema::GLOBAL_PROJECT;
use crate::models::enums::{Domain, ProposalScope};
use std::path::{Component, Path, PathBuf};

const PARENT_SEGMENT: &str = "..";
const ALTERNATE_SEPARATOR: char = '\\';

/// `Knowledge/_global/<domain>/<slug>.md` or `Knowledge/<repo>/notes/<domain>/<slug>.md`, vault-relative.
pub fn note_rel_path(scope: ProposalScope, repo: &str, domain: Domain, slug: &str) -> String {
    let owner = match scope {
        ProposalScope::Global => GLOBAL_PROJECT.to_string(),
        ProposalScope::Project => format!("{repo}{FOLDER_SEPARATOR}{NOTES_SUBDIR}"),
    };
    format!("{KNOWLEDGE_DIR}{FOLDER_SEPARATOR}{owner}{FOLDER_SEPARATOR}{domain}{FOLDER_SEPARATOR}{slug}.{NOTE_EXTENSION}")
}

/// `Knowledge/_inbox`, vault-relative.
pub fn inbox_rel_dir() -> String {
    format!("{KNOWLEDGE_DIR}{FOLDER_SEPARATOR}{INBOX_DIR}")
}

/// A vault-relative path under `Knowledge/`; `..`, absolute paths and other folders are refused.
pub fn knowledge_rel_path(raw: &str) -> Result<String> {
    let unified = raw
        .trim()
        .replace(ALTERNATE_SEPARATOR, &FOLDER_SEPARATOR.to_string());
    let prefix = format!("{KNOWLEDGE_DIR}{FOLDER_SEPARATOR}");
    let relative = unified.strip_prefix(&prefix).unwrap_or(&unified);
    let path = Path::new(relative);
    let escapes = path.is_absolute()
        || relative.is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || relative
            .split(FOLDER_SEPARATOR)
            .any(|part| part == PARENT_SEGMENT);
    if escapes {
        return Err(MemoryError::Invalid(format!(
            "`{raw}` is not a path under {KNOWLEDGE_DIR}/"
        )));
    }
    Ok(format!("{prefix}{relative}"))
}

/// The absolute path of a vault-relative path.
pub fn absolute(vault_root: &Path, rel: &str) -> PathBuf {
    rel.split(FOLDER_SEPARATOR)
        .fold(vault_root.to_path_buf(), |path, part| path.join(part))
}

/// `[[Knowledge/.../slug]]` for a vault-relative note path, quoted for a YAML value.
pub fn wikilink(rel: &str) -> String {
    let stem = rel
        .strip_suffix(&format!(".{NOTE_EXTENSION}"))
        .unwrap_or(rel);
    format!("\"{WIKILINK_OPEN}{stem}{WIKILINK_CLOSE}\"")
}

/// `<dir>/<stem>.md`, or `<stem>-2.md`, `<stem>-3.md`... when the name is taken.
pub fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.{NOTE_EXTENSION}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = dir.join(format!(
            "{stem}{NAME_COLLISION_SEPARATOR}{counter}.{NOTE_EXTENSION}"
        ));
        counter += 1;
    }
    candidate
}

/// A path as stored in a note: relative to the vault, with `/`.
pub fn vault_relative(vault_root: &Path, path: &Path) -> String {
    path.strip_prefix(vault_root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(&FOLDER_SEPARATOR.to_string())
}

#[cfg(test)]
#[path = "../tests/unit/note_paths_tests.rs"]
mod tests;
