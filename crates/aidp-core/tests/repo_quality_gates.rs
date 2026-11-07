//! Integration tests asserting the repository's quality gate configuration
//! cannot silently soften. These tests read configuration files as text and
//! verify that critical quality standards remain in place.
//!
//! Each test is named for what it protects:
//! - `every_crate_manifest_opts_into_workspace_lints` ensures all crates inherit workspace lint rules
//! - `workspace_forbids_unsafe_and_declares_msrv` ensures the workspace declares unsafe forbid and MSRV
//! - `toolchain_pin_matches_declared_msrv` ensures the toolchain pin matches the workspace MSRV
//! - `deny_config_traces_the_yaml_parser_risk` ensures supply chain config mentions known risks
//! - `workspace_code_carries_no_panic_safety_net` ensures no crate hides a panic or slices a timestamp by bytes
//! - `every_integration_test_file_is_compiled` ensures `autotests = false` never drops a test file
//!
//! All tests use only `std` and fail with clear error messages identifying missing files.

use std::fs;
use std::path::{Path, PathBuf};

const AUTOTESTS_OFF: &str = "autotests = false";
const INTEGRATION_ROOT: &str = "integration.rs";

/// The repository root, two levels above `CARGO_MANIFEST_DIR` (`<repo>/crates/aidp-core`).
fn repo_root() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path
}

#[test]
fn every_crate_manifest_opts_into_workspace_lints() {
    let repo = repo_root();
    let crates_dir = repo.join("crates");

    // Enumerate all directories in crates
    let entries = fs::read_dir(&crates_dir)
        .unwrap_or_else(|_| panic!("Failed to read crates directory: {}", crates_dir.display()));

    let mut found_crates = 0;
    let mut failures = Vec::new();

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        // Only check directories
        if !path.is_dir() {
            continue;
        }

        let cargo_toml = path.join("Cargo.toml");
        if !cargo_toml.exists() {
            continue;
        }

        found_crates += 1;
        let content = fs::read_to_string(&cargo_toml)
            .unwrap_or_else(|_| panic!("Failed to read Cargo.toml: {}", cargo_toml.display()));

        // Check for [lints] section with workspace = true
        if !content.contains("[lints]") {
            failures.push(format!("{}: missing [lints] section", cargo_toml.display()));
        } else if !content.contains("workspace = true") {
            failures.push(format!(
                "{}: [lints] section does not contain 'workspace = true'",
                cargo_toml.display()
            ));
        }

        // Check for license.workspace = true
        if !content.contains("license.workspace = true") {
            failures.push(format!(
                "{}: missing 'license.workspace = true'",
                cargo_toml.display()
            ));
        }
    }

    assert!(
        found_crates > 0,
        "No crates found in {}; check directory structure",
        crates_dir.display()
    );

    if !failures.is_empty() {
        panic!(
            "Crate manifest validation failed ({} crates checked):\n{}",
            found_crates,
            failures.join("\n")
        );
    }
}

#[test]
fn workspace_forbids_unsafe_and_declares_msrv() {
    let repo = repo_root();
    let workspace_toml = repo.join("Cargo.toml");

    let content = fs::read_to_string(&workspace_toml).unwrap_or_else(|_| {
        panic!(
            "Expected workspace manifest at {}",
            workspace_toml.display()
        )
    });

    // Check for [workspace.lints.rust] section
    assert!(
        content.contains("[workspace.lints.rust]"),
        "Workspace Cargo.toml must contain [workspace.lints.rust] section"
    );

    // Check for unsafe_code = "forbid"
    assert!(
        content.contains("unsafe_code = \"forbid\""),
        "Workspace lints must forbid unsafe_code"
    );

    // Check for rust-version
    assert!(
        content.contains("rust-version = \"1.95\""),
        "Workspace must declare rust-version = \"1.95\""
    );
}

#[test]
fn toolchain_pin_matches_declared_msrv() {
    let repo = repo_root();
    let workspace_toml = repo.join("Cargo.toml");
    let toolchain_toml = repo.join("rust-toolchain.toml");

    let workspace_content = fs::read_to_string(&workspace_toml).unwrap_or_else(|_| {
        panic!(
            "Expected workspace manifest at {}",
            workspace_toml.display()
        )
    });

    let toolchain_content = fs::read_to_string(&toolchain_toml)
        .unwrap_or_else(|_| panic!("Expected toolchain file at {}", toolchain_toml.display()));

    // Extract rust-version from workspace (e.g., "1.95" from "1.95")
    let msrv_line = workspace_content
        .lines()
        .find(|line| line.contains("rust-version"))
        .expect("rust-version not found in workspace Cargo.toml");

    // Extract the version number (simplified: expect pattern like 1.95)
    let msrv_version = msrv_line
        .split('=')
        .nth(1)
        .expect("Failed to parse rust-version value")
        .trim()
        .trim_matches('"')
        .trim();

    // Extract channel from toolchain (e.g., "1.95.0" from channel = "1.95.0")
    let toolchain_line = toolchain_content
        .lines()
        .find(|line| line.contains("channel"))
        .expect("channel not found in rust-toolchain.toml");

    let channel_version = toolchain_line
        .split('=')
        .nth(1)
        .expect("Failed to parse channel value")
        .trim()
        .trim_matches('"')
        .trim();

    // Extract major.minor from channel (1.95.0 -> 1.95)
    let channel_major_minor: String = channel_version
        .split('.')
        .take(2)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(".");

    assert_eq!(
        msrv_version, channel_major_minor,
        "MSRV mismatch: workspace declares {} but toolchain pins {}.x",
        msrv_version, channel_major_minor
    );
}

