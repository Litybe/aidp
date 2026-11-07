use super::*;
use aidp_testkit::TempWorkspace;

const SETTINGS_NAME: &str = "settings.json";
const RUN_SUMMARY_ONLY: &str = r#"{
  "hooks": {
"Stop": [
  { "hooks": [ { "type": "command", "command": "aidp run summary --hook" } ] }
]
  }
}
"#;
const RETIRED_CI_HOOKS_ONLY: &str = r#"{
  "hooks": {
"Stop": [
  { "hooks": [ { "type": "command", "command": "aidp ci hook stop" } ] }
]
  }
}
"#;

/// The run-summary Stop hook alone counts as installed hooks, not as the removed guard hooks.
#[test]
fn run_summary_only_settings_detected_as_hooks_installed() {
    let workspace = TempWorkspace::new("hook-detect");
    let path = workspace.child(SETTINGS_NAME);
    std::fs::write(&path, RUN_SUMMARY_ONLY).unwrap();
    assert!(settings_has_aidp_hook(&path));
    assert!(!settings_has_guard_hooks(&path));
}

/// The retired CI autopilot hooks alone no longer count as installed aidp hooks.
#[test]
fn retired_ci_hooks_alone_are_not_aidp_hooks() {
    let workspace = TempWorkspace::new("hook-detect-ci");
    let path = workspace.child(SETTINGS_NAME);
    std::fs::write(&path, RETIRED_CI_HOOKS_ONLY).unwrap();
    assert!(!settings_has_aidp_hook(&path));
}
