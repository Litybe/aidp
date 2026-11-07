use crate::models::enums::FieldKind;

/// One editable leaf of `TODO/config.yaml`: its key path and how it is validated.
#[derive(Debug, Clone, Copy)]
pub struct SettingField {
    pub path: &'static [&'static str],
    pub kind: FieldKind,
}

impl SettingField {
    /// The dotted path used in error reports, e.g. `budget.per_ticket.code`.
    pub fn dotted(&self) -> String {
        self.path.join(".")
    }
}
