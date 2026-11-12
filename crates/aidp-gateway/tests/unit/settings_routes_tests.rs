use super::*;
use aidp_testkit::TempWorkspace;
use std::fs;

const CONFIG: &str = include_str!("../fixtures/settings/config.yaml");
const ROUTING: &str = include_str!("../fixtures/settings/routing.yaml");
const TEMP_FILE: &str = "config.yaml.settings-tmp";

fn vault_with(config: Option<&str>, routing: Option<&str>) -> TempWorkspace {
    let vault = TempWorkspace::new("settings-vault");
    let todo = vault.path().join(TODO_DIR);
    fs::create_dir_all(&todo).unwrap();
    if let Some(text) = config {
        fs::write(todo.join(CONFIG_FILE), text).unwrap();
    }
    if let Some(text) = routing {
        fs::write(todo.join(ROUTING_FILE), text).unwrap();
    }
    vault
}

fn text_of(vault: &TempWorkspace, name: &str) -> String {
    fs::read_to_string(vault.path().join(TODO_DIR).join(name)).unwrap()
}

fn paths_of(outcome: Outcome) -> Vec<String> {
    match outcome {
        Err(SettingsFailure::Issues(issues)) => {
            issues.into_iter().map(|found| found.path).collect()
        }
        other => panic!("expected issues, got {other:?}"),
    }
}

fn assert_reported(paths: &[String], expected: &[&str]) {
    for path in expected {
        assert!(
            paths.iter().any(|found| found == path),
            "missing {path} in {paths:?}"
        );
    }
}

#[test]
fn reading_the_budget_of_a_missing_file_gives_the_defaults() {
    let vault = vault_with(None, None);

    let body = read(vault.path(), SettingsSection::Budget).unwrap();

    assert_eq!(body["budget"]["per_ticket"]["code"], 3_000_000);
    assert_eq!(body["budget"]["pixellab"]["per_ticket"], 10);
    assert_eq!(body["dispatcher"]["slots"], 3);
    assert_eq!(body["dispatcher"]["test_slots"], 1);
    assert_eq!(body["dispatcher"]["bug_slots"], 1);
    assert_eq!(body["dispatcher"]["night"]["start"], "22:00");
    assert_eq!(body["dispatcher"]["stop_at_7d_percent"], Value::Null);
}

#[test]
fn a_budget_put_changes_only_the_given_keys_and_keeps_the_comments() {
    let vault = vault_with(Some(CONFIG), None);
    let body = json!({
        "budget": { "per_ticket": { "code": "4M", "art": 900000 }, "pixellab": { "min_reserve": 2.5 } },
        "dispatcher": { "slots": 2, "test_slots": 2, "bug_slots": 2, "stop_at_7d_percent": 85, "night": { "enabled": true } },
    });

    let (reply, changed) = write(vault.path(), SettingsSection::Budget, &body).unwrap();

    assert!(changed);
    assert_eq!(reply["reloadRequired"], false);
    assert_eq!(reply["budget"]["per_ticket"]["code"], 4_000_000);
    assert_eq!(reply["budget"]["per_ticket"]["art"], 900_000);
    assert_eq!(reply["dispatcher"]["slots"], 2);
    assert_eq!(reply["dispatcher"]["test_slots"], 2);
    assert_eq!(reply["dispatcher"]["bug_slots"], 2);
    assert_eq!(reply["dispatcher"]["stop_at_7d_percent"], 85.0);
    assert_eq!(reply["dispatcher"]["night"]["enabled"], true);
    assert_eq!(reply["dispatcher"]["night"]["end"], "07:00");
    let text = text_of(&vault, CONFIG_FILE);
    assert!(text.contains("# aidp ticket dispatcher configuration"));
    assert!(text.contains("# tickets in-progress at once"));
    assert!(text.contains("dispatcher_extra_note: kept"));
    assert!(text.contains("    code: 4M"));
    assert!(!vault.path().join(TODO_DIR).join(TEMP_FILE).exists());
}

#[test]
fn a_null_stop_threshold_removes_the_key() {
    let vault = vault_with(
        Some("dispatcher:\n  slots: 2\n  stop_at_7d_percent: 90\n"),
        None,
    );

    let (reply, changed) = write(
        vault.path(),
        SettingsSection::Budget,
        &json!({ "dispatcher": { "stop_at_7d_percent": null } }),
    )
    .unwrap();

    assert!(changed);
    assert_eq!(reply["dispatcher"]["stop_at_7d_percent"], Value::Null);
    assert_eq!(text_of(&vault, CONFIG_FILE), "dispatcher:\n  slots: 2\n");
}

