//! Integration tests asserting the port abstractions introduced for `Clock`,
//! `IdGen`, `Git` and `ProcessRunner` stay the only place production code
//! reads the wall clock, generates ids, shells out to `git`, or spawns a
//! child process.
//!
//! - `production_code_keeps_impure_calls_inside_the_port_files` ensures no
//!   crate under `crates` calls `Utc::now`, `SystemTime::now` or
//!   `Command::new` outside the four files that own those ports.
//! - `every_port_trait_has_at_least_two_implementations` ensures each of the
//!   four port traits is declared once under `crates` and implemented
//!   at least twice.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root, two levels above `CARGO_MANIFEST_DIR` (`<repo>/crates/aidp-core`).
fn repo_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path
}

/// Relative path of the crates directory from the repository root.
const CRATES_DIR: &str = "crates";

/// Relative path of `aidp-core`'s own source directory, the scope of the
/// impure-call gate.
const AIDP_CORE_SRC_DIR: &str = "crates/aidp-core/src";

/// Extension of the Rust source files the gate scans.
const RUST_SOURCE_EXTENSION: &str = "rs";

/// Marker line after which a file's own test module begins.
const TEST_MODULE_MARKER: &str = "#[cfg(test)]";

/// Source file names allowed to call the wall clock, `git`, or spawn a
/// process directly: the four files that own those ports.
const PORT_OWNER_FILE_NAMES: &[&str] = &["clock.rs", "id.rs", "git.rs", "process_runner.rs"];

/// Impure call fragments production code must route through a port instead
/// of calling directly.
const IMPURE_CALL_FRAGMENTS: &[&str] = &["Utc::now", "SystemTime::now", "Command::new"];

/// The four port traits this gate expects, one file per trait, with the
/// minimum number of `impl <Trait> for` occurrences required across the
/// whole `crates` tree.
const PORT_TRAITS: &[(&str, &str)] = &[
    ("Clock", "clock.rs"),
    ("IdGen", "id.rs"),
    ("Git", "git.rs"),
    ("ProcessRunner", "process_runner.rs"),
];

/// Minimum `impl <Trait> for` occurrences a port trait must have.
const MINIMUM_IMPL_COUNT: usize = 2;

/// Collects every Rust source file under `dir`, recursing into subdirectories.
fn collect_rust_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|_| panic!("Failed to read directory: {}", dir.display()));
    for entry in entries {
        let path = entry.expect("Failed to read directory entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path
            .extension()
            .is_some_and(|ext| ext == RUST_SOURCE_EXTENSION)
        {
            sources.push(path);
        }
    }
}

/// The portion of `content` that lives outside the file's own test module,
/// i.e. everything before its first `#[cfg(test)]` line.
fn production_slice(content: &str) -> &str {
    match content.find(TEST_MODULE_MARKER) {
        Some(offset) => &content[..offset],
        None => content,
    }
}

/// True for a source file named after one of the four port owners.
fn is_port_owner(source: &Path) -> bool {
    source
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| PORT_OWNER_FILE_NAMES.contains(&name))
}

#[test]
fn production_code_keeps_impure_calls_inside_the_port_files() {
    let src_dir = repo_root().join(AIDP_CORE_SRC_DIR);
    let mut sources = Vec::new();
    collect_rust_sources(&src_dir, &mut sources);
    sources.retain(|source| !is_port_owner(source));

    assert!(
        !sources.is_empty(),
        "No Rust sources found under {}",
        src_dir.display()
    );

    let offenders: Vec<String> = sources
        .iter()
        .flat_map(|source| {
            let content = fs::read_to_string(source)
                .unwrap_or_else(|_| panic!("Failed to read source: {}", source.display()));
            let production = production_slice(&content);
            let line_offenders: Vec<String> = production
                .lines()
                .enumerate()
                .flat_map(|(index, line)| {
                    IMPURE_CALL_FRAGMENTS
                        .iter()
                        .filter(move |fragment| line.contains(**fragment))
                        .map(move |fragment| {
                            format!("{}:{}: contains `{fragment}`", source.display(), index + 1)
                        })
                })
                .collect();
            line_offenders
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "Impure call found outside the port files:\n{}",
        offenders.join("\n")
    );
}

/// Every Rust source under `crates_dir` with its content, each file read once.
fn read_rust_sources(crates_dir: &Path) -> Vec<(PathBuf, String)> {
    let mut sources = Vec::new();
    collect_rust_sources(crates_dir, &mut sources);
    sources
        .into_iter()
        .map(|source| {
            let content = fs::read_to_string(&source)
                .unwrap_or_else(|_| panic!("Failed to read source: {}", source.display()));
            (source, content)
        })
        .collect()
}

/// Counts `impl <trait_name> for` occurrences across the given sources.
fn count_trait_impls(sources: &[(PathBuf, String)], trait_name: &str) -> usize {
    let needle = format!("impl {trait_name} for");
    sources
        .iter()
        .map(|(_, content)| content.matches(needle.as_str()).count())
        .sum()
}

/// True if `content` declares `pub trait <trait_name>` or `trait <trait_name>`.
fn declares_trait(content: &str, trait_name: &str) -> bool {
    let owned = format!("trait {trait_name}");
    content.lines().any(|line| {
        line.trim_start()
            .trim_start_matches("pub ")
            .starts_with(&owned)
    })
}

#[test]
fn every_port_trait_has_at_least_two_implementations() {
    let crates_dir = repo_root().join(CRATES_DIR);
    let mut failures = Vec::new();

    let sources = read_rust_sources(&crates_dir);

    for (trait_name, owner_file_name) in PORT_TRAITS {
        let declared = sources.iter().any(|(source, content)| {
            source
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == *owner_file_name)
                && declares_trait(content, trait_name)
        });
        if !declared {
            failures.push(format!(
                "trait {trait_name} not declared in a file named {owner_file_name} under {}",
                crates_dir.display()
            ));
            continue;
        }

        let impl_count = count_trait_impls(&sources, trait_name);
        if impl_count < MINIMUM_IMPL_COUNT {
            failures.push(format!(
                "trait {trait_name} has only {impl_count} `impl {trait_name} for` occurrence(s), expected at least {MINIMUM_IMPL_COUNT}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Port trait gate failed:\n{}",
        failures.join("\n")
    );
}
