//! A relocated binary with no `AIDP_SKILL_ROOT` must name the variable and must
//! not fall back to the templates of the tree it was built in.

use std::fs;

const BACKSLASH: char = '\\';

/// Copies the built binary into an isolated directory and resolves a pipeline with no skill root.
#[test]
fn moved_binary_without_skill_root_names_the_variable() {
    let sandbox = aidp_testkit::TempWorkspace::new("skill-root-moved");
    let bin_dir = sandbox.child("bin");
    let home_dir = sandbox.child("home");
    let cwd_dir = sandbox.child("cwd");
    for dir in [&bin_dir, &home_dir, &cwd_dir] {
        fs::create_dir_all(dir).expect("create dir");
    }
    let moved = bin_dir.join(
        std::path::Path::new(env!("CARGO_BIN_EXE_aidp"))
            .file_name()
            .unwrap(),
    );
    fs::copy(env!("CARGO_BIN_EXE_aidp"), &moved).expect("copy binary");

    let output = std::process::Command::new(&moved)
        .args(["pipeline", "show", "dev"])
        .current_dir(&cwd_dir)
        .env("HOME", &home_dir)
        .env("USERPROFILE", &home_dir)
        .env_remove("AIDP_SKILL_ROOT")
        .env("AIDP_VAULT", "off")
        .output()
        .expect("run moved aidp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("observed exit code: {:?}", output.status.code());
    assert!(
        output.status.code().is_some(),
        "process must not be killed by a signal"
    );
    assert!(stderr.contains("AIDP_SKILL_ROOT"), "stderr: {stderr}");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let templates_dir = std::path::Path::new(manifest_dir)
        .join("../../../templates")
        .to_string_lossy()
        .replace(BACKSLASH, "/");
    for stream in [
        stdout.replace(BACKSLASH, "/"),
        stderr.replace(BACKSLASH, "/"),
    ] {
        assert!(!stream.contains(&manifest_dir.replace(BACKSLASH, "/")));
        assert!(!stream.contains(&templates_dir));
    }
}
