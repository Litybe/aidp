//! Deterministic recognition of passing faults: the doctor repairs them without a model.
//! Pure text matching on the pause reason of a run and on the reply of its stage.

use crate::models::enums::DoctorCategory;

/// A passing fault the doctor resumes without investigating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transient {
    /// `CONNECT_TIMEOUT`, `MCP server … connection timed out`.
    McpConnect,
    /// The Unity compile gate timed out.
    UnityCompile,
    /// The agent session was lost.
    SessionLost,
    /// The provider is overloaded or the rate limit was hit.
    ApiOverload,
}

impl Transient {
    /// The category the incident is filed under.
    pub const fn category(self) -> DoctorCategory {
        match self {
            Self::McpConnect => DoctorCategory::McpConnectTimeout,
            Self::UnityCompile => DoctorCategory::UnityCompileTimeout,
            Self::SessionLost => DoctorCategory::SessionLost,
            Self::ApiOverload => DoctorCategory::ApiOverloaded,
        }
    }

    /// One line for the incident's note.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::McpConnect => "MCP server connection timed out",
            Self::UnityCompile => "Unity compile gate timed out",
            Self::SessionLost => "agent session lost",
            Self::ApiOverload => "API overloaded or rate limited",
        }
    }
}

const SESSION_LOST: [&str; 5] = [
    "session lost",
    "session was lost",
    "session expired",
    "session not found",
    "no conversation found",
];
const API_OVERLOAD: [&str; 6] = [
    "overloaded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "usage limit",
    "status 429",
];

/// The passing fault `text` describes, if it describes one.
pub fn classify(text: &str) -> Option<Transient> {
    let lower = text.to_lowercase();
    let timed_out = lower.contains("timed out") || lower.contains("timeout");
    if lower.contains("connect_timeout")
        || (lower.contains("mcp server") && lower.contains("connection") && timed_out)
    {
        return Some(Transient::McpConnect);
    }
    if lower.contains("compile") && timed_out && (lower.contains("gate") || lower.contains("unity"))
    {
        return Some(Transient::UnityCompile);
    }
    if SESSION_LOST.iter().any(|needle| lower.contains(needle)) {
        return Some(Transient::SessionLost);
    }
    if API_OVERLOAD.iter().any(|needle| lower.contains(needle))
        || lower.contains("http 429")
        || lower.contains("error 429")
        || lower.contains("status 529")
    {
        return Some(Transient::ApiOverload);
    }
    None
}

/// True when a run of template `template` is the art pipeline's.
pub fn is_art_template(template: &str) -> bool {
    crate::models::constants::doctor::ART_TEMPLATE_PREFIXES
        .iter()
        .any(|prefix| template.starts_with(prefix))
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_transient_tests.rs"]
mod tests;
