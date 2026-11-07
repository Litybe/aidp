use serde::{Deserialize, Serialize};

/// The work route of a ticket, as the token budget and the model policy see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetRoute {
    Code,
    Art,
    Map,
    Doc,
}

impl BudgetRoute {
    /// Every route, in config order.
    pub const ALL: [BudgetRoute; 4] = [
        BudgetRoute::Code,
        BudgetRoute::Art,
        BudgetRoute::Map,
        BudgetRoute::Doc,
    ];

    /// The wire and config spelling of the route.
    pub const fn as_str(self) -> &'static str {
        match self {
            BudgetRoute::Code => "code",
            BudgetRoute::Art => "art",
            BudgetRoute::Map => "map",
            BudgetRoute::Doc => "doc",
        }
    }

    /// Parses a route name, ignoring case and surrounding blanks.
    pub fn parse(value: &str) -> Option<BudgetRoute> {
        let wanted = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|route| route.as_str() == wanted)
    }
}

impl std::fmt::Display for BudgetRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
