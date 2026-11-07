use serde::Serialize;

/// Verdict of one gate (GitHub checks or Sonar) in the CI report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowGateState {
    Success,
    Failure,
    Timeout,
    Absent,
}
