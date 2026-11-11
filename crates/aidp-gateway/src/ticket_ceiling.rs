//! The per-ticket ceilings applied to the flows: `budget.per_ticket` (tokens, by route) and
//! `budget.pixellab.per_ticket` (generations). The flow executor asks `exceeded` before it starts a
//! pipeline step; an approved "continue?" calls `raise`, which lifts the ceilings of that ticket only.

use crate::cost_routes::route_of_key_in;
use crate::sessions::SessionRecord;
use crate::ticket_cost::{session_tokens, ticket_of};
use crate::vault_index::{self, VaultSnapshot};
use aidp_tickets::budget_ticket::{check, exceeded_prompt, TicketBudgetCheck, TicketCeilings};
use aidp_tickets::models::enums::BudgetRoute;
use serde_json::{json, Value};
use std::path::Path;

const WHAT_TOKENS: &str = "tokens";
const WHAT_GENERATIONS: &str = "générations Pixellab";

/// A ceiling a ticket reached, with the question to put to the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeilingHit {
    pub what: &'static str,
    pub used: u64,
    pub limit: u64,
    pub prompt: String,
}

struct TicketUse {
    route: Option<BudgetRoute>,
    tokens: u64,
    generations: u32,
}

fn usage_of(
    index: &VaultSnapshot,
    vault: &Path,
    key: &str,
    records: &[SessionRecord],
) -> TicketUse {
    let tokens = records
        .iter()
        .filter(|record| ticket_of(record) == Some(key))
        .map(|record| session_tokens(record).total())
        .sum();
    TicketUse {
        route: route_of_key_in(index, vault, key, records)
            .as_deref()
            .and_then(BudgetRoute::parse),
        tokens,
        generations: index.pixellab_of(key).generations,
    }
}

/// The ceiling `key` reached, `None` while it is within its tokens and its generations.
pub fn exceeded(vault: &Path, key: &str, records: &[SessionRecord]) -> Option<CeilingHit> {
    if vault.as_os_str().is_empty() {
        return None;
    }
    let config = aidp_tickets::config::load(vault).budget;
    let ceilings = TicketCeilings::load(vault);
    let used = usage_of(
        &vault_index::refresh_ticket(vault, key),
        vault,
        key,
        records,
    );
    let token_limit = ceilings.token_limit(&config.per_ticket, used.route, key);
    if check(used.tokens, token_limit).is_exceeded() {
        return Some(hit(key, WHAT_TOKENS, used.tokens, token_limit));
    }
    let generation_limit = u64::from(ceilings.generation_limit(config.pixellab.per_ticket, key));
    let generations = u64::from(used.generations);
    check(generations, generation_limit)
        .is_exceeded()
        .then(|| hit(key, WHAT_GENERATIONS, generations, generation_limit))
}

fn hit(key: &str, what: &'static str, used: u64, limit: u64) -> CeilingHit {
    CeilingHit {
        what,
        used,
        limit,
        prompt: exceeded_prompt(key, what, used, limit),
    }
}

/// Lifts the ceilings of `key` by one more allowance on top of what it used.
pub fn raise(vault: &Path, key: &str, records: &[SessionRecord]) {
    if vault.as_os_str().is_empty() {
        return;
    }
    let config = aidp_tickets::config::load(vault).budget;
    let used = usage_of(
        &vault_index::refresh_ticket(vault, key),
        vault,
        key,
        records,
    );
    let mut ceilings = TicketCeilings::load(vault);
    ceilings.raise(
        key,
        used.tokens,
        config.per_ticket.limit_for(used.route),
        used.generations,
        config.pixellab.per_ticket,
    );
    if let Err(error) = ceilings.save(vault) {
        tracing::warn!(key, %error, "ticket ceiling not saved");
    }
}

/// The `budget` and `pixellab` blocks of `GET /api/tickets/:key/cost`.
pub fn report(vault: &Path, key: &str, records: &[SessionRecord]) -> (Value, Value) {
    let config = aidp_tickets::config::load(vault).budget;
    let ceilings = TicketCeilings::load(vault);
    let index = vault_index::snapshot(vault);
    let used = usage_of(&index, vault, key, records);
    let configured = config.per_ticket.limit_for(used.route);
    let limit = ceilings.token_limit(&config.per_ticket, used.route, key);
    let raised_to = ceilings
        .tokens
        .get(key)
        .copied()
        .filter(|raised| *raised > configured);
    let exceeded = matches!(
        check(used.tokens, limit),
        TicketBudgetCheck::Exceeded { .. }
    );
    let usage = index.pixellab_of(key);
    let budget = json!({
        "limitTokens": limit,
        "usedTokens": used.tokens,
        "raisedTo": raised_to,
        "exceeded": exceeded,
    });
    let pixellab = json!({
        "generations": usage.generations,
        "limit": ceilings.generation_limit(config.pixellab.per_ticket, key),
        "spent": usage.spent,
        "runs": usage.runs,
    });
    (budget, pixellab)
}

#[cfg(test)]
#[path = "../tests/unit/ticket_ceiling_tests.rs"]
mod tests;
