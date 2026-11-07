//! Process shim: a fake executable that captures command-line arguments for testing.
//!
//! `ProcShim` creates a temporary executable (a copied `argv_recorder.exe` on Windows, a `.sh` on Unix)
//! that logs all arguments it receives to a file, then places its directory at the
//! front of `PATH` so the real `std::process::Command` finds it first.
//!
//! Example:
//! ```ignore
//! let ws = TempWorkspace::new("test");
//! let shim = ProcShim::new("gh", &ws);
//! std::process::Command::new("gh").args(["pr", "create"]).output().unwrap();
//! assert_eq!(shim.argv(), vec!["pr", "create"]);
//! // When shim is dropped, PATH is automatically restored.
//! ```

use crate::env_guard::EnvGuard;
use crate::temp_workspace::TempWorkspace;
use std::path::PathBuf;

/// Separates the arguments of one call on a line of the calls log.
pub const ARG_SEPARATOR: char = '\u{1f}';
/// Extension of the file holding the arguments of the last call.
pub const ARGV_EXTENSION: &str = "argv";
/// Extension of the file logging one line per call.
pub const CALLS_EXTENSION: &str = "calls";
/// Extension of the file holding the canned standard output.
pub const STDOUT_EXTENSION: &str = "stdout";

/// A fake executable that captures and records its command-line arguments.
pub struct ProcShim {
    /// Where the arguments of the last call are written.
    argv_file: PathBuf,
    /// Where one line per call is appended.
    calls_file: PathBuf,
    /// Where the canned standard output is read from.
    stdout_file: PathBuf,
    /// Holds the modified PATH; when dropped, restores it.
    _path_guard: EnvGuard,
}

impl ProcShim {
    /// Create a new process shim with the given name in the workspace.
    ///
    /// Writes a fake executable (`<name>.exe` on Windows, a shell script elsewhere) to the workspace,
    /// then prepends the workspace directory to `PATH` so `std::process::Command::new(name)`
    /// finds it first. Arguments passed to the shim are logged to a file inside the workspace.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains characters other than ASCII alphanumerics, hyphens, or underscores,
    /// or if the shim file cannot be written or (on Unix) permissions cannot be set.
    pub fn new(name: &str, ws: &TempWorkspace) -> Self {
        // Validate name to prevent injection into shim scripts.
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "ProcShim name must be alphanumeric/-/_ only, got {:?}",
            name
        );
        let ws_path = ws.path();
        let argv_file = ws_path.join(format!("{name}.{ARGV_EXTENSION}"));
        let calls_file = ws_path.join(format!("{name}.{CALLS_EXTENSION}"));
        let stdout_file = ws_path.join(format!("{name}.{STDOUT_EXTENSION}"));

        #[cfg(windows)]
        {
            // `Command::new(name)` never resolves a `.bat`/`.cmd` through PATH on
            // Windows, so the shim must be a real `.exe`: a copy of `argv_recorder`,
            // which writes its arguments to `<its stem>.argv`, i.e. `argv_file`.
            let exe_path = ws_path.join(format!("{}.exe", name));
            std::fs::copy(argv_recorder_exe(), &exe_path).expect("copy argv_recorder shim");
            debug_assert_eq!(exe_path.with_extension(ARGV_EXTENSION), argv_file);
        }

        #[cfg(not(windows))]
        {
            let sh_path = ws_path.join(name);
            let script = unix_script(&argv_file, &calls_file, &stdout_file);
            std::fs::write(&sh_path, script).expect("write shim .sh file");
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&sh_path)
                .expect("read shim metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&sh_path, perms).expect("set shim permissions");
        }

        // Prepend the workspace directory to PATH.
        let separator = if cfg!(windows) { ";" } else { ":" };
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let new_path = format!(
            "{}{}{}",
            ws_path.display(),
            separator,
            old_path.to_string_lossy()
        );
        let path_guard = EnvGuard::set("PATH", &new_path);

