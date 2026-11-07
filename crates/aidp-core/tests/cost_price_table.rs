//! `PriceTable` is an owned value with no shared state, so two grids can
//! coexist in the same process and each test observes only the grid it
//! built — order and thread count never matter.

use aidp_core::cost::PriceTable;
use aidp_core::tokens::TokenUsage;

const PRICING_VERSION_A: &str = "grid-a";
const PRICING_VERSION_B: &str = "grid-b";

const GRID_A_JSON: &str = r#"{
  "pricing_version": "grid-a",
  "tiers": {
    "sonnet": { "input": 3.0, "output": 15.0, "cache_write_5m": 3.75, "cache_write_1h": 6.0, "cache_read": 0.3 },
    "unknown": { "input": 0.0, "output": 0.0, "cache_write_5m": 0.0, "cache_write_1h": 0.0, "cache_read": 0.0 }
  }
}"#;

const GRID_B_JSON: &str = r#"{
  "pricing_version": "grid-b",
  "tiers": {
    "sonnet": { "input": 0.0, "output": 0.0, "cache_write_5m": 0.0, "cache_write_1h": 0.0, "cache_read": 0.0 },
    "unknown": { "input": 0.0, "output": 0.0, "cache_write_5m": 0.0, "cache_write_1h": 0.0, "cache_read": 0.0 }
  }
}"#;

fn sample_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 1_000,
        output_tokens: 1_000,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        total_tokens: 2_000,
        billable_total: 2_000,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
    }
}

#[test]
fn grid_a_reports_its_own_nonzero_version_and_amount() {
    let table = PriceTable::from_json(GRID_A_JSON);

    assert_eq!(table.pricing_version(), PRICING_VERSION_A);
    let cost = table.cost_usd("sonnet", &sample_usage(), None);
    assert!(cost > 0.0, "grid-a's sonnet row is priced above zero");
}

#[test]
fn grid_b_reports_its_own_zero_version_and_amount() {
    let table = PriceTable::from_json(GRID_B_JSON);

    assert_eq!(table.pricing_version(), PRICING_VERSION_B);
    let cost = table.cost_usd("sonnet", &sample_usage(), None);
    assert_eq!(cost, 0.0, "grid-b's sonnet row is priced at zero");
}
