use super::Price;
use crate::run::StageUsage;

/// Tokens covered by one published per-million tariff unit.
pub const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// A named all-one-model baseline for counterfactual repricing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterfactualBaseline {
    AllOpus,
    AllSonnet,
    AllHaiku,
}

impl CounterfactualBaseline {
    /// Model the baseline reprices every stage at.
    pub const fn model(self) -> &'static str {
        match self {
            Self::AllOpus => OPUS_MODEL,
            Self::AllSonnet => SONNET_MODEL,
            Self::AllHaiku => HAIKU_MODEL,
        }
    }

    /// Display label: the prefix followed by the model name.
    pub fn label(self) -> String {
        format!("{BASELINE_LABEL_PREFIX}{}", self.model())
    }
}

pub const BASELINE_LABEL_PREFIX: &str = "all-";
pub const OPUS_MODEL: &str = "opus";
pub const SONNET_MODEL: &str = "sonnet";
pub const HAIKU_MODEL: &str = "haiku";
pub const ALL_BASELINES_SELECTOR: &str = "all";

pub const COUNTERFACTUAL_BASELINES: &[CounterfactualBaseline] = &[
    CounterfactualBaseline::AllOpus,
    CounterfactualBaseline::AllSonnet,
    CounterfactualBaseline::AllHaiku,
];

/// Cost in USD of the four token counters at `price`, cache writes at the 5m rate.
pub fn reprice_counters(
    input: u64,
    output: u64,
    cache_creation: u64,
    cache_read: u64,
    price: &Price,
) -> f64 {
    (input as f64) * price.input / TOKENS_PER_MILLION
        + (output as f64) * price.output / TOKENS_PER_MILLION
        + (cache_creation as f64) * price.cache_write_5m / TOKENS_PER_MILLION
        + (cache_read as f64) * price.cache_read / TOKENS_PER_MILLION
}

/// Cost in USD of one model's usage at `price`.
pub fn reprice_usage(usage: &StageUsage, price: &Price) -> f64 {
    reprice_counters(
        usage.input,
        usage.output,
        usage.cache_creation,
        usage.cache_read,
        price,
    )
}

#[cfg(test)]
#[path = "../../tests/unit/cost_repricing_tests.rs"]
mod tests;
