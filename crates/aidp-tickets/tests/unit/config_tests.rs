//! Tests for the Jira defaults of the dispatcher configuration.

use super::*;

/// Dispatcher config selecting the flow launch mode.
const FLOW_LAUNCH_CONFIG: &str = "dispatcher:\n  launch: flow\n";

/// A config without `launch` launches sessions.
#[test]
fn config_without_launch_defaults_to_session() {
    let cfg: Config = serde_yaml::from_str("dispatcher:\n  slots: 2\n").unwrap();
    assert_eq!(cfg.dispatcher.launch, DispatcherLaunch::Session);
    assert_eq!(
        DispatcherConfig::default().launch,
        DispatcherLaunch::Session
    );
}

/// `launch: flow` selects the flow launch mode.
#[test]
fn config_launch_flow_parses_to_flow() {
    let cfg: Config = serde_yaml::from_str(FLOW_LAUNCH_CONFIG).unwrap();
    assert_eq!(cfg.dispatcher.launch, DispatcherLaunch::Flow);
}

/// `per_ticket` takes numbers and suffixed amounts and keeps the defaults of the other routes.
#[test]
fn per_ticket_budget_parses_suffixed_amounts() {
    let cfg: Config =
        serde_yaml::from_str("budget:\n  per_ticket:\n    code: 4M\n    doc: 250000\n").unwrap();
    let per_ticket = &cfg.budget.per_ticket;
    assert_eq!(per_ticket.limit_for(Some(BudgetRoute::Code)), 4_000_000);
    assert_eq!(per_ticket.limit_for(Some(BudgetRoute::Doc)), 250_000);
    assert_eq!(per_ticket.limit_for(Some(BudgetRoute::Art)), 1_500_000);
    assert_eq!(per_ticket.limit_for(None), 2_000_000);
}

/// Night mode is off by default and `stop_at_7d_percent` is unset.
#[test]
fn night_mode_defaults_to_off() {
    let cfg = Config::default();
    assert!(!cfg.dispatcher.night.enabled);
    assert_eq!(cfg.dispatcher.stop_at_7d_percent, None);
}

/// The model policy reads route, stage and tier.
#[test]
fn models_policy_parses() {
    let cfg: Config = serde_yaml::from_str("models:\n  art:\n    review: sonnet\n").unwrap();
    assert_eq!(
        cfg.models["art"]["review"],
        aidp_core::models::enums::ModelTier::Sonnet
    );
}

/// Pixellab defaults: no reserve, ten generations per ticket.
#[test]
fn pixellab_budget_defaults() {
    let pixellab = Config::default().budget.pixellab;
    assert_eq!(pixellab.min_reserve, 0.0);
    assert_eq!(pixellab.per_ticket, 10);
}
