//! Tests ensuring the CLI test harness is hermetic and leaks no machine state.
//!
//! These tests verify:
//! - No writes escape the fake home or vault directories
//! - Home directory resolution prefers USERPROFILE over HOME
//! - Poisoned parent environment variables have no effect on child processes

use crate::common;
use std::collections::BTreeMap;
use std::path::Path;

/// Helper to recursively walk a directory and collect all file paths relative to root.
fn walk_dir(root: &Path) -> std::io::Result<Vec<String>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    fn visit_dir(dir: &Path, base: &Path, files: &mut Vec<String>) -> std::io::Result<()> {
        if dir.is_dir() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit_dir(&path, base, files)?;
                } else {
                    if let Ok(rel) = path.strip_prefix(base) {
                        files.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
        Ok(())
    }
    visit_dir(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

/// Test that running all 23 commands of the net (root + subcommands) writes nothing
/// into the fake home or vault directories.
///
/// Rationale: `aidp vault init` writes `~/.aidp/config.json` unless `--no-config`,
/// and `aidp setup agents` writes into `~/.claude/agents/` unless `--dest` — this
/// test is the tripwire that the net never invokes them for real.
#[test]
fn no_write_into_fake_home() {
    let ws = common::workspace();

    // Run root --help
    let mut cmd = common::aidp(&ws);
    cmd.arg("--help");
    let assert = cmd.assert().success();
    let _output = assert.get_output();

    // Run each subcommand with --help
    for subcommand in common::SUBCOMMANDS {
        let mut cmd = common::aidp(&ws);
        cmd.arg(subcommand).arg("--help");
        let assert = cmd.assert().success();
        let _output = assert.get_output();
    }

    // Assert no writes to fake home/.aidp/config.json
    let config_file = ws.child("home").join(".aidp").join("config.json");
    assert!(
        !config_file.exists(),
        "config file should not exist: {}",
        config_file.display()
    );

    // Assert no writes to fake home/.claude/agents/
    let agents_dir = ws.child("home").join(".claude").join("agents");
    if agents_dir.exists() {
        let entries = std::fs::read_dir(&agents_dir)
            .and_then(|mut entries| entries.next().transpose())
            .unwrap_or(None);
        assert!(
            entries.is_none(),
            ".claude/agents directory should be empty or not exist"
        );
    }

    // Assert vault remains empty (no new files written)
    let vault_final = walk_dir(&ws.child("vault")).unwrap_or_default();
    assert!(
        vault_final.is_empty(),
        "vault should remain empty after running all commands, but found: {:?}",
        vault_final
    );
}

/// Test that home_dir() prefers USERPROFILE over HOME.
///
/// This is the ONLY test in the whole crate allowed to mutate the parent process
/// environment. The USERPROFILE-first precedence in vault.rs (lines 585-589) must
/// remain stable so that aidp on Windows always uses %USERPROFILE%.
#[test]
fn home_dir_prefers_userprofile_over_home() {
    let ws = common::workspace();
    let home_path = ws.child("home").to_string_lossy().to_string();
    let profile_path = ws.child("profile").to_string_lossy().to_string();

    // Take the guard, setting BOTH HOME and USERPROFILE to distinct paths.
    // The guard serializes this test against home_dir_prefers_userprofile_over_home
    // to prevent interference. Keep the scope as short as possible.
    let _guard = aidp_testkit::EnvGuard::with(&[
        ("HOME", Some(&home_path)),
        ("USERPROFILE", Some(&profile_path)),
    ]);

    // Call aidp_core::vault::home_dir() and verify it returns the USERPROFILE path.
    let resolved = aidp_core::vault::home_dir().expect("home_dir should resolve");
    let expected = std::path::PathBuf::from(&profile_path);
    assert_eq!(
        resolved,
        expected,
        "home_dir() should prefer USERPROFILE over HOME; got {}, expected {}",
        resolved.display(),
        expected.display()
    );
}

/// Test that poisoning the parent process environment has no effect on child commands.
///
/// The child process (aidp binary) inherits certain environment variables from the
/// parent but should ignore poisoned values because common::aidp() explicitly sets
/// them on the Command. This test verifies that output and exit codes remain identical
/// whether the parent process has poisoned values or not.
#[test]
fn poisoned_parent_env_changes_nothing() {
    let ws = common::workspace();

    // Collect output from all 23 commands with a clean parent environment
    let clean_outputs = {
        let mut outputs = BTreeMap::new();

        // Root --help
        let mut cmd = common::aidp(&ws);
        cmd.arg("--help");
        let output = cmd.output().expect("root --help should run");
        assert!(
            output.status.success(),
            "root --help should exit 0 with clean env"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        outputs.insert("root".to_string(), common::normalize_output(&stdout, &ws));

        // Each subcommand --help
        for subcommand in common::SUBCOMMANDS {
            let mut cmd = common::aidp(&ws);
            cmd.arg(subcommand).arg("--help");
            let output = cmd
                .output()
                .unwrap_or_else(|e| panic!("{} --help should run: {}", subcommand, e));
            assert!(
                output.status.success(),
                "{} --help should exit 0 with clean env",
                subcommand
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            outputs.insert(
                subcommand.to_string(),
                common::normalize_output(&stdout, &ws),
            );
        }

        outputs
    };

    // Poison the parent environment with all 11 variables
    let poison_home = ws.child("poison-home").to_string_lossy().to_string();
    let poison_profile = ws.child("poison-profile").to_string_lossy().to_string();
    let poison_vault = ws.child("poison-vault").to_string_lossy().to_string();
    let poison_nowhere = ws.child("nowhere").to_string_lossy().to_string();
    let poison_prices = "C:/nowhere.json".to_string();

    let _guard = aidp_testkit::EnvGuard::with(&[
        ("HOME", Some(&poison_home)),
        ("USERPROFILE", Some(&poison_profile)),
        ("AIDP_VAULT", Some(&poison_vault)),
        ("AIDP_SKILL_ROOT", Some(&poison_nowhere)),
        ("AIDP_CLAUDE_PROJECTS_DIR", Some(&poison_nowhere)),
        ("AIDP_MODEL_PRICES", Some(&poison_prices)),
        ("AIDP_RUN", Some("poison")),
        ("AIDP_STAGE", Some("poison")),
        ("AIDP_AUTO_APPROVE_GATES", Some("1")),
        ("AIDP_GATEWAY_URL", Some("http://127.0.0.1:1")),
        ("AIDP_HOOK_TOKEN", Some("poison")),
    ]);

    // Collect output from all 23 commands with poisoned parent environment
    let poisoned_outputs = {
        let mut outputs = BTreeMap::new();

        // Root --help
        let mut cmd = common::aidp(&ws);
        cmd.arg("--help");
        let output = cmd.output().expect("root --help should run");
        assert!(
            output.status.success(),
            "root --help should exit 0 with poisoned env"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        outputs.insert("root".to_string(), common::normalize_output(&stdout, &ws));

        // Each subcommand --help
        for subcommand in common::SUBCOMMANDS {
            let mut cmd = common::aidp(&ws);
            cmd.arg(subcommand).arg("--help");
            let output = cmd
                .output()
                .unwrap_or_else(|e| panic!("{} --help should run: {}", subcommand, e));
            assert!(
                output.status.success(),
                "{} --help should exit 0 with poisoned env",
                subcommand
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            outputs.insert(
                subcommand.to_string(),
                common::normalize_output(&stdout, &ws),
            );
        }

        outputs
    };

    // Verify outputs are byte-identical
    assert_eq!(
        clean_outputs, poisoned_outputs,
        "Help output should be identical with poisoned and clean parent environment"
    );
}

/// `aidp doctor` resolves `aidp` through PATH x PATHEXT, so an `aidp.cmd`-only
/// install is found and its canonical path appears in the `aidp_on_path` detail.
#[cfg(windows)]
#[test]
fn doctor_resolves_cmd_only_aidp_on_path() {
    let ws = common::workspace();
    let bin_dir = ws.child("cmd-only-bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let shim = bin_dir.join("aidp.cmd");
    std::fs::write(&shim, b"@echo off\r\n").unwrap();
    let expected = std::fs::canonicalize(&shim).unwrap();

    let mut cmd = common::aidp(&ws);
    cmd.env("PATH", bin_dir.as_os_str());
    cmd.env("PATHEXT", ".EXE;.CMD");
    cmd.args(["doctor", "--format", "json-compact"]);
    let output = cmd.output().expect("doctor should run");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor stdout should be JSON");
    let detail = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "aidp_on_path")
        .and_then(|check| check["detail"].as_str())
        .expect("aidp_on_path check should be present")
        .to_string();
    assert!(
        detail.contains(&expected.display().to_string()),
        "detail {detail:?} should name {}",
        expected.display()
    );
}
