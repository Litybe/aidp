use super::constants::aidp::{
    AIDP_PROGRAM, FORMAT_FLAG, FORMAT_JSON_COMPACT, FROM_RESULT_FLAG, RECORD_RESUME_SUBCOMMAND,
    RUN_FLAG, RUN_SUBCOMMAND,
};
use std::fmt;
use std::path::Path;

/// An `aidp` command line, composed from named tokens and rendered for display.
pub struct AidpCommand {
    args: Vec<String>,
}

impl AidpCommand {
    /// `aidp <args...> --format json-compact`, the shape of a flow action.
    pub fn json_action<'a>(args: impl IntoIterator<Item = &'a str>) -> Self {
        let args = args
            .into_iter()
            .chain([FORMAT_FLAG, FORMAT_JSON_COMPACT])
            .map(String::from)
            .collect();
        Self { args }
    }

    /// The arguments after the program name.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// `aidp run record-resume --run <run_id> --from-result <path>`.
    pub fn record_resume_from_result(run_id: &str, result_path: &Path) -> Self {
        let args = [
            RUN_SUBCOMMAND,
            RECORD_RESUME_SUBCOMMAND,
            RUN_FLAG,
            run_id,
            FROM_RESULT_FLAG,
        ]
        .into_iter()
        .map(String::from)
        .chain(std::iter::once(result_path.display().to_string()))
        .collect();
        Self { args }
    }
}

impl fmt::Display for AidpCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{AIDP_PROGRAM}")?;
        self.args
            .iter()
            .try_for_each(|arg| write!(formatter, " {arg}"))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/aidp_command_tests.rs"]
mod aidp_command_tests;
