//! The cost line closing a report posted on an issue (contract C7, e.g. `Coût : 1,2 M tokens
//! (in 0,3 M / out 0,1 M / cache 0,8 M), 4 % de la fenêtre 5 h`).
//!
//! Call point of `aidp ticket comment --cost-footer` (flow action `gh.issue_comment` with
//! `with: { cost_footer: true }`). The usage comes from the daemon's per-ticket cost report (P5,
//! key `GH-<n>`), the same source as `aidp cost ticket <key> --format footer`.

use aidp_gateway::ticket_cost::{ticket_footer, TicketCost};

/// The cost line of ticket `key`, or `None` when the daemon cannot answer (report posted as is).
pub fn cost_footer_for_ticket(key: &str) -> Option<String> {
    let value = aidp_gateway::client::ticket_cost(&None, key).ok()?;
    footer_of(value)
}

/// The C7 footer of a cost report as the daemon serves it.
pub(crate) fn footer_of(report: serde_json::Value) -> Option<String> {
    let cost: TicketCost = serde_json::from_value(report).ok()?;
    Some(ticket_footer(&cost))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cost_report_becomes_the_c7_footer() {
        let cost = aidp_gateway::ticket_cost::ticket_cost("GH-7", &[], &[], None, None);
        let report = serde_json::to_value(&cost).unwrap();
        let footer = footer_of(report).expect("footer");
        assert_eq!(footer, ticket_footer(&cost));
        assert!(footer.starts_with("Coût : "), "{footer}");
    }

    #[test]
    fn a_malformed_report_gives_no_footer() {
        assert_eq!(footer_of(serde_json::json!({ "nope": 1 })), None);
    }
}
