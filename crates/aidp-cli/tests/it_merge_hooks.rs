//! `aidp setup merge-hooks` must be idempotent over settings that already hold
//! the Stop hooks.

use crate::common;
use common::*;
use std::fs;

/// Runs merge-hooks once and returns the reported `hooks_added`.
fn merge_once(ws: &aidp_testkit::TempWorkspace, target: &std::path::Path) -> u64 {
    let example = skill_root().join("hooks/settings.example.json");
    let output = aidp(ws)
        .args(["setup", "merge-hooks", "--example"])
        .arg(example)
        .arg("--target")
        .arg(target)
        .output()
        .expect("run aidp");
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    report["hooks_added"]
        .as_u64()
        .expect("hooks_added is a number")
}

/// A second merge adds nothing and leaves the file bytes untouched.
#[test]
fn second_merge_adds_nothing_and_keeps_bytes() {
    let ws = workspace();
    let target = ws.child("home/settings.json");
    let fixture = crate_dir().join("tests/fixtures/merge_hooks/settings.json");
    fs::create_dir_all(target.parent().expect("target has a parent")).expect("create settings dir");
    fs::copy(fixture, &target).expect("copy settings fixture");

    merge_once(&ws, &target);
    let after_first = fs::read(&target).expect("read settings");
    let added_second = merge_once(&ws, &target);
    let after_second = fs::read(&target).expect("read settings");

    assert_eq!(added_second, 0);
    assert_eq!(after_first, after_second);
}
