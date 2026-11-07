//! Drives the bootstrap install and PATH-migration libraries in a tempdir.
//!
//! The PowerShell library is exercised on Windows and the bash library on Unix. Every
//! path reaches the child interpreter through environment variables, so the real user
//! PATH, the rc files and `cargo build` are never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const SCRIPTS_DIR: &str = "scripts";
const ENV_LIB: &str = "AIDP_TEST_LIB";
const ENV_SOURCE: &str = "AIDP_SOURCE";
const ENV_INSTALL_DIR: &str = "AIDP_INSTALL_DIR";
const STAGING_SUFFIX: &str = ".new";
const ORIGINAL_BYTES: &[u8] = b"original binary bytes";
const UPDATED_BYTES: &[u8] = b"updated binary bytes";
const SKIP_NOTE: &str = "bootstrap e2e: interpreter unavailable, test skipped";

#[cfg(windows)]
const BINARY_NAME: &str = "aidp.exe";
#[cfg(unix)]
const BINARY_NAME: &str = "aidp";

#[cfg(windows)]
const INSTALL_LIB: &str = "aidp-install.ps1";
#[cfg(unix)]
const INSTALL_LIB: &str = "aidp-install.sh";

/// Root of the repository, where the install scripts live, from the crate manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Path of the install library for the current platform.
fn install_lib() -> PathBuf {
    repo_root().join(SCRIPTS_DIR).join(INSTALL_LIB)
}

/// Text written on stdout by a finished child process.
fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Text written on stderr by a finished child process.
fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Reports that the interpreter for this platform could not be started.
fn skip_note() {
    eprintln!("{SKIP_NOTE}");
}

/// Runs the platform install function on `source`, or `None` when the interpreter is missing.
fn install(source: &Path, install_dir: &Path) -> Option<Output> {
    #[cfg(windows)]
    {
        powershell::run(
            powershell::INSTALL_SCRIPT,
            &[
                (ENV_SOURCE, source.as_os_str()),
                (ENV_INSTALL_DIR, install_dir.as_os_str()),
            ],
        )
    }
    #[cfg(unix)]
    {
        bash::run(
            bash::INSTALL_SCRIPT,
            &[
                (ENV_SOURCE, source.as_os_str()),
                (ENV_INSTALL_DIR, install_dir.as_os_str()),
            ],
        )
    }
}

/// Creates a source binary and an install directory inside `temp`.
fn stage_source(temp: &TempDir, bytes: &[u8]) -> (PathBuf, PathBuf) {
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join(BINARY_NAME);
    fs::write(&source, bytes).unwrap();
    (source, temp.path().join("install"))
}

#[test]
fn install_copies_bytes_and_leaves_no_staging_file() {
    let temp = TempDir::new().unwrap();
    let (source, install_dir) = stage_source(&temp, ORIGINAL_BYTES);
    let Some(output) = install(&source, &install_dir) else {
        return skip_note();
    };
    assert!(output.status.success(), "{}", stderr_text(&output));
    let target = install_dir.join(BINARY_NAME);
    assert_eq!(fs::read(&target).unwrap(), ORIGINAL_BYTES);
    assert!(stdout_text(&output).trim().ends_with(BINARY_NAME));
    assert!(!install_dir
        .join(format!("{BINARY_NAME}{STAGING_SUFFIX}"))
        .exists());

    fs::write(&source, UPDATED_BYTES).unwrap();
    assert_eq!(fs::read(&target).unwrap(), ORIGINAL_BYTES);
}

#[test]
fn install_removes_stale_staging_file_before_copying() {
    let temp = TempDir::new().unwrap();
    let (source, install_dir) = stage_source(&temp, UPDATED_BYTES);
    fs::create_dir_all(&install_dir).unwrap();
    let staging = install_dir.join(format!("{BINARY_NAME}{STAGING_SUFFIX}"));
    fs::write(&staging, b"stale staging bytes").unwrap();
    let Some(output) = install(&source, &install_dir) else {
        return skip_note();
    };
    assert!(output.status.success(), "{}", stderr_text(&output));
    assert!(!staging.exists());
    assert_eq!(
        fs::read(install_dir.join(BINARY_NAME)).unwrap(),
        UPDATED_BYTES
    );
}

