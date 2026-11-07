//! `aidp gateway start --workspace` is validated by clap before any daemon call.

use crate::common;
use aidp_core::ExitStatus;
use common::{aidp, workspace};

const UNREACHABLE_DAEMON: &str = "http://127.0.0.1:1";
const VALID_SPECS: [&str; 4] = ["project", "ticket:LOC-1", "scratch:try", "path:/tmp/x"];
const INVALID_SPECS: [&str; 5] = ["bogus", "ticket", "scratch:", "path", "project:extra"];

fn start_with(spec: &str) -> Option<i32> {
    let ws = workspace();
    aidp(&ws)
        .args(["gateway", "start", "--url", UNREACHABLE_DAEMON])
        .args(["--project", "p", "--prompt", "go", "--workspace", spec])
        .output()
        .expect("gateway start should execute")
        .status
        .code()
}

#[test]
fn valid_specs_pass_parsing() {
    for spec in VALID_SPECS {
        assert_ne!(
            start_with(spec),
            Some(ExitStatus::Usage.code() as i32),
            "{spec}"
        );
    }
}

#[test]
fn invalid_specs_are_usage_errors() {
    for spec in INVALID_SPECS {
        assert_eq!(
            start_with(spec),
            Some(ExitStatus::Usage.code() as i32),
            "{spec}"
        );
    }
}
