use super::*;

const SAMPLE_PRICE: Price = Price {
    input: 15.0,
    output: 75.0,
    cache_write_5m: 18.75,
    cache_write_1h: 30.0,
    cache_read: 1.5,
};

#[test]
fn counters_match_the_inline_formula() {
    let (input, output, creation, read) = (1_000_000u64, 200_000u64, 500_000u64, 4_000_000u64);
    let expected = (input as f64) * 15.0 / 1_000_000.0
        + (output as f64) * 75.0 / 1_000_000.0
        + (creation as f64) * 18.75 / 1_000_000.0
        + (read as f64) * 1.5 / 1_000_000.0;
    let actual = reprice_counters(input, output, creation, read, &SAMPLE_PRICE);
    assert_eq!(actual.to_bits(), expected.to_bits());
}

#[test]
fn zero_usage_costs_nothing() {
    assert_eq!(reprice_counters(0, 0, 0, 0, &SAMPLE_PRICE), 0.0);
}

#[test]
fn baselines_cover_the_three_families() {
    let labels: Vec<String> = COUNTERFACTUAL_BASELINES.iter().map(|b| b.label()).collect();
    assert_eq!(labels, ["all-opus", "all-sonnet", "all-haiku"]);
    for baseline in COUNTERFACTUAL_BASELINES {
        assert_eq!(
            baseline.label().trim_start_matches(BASELINE_LABEL_PREFIX),
            baseline.model()
        );
    }
}
