//! Source-wide gates, run against one read of the repository's Rust sources: a walk and a read per
//! rule cost several seconds each under a virus scanner, so every rule shares one index.
//!
//! - boundary: no source reads a file outside the repository or from another repository
//! - panic net: no production source hides a panic or slices a timestamp by bytes
//! - impure calls: only the port files call the wall clock, `git` or spawn a process
//! - port traits: each port trait is declared once and implemented at least twice
//! - ureq: outbound HTTP never goes through a ureq free function

use regex::Regex;
use std::fs;
use std::path::{Component, Path, PathBuf};

const CRATES_DIR: &str = "crates";
const AIDP_CORE_SRC_DIR: &str = "crates/aidp-core/src";
const RUST_EXTENSION: &str = "rs";
const WORKSPACE_LEVELS_UP: &str = "../..";
const GIT_DIR: &str = ".git";
const TARGET_DIR_PREFIX: &str = "target";
const SOURCE_DIR: &str = "src";
const TESTS_DIR: &str = "tests";
const READ_THREADS: usize = 16;
const LEADING_SEPARATOR: char = '/';

/// Split so this file never carries a literal `include` macro call of its own.
const INCLUDE_MACRO: &str = concat!("include", "_str!");

const RULE_BOUNDARY: &str = "boundary";
const RULE_PANIC_NET: &str = "panic-net";
const RULE_IMPURE_CALLS: &str = "impure-calls";
const RULE_PORT_TRAITS: &str = "port-traits";
const RULE_UREQ: &str = "ureq-free-functions";

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

/// Source fragments no production crate may contain, each split with `concat!`
/// so this file never carries the literal it forbids.
const FORBIDDEN_PANIC_NET_FRAGMENTS: &[&str] = &[
    concat!("catch", "_unwind"),
    concat!("expect(\"model-prices.json", " must be valid\")"),
    concat!("started_at", "[..10]"),
];
/// Crate whose panic-behaviour tests legitimately need the forbidden fragments.
const TEST_HARNESS_CRATE_DIR: &str = "aidp-testkit";
/// The single production file allowed to catch a panic: the gateway's resident-loop
/// supervisor, which counts and logs every panic instead of hiding it.
const RESIDENT_LOOP_SUPERVISOR: &str = "aidp-gateway/src/threads.rs";

/// Marker line after which a file's own test module begins.
const TEST_MODULE_MARKER: &str = "#[cfg(test)]";
/// Source file names allowed to call the wall clock, `git`, or spawn a process directly.
const PORT_OWNER_FILE_NAMES: &[&str] = &["clock.rs", "id.rs", "git.rs", "process_runner.rs"];
/// Impure call fragments production code must route through a port instead of calling directly.
const IMPURE_CALL_FRAGMENTS: &[&str] = &["Utc::now", "SystemTime::now", "Command::new"];
/// The port traits, each with the file name that declares it.
const PORT_TRAITS: &[(&str, &str)] = &[
    ("Clock", "clock.rs"),
    ("IdGen", "id.rs"),
    ("Git", "git.rs"),
    ("ProcessRunner", "process_runner.rs"),
];
const MINIMUM_IMPL_COUNT: usize = 2;

/// Call prefixes that build a request on ureq's implicit, timeout-less agent.
const FORBIDDEN_UREQ_CALLS: [&str; 5] = [
    "ureq::get(",
    "ureq::post(",
    "ureq::put(",
    "ureq::delete(",
    "ureq::request(",
];

/// A Rust source of the repository: its path from the repository root and its text.
struct SourceFile {
    relative: PathBuf,
    content: String,
}

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

