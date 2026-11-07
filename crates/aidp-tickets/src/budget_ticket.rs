//! The per-ticket token ceiling: `budget.per_ticket` caps what one ticket may burn, by route.
//! Past the ceiling the flow asks "continue?" instead of burning the window; approving raises
//! the ceiling of that ticket only. Pure decisions, plus the small file that remembers the
//! raised ceilings at `<vault>/TODO/.state/ticket-budget.json`.

use crate::config::PerTicketBudget;
use crate::models::enums::BudgetRoute;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const STATE_DIR: &str = ".state";
const STATE_FILE: &str = "ticket-budget.json";
const TICKETS_DIR: &str = "TODO";

/// Reason recorded on a step blocked by the ceiling, and on the ticket `blocked: budget`.
pub const BLOCKED_REASON: &str = "budget";

/// The ceilings an approved "continue?" raised, per ticket key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TicketCeilings {
    /// New token ceiling of a ticket.
    pub tokens: BTreeMap<String, u64>,
    /// New Pixellab generation ceiling of a ticket.
    pub generations: BTreeMap<String, u32>,
}

/// Where a ticket stands against its token ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum TicketBudgetCheck {
    /// No ceiling applies (`0`).
    Unlimited,
    Within {
        used: u64,
        limit: u64,
    },
    Exceeded {
        used: u64,
        limit: u64,
    },
}

impl TicketBudgetCheck {
    /// True once the ticket reached its ceiling.
    pub fn is_exceeded(&self) -> bool {
        matches!(self, TicketBudgetCheck::Exceeded { .. })
    }
}

impl TicketCeilings {
    /// Where the file lives.
    pub fn path(vault_root: &Path) -> PathBuf {
        vault_root
            .join(TICKETS_DIR)
            .join(STATE_DIR)
            .join(STATE_FILE)
    }

    /// Loads the file; a missing or invalid one means no raised ceiling.
    pub fn load(vault_root: &Path) -> TicketCeilings {
        std::fs::read_to_string(Self::path(vault_root))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the file atomically.
    pub fn save(&self, vault_root: &Path) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        crate::store::write_atomic(&Self::path(vault_root), &text)
    }

    /// The token ceiling of ticket `key` on `route`: the configured one, or the raised one if higher.
    /// `0` means unlimited.
    pub fn token_limit(&self, cfg: &PerTicketBudget, route: Option<BudgetRoute>, key: &str) -> u64 {
        let configured = cfg.limit_for(route);
        if configured == 0 {
            return 0;
        }
        configured.max(self.tokens.get(key).copied().unwrap_or(0))
    }

    /// The Pixellab generation ceiling of ticket `key`; `0` means unlimited.
    pub fn generation_limit(&self, configured: u32, key: &str) -> u32 {
        if configured == 0 {
            return 0;
        }
        configured.max(self.generations.get(key).copied().unwrap_or(0))
    }

    /// Raises the ceilings of `key` by one more allowance on top of what it already used.
    pub fn raise(
        &mut self,
        key: &str,
        used_tokens: u64,
        base_tokens: u64,
        used_generations: u32,
        base_generations: u32,
    ) {
        if base_tokens > 0 {
            self.tokens
                .insert(key.to_string(), used_tokens.saturating_add(base_tokens));
        }
        if base_generations > 0 {
            self.generations.insert(
                key.to_string(),
                used_generations.saturating_add(base_generations),
            );
        }
    }
}

/// Compares what a ticket burnt with its ceiling.
pub fn check(used: u64, limit: u64) -> TicketBudgetCheck {
    if limit == 0 {
        TicketBudgetCheck::Unlimited
    } else if used >= limit {
        TicketBudgetCheck::Exceeded { used, limit }
    } else {
        TicketBudgetCheck::Within { used, limit }
    }
}

/// The "continue?" question put to the human when a ticket hits a ceiling.
pub fn exceeded_prompt(key: &str, what: &str, used: u64, limit: u64) -> String {
    format!(
        "Plafond atteint pour {key} : {what} {used} / {limit}. Continuer ? \
         (approve : plafond relevé pour ce ticket, reject : bloqué, budget)"
    )
}

#[cfg(test)]
#[path = "../tests/unit/budget_ticket_tests.rs"]
mod tests;