#[test]
fn install_over_existing_target_replaces_its_bytes() {
    let temp = TempDir::new().unwrap();
    let (source, install_dir) = stage_source(&temp, ORIGINAL_BYTES);
    let Some(first) = install(&source, &install_dir) else {
        return skip_note();
    };
    assert!(first.status.success(), "{}", stderr_text(&first));
    fs::write(&source, UPDATED_BYTES).unwrap();
    let second = install(&source, &install_dir).unwrap();
    assert!(second.status.success(), "{}", stderr_text(&second));
    assert_eq!(
        fs::read(install_dir.join(BINARY_NAME)).unwrap(),
        UPDATED_BYTES
    );
    assert!(!install_dir
        .join(format!("{BINARY_NAME}{STAGING_SUFFIX}"))
        .exists());
}

#[cfg(windows)]
mod powershell {
    use super::*;
    use std::ffi::OsStr;
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const ENV_USER_PATH: &str = "AIDP_USER_PATH";
    const ENV_LEGACY_DIR: &str = "AIDP_LEGACY_DIR";
    const SEPARATOR: &str = ";";
    const BOOTSTRAP_FILE: &str = "bootstrap.ps1";
    const RELEASE_SUFFIX: &[&str] = &["rust", "target", "release"];
    const NO_SHARING: u32 = 0;
    const IN_USE_MESSAGE: &str = "in use";