/// Every `.rs` file under `root` with its text, skipping `target*` and `.git`, each read once
/// across `READ_THREADS` threads: a virus scanner makes each open slow, not the disk.
fn load_sources(root: &Path) -> Vec<SourceFile> {
    let paths: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_dir()
                && (name == GIT_DIR || name.starts_with(TARGET_DIR_PREFIX)))
        })
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().extension().and_then(|ext| ext.to_str()) == Some(RUST_EXTENSION)
        })
        .map(|entry| entry.into_path())
        .collect();
    let chunk_size = paths.len().div_ceil(READ_THREADS).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|path| SourceFile {
                            relative: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
                            content: fs::read_to_string(path).unwrap_or_default(),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("reader thread"))
            .collect()
    })
}

fn under(source: &SourceFile, dir: &str) -> bool {
    source.relative.starts_with(dir)
}

fn file_name(source: &SourceFile) -> &str {
    source
        .relative
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

fn violation_line(rule: &str, detail: String) -> String {
    format!("[{rule}] {detail}")
}

/// Why `target`, read from `file`, breaks the boundary; `None` when it stays inside.
fn boundary_violation(root: &Path, file: &Path, target: &Path) -> Option<String> {
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

fn boundary_rule(root: &Path, sources: &[SourceFile]) -> Vec<String> {
    let file_relative = Regex::new(FILE_RELATIVE_PATTERN).unwrap();
    let crate_relative = Regex::new(CRATE_RELATIVE_PATTERN).unwrap();
    let mut violations = Vec::new();
    for source in sources.iter().filter(|source| under(source, CRATES_DIR)) {
        let file = root.join(&source.relative);
        let file_dir = file.parent().expect("file directory").to_path_buf();
        let crate_name = source
            .relative
            .components()
            .nth(1)
            .expect("crate directory");
        let crate_dir = root.join(CRATES_DIR).join(crate_name.as_os_str());
        for path in captured(&file_relative, &source.content) {
            violations.extend(boundary_violation(
                root,
                &file,
                &normalize(&file_dir.join(&path)),
            ));
        }
        for path in captured(&crate_relative, &source.content) {
            let relative = path.trim_start_matches(LEADING_SEPARATOR);
            violations.extend(boundary_violation(
                root,
                &file,
                &normalize(&crate_dir.join(relative)),
            ));
        }
    }
    violations
        .into_iter()
        .map(|detail| violation_line(RULE_BOUNDARY, detail))
        .collect()
}

/// True for a crate's integration tests, its `src/tests` unit tests and the
/// resident-loop supervisor.
fn is_exempt_from_panic_gate(crates_dir: &Path, source: &Path) -> bool {
    let relative = source.strip_prefix(crates_dir).unwrap_or(source);
    let mut components = relative
        .components()
        .skip(1)
        .map(|component| component.as_os_str());
    let first = components.next();
    let second = components.next();
    let in_integration_tests = first.is_some_and(|name| name == TESTS_DIR);
    let in_source_tests = first.is_some_and(|name| name == SOURCE_DIR)
        && second.is_some_and(|name| name == TESTS_DIR);
    in_integration_tests || in_source_tests || relative == Path::new(RESIDENT_LOOP_SUPERVISOR)
}

fn panic_net_rule(sources: &[SourceFile]) -> Vec<String> {
    let crates_dir = Path::new(CRATES_DIR);
    sources
        .iter()
        .filter(|source| under(source, CRATES_DIR))
        .filter(|source| {
            source
                .relative
                .components()
                .nth(1)
                .is_none_or(|name| name.as_os_str() != TEST_HARNESS_CRATE_DIR)
        })
        .filter(|source| !is_exempt_from_panic_gate(crates_dir, &source.relative))
        .flat_map(|source| {
            FORBIDDEN_PANIC_NET_FRAGMENTS
                .iter()
                .filter(|fragment| source.content.contains(**fragment))
                .map(|fragment| {
                    violation_line(
                        RULE_PANIC_NET,
                        format!("{}: contains `{fragment}`", source.relative.display()),
                    )
                })
        })
        .collect()
}

/// The part of `content` before the file's own test module.
fn production_slice(content: &str) -> &str {
    match content.find(TEST_MODULE_MARKER) {
        Some(offset) => &content[..offset],
        None => content,
    }
}

fn impure_calls_rule(sources: &[SourceFile]) -> Vec<String> {
    sources
        .iter()
        .filter(|source| under(source, AIDP_CORE_SRC_DIR))
        .filter(|source| !PORT_OWNER_FILE_NAMES.contains(&file_name(source)))
        .flat_map(|source| {
            production_slice(&source.content)
                .lines()
                .enumerate()
                .flat_map(move |(index, line)| {
                    IMPURE_CALL_FRAGMENTS
                        .iter()
                        .filter(move |fragment| line.contains(**fragment))
                        .map(move |fragment| {
                            violation_line(
                                RULE_IMPURE_CALLS,
                                format!(
                                    "{}:{}: contains `{fragment}`",
                                    source.relative.display(),
                                    index + 1
                                ),
                            )
                        })
                })
        })
        .collect()
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

fn port_traits_rule(sources: &[SourceFile]) -> Vec<String> {
    let crate_sources: Vec<&SourceFile> = sources
        .iter()
        .filter(|source| under(source, CRATES_DIR))
        .collect();
    let mut failures = Vec::new();
    for (trait_name, owner_file_name) in PORT_TRAITS {
        let declared = crate_sources.iter().any(|source| {
            file_name(source) == *owner_file_name && declares_trait(&source.content, trait_name)
        });
        if !declared {
            failures.push(format!(
                "trait {trait_name} not declared in a file named {owner_file_name} under {CRATES_DIR}"
            ));
            continue;
        }
        let needle = format!("impl {trait_name} for");
        let impl_count: usize = crate_sources
            .iter()
            .map(|source| source.content.matches(needle.as_str()).count())
            .sum();
        if impl_count < MINIMUM_IMPL_COUNT {
            failures.push(format!(
                "trait {trait_name} has only {impl_count} `impl {trait_name} for` occurrence(s), expected at least {MINIMUM_IMPL_COUNT}"
            ));
        }
    }
    failures
        .into_iter()
        .map(|detail| violation_line(RULE_PORT_TRAITS, detail))
        .collect()
}

/// True for `crates/<crate>/src/...` outside `crates/<crate>/src/tests`.
fn is_production_source(source: &SourceFile) -> bool {
    let mut components = source.relative.components().map(|part| part.as_os_str());
    let in_crate_src = components.next().is_some_and(|name| name == CRATES_DIR)
        && components.nth(1).is_some_and(|name| name == SOURCE_DIR);
    in_crate_src && components.next().is_none_or(|name| name != TESTS_DIR)
}

/// Outbound HTTP must go through a shared ureq agent with explicit timeouts, never a free function.
fn ureq_rule(sources: &[SourceFile]) -> Vec<String> {
    sources
        .iter()
        .filter(|source| is_production_source(source))
        .flat_map(|source| {
            source
                .content
                .lines()
                .enumerate()
                .filter(|(_, line)| FORBIDDEN_UREQ_CALLS.iter().any(|call| line.contains(call)))
                .map(move |(index, _)| {
                    violation_line(
                        RULE_UREQ,
                        format!("{}:{}", source.relative.display(), index + 1),
                    )
                })
        })
        .collect()
}

#[test]
fn repository_sources_respect_every_gate() {
    let root = workspace_root();
    let sources = load_sources(&root);
    assert!(
        sources.iter().any(|source| under(source, CRATES_DIR)),
        "no Rust source under {}",
        root.display()
    );
    let mut violations = boundary_rule(&root, &sources);
    violations.extend(panic_net_rule(&sources));
    violations.extend(impure_calls_rule(&sources));
    violations.extend(port_traits_rule(&sources));
    violations.extend(ureq_rule(&sources));
    assert!(
        violations.is_empty(),
        "repository source gates failed:\n{}",
        violations.join("\n")
    );
}

fn fake(relative: &str, content: &str) -> SourceFile {
    SourceFile {
        relative: PathBuf::from(relative),
        content: content.to_string(),
    }
}

#[test]
fn boundary_rule_reports_a_read_outside_the_repository() {
    let root = Path::new("/repo");
    let include = |path: &str| format!("{INCLUDE_MACRO}(\"{path}\")");
    let outside = fake("crates/x/src/a.rs", &include("../../../../outside.txt"));
    let foreign = fake("crates/x/src/b.rs", &include("../../../SKILL.md"));
    let inside = fake("crates/x/src/c.rs", &include("../fixtures/ok.txt"));
    let found = boundary_rule(root, &[outside, foreign, inside]);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|line| line.starts_with("[boundary]")));
}

#[test]
fn panic_net_rule_reports_a_forbidden_fragment_and_keeps_exemptions() {
    let forbidden = FORBIDDEN_PANIC_NET_FRAGMENTS[0];
    let production = fake("crates/x/src/lib.rs", forbidden);
    let integration = fake("crates/x/tests/a.rs", forbidden);
    let unit = fake("crates/x/src/tests/units/a.rs", forbidden);
    let harness = fake("crates/aidp-testkit/src/lib.rs", forbidden);
    let supervisor = fake("crates/aidp-gateway/src/threads.rs", forbidden);
    let found = panic_net_rule(&[production, integration, unit, harness, supervisor]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("crates/x/src/lib.rs") || found[0].contains("crates\\x\\src\\lib.rs")
    );
}

#[test]
fn panic_gate_exempts_only_test_directories_at_crate_level() {
    let crates_dir = Path::new("crates");
    let exempt = |path: &str| is_exempt_from_panic_gate(crates_dir, Path::new(path));
    assert!(exempt("crates/x/src/tests/units/a.rs"));
    assert!(exempt("crates/x/tests/a.rs"));
    assert!(!exempt("crates/x/src/lib.rs"));
    assert!(!exempt("crates/x/src/deep/tests/a.rs"));
}

#[test]
fn impure_calls_rule_reports_a_call_outside_the_port_files() {
    let offender = fake(
        "crates/aidp-core/src/run.rs",
        "fn a() {\n    Utc::now();\n}\n",
    );
    let owner = fake("crates/aidp-core/src/clock.rs", "Utc::now();");
    let in_tests = fake(
        "crates/aidp-core/src/run.rs",
        "fn a() {}\n#[cfg(test)]\nmod t { fn b() { Utc::now(); } }\n",
    );
    let elsewhere = fake("crates/aidp-cli/src/main.rs", "Utc::now();");
    let found = impure_calls_rule(&[offender, owner, in_tests, elsewhere]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains(":2:"));
}

#[test]
fn port_traits_rule_reports_a_missing_declaration_and_too_few_impls() {
    let mut sources = vec![
        fake(
            "crates/a/src/clock.rs",
            "pub trait Clock {}\nimpl Clock for A {}",
        ),
        fake("crates/a/src/id.rs", "pub trait IdGen {}"),
        fake(
            "crates/a/src/process_runner.rs",
            "pub trait ProcessRunner {}",
        ),
    ];
    let found = port_traits_rule(&sources);
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found
        .iter()
        .any(|line| line.contains("trait Git not declared")));
    sources.push(fake("crates/b/src/x.rs", "impl Clock for B {}"));
    let after = port_traits_rule(&sources);
    assert_eq!(after.len(), 3, "{after:?}");
}

#[test]
fn ureq_rule_reports_a_free_function_and_excludes_only_crate_level_unit_tests() {
    let call = format!("{}\n", FORBIDDEN_UREQ_CALLS[0]);
    let production = fake("crates/x/src/b.rs", &call);
    let nested = fake("crates/x/src/inner/tests/c.rs", &call);
    let unit = fake("crates/x/src/tests/units/a.rs", &call);
    let integration = fake("crates/x/tests/d.rs", &call);
    let found = ureq_rule(&[production, nested, unit, integration]);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found
        .iter()
        .all(|line| line.starts_with("[ureq-free-functions]")));
}
