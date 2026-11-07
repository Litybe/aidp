//! Every `gh`/`git` spawn in `gh.rs` resolves its program through
//! `resolve_program`, proven with a `PATH` shim that records its own argv.

use aidp_testkit::{EnvGuard, TempWorkspace};
use std::io::Read;
use std::path::Path;

const PR_TITLE: &str = "t";
const PR_BODY: &str = "b";
const PR_NUMBER: &str = "1";

#[cfg(windows)]
const SHIM_FILE_NAME: &str = "gh.cmd";
#[cfg(not(windows))]
const SHIM_FILE_NAME: &str = "gh";

#[cfg(windows)]
fn shim_script(marker_path: &Path) -> String {
    format!(
        "@echo off\r\necho %* >>\"{}\"\r\nexit /b 0\r\n",
        marker_path.display()
    )
}

#[cfg(not(windows))]
fn shim_script(marker_path: &Path) -> String {
    format!(
        "#!/bin/sh\necho \"$*\" >> \"{}\"\nexit 0\n",
        marker_path.display()
    )
}

fn install_shim(bin_dir: &Path, marker_path: &Path) {
    let script_path = bin_dir.join(SHIM_FILE_NAME);
    std::fs::write(&script_path, shim_script(marker_path)).expect("write gh shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("shim metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("make shim executable");
    }
}

fn read_marker_lines(marker_path: &Path) -> Vec<String> {
    let mut contents = String::new();
    if let Ok(mut file) = std::fs::File::open(marker_path) {
        file.read_to_string(&mut contents)
            .expect("read marker file");
    }
    contents
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn gh_verbs_spawn_through_resolve_program() {
    let workspace = TempWorkspace::new("gh-resolve-program");
    let bin_dir = workspace.child("bin");
    std::fs::create_dir_all(&bin_dir).expect("create bin dir");
    let marker_path = workspace.child("marker.txt");
    install_shim(&bin_dir, &marker_path);

    let _path_guard = EnvGuard::set("PATH", &bin_dir.display().to_string());

    aidp_integrations::gh::pr_create(PR_TITLE, PR_BODY, None)
        .expect("pr_create through the shim must succeed");

    aidp_integrations::gh::pr_view(PR_NUMBER).expect("pr_view through the shim must succeed");

    aidp_integrations::gh::pr_diff(PR_NUMBER).expect("pr_diff through the shim must succeed");

    let lines = read_marker_lines(&marker_path);
    assert_eq!(
        lines.len(),
        3,
        "expected exactly 3 recorded invocations, got {lines:?}"
    );
    assert!(
        lines[0].starts_with("pr create"),
        "unexpected: {}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("pr view 1"),
        "unexpected: {}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("pr diff 1"),
        "unexpected: {}",
        lines[2]
    );
}
