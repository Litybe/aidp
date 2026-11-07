//! Compiles a scripted fake `gh` binary shared by the `aidp gh` end-to-end tests.

use std::fs;
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

/// Compiles the fake `gh` into `<dir>/bin`, writes `reply` as its default stdout and returns the bin dir.
pub fn install_gh_shim(dir: &Path, reply: &str) -> PathBuf {
    let bin_dir = dir.join(SHIM_DIR);
    fs::create_dir_all(&bin_dir).expect("create shim dir");
    let source = bin_dir.join(SHIM_SOURCE_FILE);
    fs::write(&source, shim_source()).expect("write shim source");
    let compiled = Command::new(RUSTC)
        .arg(&source)
        .arg(OUTPUT_FLAG)
        .arg(bin_dir.join(SHIM_EXE))
        .status()
        .expect("run rustc");
    assert!(compiled.success(), "shim must compile");
    fs::write(bin_dir.join(REPLY_FILE), reply).expect("write canned reply");
    bin_dir
}
