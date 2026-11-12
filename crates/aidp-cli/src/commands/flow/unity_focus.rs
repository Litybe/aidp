//! `flow unity-focus`: brings the Unity editor open on the workspace to the foreground and
//! answers its reload or save dialogs, so that a PlayMode run starts with the keyboard focus.
//! The win32 calls live in an embedded PowerShell script (`unsafe` is forbidden in the workspace).

use super::CommandOutput;
use crate::models::constants::flow_unity::*;
use crate::models::unity_focus_report::UnityFocusReport;
use std::path::{Path, PathBuf};

pub(super) fn unity_focus(workspace: &Path) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::text(serde_json::to_string(&focus_editor(
        workspace,
    ))?))
}

/// Runs the embedded script on the editor of `workspace` and reads its verdict.
pub(super) fn focus_editor(workspace: &Path) -> UnityFocusReport {
    match run_script(workspace) {
        Ok(stdout) => parse(&stdout),
        Err(error) => UnityFocusReport {
            message: format!("focus script failed: {error}"),
            ..UnityFocusReport::default()
        },
    }
}

fn run_script(workspace: &Path) -> std::io::Result<String> {
    let script = std::env::temp_dir().join(FOCUS_SCRIPT_FILE);
    std::fs::write(&script, FOCUS_SCRIPT)?;
    let log = FOCUS_LOG
        .iter()
        .fold(workspace.to_path_buf(), |path: PathBuf, part| {
            path.join(part)
        });
    let output = std::process::Command::new(POWERSHELL_PROGRAM)
        .args(POWERSHELL_ARGS)
        .arg(POWERSHELL_FILE_FLAG)
        .arg(&script)
        .arg(POWERSHELL_PROJECT_FLAG)
        .arg(workspace)
        .arg(POWERSHELL_LOG_FLAG)
        .arg(log)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads the script's lines: `FOCUS=True|False`, `ABSENT`, `NOWINDOW` and the dialog lines.
pub(super) fn parse(stdout: &str) -> UnityFocusReport {
    let mut report = UnityFocusReport {
        editor_found: true,
        ..UnityFocusReport::default()
    };
    for line in stdout.lines().map(str::trim) {
        if line == FOCUS_ABSENT {
            report.editor_found = false;
            report.message = "no Unity editor open on the workspace".to_string();
        } else if line == FOCUS_NO_WINDOW {
            report.editor_found = false;
            report.message = "the Unity editor has no main window".to_string();
        } else if let Some(result) = line.strip_prefix(FOCUS_RESULT_PREFIX) {
            report.focused = line == FOCUS_RESULT_OK;
            if report.message.is_empty() {
                report.message = format!("focus {}", result.to_lowercase());
            }
        } else if FOCUS_DIALOG_PREFIXES
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            report.dialogs.push(line.to_string());
        }
    }
    report
}

#[cfg(test)]
#[path = "../../../tests/unit/flow_unity_focus_tests.rs"]
mod tests;