        Self {
            argv_file,
            calls_file,
            stdout_file,
            _path_guard: path_guard,
        }
    }

    /// Sets the standard output every later call of the shim prints.
    pub fn set_stdout(&self, text: &str) {
        std::fs::write(&self.stdout_file, text).expect("write shim canned stdout");
    }

    /// Returns the arguments of every call so far, in call order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(&self.calls_file)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                line.split(ARG_SEPARATOR)
                    .filter(|arg| !arg.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .collect()
    }

    /// Read and return the captured command-line arguments, one per line.
    pub fn argv(&self) -> Vec<String> {
        if !self.argv_file.exists() {
            return Vec::new();
        }
        std::fs::read_to_string(&self.argv_file)
            .unwrap_or_default()
            .lines()
            .map(|s| s.to_string())
            .collect()
    }
}

/// Shell script of the Unix shim: records the last call, logs the call, replays the canned stdout.
#[cfg(not(windows))]
fn unix_script(
    argv_file: &std::path::Path,
    calls_file: &std::path::Path,
    stdout_file: &std::path::Path,
) -> String {
    format!(
        "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"{argv}\"\nfor a in \"$@\"; do printf '%s\\037' \"$a\" >> \"{calls}\"; done\nprintf '\\n' >> \"{calls}\"\nif [ -f \"{out}\" ]; then cat \"{out}\"; fi\n",
        argv = argv_file.display(),
        calls = calls_file.display(),
        out = stdout_file.display(),
    )
}

/// Locate the `argv_recorder` binary built next to the running test executable.
///
/// A test binary lives in `target/<profile>/deps/`, the recorder in `target/<profile>/`,
/// so walk up from `current_exe()` until one ancestor holds it. `cargo nextest run -p <crate>`
/// does not build the bins of `aidp-testkit`, so the first lookup that misses builds them once.
#[cfg(windows)]
fn argv_recorder_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("resolve current test executable");
    let find = || {
        exe.ancestors()
            .skip(1)
            .take(4)
            .map(|dir| dir.join("argv_recorder.exe"))
            .find(|p| p.is_file())
    };
    find()
        .or_else(|| {
            build_testkit_bins();
            find()
        })
        .unwrap_or_else(|| {
            panic!(
                "ProcShim: argv_recorder.exe not found above {}; build it with `cargo build -p aidp-testkit --bins`",
                exe.display()
            )
        })
}

/// Builds the bins of this crate once per test process; a failure surfaces as the missing recorder.
#[cfg(windows)]
fn build_testkit_bins() {
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let _ = std::process::Command::new(cargo)
            .args(["build", "-p", "aidp-testkit", "--bins"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_gh_shim_captures_argv() {
        let ws = TempWorkspace::new("proc_shim_test");
        let shim = ProcShim::new("gh", &ws);

        // Run the shim via Command (it will be found on PATH).
        let _output = std::process::Command::new("gh")
            .args(["pr", "create", "--body-file", "x"])
            .output()
            .expect("run gh shim");

        let argv = shim.argv();
        assert!(argv.contains(&"pr".to_string()), "argv should contain 'pr'");
        assert!(
            argv.contains(&"create".to_string()),
            "argv should contain 'create'"
        );
        assert!(
            argv.contains(&"--body-file".to_string()),
            "argv should contain '--body-file'"
        );
        assert_eq!(argv, vec!["pr", "create", "--body-file", "x"]);

        // Verify that PATH is restored after shim is dropped.
        drop(shim);
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let path_str = current_path.to_string_lossy();
        let ws_path = ws.path().display().to_string();
        assert!(
            !path_str.starts_with(&ws_path),
            "PATH should not start with workspace directory after shim drop"
        );
    }

    #[test]
    fn it_shim_keeps_argument_with_spaces_whole() {
        let ws = TempWorkspace::new("proc_shim_spaces");
        let shim = ProcShim::new("fakecli", &ws);
        std::process::Command::new("fakecli")
            .args(["--title", "a title with spaces"])
            .output()
            .expect("run fakecli shim");
        assert_eq!(shim.argv(), vec!["--title", "a title with spaces"]);
    }

    #[test]
    fn it_proc_shim_under_a_held_env_guard_does_not_deadlock() {
        const HELD_KEY: &str = "AIDP_TESTKIT_SHIM_HELD";
        let ws = TempWorkspace::new("proc_shim_held_guard");
        let _held = EnvGuard::set(HELD_KEY, "held");
        let shim = ProcShim::new("gh", &ws);
        assert!(shim.argv().is_empty());
        assert_eq!(std::env::var(HELD_KEY).unwrap(), "held");
    }
}
