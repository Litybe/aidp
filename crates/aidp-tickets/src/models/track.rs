use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

const QUICK: &str = "quick";
const STANDARD: &str = "standard";
const DEEP: &str = "deep";
const BUG: &str = "bug";
const EXPECTED: &str = "quick/standard/deep/bug";

/// Run depth a ticket is dispatched with; `bug` writes the failing repro test first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Quick,
    Standard,
    Deep,
    Bug,
}

impl Track {
    /// The wire form, e.g. `standard`.
    pub fn as_str(self) -> &'static str {
        match self {
            Track::Quick => QUICK,
            Track::Standard => STANDARD,
            Track::Deep => DEEP,
            Track::Bug => BUG,
        }
    }
}

impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Track {
    type Err = anyhow::Error;

    /// Case-insensitive, surrounding whitespace ignored.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            QUICK => Ok(Track::Quick),
            STANDARD => Ok(Track::Standard),
            DEEP => Ok(Track::Deep),
            BUG => Ok(Track::Bug),
            _ => anyhow::bail!("unknown track `{s}`: expected {EXPECTED}"),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/track_tests.rs"]
mod tests;
