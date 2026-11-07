use super::*;
use aidp_testkit::TempWorkspace;

const PIPELINE: &str = r#"
version: 5
id: t
stages:
  - name: plan
    agent: planner
  - name: implement
  - name: review
    model: sonnet
  - name: report
"#;

const CONFIG: &str =
    "models:\n  code:\n    report: haiku\n    review: opus\n  art:\n    review: sonnet\n";

fn pipe() -> Pipeline {
    serde_yaml::from_str(PIPELINE).unwrap()
}

fn vault_with_config(tag: &str) -> TempWorkspace {
    let vault = TempWorkspace::new(tag);
    std::fs::create_dir_all(vault.path().join("TODO")).unwrap();
    std::fs::write(vault.path().join("TODO").join("config.yaml"), CONFIG).unwrap();
    vault
}

fn state_for_route(route: Option<&str>) -> RunState {
    let mut state = RunState::default();
    if let Some(route) = route {
        state
            .inputs
            .insert(INPUT_ROUTE.to_string(), serde_json::json!(route));
    }
    state
}

#[test]
fn policy_is_read_from_the_vault_config() {
    let vault = vault_with_config("policy-read");
    let policy = load_policy(vault.path());
    assert_eq!(policy["code"]["report"], ModelTier::Haiku);
    assert_eq!(policy["art"]["review"], ModelTier::Sonnet);
}

#[test]
fn missing_or_invalid_config_gives_an_empty_policy() {
    let vault = TempWorkspace::new("policy-missing");
    assert!(load_policy(vault.path()).is_empty());
    std::fs::create_dir_all(vault.path().join("TODO")).unwrap();
    std::fs::write(
        vault.path().join("TODO").join("config.yaml"),
        "models:\n  code:\n    report: gigantic\n",
    )
    .unwrap();
    assert!(load_policy(vault.path()).is_empty());
}

#[test]
fn apply_pins_the_stages_of_the_input_route() {
    let vault = vault_with_config("policy-apply");
    let mut state = state_for_route(Some("code"));
    assert_eq!(apply(&mut state, Some(vault.path()), &BTreeMap::new()), 2);
    assert_eq!(state.model_overrides["report"], ModelTier::Haiku);
    assert_eq!(state.model_overrides["review"], ModelTier::Opus);
}

#[test]
fn apply_without_a_route_pins_nothing() {
    let vault = vault_with_config("policy-no-route");
    let mut state = state_for_route(None);
    assert_eq!(apply(&mut state, Some(vault.path()), &BTreeMap::new()), 0);
}

#[test]
fn cli_override_wins_over_the_policy() {
    let vault = vault_with_config("policy-cli");
    let mut state = state_for_route(Some("code"));
    let cli = parse_cli_overrides(&["report=opus".to_string()]).unwrap();
    apply(&mut state, Some(vault.path()), &cli);
    assert_eq!(state.model_overrides["report"], ModelTier::Opus);
    assert_eq!(state.model_overrides["review"], ModelTier::Opus);
}

#[test]
fn malformed_cli_override_is_refused() {
    assert!(parse_cli_overrides(&["report".to_string()]).is_err());
    assert!(parse_cli_overrides(&["=opus".to_string()]).is_err());
    assert!(parse_cli_overrides(&["report=gigantic".to_string()]).is_err());
}

#[test]
fn effective_tiers_follow_override_then_template_then_role() {
    let overrides = BTreeMap::from([("report".to_string(), ModelTier::Haiku)]);
    let tiers: BTreeMap<String, ModelTier> =
        effective_tiers(&pipe(), &overrides).into_iter().collect();
    assert_eq!(tiers["plan"], ModelTier::Opus);
    assert_eq!(tiers["implement"], ModelTier::Sonnet);
    assert_eq!(tiers["review"], ModelTier::Sonnet);
    assert_eq!(tiers["report"], ModelTier::Haiku);
}

#[test]
fn a_run_is_heavy_while_an_opus_stage_remains() {
    assert!(is_heavy(&pipe(), &BTreeMap::new()));
    let downgraded = BTreeMap::from([("plan".to_string(), ModelTier::Sonnet)]);
    assert!(!is_heavy(&pipe(), &downgraded));
}
