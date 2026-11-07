use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command;

const HELPERS_FILE: &str = "helpers.js";
const RETRY_HELPERS_FILE: &str = "retry_helpers.js";
const FANOUT_HELPERS_FILE: &str = "fanout_helpers.js";
const PLAN_HELPERS_FILE: &str = "plan_helpers.js";
const POOL_HELPERS_FILE: &str = "pool_helpers.js";

const JS_FILES: &[&str] = &[
    HELPERS_FILE,
    RETRY_HELPERS_FILE,
    FANOUT_HELPERS_FILE,
    PLAN_HELPERS_FILE,
    POOL_HELPERS_FILE,
];

const JS_EXTENSION: &str = "js";
const NODE_BINARY: &str = "node";
const NODE_CHECK_FLAG: &str = "--check";

fn js_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("js")
}

/// Runs `node --check` on one asset file. When `node` cannot be spawned the
/// test prints an explicit skip line to stderr and returns without failing.
fn assert_valid_syntax(file_name: &str) {
    let path = js_dir().join(file_name);
    let output = match Command::new(NODE_BINARY)
        .arg(NODE_CHECK_FLAG)
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("SKIP: node not found on PATH, cannot check {file_name}");
            return;
        }
        Err(error) => panic!("failed to spawn node for {file_name}: {error}"),
    };
    assert!(
        output.status.success(),
        "node --check failed for {file_name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn it_helpers_js_is_valid_syntax() {
    assert_valid_syntax(HELPERS_FILE);
}

#[test]
fn it_retry_helpers_js_is_valid_syntax() {
    assert_valid_syntax(RETRY_HELPERS_FILE);
}

#[test]
fn it_fanout_helpers_js_is_valid_syntax() {
    assert_valid_syntax(FANOUT_HELPERS_FILE);
}

#[test]
fn it_plan_helpers_js_is_valid_syntax() {
    assert_valid_syntax(PLAN_HELPERS_FILE);
}

#[test]
fn it_pool_helpers_js_is_valid_syntax() {
    assert_valid_syntax(POOL_HELPERS_FILE);
}

#[test]
fn it_js_directory_holds_exactly_the_expected_files() {
    let mut found: Vec<String> = std::fs::read_dir(js_dir())
        .expect("assets/js directory must exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(JS_EXTENSION))
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    found.sort();
    let mut expected: Vec<String> = JS_FILES.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(found, expected);
}
