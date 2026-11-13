//! Compiles a scripted fake `gh` binary shared by the `aidp gh` end-to-end tests.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SHIM_DIR: &str = "bin";
pub const RUSTC: &str = "rustc";
pub const REPLY_FILE: &str = "reply.json";
pub const INPUT_FLAG: &str = "--input";
pub const BODIES_FILE: &str = "gh.bodies";
pub const RULES_FILE: &str = "gh.rules";
pub const CALLS_FILE: &str = "gh.calls";
const SHIM_SOURCE_FILE: &str = "gh_shim.rs";
const SHIM_EXE: &str = if cfg!(windows) { "gh.exe" } else { "gh" };
const OUTPUT_FLAG: &str = "-o";
const CACHE_DIR: &str = "gh_shim_cache";
const CACHE_EXE_PREFIX: &str = "gh-";
const BUILD_EXE_PREFIX: &str = "build-";
const LOCK_SUFFIX: &str = ".lock";
const CACHE_EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };
const INPUT_FLAG_TOKEN: &str = "@INPUT_FLAG@";
const BODIES_FILE_TOKEN: &str = "@BODIES_FILE@";
const REPLY_FILE_TOKEN: &str = "@REPLY_FILE@";
const RULES_FILE_TOKEN: &str = "@RULES_FILE@";
const CALLS_FILE_TOKEN: &str = "@CALLS_FILE@";
const SHIM_TEMPLATE: &str = r#"
use std::io::Write;

fn append_line(path: std::path::PathBuf, line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

fn main() {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let joined = args.join(" ");
    append_line(dir.join("@CALLS_FILE@"), &joined);
    if let Some(position) = args.iter().position(|arg| arg == "@INPUT_FLAG@") {
        let body = std::fs::read_to_string(&args[position + 1]).unwrap_or_default();
        append_line(dir.join("@BODIES_FILE@"), &body);
    }
    let rules = std::fs::read_to_string(dir.join("@RULES_FILE@")).unwrap_or_default();
    for line in rules.lines() {
        let mut parts = line.splitn(3, '\t');
        let needle = parts.next().unwrap_or("");
        let code: i32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let stdout_file = parts.next().unwrap_or("");
        if !needle.is_empty() && joined.contains(needle) {
            if !stdout_file.is_empty() {
                print!("{}", std::fs::read_to_string(stdout_file).unwrap_or_default());
            }
            if code != 0 {
                eprintln!("refused");
            }
            std::process::exit(code);
        }
    }
    print!("{}", std::fs::read_to_string(dir.join("@REPLY_FILE@")).unwrap_or_default());
}
"#;

/// Renders the shim source with every file and flag name injected from the constants.
fn shim_source() -> String {
    SHIM_TEMPLATE
        .replace(INPUT_FLAG_TOKEN, INPUT_FLAG)
        .replace(BODIES_FILE_TOKEN, BODIES_FILE)
        .replace(REPLY_FILE_TOKEN, REPLY_FILE)
        .replace(RULES_FILE_TOKEN, RULES_FILE)
        .replace(CALLS_FILE_TOKEN, CALLS_FILE)
}

/// Returns the shim executable compiled once per source and shared by every test process.
fn cached_shim_exe(rendered: &str, source: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    rendered.hash(&mut hasher);
    let cache_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(CACHE_DIR);
    fs::create_dir_all(&cache_dir).expect("create shim cache dir");
    let cached = cache_dir.join(format!(
        "{CACHE_EXE_PREFIX}{:016x}{CACHE_EXE_SUFFIX}",
        hasher.finish()
    ));
    if cached.is_file() {
        return cached;
    }
    let mut lock_name = cached.clone().into_os_string();
    lock_name.push(LOCK_SUFFIX);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PathBuf::from(lock_name))
        .expect("open shim lock");
    lock_file.lock().expect("lock shim compilation");
    if !cached.is_file() {
        let building = cache_dir.join(format!(
            "{BUILD_EXE_PREFIX}{}{CACHE_EXE_SUFFIX}",
            std::process::id()
        ));
        let compiled = Command::new(RUSTC)
            .arg(source)
            .arg(OUTPUT_FLAG)
            .arg(&building)
            .status()
            .expect("run rustc");
        assert!(compiled.success(), "shim must compile");
        if fs::rename(&building, &cached).is_err() {
            let _ = fs::remove_file(&building);
        }
    }
    drop(lock_file);
    assert!(cached.is_file(), "cached shim must exist");
    cached
}

/// Writes `source_text` into `bin_dir` and installs its executable there as the fake `gh`, compiled once per source.
pub fn install_compiled_shim(bin_dir: &Path, source_text: &str) {
    let source = bin_dir.join(SHIM_SOURCE_FILE);
    fs::write(&source, source_text).expect("write shim source");
    fs::copy(
        cached_shim_exe(source_text, &source),
        bin_dir.join(SHIM_EXE),
    )
    .expect("copy cached shim");
}

/// Installs the fake `gh` into `<dir>/bin`, writes `reply` as its default stdout and returns the bin dir.
pub fn install_gh_shim(dir: &Path, reply: &str) -> PathBuf {
    let bin_dir = dir.join(SHIM_DIR);
    fs::create_dir_all(&bin_dir).expect("create shim dir");
    install_compiled_shim(&bin_dir, &shim_source());
    fs::write(bin_dir.join(REPLY_FILE), reply).expect("write canned reply");
    bin_dir
}
