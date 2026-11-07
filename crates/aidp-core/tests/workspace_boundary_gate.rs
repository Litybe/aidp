//! Keeps the workspace self-contained: no source or test of any crate reads a file outside the
//! repository, nor the skill or the benchmark, which live in their own repositories. Tests
//! compile the frozen skill under `fixtures/skill` instead.

use regex::Regex;
use std::path::{Component, Path, PathBuf};

const CRATES_DIR: &str = "crates";
const TARGET_DIR: &str = "target";
const RUST_EXTENSION: &str = "rs";
const WORKSPACE_LEVELS_UP: &str = "../..";
/// Top-level entries of the skill and of the e2e suite: they are never read from here.
const FOREIGN_ENTRIES: &[&str] = &[
    "templates",
    "agents",
    "commands",
    "hooks",
    "SKILL.md",
    "USER-GUIDE.md",
    "benchmark",
];
/// `include_str!`, `include_bytes!` and `#[path]`: relative to the file that holds them.
const FILE_RELATIVE_PATTERN: &str =
    r#"(?:include_str!|include_bytes!)\(\s*"([^"]+)"|#\[path\s*=\s*"([^"]+)"\]"#;
/// `concat!(env!("CARGO_MANIFEST_DIR"), "...")` and `...(env!("CARGO_MANIFEST_DIR")).join("...")`:
/// relative to the crate.
const CRATE_RELATIVE_PATTERN: &str = r#"concat!\(\s*env!\("CARGO_MANIFEST_DIR"\)\s*,\s*"([^"]+)"|env!\("CARGO_MANIFEST_DIR"\)\)\s*\.join\(\s*"([^"]+)""#;
const LEADING_SEPARATOR: char = '/';

fn workspace_root() -> PathBuf {
    normalize(&Path::new(env!("CARGO_MANIFEST_DIR")).join(WORKSPACE_LEVELS_UP))
}

/// `path` with every `.` and `..` resolved lexically, without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != TARGET_DIR)
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().extension().and_then(|ext| ext.to_str()) == Some(RUST_EXTENSION)
        })
        .map(|entry| entry.into_path())
        .collect()
}

/// The crate directory `file` belongs to: the first directory under `crates/`.
fn crate_dir_of(root: &Path, file: &Path) -> PathBuf {
    let crates = root.join(CRATES_DIR);
    let relative = file.strip_prefix(&crates).expect("file under crates/");
    let name = relative.components().next().expect("crate directory");
    crates.join(name.as_os_str())
}

/// Why `target`, read from `file`, breaks the boundary; `None` when it stays inside.
fn violation(root: &Path, file: &Path, target: &Path) -> Option<String> {
    let Ok(relative) = target.strip_prefix(root) else {
        return Some(format!(
            "{} reads {} outside the repository",
            file.display(),
            target.display()
        ));
    };
    let first = relative
        .components()
        .next()?
        .as_os_str()
        .to_string_lossy()
        .to_string();
    FOREIGN_ENTRIES.contains(&first.as_str()).then(|| {
        format!(
            "{} reads {}, which belongs to another repository; use fixtures/ instead",
            file.display(),
            relative.display()
        )
    })
}

fn captured(regex: &Regex, text: &str) -> Vec<String> {
    regex
        .captures_iter(text)
        .filter_map(|caps| {
            caps.iter()
                .skip(1)
                .flatten()
                .next()
                .map(|m| m.as_str().to_string())
        })
        .collect()
}

#[test]
fn no_crate_reads_a_file_outside_the_workspace_or_from_another_repository() {
    let root = workspace_root();
    let file_relative = Regex::new(FILE_RELATIVE_PATTERN).unwrap();
    let crate_relative = Regex::new(CRATE_RELATIVE_PATTERN).unwrap();
    let sources = rust_sources(&root.join(CRATES_DIR));
    assert!(
        !sources.is_empty(),
        "no Rust source under {}",
        root.display()
    );
    let mut violations = Vec::new();
    for file in sources {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let file_dir = file.parent().expect("file directory").to_path_buf();
        let crate_dir = crate_dir_of(&root, &file);
        for path in captured(&file_relative, &text) {
            violations.extend(violation(&root, &file, &normalize(&file_dir.join(&path))));
        }
        for path in captured(&crate_relative, &text) {
            let relative = path.trim_start_matches(LEADING_SEPARATOR);
            violations.extend(violation(
                &root,
                &file,
                &normalize(&crate_dir.join(relative)),
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn the_frozen_skill_is_where_the_tests_expect_it() {
    assert!(aidp_testkit::fixtures::templates_dir().is_dir());
    assert!(aidp_testkit::fixtures::mocks_dir().is_dir());
    assert!(aidp_testkit::fixtures::template_inputs().is_file());
}
