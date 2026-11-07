//! `aidp gh pr-comment` posts a conversation comment through a scripted fake `gh`.

use crate::common;
use common::gh_shim::{install_gh_shim, BODIES_FILE};
use common::*;
use serde_json::Value;
use std::fs;

const GH: &str = "gh";
const PR_COMMENT: &str = "pr-comment";
const BODY_FILE_FLAG: &str = "--body-file";
const STDIN_MARKER: &str = "-";
const PATH_VAR: &str = "PATH";
const REPO_SLUG: &str = "owner/name";
const PR_NUMBER: &str = "7";
const COMMENT_BODY: &str = "correction applied after the test pass";
const COMMENT_URL: &str = "https://github.com/owner/name/pull/7#issuecomment-99";
const REPLY_JSON: &str =
    r#"{"id":99,"html_url":"https://github.com/owner/name/pull/7#issuecomment-99"}"#;

/// The stdin body reaches the gh call and the reply is emitted as `{id, url}`.
#[test]
fn pr_comment_posts_the_stdin_body_and_emits_id_and_url() {
    let ws = workspace();
    let dir = install_gh_shim(ws.path(), REPLY_JSON);

    let existing = std::env::var_os(PATH_VAR).unwrap_or_default();
    let mut paths = vec![dir.clone()];
    paths.extend(std::env::split_paths(&existing));
    let joined = std::env::join_paths(paths).expect("join PATH");
    let output = aidp(&ws)
        .env(PATH_VAR, joined)
        .args([
            GH,
            PR_COMMENT,
            REPO_SLUG,
            PR_NUMBER,
            BODY_FILE_FLAG,
            STDIN_MARKER,
        ])
        .write_stdin(COMMENT_BODY)
        .output()
        .expect("run aidp gh");

    assert_eq!(output.status.code(), Some(0));
    let reply: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(reply["id"], 99);
    assert_eq!(reply["url"], COMMENT_URL);
    let bodies = fs::read_to_string(dir.join(BODIES_FILE)).expect("gh received a body");
    assert!(bodies.contains(COMMENT_BODY), "bodies: {bodies}");
}