    pub(super) const INSTALL_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        . $env:AIDP_TEST_LIB; \
        try { \
            $target = Install-AidpBinary -Source $env:AIDP_SOURCE -InstallDir $env:AIDP_INSTALL_DIR; \
            [Console]::Out.Write($target); exit 0 \
        } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }";

    const MIGRATE_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        . $env:AIDP_TEST_LIB; \
        try { \
            $migrated = Get-AidpMigratedPath -UserPath $env:AIDP_USER_PATH \
                -LegacyDirs @($env:AIDP_LEGACY_DIR) -InstallDir $env:AIDP_INSTALL_DIR; \
            [Console]::Out.Write($migrated); exit 0 \
        } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }";

    /// Runs a script in Windows PowerShell with the install library dot-sourced.
    pub(super) fn run(script: &str, envs: &[(&str, &OsStr)]) -> Option<Output> {
        let mut command = Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .env(ENV_LIB, install_lib());
        for (name, value) in envs {
            command.env(name, value);
        }
        command.output().ok()
    }

    /// Calls `Get-AidpMigratedPath`, or `None` when PowerShell is missing.
    fn migrated_path(user_path: &str, legacy_dir: &Path, install_dir: &Path) -> Option<String> {
        let output = run(
            MIGRATE_SCRIPT,
            &[
                (ENV_USER_PATH, OsStr::new(user_path)),
                (ENV_LEGACY_DIR, legacy_dir.as_os_str()),
                (ENV_INSTALL_DIR, install_dir.as_os_str()),
            ],
        )?;
        assert!(output.status.success(), "{}", stderr_text(&output));
        Some(stdout_text(&output))
    }

    fn release_dir(root: &Path) -> PathBuf {
        RELEASE_SUFFIX
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn join_paths(entries: &[&str]) -> String {
        entries.join(SEPARATOR)
    }

    #[test]
    fn install_fails_with_in_use_message_and_keeps_locked_target() {
        let temp = TempDir::new().unwrap();
        let (source, install_dir) = stage_source(&temp, ORIGINAL_BYTES);
        let Some(first) = install(&source, &install_dir) else {
            return skip_note();
        };
        assert!(first.status.success(), "{}", stderr_text(&first));
        let target = install_dir.join(BINARY_NAME);
        fs::write(&source, UPDATED_BYTES).unwrap();

        let held = OpenOptions::new()
            .read(true)
            .share_mode(NO_SHARING)
            .open(&target)
            .unwrap();
        let output = install(&source, &install_dir).unwrap();
        drop(held);

        assert!(!output.status.success());
        assert!(
            stderr_text(&output).contains(IN_USE_MESSAGE),
            "{}",
            stderr_text(&output)
        );
        assert_eq!(fs::read(&target).unwrap(), ORIGINAL_BYTES);
        assert!(!install_dir
            .join(format!("{BINARY_NAME}{STAGING_SUFFIX}"))
            .exists());
    }

    #[test]
    fn path_migration_drops_legacy_and_empty_entries_and_prepends_install_dir() {
        let temp = TempDir::new().unwrap();
        let install_dir = temp.path().join("install");
        let release = release_dir(temp.path());
        let release_text = release.to_string_lossy().into_owned();
        let user_path = join_paths(&["C:\\a", &release_text, "C:\\b", ""]);
        let Some(migrated) = migrated_path(&user_path, &release, &install_dir) else {
            return skip_note();
        };
        let install_text = install_dir.to_string_lossy().into_owned();
        assert_eq!(migrated, join_paths(&[&install_text, "C:\\a", "C:\\b"]));
    }

    #[test]
    fn path_migration_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let install_dir = temp.path().join("install");
        let release = release_dir(temp.path());
        let user_path = join_paths(&["C:\\a", &release.to_string_lossy(), "C:\\b"]);
        let Some(once) = migrated_path(&user_path, &release, &install_dir) else {
            return skip_note();
        };
        let twice = migrated_path(&once, &release, &install_dir).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn path_migration_ignores_case_and_trailing_separator() {
        let temp = TempDir::new().unwrap();
        let install_dir = temp.path().join("install");
        let install_text = install_dir.to_string_lossy().into_owned();
        let shouting = format!("{}\\", install_text.to_uppercase());
        let user_path = join_paths(&["C:\\a", &shouting]);
        let Some(migrated) = migrated_path(&user_path, Path::new(""), &install_dir) else {
            return skip_note();
        };
        assert_eq!(migrated, join_paths(&[&install_text, "C:\\a"]));
    }

    #[test]
    fn path_migration_removes_other_checkout_release_dir_only_with_bootstrap_script() {
        let temp = TempDir::new().unwrap();
        let install_dir = temp.path().join("install");
        let install_text = install_dir.to_string_lossy().into_owned();
        let checkout = temp.path().join("other-checkout");
        let other_release = release_dir(&checkout);
        fs::create_dir_all(&other_release).unwrap();
        let other_text = other_release.to_string_lossy().into_owned();
        let user_path = join_paths(&["C:\\a", &other_text]);

        fs::write(checkout.join(BOOTSTRAP_FILE), b"").unwrap();
        let Some(removed) = migrated_path(&user_path, Path::new(""), &install_dir) else {
            return skip_note();
        };
        assert_eq!(removed, join_paths(&[&install_text, "C:\\a"]));

        fs::remove_file(checkout.join(BOOTSTRAP_FILE)).unwrap();
        let kept = migrated_path(&user_path, Path::new(""), &install_dir).unwrap();
        assert_eq!(kept, join_paths(&[&install_text, "C:\\a", &other_text]));
    }
}

#[cfg(unix)]
mod bash {
    use super::*;
    use std::ffi::OsStr;

    const ENV_RC: &str = "AIDP_RC";
    const ENV_SHELL: &str = "AIDP_SHELL";
    const MARKER: &str = "# aidp v7 PATH";
    const POSIX_SHELL: &str = "bash";
    const FISH_SHELL: &str = "fish";
    const OLD_LINE: &str = "export PATH=\"/old/rust/target/release:$PATH\"";
    const UNRELATED_HEAD: &str = "alias ll='ls -l'\n";
    const UNRELATED_TAIL: &str = "export EDITOR=vim\n";
    const ADDED: &str = "added";
    const MIGRATED: &str = "migrated";
    const UNCHANGED: &str = "unchanged";

    pub(super) const INSTALL_SCRIPT: &str =
        ". \"$AIDP_TEST_LIB\" && aidp_install_binary \"$AIDP_SOURCE\" \"$AIDP_INSTALL_DIR\"";
    const MIGRATE_SCRIPT: &str =
        ". \"$AIDP_TEST_LIB\" && aidp_migrate_rc \"$AIDP_RC\" \"$AIDP_INSTALL_DIR\" \"$AIDP_SHELL\"";
    const PATH_LINE_SCRIPT: &str =
        ". \"$AIDP_TEST_LIB\" && aidp_path_line \"$AIDP_INSTALL_DIR\" \"$AIDP_SHELL\"";

