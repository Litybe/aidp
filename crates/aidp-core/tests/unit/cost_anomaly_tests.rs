use super::*;

#[test]
fn median_handles_odd_even_and_empty() {
    assert_eq!(median_of(&mut [3.0, 1.0, 2.0]), 2.0);
    assert_eq!(median_of(&mut [1.0, 2.0, 3.0, 4.0]), 2.5);
    assert_eq!(median_of(&mut []), 0.0);
}

#[test]
fn median_and_mad_of_outlier_series() {
    let (median, mad) = median_and_mad(&[1.00, 1.10, 1.20, 1.30, 20.00]);
    assert!((median - 1.2).abs() < 1e-9);
    assert!((mad - 0.1).abs() < 1e-9);
}

#[test]
fn outlier_is_flagged() {
    let scores = modified_z_scores(&[1.00, 1.10, 1.20, 1.30, 20.00]);
    assert!(is_anomaly(scores[4]));
    assert!(!is_anomaly(scores[2]));
}

#[test]
fn anomaly_handles_zero_mad() {
    let values = [2.0, 2.0, 2.0, 2.0, 50.0];
    let (_, mad) = median_and_mad(&values);
    assert_eq!(mad, 0.0);
    let scores = modified_z_scores(&values);
    assert!(scores.iter().all(|z| *z == 0.0));
    assert!(scores.iter().all(|z| !is_anomaly(*z)));
}
