use super::*;
use crate::commands::Payload;

fn point(run_id: &str, total_usd: f64) -> AnomalyPoint {
    AnomalyPoint {
        run_id: run_id.to_string(),
        total_usd,
    }
}

#[test]
fn an_outlier_run_exits_with_a_negative_verdict() {
    let points = vec![
        point("a", 1.00),
        point("b", 1.10),
        point("c", 1.20),
        point("d", 1.30),
        point("e", 20.00),
    ];
    let output = anomaly_report(None, &points);
    assert_eq!(output.exit, Some(ExitStatus::NegativeVerdict));
    let Payload::Json { value, .. } = output.payload else {
        panic!("anomaly report is JSON");
    };
    assert_eq!(value["any_flagged"], serde_json::json!(true));
    assert_eq!(value["points"][4]["flagged"], serde_json::json!(true));
    assert_eq!(value["points"][0]["flagged"], serde_json::json!(false));
}

#[test]
fn identical_costs_flag_nothing_because_the_mad_is_zero() {
    let points = vec![point("a", 2.0), point("b", 2.0), point("c", 2.0)];
    let output = anomaly_report(None, &points);
    assert_eq!(output.exit, None);
}

#[test]
fn cost_points_skip_runs_without_a_state_file() {
    let dir = std::env::temp_dir().join(format!("aidp-cost-points-{}", std::process::id()));
    let run_a = dir.join("feature").join("20260102-000000-aaaa");
    let run_b = dir.join("feature").join("20260101-000000-bbbb");
    std::fs::create_dir_all(&run_a).unwrap();
    std::fs::create_dir_all(&run_b).unwrap();
    assert!(cost_points(&dir, &None).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
