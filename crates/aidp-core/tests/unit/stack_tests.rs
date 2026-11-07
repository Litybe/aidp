use super::*;

const README: &str = "README.md";
const SOLUTION_FILE: &str = "Svc.sln";
const CSPROJ_FILE: &str = "src/Svc/Svc.csproj";
const HELPER_CSPROJ_FILE: &str = "tools/Helper.csproj";
const DEEP_CSPROJ_FILE: &str = "a/b/c/D.csproj";
const NESTED_CARGO_TOML: &str = "rust/Cargo.toml";
const SKIPPED_CARGO_TOML: &str = "node_modules/x/Cargo.toml";
const HIDDEN_SOLUTION_FILE: &str = ".git/a.sln";
const TAG_RUST_ROOT: &str = "rust-root";
const TAG_RUST_SUB: &str = "rust-sub";
const TAG_CS_SLN: &str = "cs-sln";
const TAG_CS_PROJ: &str = "cs-proj";
const TAG_TS: &str = "ts";
const TAG_NONE: &str = "none";
const TAG_TS_OVER_CS: &str = "ts-over-cs";
const TAG_PKG_ONLY: &str = "pkg-only";
const TAG_SKIPPED: &str = "skipped";
const TAG_TOO_DEEP: &str = "too-deep";
const TAG_NG_GENERIC: &str = "ng-generic";
const TAG_PROJECT_CS: &str = "p-cs";
const TAG_PROJECT_TS: &str = "p-ts";
const TAG_PROJECT_SAM: &str = "p-sam";
const TAG_PROJECT_NG: &str = "p-ng";
const TAG_PROJECT_RS: &str = "p-rs";
const TAG_PROJECT_NODE: &str = "p-node";
const TAG_PROJECT_NONE: &str = "p-none";

/// A fresh, empty directory unique to this test process.
fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aidp-stack-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Creates the empty file `rel` under `root`, with its parent directories.
fn touch(root: &Path, rel: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "").unwrap();
}

/// What [`detect`] answers for a workspace holding exactly `files`.
fn detect_with(tag: &str, files: &[&str]) -> &'static str {
    let dir = tmp_dir(tag);
    for file in files {
        touch(&dir, file);
    }
    let detected = detect(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    detected
}

/// Each stack is recognised by its own marker file.
#[test]
fn detect_recognises_each_stack_by_its_marker() {
    assert_eq!(detect_with(TAG_RUST_ROOT, &[CARGO_TOML]), RUST);
    assert_eq!(detect_with(TAG_RUST_SUB, &[NESTED_CARGO_TOML]), RUST);
    assert_eq!(detect_with(TAG_CS_SLN, &[SOLUTION_FILE]), CSHARP);
    assert_eq!(detect_with(TAG_CS_PROJ, &[CSPROJ_FILE]), CSHARP);
    assert_eq!(
        detect_with(TAG_TS, &[PACKAGE_JSON, SERVERLESS_YML]),
        LAMBDA_TS
    );
    assert_eq!(detect_with(TAG_NONE, &[README]), GENERIC);
}

/// A Lambda repo carrying a helper `.csproj` is still a Lambda repo.
#[test]
fn detect_prefers_lambda_ts_over_a_helper_csproj() {
    assert_eq!(
        detect_with(
            TAG_TS_OVER_CS,
            &[PACKAGE_JSON, SERVERLESS_TS, HELPER_CSPROJ_FILE]
        ),
        LAMBDA_TS
    );
}

/// A `package.json` without a Serverless file is not a Lambda repo: a React
/// client carries one too.
#[test]
fn detect_needs_a_serverless_file_beside_package_json() {
    assert_eq!(detect_with(TAG_PKG_ONLY, &[PACKAGE_JSON]), GENERIC);
}

/// Markers under skipped, hidden or too deep directories are ignored.
#[test]
fn detect_ignores_skipped_hidden_and_deep_markers() {
    assert_eq!(
        detect_with(TAG_SKIPPED, &[SKIPPED_CARGO_TOML, HIDDEN_SOLUTION_FILE]),
        GENERIC
    );
    assert_eq!(detect_with(TAG_TOO_DEEP, &[DEEP_CSPROJ_FILE]), GENERIC);
}

/// What [`detect_project`] answers for a workspace holding exactly `files`.
fn detect_project_with(tag: &str, files: &[&str]) -> Stack {
    let dir = tmp_dir(tag);
    for file in files {
        touch(&dir, file);
    }
    let detected = detect_project(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    detected
}

/// Each typed stack is recognised by its own markers, SAM included.
#[test]
fn detect_project_recognises_each_stack() {
    assert_eq!(
        detect_project_with(TAG_PROJECT_CS, &[SOLUTION_FILE]),
        Stack::Csharp
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_TS, &[PACKAGE_JSON, SERVERLESS_YML]),
        Stack::LambdaTs
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_SAM, &[PACKAGE_JSON, SAM_TEMPLATE_YAML]),
        Stack::LambdaTs
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_NG, &[ANGULAR_JSON]),
        Stack::Angular
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_RS, &[CARGO_TOML]),
        Stack::Rust
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_NODE, &[PACKAGE_JSON]),
        Stack::TsNode
    );
    assert_eq!(
        detect_project_with(TAG_PROJECT_NONE, &[README]),
        Stack::Unknown
    );
}

/// `detect` keeps answering `generic` for an Angular-only repo.
#[test]
fn detect_still_returns_generic_for_an_angular_only_repo() {
    assert_eq!(detect_with(TAG_NG_GENERIC, &[ANGULAR_JSON]), GENERIC);
}
