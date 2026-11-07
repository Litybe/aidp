//! `aidp gh pr-fix` against a fake daemon that records the request body.

use crate::common::*;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Output;
use std::thread::JoinHandle;

const LOOPBACK: &str = "127.0.0.1:0";
const GH_ARG: &str = "gh";
const PR_FIX_ARG: &str = "pr-fix";
const URL_FLAG: &str = "--url";
const THREADS_FLAG: &str = "--threads";
const POINTS_FLAG: &str = "--points";
const NOTE_FILE_FLAG: &str = "--note-file";
const MODEL_FLAG: &str = "--model";
const EFFORT_FLAG: &str = "--effort";
const REPO_SLUG: &str = "owner/name";
const PR_NUMBER: &str = "7";
const THREAD_IDS: &str = "T-1,T-2";
const POINT_IDS: &str = "P-1";
const MODEL: &str = "sonnet";
const EFFORT: &str = "high";
const STDIN_MARKER: &str = "-";
const NOTE_FILE: &str = "note.md";
const NOTE_TEXT: &str = "keep the public API";
const STDIN_NOTE_TEXT: &str = "note from stdin";
const HEADER_END: &str = "\r\n\r\n";
const CONTENT_LENGTH: &str = "content-length:";
const OK_STATUS: &str = "200 OK";
const FORBIDDEN_STATUS: &str = "403 Forbidden";
const OK_REPLY: &str = r#"{"sessionId":"S-1"}"#;
const REFUSAL_REPLY: &str = r#"{"error":"not-mine"}"#;
const REFUSAL_MESSAGE: &str = "not-mine";
const POLICY_REFUSAL_EXIT: i32 = 4;

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut received = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let count = stream.read(&mut chunk).expect("read request");
        received.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8_lossy(&received).to_string();
        if let Some(split) = text.find(HEADER_END) {
            let head = text[..split].to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix(CONTENT_LENGTH))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if received.len() >= split + HEADER_END.len() + length {
                return text[split + HEADER_END.len()..].to_string();
            }
        }
        if count == 0 {
            return String::new();
        }
    }
}

fn fake_daemon(status: &'static str, reply: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind(LOOPBACK).expect("bind fake daemon");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let body = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
            reply.len()
        );
        stream.write_all(response.as_bytes()).expect("write reply");
        body
    });
    (address, handle)
}

fn run_pr_fix(
    ws: &aidp_testkit::TempWorkspace,
    address: &str,
    extra: &[&str],
    stdin: Option<&str>,
) -> Output {
    let mut command = aidp(ws);
    command
        .args([GH_ARG, PR_FIX_ARG, REPO_SLUG, PR_NUMBER, URL_FLAG, address])
        .args(extra);
    if let Some(text) = stdin {
        command.write_stdin(text);
    }
    command.output().expect("run aidp gh pr-fix")
}

/// Only the selected threads and points reach the daemon, with the optional fields.
#[test]
fn gh_pr_fix_posts_the_selected_threads() {
    let ws = workspace();
    let note_path = ws.path().join(NOTE_FILE);
    fs::write(&note_path, NOTE_TEXT).expect("write note");
    let note = note_path.to_string_lossy().to_string();
    let (address, daemon) = fake_daemon(OK_STATUS, OK_REPLY);
    let output = run_pr_fix(
        &ws,
        &address,
        &[
            THREADS_FLAG,
            THREAD_IDS,
            POINTS_FLAG,
            POINT_IDS,
            NOTE_FILE_FLAG,
            &note,
            MODEL_FLAG,
            MODEL,
            EFFORT_FLAG,
            EFFORT,
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let reply: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(reply["sessionId"], "S-1");
    let body: Value = serde_json::from_str(&daemon.join().expect("daemon")).expect("request body");
    assert_eq!(body["repo"], REPO_SLUG);
    assert_eq!(body["number"], 7);
    assert_eq!(body["threads"], serde_json::json!(["T-1", "T-2"]));
    assert_eq!(body["points"], serde_json::json!(["P-1"]));
    assert_eq!(body["note"], NOTE_TEXT);
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["effort"], EFFORT);
}

/// `--note-file -` takes the note from standard input.
#[test]
fn gh_pr_fix_reads_the_note_from_stdin() {
    let ws = workspace();
    let (address, daemon) = fake_daemon(OK_STATUS, OK_REPLY);
    let output = run_pr_fix(
        &ws,
        &address,
        &[THREADS_FLAG, THREAD_IDS, NOTE_FILE_FLAG, STDIN_MARKER],
        Some(STDIN_NOTE_TEXT),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let body: Value = serde_json::from_str(&daemon.join().expect("daemon")).expect("request body");
    assert_eq!(body["note"], STDIN_NOTE_TEXT);
}

/// A 403 from the daemon exits 4 and prints the refusal on stderr.
#[test]
fn gh_pr_fix_maps_403_to_the_policy_refusal_exit() {
    let ws = workspace();
    let (address, daemon) = fake_daemon(FORBIDDEN_STATUS, REFUSAL_REPLY);
    let output = run_pr_fix(&ws, &address, &[THREADS_FLAG, THREAD_IDS], None);
    daemon.join().expect("daemon");
    assert_eq!(
        output.status.code(),
        Some(POLICY_REFUSAL_EXIT),
        "{output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(REFUSAL_MESSAGE), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "stdout: {:?}", output.stdout);
}