#[test]
fn a_put_that_changes_nothing_writes_nothing() {
    let vault = vault_with(Some(CONFIG), None);

    let (_, changed) = write(
        vault.path(),
        SettingsSection::Budget,
        &json!({ "dispatcher": { "slots": 5 }, "budget": { "learn": true, "max_utilization_5h": 0.8 } }),
    )
    .unwrap();

    assert!(!changed);
    assert_eq!(text_of(&vault, CONFIG_FILE), CONFIG);
}

#[test]
fn a_budget_put_without_a_file_starts_from_the_commented_defaults() {
    let vault = vault_with(None, None);

    write(
        vault.path(),
        SettingsSection::Budget,
        &json!({ "dispatcher": { "slots": 1 } }),
    )
    .unwrap();

    assert!(text_of(&vault, CONFIG_FILE).contains("# Model policy"));
    assert_eq!(config::load(vault.path()).dispatcher.slots, 1);
}

#[test]
fn every_refused_budget_value_is_reported_with_its_path() {
    let vault = vault_with(Some(CONFIG), None);
    let body = json!({
        "budget": {
            "per_ticket": { "code": "lots", "art": -1, "bogus": 1 },
            "max_utilization_5h": 80,
            "unknown_policy": "maybe",
            "pixellab": { "per_ticket": 1.5 },
        },
        "dispatcher": {
            "slots": 0,
            "stop_at_7d_percent": 150,
            "night": { "start": "25:00", "heavy_window_minutes": 0, "enabled": "yes" },
        },
        "other": 1,
    });

    let paths = paths_of(write(vault.path(), SettingsSection::Budget, &body));

    assert_reported(
        &paths,
        &[
            "budget.per_ticket.code",
            "budget.per_ticket.art",
            "budget.per_ticket.bogus",
            "budget.max_utilization_5h",
            "budget.unknown_policy",
            "budget.pixellab.per_ticket",
            "dispatcher.slots",
            "dispatcher.stop_at_7d_percent",
            "dispatcher.night.start",
            "dispatcher.night.heavy_window_minutes",
            "dispatcher.night.enabled",
            "other",
        ],
    );
    assert_eq!(text_of(&vault, CONFIG_FILE), CONFIG);
}

#[test]
fn a_config_file_that_is_not_yaml_is_a_conflict_not_an_overwrite() {
    let vault = vault_with(Some("dispatcher: [unclosed\n"), None);

    let outcome = write(
        vault.path(),
        SettingsSection::Budget,
        &json!({ "dispatcher": { "slots": 2 } }),
    );

    assert!(matches!(
        outcome,
        Err(SettingsFailure::Api(ApiStatus::Conflict, _))
    ));
    assert_eq!(text_of(&vault, CONFIG_FILE), "dispatcher: [unclosed\n");
}

#[test]
fn a_flow_style_section_is_rewritten_from_the_parsed_document() {
    let vault = vault_with(Some("budget: { learn: true }\n"), None);

    write(
        vault.path(),
        SettingsSection::Budget,
        &json!({ "budget": { "pixellab": { "min_reserve": 3 } } }),
    )
    .unwrap();

    let loaded = config::load(vault.path());
    assert_eq!(loaded.budget.pixellab.min_reserve, 3.0);
    assert!(loaded.budget.learn);
}

#[test]
fn routing_round_trips_and_a_put_of_the_same_content_leaves_the_file_alone() {
    let vault = vault_with(None, Some(ROUTING));
    let current = read(vault.path(), SettingsSection::Routing).unwrap();

    assert_eq!(current["rules"].as_array().unwrap().len(), 10);
    assert_eq!(current["rules"][0]["match"]["labels"][0], "pixelart");
    assert_eq!(current["rules"][0]["service"], "pixelhood-art");
    assert_eq!(current["repos"]["pixelhood-bug"]["track"], "bug");
    assert_eq!(current["repos"]["pixelhood-art"]["flow"], "game-art");
    assert_eq!(current["defaults"]["track"], "standard");

    let (_, changed) = write(vault.path(), SettingsSection::Routing, &current).unwrap();

    assert!(!changed);
    assert_eq!(text_of(&vault, ROUTING_FILE), ROUTING);
}

#[test]
fn a_routing_put_replaces_the_given_sections_only() {
    let vault = vault_with(None, Some(ROUTING));
    let body = json!({
        "repos": { "pixelhood-code": { "route": "code", "flow": "game-code-fast", "track": "quick" } },
        "rules": [ { "match": { "label": "bug" }, "route": "pixelhood-code" } ],
    });

    let (reply, changed) = write(vault.path(), SettingsSection::Routing, &body).unwrap();

    assert!(changed);
    assert_eq!(reply["repos"].as_object().unwrap().len(), 1);
    assert_eq!(reply["repos"]["pixelhood-code"]["flow"], "game-code-fast");
    assert_eq!(reply["rules"][0]["service"], "pixelhood-code");
    assert_eq!(reply["defaults"]["track"], "standard");
    assert!(text_of(&vault, ROUTING_FILE).starts_with("# TODO/routing.yaml for PixelHood"));
    assert_eq!(
        config::load_routing(vault.path()).flow_for("pixelhood-code"),
        Some("game-code-fast")
    );
}

