use serde::{Deserialize, Serialize};

const ADOPTED: &str = "adopted";
const ADOPTED_MERGED: &str = "adopted-merged";
const ADOPT: &str = "adopt";

/// Why a ticket skipped its run: a delivered PR was found for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdoptionReason {
    Adopted,
    AdoptedMerged,
    Adopt,
}

impl AdoptionReason {
    /// Wire value written to the ledger event `reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            AdoptionReason::Adopted => ADOPTED,
            AdoptionReason::AdoptedMerged => ADOPTED_MERGED,
            AdoptionReason::Adopt => ADOPT,
        }
    }

    /// Parses a ledger event `reason`, `None` when it is not an adoption reason.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            ADOPTED => Some(AdoptionReason::Adopted),
            ADOPTED_MERGED => Some(AdoptionReason::AdoptedMerged),
            ADOPT => Some(AdoptionReason::Adopt),
            _ => None,
        }
    }
}
