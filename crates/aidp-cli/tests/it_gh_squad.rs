//! `aidp gh squad-repos` and `aidp gh squad-prs` argument surface, with no network.

use crate::common;
use common::{aidp, workspace};

const GH_ARG: &str = "gh";
const SQUAD_PRS_ARG: &str = "squad-prs";
const SQUAD_REPOS_ARG: &str = "squad-repos";
const BOTS_FLAG: &str = "--bots";
const INVALID_BOTS_VALUE: &str = "bogus";
const HELP_FLAG: &str = "--help";
const USAGE_EXIT: i32 = 7;
const SUCCESS_EXIT: i32 = 0;

#[test]
fn bots_flag_rejects_an_unknown_value_at_parse_time() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([GH_ARG, SQUAD_PRS_ARG, BOTS_FLAG, INVALID_BOTS_VALUE])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(USAGE_EXIT));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("possible values: hide, show, only"),
        "{stderr}"
    );
}

#[test]
fn gh_help_lists_both_squad_verbs() {
    let ws = workspace();
    let output = aidp(&ws).args([GH_ARG, HELP_FLAG]).output().unwrap();

    assert_eq!(output.status.code(), Some(SUCCESS_EXIT));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(SQUAD_REPOS_ARG), "{stdout}");
    assert!(stdout.contains(SQUAD_PRS_ARG), "{stdout}");
}
