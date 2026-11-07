use serde::Serialize;
use serde_json::Value;

/// The `current_exe()` resolution failed before a doctor probe could even
/// be spawned.
pub const CURRENT_EXE_UNAVAILABLE: &str = "current_exe unavailable";

/// The `aidp doctor --format json-compact` child process could not be
/// spawned.
pub const DOCTOR_SPAWN_FAILED: &str = "failed to spawn doctor";

/// The doctor probe's stdout was not parseable JSON.
pub const DOCTOR_OUTPUT_UNPARSEABLE: &str = "unparseable doctor output";

/// The doctor probe itself panicked while running on its own thread.
pub const DOCTOR_PROBE_PANICKED: &str = "doctor probe panicked";

/// Envelope reported by `doctor_json`: the `aidp doctor` verdict, reduced to
/// what `GET /api/health` needs. `checks` is the total count of checks the
/// probe ran; `failing` names the ones that came back not-ok. `error` is
/// `null` in the serialized JSON when the probe succeeded.
#[derive(Serialize)]
pub struct DoctorSummary {
    pub ok: bool,
    pub checks: usize,
    pub failing: Vec<String>,
    pub error: Option<String>,
}

impl DoctorSummary {
    /// A summary reported when the doctor probe could not produce one at
    /// all: zero checks, nothing failing, `error` naming why.
    pub fn unavailable(error: &str) -> Self {
        DoctorSummary {
            ok: false,
            checks: 0,
            failing: Vec::new(),
            error: Some(error.to_string()),
        }
    }

    /// Serialize to the same `Value` shape `doctor_json` has always
    /// returned, so callers reading it as a `serde_json::Value` see no
    /// difference.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