#[test]
fn every_refused_routing_value_is_reported_with_its_path() {
    let vault = vault_with(None, Some(ROUTING));
    let body = json!({
        "defaults": { "track": "slow", "color": "red" },
        "rules": [
            { "match": {}, "service": "a" },
            { "match": { "labels": [], "summary_regex": "(", "colour": 1 }, "service": "a" },
            { "match": { "label": "x" } },
            { "match": { "label": "x" }, "service": "a", "route": "b" },
            "text",
        ],
        "repos": {
            "ok": { "route": "code" },
            "bad": { "route": "sound", "track": "fast", "flow": "", "mystery": 1 },
            "worse": 3,
        },
        "extra": 1,
    });

    let paths = paths_of(write(vault.path(), SettingsSection::Routing, &body));

    assert_reported(
        &paths,
        &[
            "defaults.track",
            "defaults.color",
            "rules[0].match",
            "rules[1].match.labels",
            "rules[1].match.summary_regex",
            "rules[1].match.colour",
            "rules[2].service",
            "rules[3].route",
            "rules[4]",
            "repos.bad.route",
            "repos.bad.track",
            "repos.bad.flow",
            "repos.bad.mystery",
            "repos.worse",
            "extra",
        ],
    );
    assert_eq!(text_of(&vault, ROUTING_FILE), ROUTING);
}

#[test]
fn a_routing_put_creates_the_missing_file() {
    let vault = vault_with(None, None);

    write(
        vault.path(),
        SettingsSection::Routing,
        &json!({ "repos": { "svc": { "template": "dev" } }, "rules": [] }),
    )
    .unwrap();

    assert_eq!(
        config::load_routing(vault.path()).template_for("svc"),
        Some("dev")
    );
}

#[test]
fn the_model_policy_is_read_replaced_and_dropped() {
    let vault = vault_with(Some(CONFIG), None);
    let read_back = read(vault.path(), SettingsSection::Models).unwrap();
    assert_eq!(read_back["models"], json!({}));
    assert_eq!(read_back["routes"], json!(["code", "art", "map", "doc"]));
    assert_eq!(
        read_back["tiers"],
        json!(["haiku", "sonnet", "opus", "fable"])
    );
    assert!(read_back["stages"]
        .as_array()
        .unwrap()
        .contains(&json!("verdict")));

    let body = json!({ "models": { "code": { "report": "haiku", "review": "sonnet" }, "art": { "review": "opus" } } });
    let (reply, changed) = write(vault.path(), SettingsSection::Models, &body).unwrap();

    assert!(changed);
    assert_eq!(reply["models"]["code"]["report"], "haiku");
    assert_eq!(load_policy(vault.path()).len(), 2);
    let text = text_of(&vault, CONFIG_FILE);
    assert!(text.contains("# aidp ticket dispatcher configuration"));
    assert!(text.contains("dispatcher_extra_note: kept"));
    assert_eq!(config::load(vault.path()).dispatcher.slots, 5);

    let (_, changed_again) = write(vault.path(), SettingsSection::Models, &body).unwrap();
    assert!(!changed_again);

    let (cleared, _) = write(
        vault.path(),
        SettingsSection::Models,
        &json!({ "models": {} }),
    )
    .unwrap();
    assert_eq!(cleared["models"], json!({}));
    assert!(!text_of(&vault, CONFIG_FILE).contains("\nmodels:"));
}

#[test]
fn every_refused_model_value_is_reported_with_its_path() {
    let vault = vault_with(Some(CONFIG), None);
    let body = json!({ "models": { "code": { "report": "gpt", "bad stage": "haiku" }, "sound": { "x": "haiku" }, "doc": 3 } });

    let paths = paths_of(write(vault.path(), SettingsSection::Models, &body));

    assert_reported(
        &paths,
        &[
            "models.code.report",
            "models.code.bad stage",
            "models.sound",
            "models.doc",
        ],
    );
    assert_eq!(
        paths_of(write(vault.path(), SettingsSection::Models, &json!({}))),
        ["models"]
    );
}

#[test]
fn a_missing_vault_is_a_configuration_error() {
    let outcome = write(Path::new(""), SettingsSection::Budget, &json!({}));

    assert!(matches!(
        outcome,
        Err(SettingsFailure::Api(ApiStatus::Config, _))
    ));
}

#[test]
fn only_the_three_sections_are_routed() {
    assert_eq!(
        SettingsSection::parse("routing"),
        Some(SettingsSection::Routing)
    );
    assert_eq!(SettingsSection::parse("unknown"), None);
}