    /// Runs a script in bash with the install library path exported.
    pub(super) fn run(script: &str, envs: &[(&str, &OsStr)]) -> Option<Output> {
        let mut command = Command::new("bash");
        command.args(["-c", script]).env(ENV_LIB, install_lib());
        for (name, value) in envs {
            command.env(name, value);
        }
        command.output().ok()
    }

    /// Runs `aidp_migrate_rc` and returns its verdict, or `None` when bash is missing.
    fn migrate(rc: &Path, install_dir: &Path, shell: &str) -> Option<String> {
        let output = run(
            MIGRATE_SCRIPT,
            &[
                (ENV_RC, rc.as_os_str()),
                (ENV_INSTALL_DIR, install_dir.as_os_str()),
                (ENV_SHELL, OsStr::new(shell)),
            ],
        )?;
        assert!(output.status.success(), "{}", stderr_text(&output));
        Some(stdout_text(&output).trim().to_owned())
    }

    fn block(install_dir: &Path) -> String {
        format!(
            "{MARKER}\nexport PATH=\"{}:$PATH\"\n",
            install_dir.display()
        )
    }

    #[test]
    fn rc_migration_replaces_old_block_with_a_single_new_block() {
        let temp = TempDir::new().unwrap();
        let rc = temp.path().join("rc");
        let install_dir = temp.path().join("install");
        let old = format!("{UNRELATED_HEAD}\n{MARKER}\n{OLD_LINE}\n{UNRELATED_TAIL}");
        fs::write(&rc, old).unwrap();
        let Some(verdict) = migrate(&rc, &install_dir, POSIX_SHELL) else {
            return skip_note();
        };
        assert_eq!(verdict, MIGRATED);
        let expected = format!("{UNRELATED_HEAD}{UNRELATED_TAIL}\n{}", block(&install_dir));
        assert_eq!(fs::read_to_string(&rc).unwrap(), expected);
    }

    #[test]
    fn rc_migration_is_unchanged_and_byte_identical_on_second_run() {
        let temp = TempDir::new().unwrap();
        let rc = temp.path().join("rc");
        let install_dir = temp.path().join("install");
        fs::write(&rc, format!("{UNRELATED_HEAD}\n{MARKER}\n{OLD_LINE}\n")).unwrap();
        let Some(_) = migrate(&rc, &install_dir, POSIX_SHELL) else {
            return skip_note();
        };
        let before = fs::read(&rc).unwrap();
        let verdict = migrate(&rc, &install_dir, POSIX_SHELL).unwrap();
        assert_eq!(verdict, UNCHANGED);
        assert_eq!(fs::read(&rc).unwrap(), before);
        assert_eq!(
            String::from_utf8(before).unwrap().matches(MARKER).count(),
            1
        );
    }

    #[test]
    fn rc_migration_creates_missing_rc_file() {
        let temp = TempDir::new().unwrap();
        let rc = temp.path().join("nested").join("rc");
        let install_dir = temp.path().join("install");
        let Some(verdict) = migrate(&rc, &install_dir, POSIX_SHELL) else {
            return skip_note();
        };
        assert_eq!(verdict, ADDED);
        assert!(fs::read_to_string(&rc)
            .unwrap()
            .contains(&block(&install_dir)));
    }

    #[test]
    fn path_line_uses_set_gx_form_for_fish() {
        let temp = TempDir::new().unwrap();
        let install_dir = temp.path().join("install");
        let Some(output) = run(
            PATH_LINE_SCRIPT,
            &[
                (ENV_INSTALL_DIR, install_dir.as_os_str()),
                (ENV_SHELL, OsStr::new(FISH_SHELL)),
            ],
        ) else {
            return skip_note();
        };
        assert!(output.status.success(), "{}", stderr_text(&output));
        let expected = format!("set -gx PATH \"{}\" $PATH", install_dir.display());
        assert_eq!(stdout_text(&output).trim(), expected);
    }
}
