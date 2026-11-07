//! Argv recorder used by `ProcShim` on Windows.
//!
//! Writes each argument on its own line to the file named by `ARGV_FILE`, or,
//! when that variable is unset, to `<own stem>.argv` next to this executable.
//! `ProcShim` copies this binary as `<name>.exe`, so the second form is what
//! keeps two shims from sharing one file. Each call also appends one line to
//! `<stem>.calls` and replays `<stem>.stdout` when that file exists.

use aidp_testkit::proc_shim::{ARG_SEPARATOR, CALLS_EXTENSION, STDOUT_EXTENSION};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let own_path = std::env::current_exe().expect("argv_recorder: resolve own path");
    let target = match std::env::var_os("ARGV_FILE") {
        Some(p) => PathBuf::from(p),
        None => own_path.with_extension("argv"),
    };
    let mut out = std::fs::File::create(&target).expect("argv_recorder: create argv file");
    let mut call_line = String::new();
    for arg in std::env::args_os().skip(1) {
        let text = arg.to_string_lossy();
        writeln!(out, "{text}").expect("argv_recorder: write argv");
        call_line.push_str(&text);
        call_line.push(ARG_SEPARATOR);
    }
    let mut calls = OpenOptions::new()
        .create(true)
        .append(true)
        .open(own_path.with_extension(CALLS_EXTENSION))
        .expect("argv_recorder: open calls file");
    writeln!(calls, "{call_line}").expect("argv_recorder: write calls");
    if let Ok(canned) = std::fs::read(own_path.with_extension(STDOUT_EXTENSION)) {
        std::io::stdout()
            .write_all(&canned)
            .expect("argv_recorder: write stdout");
    }
}
