/// Validation outcome printed by `aidp flow validate` in text format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    Ok,
    Invalid,
}

impl ValidationStatus {
    pub fn of(valid: bool) -> Self {
        if valid {
            ValidationStatus::Ok
        } else {
            ValidationStatus::Invalid
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ValidationStatus::Ok => "ok",
            ValidationStatus::Invalid => "invalid",
        }
    }
}
