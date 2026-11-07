//! Environment a gateway-launched `claude` child needs to resolve the
//! `aidp` binary without depending on the inherited PATH.

use crate::session_error::SessionError;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

pub const AIDP_BIN_ENV: &str = "AIDP_BIN";
pub const PATH_ENV: &str = "PATH";

/// Builds the `AIDP_BIN` and `PATH` pairs for a child session.
///
/// Pure: the caller supplies the user override, the running executable and
/// the inherited PATH. A non-empty `user_bin` always wins over `current_exe`.
pub fn resolve_child_env(
    user_bin: Option<OsString>,
    current_exe: io::Result<PathBuf>,
    inherited_path: Option<OsString>,
) -> Result<Vec<(String, String)>, SessionError> {
    let binary = match user_bin.filter(|value| !value.is_empty()) {
        Some(value) => validate_user_binary(PathBuf::from(value))?,
        None => validate_current_exe(current_exe)?,
    };
    let binary_dir = binary.parent().map(Path::to_path_buf).unwrap_or_default();
    let path_value = prefixed_path(&binary_dir, inherited_path)?;
    Ok(vec![
        (AIDP_BIN_ENV.to_string(), path_to_string(&binary)?),
        (PATH_ENV.to_string(), path_value),
    ])
}

/// Reads the process environment and executable, then delegates to
/// [`resolve_child_env`].
pub fn child_env_from_process() -> Result<Vec<(String, String)>, SessionError> {
    resolve_child_env(
        std::env::var_os(AIDP_BIN_ENV),
        std::env::current_exe(),
        std::env::var_os(PATH_ENV),
    )
}

/// A user-provided binary must be an absolute path to an existing file.
fn validate_user_binary(binary: PathBuf) -> Result<PathBuf, SessionError> {
    if binary.is_absolute() && binary.is_file() {
        return Ok(binary);
    }
    Err(SessionError::Config(format!(
        "{AIDP_BIN_ENV}={} is not an absolute path to an existing file",
        binary.display()
    )))
}

/// The running executable is the fallback and must still exist on disk.
fn validate_current_exe(current_exe: io::Result<PathBuf>) -> Result<PathBuf, SessionError> {
    match current_exe {
        Ok(binary) if binary.is_file() => Ok(binary),
        Ok(binary) => Err(SessionError::Config(format!(
            "aidp binary not found at {}; set {AIDP_BIN_ENV} to its absolute path",
            binary.display()
        ))),
        Err(error) => Err(SessionError::Config(format!(
            "could not locate the aidp binary ({error}); set {AIDP_BIN_ENV} to its absolute path"
        ))),
    }
}

/// Puts `binary_dir` first in the PATH, unless it already is the first entry.
fn prefixed_path(
    binary_dir: &Path,
    inherited_path: Option<OsString>,
) -> Result<String, SessionError> {
    let inherited_entries: Vec<PathBuf> = inherited_path
        .as_deref()
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();
    let mut entries: Vec<PathBuf> = Vec::with_capacity(inherited_entries.len() + 1);
    if inherited_entries.first().map(PathBuf::as_path) != Some(binary_dir) {
        entries.push(binary_dir.to_path_buf());
    }
    entries.extend(inherited_entries);
    let joined = std::env::join_paths(&entries).map_err(|error| {
        SessionError::Config(format!(
            "cannot build {PATH_ENV} with {}: {error}",
            binary_dir.display()
        ))
    })?;
    joined.into_string().map_err(|_| {
        SessionError::Config(format!(
            "{PATH_ENV} containing {} is not valid UTF-8",
            binary_dir.display()
        ))
    })
}

fn path_to_string(path: &Path) -> Result<String, SessionError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| SessionError::Config(format!("{} is not valid UTF-8", path.display())))
}

#[cfg(test)]
#[path = "../tests/unit/aidp_bin_tests.rs"]
mod aidp_bin_tests;
