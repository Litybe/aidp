use serde::Serialize;

/// The tokens of one ticket in `GET /api/tickets/costs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketCostBriefTotals {
    pub total_tokens: u64,
}

/// One ticket's entry of `GET /api/tickets/costs`: the `costUsd` and `totals.totalTokens` of
/// `GET /api/tickets/:key/cost`, with `costUsd` `null` while no session has a price.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketCostBrief {
    pub cost_usd: Option<f64>,
    pub totals: TicketCostBriefTotals,
}
