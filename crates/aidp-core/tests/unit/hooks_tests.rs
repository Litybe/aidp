use super::*;

const SETTINGS_JSON: &str = r#"{
  "theme": "dark",
  "permissions": {"allow": ["Bash"]},
  "hooks": {
    "Stop": [
      {"hooks": [{"type": "command", "command": "aidp run summary", "timeout": 5}]},
      {"hooks": [{"type": "command", "command": "aidp ci hook stop"}]}
    ]
  }
}"#;

fn sample() -> HookSettings {
    serde_json::from_str(SETTINGS_JSON).unwrap()
}

fn entry(matcher: Option<&str>, command: &str) -> HookMatcherEntry {
    HookMatcherEntry {
        matcher: matcher.map(str::to_string),
        hooks: vec![HookCommand {
            kind: Some("command".to_string()),
            command: Some(command.to_string()),
            extra: Map::new(),
        }],
        extra: Map::new(),
    }
}

#[test]
fn merge_into_empty_adds_every_hook() {
    let mut target = HookSettings::default();
    assert_eq!(merge_hooks(&mut target, &sample()), 2);
    assert_eq!(target.hooks["Stop"].len(), 2);
}

#[test]
fn merge_is_idempotent() {
    let source = sample();
    let mut target = HookSettings::default();
    merge_hooks(&mut target, &source);
    let snapshot = target.clone();
    assert_eq!(merge_hooks(&mut target, &source), 0);
    assert_eq!(target, snapshot);
}

#[test]
fn unknown_keys_round_trip() {
    let value = serde_json::to_value(sample()).unwrap();
    assert_eq!(value, serde_json::from_str::<Value>(SETTINGS_JSON).unwrap());
}

#[test]
fn merge_keeps_target_extra_keys() {
    let mut target = sample();
    merge_hooks(&mut target, &HookSettings::default());
    assert_eq!(target.extra["theme"], "dark");
}

#[test]
fn ci_hook_kinds_extracts_the_verb() {
    let commands = vec!["aidp ci hook post-tool".to_string(), "echo hi".to_string()];
    assert_eq!(ci_hook_kinds(&commands), ["post-tool"]);
}

#[test]
fn shared_command_is_a_duplicate() {
    let existing = [entry(None, "aidp run summary")];
    assert!(has_aidp_hook(
        &existing,
        &entry(None, "  aidp run summary ")
    ));
}

#[test]
fn different_matcher_is_not_a_duplicate() {
    let existing = [entry(Some("Bash"), "aidp ci hook stop")];
    assert!(!has_aidp_hook(
        &existing,
        &entry(Some("Edit"), "aidp ci hook stop")
    ));
}

#[test]
fn same_ci_kind_with_other_binary_path_is_a_duplicate() {
    let existing = [entry(None, "/x/aidp ci hook stop")];
    assert!(has_aidp_hook(&existing, &entry(None, "aidp ci hook stop")));
}

#[test]
fn ci_candidate_never_matches_guard_hook() {
    let existing = [entry(None, "aidp run summary")];
    assert!(!has_aidp_hook(&existing, &entry(None, "aidp ci hook stop")));
}

#[test]
fn two_guard_hooks_are_duplicates() {
    let existing = [entry(None, "aidp scope check")];
    assert!(has_aidp_hook(&existing, &entry(None, "aidp run summary")));
}
