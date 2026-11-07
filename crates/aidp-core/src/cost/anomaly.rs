/// Scales a MAD into a standard-deviation-comparable spread (Iglewicz and Hoaglin).
pub const IGLEWICZ_HOAGLIN_FACTOR: f64 = 0.6745;

/// Absolute modified z-score above which a value is flagged.
pub const ANOMALY_THRESHOLD: f64 = 3.5;

/// Median of `values`, sorting the slice in place. Empty input yields 0.0.
pub fn median_of(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Median and median absolute deviation of `values`.
pub fn median_and_mad(values: &[f64]) -> (f64, f64) {
    let median = median_of(&mut values.to_vec());
    let mut deviations: Vec<f64> = values.iter().map(|value| (value - median).abs()).collect();
    (median, median_of(&mut deviations))
}

/// Modified z-score per value. A zero MAD yields 0.0 for every value.
pub fn modified_z_scores(values: &[f64]) -> Vec<f64> {
    let (median, mad) = median_and_mad(values);
    values
        .iter()
        .map(|value| {
            if mad == 0.0 {
                0.0
            } else {
                IGLEWICZ_HOAGLIN_FACTOR * (value - median) / mad
            }
        })
        .collect()
}

/// True when the score exceeds the threshold in absolute value.
pub fn is_anomaly(z: f64) -> bool {
    z.abs() > ANOMALY_THRESHOLD
}

#[cfg(test)]
#[path = "../../tests/unit/cost_anomaly_tests.rs"]
mod tests;