#[test]
fn deny_config_traces_the_yaml_parser_risk() {
    let repo = repo_root();
    let deny_toml = repo.join("deny.toml");

    let content = fs::read_to_string(&deny_toml).unwrap_or_else(|_| {
        panic!(
            "Expected supply chain policy at {}. Has T4 completed?",
            deny_toml.display()
        )
    });

    // Check that serde_yaml is mentioned (documenting the known risk)
    assert!(
        content.to_lowercase().contains("serde_yaml"),
        "deny.toml must mention serde_yaml to document the known YAML parser risk"
    );
}

/// Source fragments no production crate may contain, each split with `concat!`
/// so this file never carries the literal it forbids.
const FORBIDDEN_PANIC_NET_FRAGMENTS: &[&str] = &[
    concat!("catch", "_unwind"),
    concat!("expect(\"model-prices.json", " must be valid\")"),
    concat!("started_at", "[..10]"),
];

/// Crate whose panic-behaviour tests legitimately need the forbidden fragments.
const TEST_HARNESS_CRATE_DIR: &str = "aidp-testkit";

/// Integration-test directory of a crate, whose panic-behaviour tests may need the fragments.
const INTEGRATION_TESTS_DIR: &str = "tests";

/// Source directory of a crate, whose `tests` subdirectory holds its unit tests.
const SOURCE_DIR: &str = "src";

/// The single production file allowed to catch a panic: the gateway's resident-loop
/// supervisor, which counts and logs every panic instead of hiding it.
const RESIDENT_LOOP_SUPERVISOR: &str = "aidp-gateway/src/threads.rs";

/// Relative path of the crates directory from the repository root.
const CRATES_DIR: &str = "crates";

/// Extension of the Rust source files the gate scans.
const RUST_SOURCE_EXTENSION: &str = "rs";

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
    let in_integration_tests = first.is_some_and(|name| name == INTEGRATION_TESTS_DIR);
    let in_source_tests = first.is_some_and(|name| name == SOURCE_DIR)
        && second.is_some_and(|name| name == INTEGRATION_TESTS_DIR);
    in_integration_tests || in_source_tests || relative == Path::new(RESIDENT_LOOP_SUPERVISOR)
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
fn workspace_code_carries_no_panic_safety_net() {
    let crates_dir = repo_root().join(CRATES_DIR);
    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates_dir)
        .unwrap_or_else(|_| panic!("Failed to read crates directory: {}", crates_dir.display()))
    {
        let crate_dir = entry.expect("Failed to read directory entry").path();
        if crate_dir.is_dir() && !crate_dir.ends_with(TEST_HARNESS_CRATE_DIR) {
            collect_rust_sources(&crate_dir, &mut sources);
        }
    }
    sources.retain(|source| !is_exempt_from_panic_gate(&crates_dir, source));

    assert!(
        !sources.is_empty(),
        "No Rust sources found under {}",
        crates_dir.display()
    );

    let offenders: Vec<String> = sources
        .iter()
        .flat_map(|source| {
            let content = fs::read_to_string(source)
                .unwrap_or_else(|_| panic!("Failed to read source: {}", source.display()));
            FORBIDDEN_PANIC_NET_FRAGMENTS
                .iter()
                .filter(move |fragment| content.contains(**fragment))
                .map(move |fragment| format!("{}: contains `{fragment}`", source.display()))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "Forbidden panic safety net found:\n{}",
        offenders.join("\n")
    );
}

/// A crate with `autotests = false` runs only the test files it names: each `tests/*.rs` must be a
/// `mod` of `tests/integration.rs` or a `[[test]]` of its own, or it silently never runs.
#[test]
fn every_integration_test_file_is_compiled() {
    let crates_dir = repo_root().join("crates");
    let mut orphans = Vec::new();
    for entry in fs::read_dir(&crates_dir).expect("read crates") {
        let crate_dir = entry.expect("read a crate entry").path();
        let Ok(manifest) = fs::read_to_string(crate_dir.join("Cargo.toml")) else {
            continue;
        };
        if !manifest.contains(AUTOTESTS_OFF) {
            continue;
        }
        let tests_dir = crate_dir.join("tests");
        let integration = fs::read_to_string(tests_dir.join(INTEGRATION_ROOT)).unwrap_or_default();
        let Ok(test_files) = fs::read_dir(&tests_dir) else {
            continue;
        };
        for file in test_files {
            let path = file.expect("read a test entry").path();
            let is_rust_file = path.extension().is_some_and(|extension| extension == "rs");
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !is_rust_file || path.ends_with(INTEGRATION_ROOT) {
                continue;
            }
            let declared_as_module = integration.contains(&format!("mod {stem};"));
            let declared_as_binary = manifest.contains(&format!("path = \"tests/{stem}.rs\""));
            if !declared_as_module && !declared_as_binary {
                orphans.push(path.display().to_string());
            }
        }
    }
    assert!(
        orphans.is_empty(),
        "test files compiled by nothing (add `mod <name>;` to tests/{INTEGRATION_ROOT}):
{}",
        orphans.join(
            "
"
        )
    );
}
