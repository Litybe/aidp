//! Launch gates the dispatcher applies to its candidates besides the budget verdict:
//! the Pixellab credit reserve for the `art` route (T4) and the night window for heavy runs (T6).
//! A held candidate stays queued and shows in the plan with the reason; a manual start is never held.

use crate::pixellab_usage::latest_balance;
use aidp_core::models::enums::ModelTier;
use aidp_tickets::budget::RateSnapshot;
use aidp_tickets::config::{BudgetConfig, DispatcherConfig};
use aidp_tickets::model::Ticket;
use aidp_tickets::models::enums::BudgetRoute;
use aidp_tickets::night_mode::heavy_decision;
use aidp_tickets::ticket_route::route_of_ticket;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const TEMPLATES_DIR: &str = "templates";
const TEMPLATE_EXTENSION: &str = "yaml";
const PIXELLAB_HELD: &str = "pixellab";

/// Everything the gates read.
pub struct GateInputs<'a> {
    pub vault: &'a Path,
    pub skill_root: Option<&'a Path>,
    pub dispatcher: &'a DispatcherConfig,
    pub budget: &'a BudgetConfig,
    pub snapshot: Option<&'a RateSnapshot>,
    pub now_ms: i64,
    pub local_minutes: u32,
}

/// The reason the Pixellab reserve holds an `art` ticket, `None` when it may start.
pub fn pixellab_hold(min_reserve: f64, balance: Option<f64>) -> Option<String> {
    let balance = balance?;
    (min_reserve > 0.0 && balance < min_reserve)
        .then(|| format!("{PIXELLAB_HELD}: solde {balance} sous la réserve {min_reserve}"))
}

/// Whether `template` of the skill, run on `route` under the vault's model policy, has an opus stage.
fn template_is_heavy(inputs: &GateInputs<'_>, template: &str, route: Option<BudgetRoute>) -> bool {
    let Some(skill_root) = inputs.skill_root else {
        return false;
    };
    let path = skill_root
        .join(TEMPLATES_DIR)
        .join(format!("{template}.{TEMPLATE_EXTENSION}"));
    let Ok(pipeline) = aidp_core::pipeline::load(&path) else {
        return false;
    };
    let policy = aidp_core::model_policy::load_policy(inputs.vault);
    let overrides: BTreeMap<String, ModelTier> =
        aidp_core::model_policy::overrides_for(&policy, route.map(BudgetRoute::as_str));
    aidp_core::model_policy::is_heavy(&pipeline, &overrides)
}

/// The `(key, reason)` of the candidates the gates hold back this tick.
pub fn held_candidates(candidates: &[Ticket], inputs: &GateInputs<'_>) -> Vec<(String, String)> {
    let reserve = inputs.budget.pixellab.min_reserve;
    let night_on = inputs.dispatcher.night.enabled;
    if reserve <= 0.0 && !night_on {
        return Vec::new();
    }
    let balance = (reserve > 0.0)
        .then(|| latest_balance(inputs.vault).and_then(|snapshot| snapshot.amount))
        .flatten();
    let resets_at = inputs
        .snapshot
        .and_then(|snapshot| snapshot.five_hour)
        .map(|window| window.resets_at_s);
    let mut heavy_cache: HashMap<(String, Option<BudgetRoute>), bool> = HashMap::new();
    let mut held = Vec::new();
    for ticket in candidates
        .iter()
        .filter(|ticket| ticket.autopilot.manual_priority != Some(true))
    {
        let route = route_of_ticket(ticket);
        if route == Some(BudgetRoute::Art) {
            if let Some(reason) = pixellab_hold(reserve, balance) {
                held.push((ticket.key.clone(), reason));
                continue;
            }
        }
        let Some(template) = ticket.template.as_deref().filter(|_| night_on) else {
            continue;
        };
        let heavy = *heavy_cache
            .entry((template.to_string(), route))
            .or_insert_with(|| template_is_heavy(inputs, template, route));
        if !heavy {
            continue;
        }
        let decision = heavy_decision(
            &inputs.dispatcher.night,
            inputs.local_minutes,
            resets_at,
            inputs.now_ms,
        );
        if let Some(reason) = decision.reason() {
            held.push((ticket.key.clone(), reason.to_string()));
        }
    }
    held
}

/// Minutes since local midnight, for the night window.
pub fn local_minutes_now() -> u32 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    now.hour() * 60 + now.minute()
}

#[cfg(test)]
#[path = "../tests/unit/launch_gates_tests.rs"]
mod tests;
