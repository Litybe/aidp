use crate::models::doctor_file_change::DoctorFileChange;
use serde::{Deserialize, Serialize};

/// What one bug of a parent changed on the parent's branch: computed from git, never by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorBugFix {
    /// Key of the bug as its branch or title names it (`GH-228`).
    pub bug: String,
    /// The commit of the parent's branch that brought the bug in (the merge, or the squashed fix).
    pub commit: String,
    pub subject: String,
    /// `<hash> <subject>` of the commits the bug brought, newest first.
    pub commits: Vec<String>,
    pub files: Vec<DoctorFileChange>,
    /// `git diff --stat` of the bug, cut to a sensible length.
    pub stat: String,
}
