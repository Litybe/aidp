use super::{headless_session_refuses, parse_cli_from};

/// Verbs only the flow runs, with global options placed before or inside the verb path.
const REFUSED: &[&[&str]] = &[
    &["aidp", "flow", "git-push"],
    &["aidp", "-v", "flow", "git-push", "--branch", "feat/KEY-1"],
    &[
        "aidp",
        "--format",
        "json-compact",
        "flow",
        "pr-create",
        "KEY-1",
        "--body-file",
        "pr-body.md",
    ],
    &[
        "aidp",
        "gh",
        "--vault",
        "vault",
        "pr-merge",
        "12",
        "--expected-head-sha",
        "abc123",
    ],
    &["aidp", "gh", "pr-update-branch", "12"],
    &["aidp", "gateway", "status"],
];

/// Verbs the pipeline stages run.
const ALLOWED: &[&[&str]] = &[
    &["aidp", "gh", "pr-resolve"],
    &["aidp", "gh", "pr-review-threads", "12"],
    &[
        "aidp",
        "-v",
        "gh",
        "thread-reply",
        "PRRT_1",
        "--body-file",
        "-",
    ],
    &[
        "aidp",
        "gh",
        "pr-comment",
        "owner/repo",
        "12",
        "--body-file",
        "-",
    ],
];

fn command_of(argv: &[&str]) -> String {
    parse_cli_from(argv.iter().copied())
        .unwrap_or_else(|error| panic!("{argv:?} does not parse: {error}"))
        .1
}

#[test]
fn headless_sessions_refuse_flow_verbs_whatever_the_global_options() {
    for argv in REFUSED {
        assert!(headless_session_refuses(&command_of(argv)), "{argv:?}");
    }
}

#[test]
fn headless_sessions_keep_the_stage_verbs() {
    for argv in ALLOWED {
        assert!(!headless_session_refuses(&command_of(argv)), "{argv:?}");
    }
}
