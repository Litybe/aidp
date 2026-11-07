use std::fs;
use std::path::{Path, PathBuf};

/// Call prefixes that build a request on ureq's implicit, timeout-less agent.
const FORBIDDEN_CALLS: [&str; 5] = [
    "ureq::get(",
    "ureq::post(",
    "ureq::put(",
    "ureq::delete(",
    "ureq::request(",
];
const SOURCE_DIR: &str = "src";
const RUST_EXTENSION: &str = "rs";
const UNIT_TESTS_DIR: &str = "tests";

/// Returns the workspace `crates/` directory, parent of this crate's manifest directory.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate manifest directory sits under crates/")
        .to_path_buf()
}

/// Collects every `.rs` file under `dir`, recursively, skipping the `excluded` directory.
fn collect_rust_files(dir: &Path, excluded: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            if path != excluded {
                collect_rust_files(&path, excluded, files);
            }
        } else if path.extension().is_some_and(|ext| ext == RUST_EXTENSION) {
            files.push(path);
        }
    }
}

/// Lists every `path:line` of `file` that calls one of the forbidden free functions.
fn offending_lines(file: &Path) -> Vec<String> {
    let content =
        fs::read_to_string(file).unwrap_or_else(|err| panic!("read {}: {err}", file.display()));
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| FORBIDDEN_CALLS.iter().any(|call| line.contains(call)))
        .map(|(index, _)| format!("{}:{}", file.display(), index + 1))
        .collect()
}

/// Collects the production sources of every crate under `crates`, leaving out `src/tests` of each.
fn production_sources(crates: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for crate_entry in fs::read_dir(crates).expect("readable crates directory") {
        let source_dir = crate_entry
            .expect("readable crate entry")
            .path()
            .join(SOURCE_DIR);
        if source_dir.is_dir() {
            let unit_tests_dir = source_dir.join(UNIT_TESTS_DIR);
            collect_rust_files(&source_dir, &unit_tests_dir, &mut files);
        }
    }
    files
}

/// Outbound HTTP must go through a shared ureq agent with explicit timeouts, never a free function.
#[test]
fn no_crate_source_calls_a_ureq_free_function() {
    let files = production_sources(&crates_dir());
    assert!(!files.is_empty(), "no source file found under crates/*/src");

    let offenders: Vec<String> = files
        .iter()
        .flat_map(|file| offending_lines(file))
        .collect();
    assert!(
        offenders.is_empty(),
        "ureq free functions found in production sources:
{}",
        offenders.join(
            "
"
        )
    );
}

/// Only `src/tests` directly under a crate is skipped; a nested `tests` directory is still scanned.
#[test]
fn only_the_crate_level_unit_tests_directory_is_excluded() {
    let root = tempfile::tempdir().expect("temp directory");
    let source_dir = root.path().join("x").join(SOURCE_DIR);
    let unit_tests = source_dir.join(UNIT_TESTS_DIR).join("units");
    let nested_tests = source_dir.join("inner").join(UNIT_TESTS_DIR);
    for dir in [&unit_tests, &nested_tests] {
        fs::create_dir_all(dir).expect("create directory");
    }
    let forbidden_call = format!(
        "{}
",
        FORBIDDEN_CALLS[0]
    );
    fs::write(unit_tests.join("a.rs"), &forbidden_call).expect("write a.rs");
    fs::write(source_dir.join("b.rs"), &forbidden_call).expect("write b.rs");
    fs::write(nested_tests.join("c.rs"), &forbidden_call).expect("write c.rs");

    let mut scanned: Vec<String> = production_sources(root.path())
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    scanned.sort();

    assert_eq!(scanned, ["b.rs", "c.rs"]);
}
