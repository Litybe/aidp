//! Which language stack a workspace holds, for an input `stack: auto`.
//!
//! `run::init` replaces `auto` with what [`detect`] finds, before the stages'
//! `skills_by` is resolved, so generic templates hand each stage the language
//! skills of the repo at hand without a human choosing them.

use crate::models::constants::stack::*;
use crate::models::enums::Stack;
use std::path::Path;

/// The stack `workspace` holds, first marker wins: [`LAMBDA_TS`] (a
/// `serverless.*` beside a root `package.json`), [`CSHARP`] (a root `*.sln` or
/// a `*.csproj` two levels down), [`RUST`] (a `Cargo.toml` at the root or one
/// level down), else [`GENERIC`]. Lambda TS comes first because a Lambda repo
/// may carry a helper `.csproj`, never the reverse.
pub fn detect(workspace: &Path) -> &'static str {
    if is_lambda_ts(workspace) {
        return LAMBDA_TS;
    }
    if is_csharp(workspace) {
        return CSHARP;
    }
    if is_rust(workspace) {
        return RUST;
    }
    GENERIC
}

/// The typed [`Stack`] of `path`; unlike [`detect`], C# is tested before Lambda TS.
pub fn detect_project(path: &Path) -> Stack {
    let root_has = |name: &str| path.join(name).is_file();
    if is_csharp(path) {
        return Stack::Csharp;
    }
    if is_lambda_ts(path) || (root_has(PACKAGE_JSON) && root_has(SAM_TEMPLATE_YAML)) {
        return Stack::LambdaTs;
    }
    if root_has(ANGULAR_JSON) {
        return Stack::Angular;
    }
    if is_rust(path) {
        return Stack::Rust;
    }
    if root_has(PACKAGE_JSON) {
        return Stack::TsNode;
    }
    Stack::Unknown
}

/// True when a root `package.json` sits beside a `serverless.*` config.
fn is_lambda_ts(dir: &Path) -> bool {
    let root_has = |name: &str| dir.join(name).is_file();
    root_has(PACKAGE_JSON) && SERVERLESS_FILES.iter().any(|f| root_has(f))
}

/// True when a `*.sln` sits at the root or a `*.csproj` up to two levels down.
fn is_csharp(dir: &Path) -> bool {
    has_ext(dir, SLN_EXT, 0) || has_ext(dir, CSPROJ_EXT, 2)
}

/// True when a `Cargo.toml` sits at the root or one level down.
fn is_rust(dir: &Path) -> bool {
    dir.join(CARGO_TOML).is_file() || child_dirs(dir).any(|d| d.join(CARGO_TOML).is_file())
}

/// The visible subdirectories of `dir`, minus [`SKIP_DIRS`].
fn child_dirs(dir: &Path) -> impl Iterator<Item = std::path::PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.starts_with('.') && !SKIP_DIRS.contains(&n))
        })
}

/// True when a file with extension `ext` sits in `dir` or up to `depth`
/// directory levels below it.
fn has_ext(dir: &Path, ext: &str, depth: usize) -> bool {
    let here = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().is_file() && e.path().extension().is_some_and(|x| x == ext));
    here || (depth > 0 && child_dirs(dir).any(|d| has_ext(&d, ext, depth - 1)))
}

#[cfg(test)]
#[path = "../tests/unit/stack_tests.rs"]
mod tests;
