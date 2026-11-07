use crate::models::enums::CaseKind;
use std::path::PathBuf;

/// One golden-corpus compilation: a template, the case name and its kind.
pub(crate) struct CaseTask {
    pub(crate) template: PathBuf,
    pub(crate) name: String,
    pub(crate) kind: CaseKind,
}
