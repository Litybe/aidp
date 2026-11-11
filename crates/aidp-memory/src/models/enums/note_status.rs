string_enum!(
    /// The lifecycle of a knowledge note.
    NoteStatus, "note status", {
        Active => "active",
        Proposed => "proposed",
        Superseded => "superseded",
    }
);

impl NoteStatus {
    /// Whether a note in this state is indexed and retrieved.
    pub fn is_retrievable(self) -> bool {
        self == NoteStatus::Active
    }
}
