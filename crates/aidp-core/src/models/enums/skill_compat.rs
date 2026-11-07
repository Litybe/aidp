/// Whether the running `aidp` is recent enough for a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCompat {
    /// The running version meets the skill's `min_aidp_version`.
    Compatible { required: String },
    /// The running version is older than the skill's `min_aidp_version`.
    TooOld { required: String },
    /// The skill has no `SKILL.md` or declares no `min_aidp_version`.
    Undeclared,
    /// `SKILL.md` or its version could not be read.
    Unreadable { reason: String },
}
