//! A controllable fake `claude` executable for daemon-level tests.
//!
//! `FakeClaude::install` hard-links the `fake_claude` binary into a workspace under a
//! chosen stem and writes its sidecar config, so each test sets its own behaviour without
//! touching process-wide environment.

use crate::temp_workspace::TempWorkspace;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

/// Extension of the sidecar config file sitting next to the fake executable.
pub const CONFIG_EXTENSION: &str = "conf";
/// Separator between a key and its value on one config line.
pub const KEY_VALUE_SEPARATOR: char = '=';
/// Config key: file the fake appends its own PID to, one line per launch.
pub const KEY_PID_FILE: &str = "pid_file";
/// Config key: number of filler bytes written to stdout.
pub const KEY_STDOUT_BYTES: &str = "stdout_bytes";
/// Config key: when `true`, block until one line arrives on stdin.
pub const KEY_READ_STDIN_LINE: &str = "read_stdin_line";
/// Config key: seconds to sleep before exiting.
pub const KEY_SLEEP_SECS: &str = "sleep_secs";
/// Config key: process exit code.
pub const KEY_EXIT_CODE: &str = "exit_code";
/// Maximum length of one stdout filler line, newline included.
pub const MAX_FILLER_LINE_BYTES: usize = 1024;

/// Name of the built binary this module copies.
const FAKE_CLAUDE_BIN: &str = "fake_claude";
/// Extension of the file the fake appends its PID to.
const PID_EXTENSION: &str = "pids";
/// Number of ancestors of the test executable searched for the binary.
const BIN_SEARCH_DEPTH: usize = 4;
/// Program listing Windows processes.
#[cfg(windows)]
const TASKLIST_PROGRAM: &str = "tasklist";
/// Program probing a Unix process with signal 0.
#[cfg(not(windows))]
const KILL_PROGRAM: &str = "kill";
/// Signal argument that only checks the process exists.
#[cfg(not(windows))]
const KILL_PROBE_SIGNAL: &str = "-0";
/// Spawn attempts made while Linux reports the installed binary as busy.
const EXEC_BUSY_ATTEMPTS: u32 = 5;
/// Base pause between two busy spawn attempts, multiplied by the attempt number.
const EXEC_BUSY_BACKOFF: Duration = Duration::from_millis(20);
/// Permission bits of the installed copy on Unix.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// Behaviour of one installed fake `claude`.
#[derive(Debug, Clone, Default)]
pub struct FakeClaudeConfig {
    /// Filler bytes written to stdout, as non-JSON lines of at most 1 KiB.
    pub stdout_bytes: usize,
    /// Block until one line arrives on stdin.
    pub read_stdin_line: bool,
    /// Seconds to sleep before exiting.
    pub sleep_secs: u64,
    /// Process exit code.
    pub exit_code: i32,
}

/// Handle on a fake `claude` installed in a workspace.
pub struct FakeClaude {
    exe_path: PathBuf,
    dir: PathBuf,
    pid_file: PathBuf,
}

impl FakeClaude {
    /// Link the fake binary to `<ws>/<stem>(.exe)` and write its sidecar config.
    ///
    /// # Panics
    ///
    /// Panics when the binary was not built, or when a file cannot be written.
    pub fn install(ws: &TempWorkspace, stem: &str, config: &FakeClaudeConfig) -> FakeClaude {
        let dir = ws.path().to_path_buf();
        let exe_path = dir.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
        install_binary(&fake_claude_exe(), &exe_path);
        let pid_file = dir.join(format!("{stem}.{PID_EXTENSION}"));
        std::fs::write(
            exe_path.with_extension(CONFIG_EXTENSION),
            render_config(config, &pid_file),
        )
        .expect("write fake_claude config");
        FakeClaude {
            exe_path,
            dir,
            pid_file,
        }
    }

    /// Path of the installed executable, suitable for `AIDP_CLAUDE_BIN`.
    pub fn exe_path(&self) -> &Path {
        &self.exe_path
    }

    /// Directory holding the executable, suitable for prepending to `PATH`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every PID recorded by a launch of this install so far, in launch order.
    pub fn pids(&self) -> Vec<u32> {
        std::fs::read_to_string(&self.pid_file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }
}

/// Spawn `command`, retrying while Linux reports the freshly installed binary as busy.
///
/// A copy of the binary can still be open for writing in a child forked by a concurrent
/// test, which makes `exec` fail with `ETXTBSY` until that child execs in turn.
pub fn spawn_retrying_busy(command: &mut Command) -> io::Result<Child> {
    let mut attempt = 1;
    loop {
        match command.spawn() {
            Err(err)
                if err.kind() == ErrorKind::ExecutableFileBusy && attempt < EXEC_BUSY_ATTEMPTS =>
            {
                std::thread::sleep(EXEC_BUSY_BACKOFF * attempt);
                attempt += 1;
            }
            result => return result,
        }
    }
}

/// Hard-link `source` to `exe_path`, copying it only when a link is impossible.
///
/// A link opens no file for writing, so no child forked concurrently can hold the
/// installed binary open and make its `exec` fail with `ETXTBSY`.
fn install_binary(source: &Path, exe_path: &Path) {
    if std::fs::hard_link(source, exe_path).is_ok() {
        return;
    }
    std::fs::copy(source, exe_path).expect("copy fake_claude binary");
    make_executable(exe_path);
}

/// Render the `key=value` config lines the fake binary reads.
fn render_config(config: &FakeClaudeConfig, pid_file: &Path) -> String {
    let entries = [
        (KEY_PID_FILE, pid_file.display().to_string()),
        (KEY_STDOUT_BYTES, config.stdout_bytes.to_string()),
        (KEY_READ_STDIN_LINE, config.read_stdin_line.to_string()),
        (KEY_SLEEP_SECS, config.sleep_secs.to_string()),
        (KEY_EXIT_CODE, config.exit_code.to_string()),
    ];
    entries
        .iter()
        .map(|(key, value)| format!("{key}{KEY_VALUE_SEPARATOR}{value}\n"))
        .collect()
}

/// Give the installed copy execute permission.
#[cfg(unix)]
fn make_executable(exe_path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(exe_path, std::fs::Permissions::from_mode(EXECUTABLE_MODE))
        .expect("set fake_claude permissions");
}

/// Windows needs no permission change for an `.exe`.
#[cfg(not(unix))]
fn make_executable(_exe_path: &Path) {}

/// Locate the `fake_claude` binary built next to the running test executable.
///
/// A test binary lives in `target/<profile>/deps/`, the fake in `target/<profile>/`,
/// so walk up from `current_exe()` until one ancestor holds it.
fn fake_claude_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("resolve current test executable");
    let file_name = format!("{FAKE_CLAUDE_BIN}{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .take(BIN_SEARCH_DEPTH)
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!(
                "FakeClaude: {file_name} not found above {}; build it with `cargo build -p aidp-testkit --bins`",
                exe.display()
            )
        })
}

/// Whether a process with this PID is currently running.
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    let pid_text = pid.to_string();
    let filter = format!("PID eq {pid_text}");
    let Ok(output) = std::process::Command::new(TASKLIST_PROGRAM)
        .args(["/FI", filter.as_str(), "/NH"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(pid_text.as_str()))
}

/// Whether a process with this PID is currently running.
#[cfg(not(windows))]
pub fn pid_alive(pid: u32) -> bool {
    std::process::Command::new(KILL_PROGRAM)
        .args([KILL_PROBE_SIGNAL, pid.to_string().as_str()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../tests/unit/fake_claude_tests.rs"]
mod fake_claude_tests;
