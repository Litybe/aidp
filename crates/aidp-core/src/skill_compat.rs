//! Compatibility between the running `aidp` and the skill it drives: the skill's `SKILL.md`
//! declares `min_aidp_version` in its frontmatter.

use crate::models::constants::skill_compat::{
    BYTE_ORDER_MARK, FRONTMATTER_DELIMITER, SKILL_MANIFEST_FILE, VERSION_SEPARATOR,
    VERSION_SUFFIX_MARKERS,
};
use crate::models::enums::SkillCompat;
use crate::models::skill_frontmatter::SkillFrontmatter;
use std::cmp::Ordering;
use std::path::Path;

/// The YAML between the opening and closing `---` lines of `manifest`, if it has a frontmatter.
fn frontmatter(manifest: &str) -> Option<&str> {
    let body = manifest.strip_prefix(BYTE_ORDER_MARK).unwrap_or(manifest);
    let mut lines = body.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != FRONTMATTER_DELIMITER {
        return None;
    }
    let start = first.len();
    let mut end = start;
    for line in lines {
        if line.trim_end() == FRONTMATTER_DELIMITER {
            return Some(&body[start..end]);
        }
        end += line.len();
    }
    None
}

/// The `min_aidp_version` declared by a `SKILL.md` text, `None` when it declares none.
pub fn min_aidp_version(manifest: &str) -> Result<Option<String>, String> {
    let Some(yaml) = frontmatter(manifest) else {
        return Ok(None);
    };
    let parsed: SkillFrontmatter =
        serde_yaml::from_str(yaml).map_err(|error| format!("invalid frontmatter: {error}"))?;
    Ok(parsed.min_aidp_version)
}

/// The numeric parts of `version` (`7.1.0-rc.1` gives `[7, 1, 0]`).
fn version_parts(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .split(VERSION_SUFFIX_MARKERS)
        .next()
        .unwrap_or_default();
    core.split(VERSION_SEPARATOR)
        .map(|part| part.parse().ok())
        .collect()
}

/// Orders two versions part by part, a missing part counting as zero.
fn compare_versions(left: &[u64], right: &[u64]) -> Ordering {
    let width = left.len().max(right.len());
    (0..width)
        .map(|index| {
            let left_part = left.get(index).copied().unwrap_or_default();
            let right_part = right.get(index).copied().unwrap_or_default();
            left_part.cmp(&right_part)
        })
        .find(|ordering| ordering.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Checks a `SKILL.md` text against the `running` version.
pub fn check_manifest(manifest: &str, running: &str) -> SkillCompat {
    let required = match min_aidp_version(manifest) {
        Ok(Some(required)) => required,
        Ok(None) => return SkillCompat::Undeclared,
        Err(reason) => return SkillCompat::Unreadable { reason },
    };
    let (Some(required_parts), Some(running_parts)) =
        (version_parts(&required), version_parts(running))
    else {
        return SkillCompat::Unreadable {
            reason: format!("cannot compare version {running} with min_aidp_version {required}"),
        };
    };
    if compare_versions(&running_parts, &required_parts).is_lt() {
        SkillCompat::TooOld { required }
    } else {
        SkillCompat::Compatible { required }
    }
}

/// Checks the skill at `skill_root` against the `running` version.
pub fn check(skill_root: &Path, running: &str) -> SkillCompat {
    let manifest_path = skill_root.join(SKILL_MANIFEST_FILE);
    match std::fs::read_to_string(&manifest_path) {
        Ok(manifest) => check_manifest(&manifest, running),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SkillCompat::Undeclared,
        Err(error) => SkillCompat::Unreadable {
            reason: format!("cannot read {}: {error}", manifest_path.display()),
        },
    }
}

#[cfg(test)]
#[path = "../tests/unit/skill_compat_tests.rs"]
mod tests;
