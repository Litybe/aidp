use aidp_flows::Issue;
use serde::Serialize;
use std::path::Path;

/// JSON body printed by `aidp flow validate` outside text format.
#[derive(Serialize)]
pub struct ValidationReportBody<'a> {
    pub ok: bool,
    pub id: &'a str,
    pub path: &'a Path,
    pub errors: &'a [Issue],
    pub warnings: &'a [Issue],
}
