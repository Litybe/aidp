use super::*;
use crate::commands::Payload;
use aidp_core::process_runner::OsProcessRunner;
use std::path::PathBuf;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, relative: &str, text: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Repo {
    _root: tempfile::TempDir,
    dir: PathBuf,
    ctx: Ctx,
}

/// A repository on `main`, a Unity project when `unity` is set.
fn repo(unity: bool) -> Repo {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("game");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "--initial-branch=main"]);
    write(&dir, "Assets/a.txt", "a");
    if unity {
        write(
            &dir,
            "ProjectSettings/ProjectVersion.txt",
            "m_EditorVersion: 6000",
        );
    }
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "init"]);
    let mut ctx = Ctx::for_test(dir.clone(), None);
    ctx.runner = Box::new(OsProcessRunner);
    Repo {
        _root: root,
        dir,
        ctx,
    }
}

#[test]
fn untracked_meta_of_the_editor_is_not_dirty_in_a_unity_project() {
    let repo = repo(true);
    write(&repo.dir, "Assets/Sprites/logo.png.meta", "guid");
    write(&repo.dir, "Assets/Sprites/logo.aseprite.meta", "guid");
    assert_eq!(
        dirty_files(&repo.ctx, &repo.dir).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn untracked_meta_is_dirty_outside_a_unity_project() {
    let repo = repo(false);
    write(&repo.dir, "Assets/logo.png.meta", "guid");
    let files = dirty_files(&repo.ctx, &repo.dir).unwrap();
    assert_eq!(files, ["?? Assets/logo.png.meta"]);
}

#[test]
fn other_untracked_and_modified_tracked_files_stay_dirty() {
    let repo = repo(true);
    write(&repo.dir, "Assets/logo.png", "png");
    write(&repo.dir, "Assets/a.txt", "changed");
    write(&repo.dir, "Assets/logo.png.meta", "guid");
    let files = dirty_files(&repo.ctx, &repo.dir).unwrap();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.contains(&"?? Assets/logo.png".to_string()));
    assert!(files.iter().any(|file| file.ends_with("Assets/a.txt")));
}

#[test]
fn a_modified_tracked_meta_is_dirty() {
    let repo = repo(true);
    write(&repo.dir, "Assets/b.png.meta", "one");
    git(&repo.dir, &["add", "."]);
    git(&repo.dir, &["commit", "-m", "meta"]);
    write(&repo.dir, "Assets/b.png.meta", "two");
    let files = dirty_files(&repo.ctx, &repo.dir).unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
}

#[test]
fn editor_owned_files_are_not_dirty_in_a_unity_project() {
    let repo = repo(true);
    write(
        &repo.dir,
        "ProjectSettings/ProjectVersion.txt",
        "m_EditorVersion: 6001",
    );
    write(&repo.dir, "Packages/packages-lock.json", "{}");
    assert_eq!(
        dirty_files(&repo.ctx, &repo.dir).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn restore_goes_back_to_base_despite_untracked_meta() {
    let repo = repo(true);
    git(&repo.dir, &["switch", "-c", "feat/GH-252"]);
    write(&repo.dir, "Assets/logo.png.meta", "guid");
    let output = git_restore(&repo.ctx, "GH-252").unwrap();
    assert!(
        matches!(output.payload, Payload::Text(ref text) if text.contains("\"restored\":true"))
    );
    assert_eq!(git(&repo.dir, &["branch", "--show-current"]), "main");
    assert!(repo.dir.join("Assets/logo.png.meta").is_file());
}

#[test]
fn switch_removes_an_untracked_meta_that_the_target_branch_tracks() {
    let repo = repo(true);
    git(&repo.dir, &["switch", "-c", "feat/GH-1"]);
    write(&repo.dir, "Assets/logo.png.meta", "from the branch");
    git(&repo.dir, &["add", "."]);
    git(&repo.dir, &["commit", "-m", "meta"]);
    git(&repo.dir, &["switch", "main"]);
    write(&repo.dir, "Assets/logo.png.meta", "from the editor");
    switch(&repo.ctx, &repo.dir, &[GIT_SWITCH, "feat/GH-1"]).unwrap();
    assert_eq!(git(&repo.dir, &["branch", "--show-current"]), "feat/GH-1");
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("Assets/logo.png.meta")).unwrap(),
        "from the branch"
    );
}

#[test]
fn switch_keeps_the_failure_outside_a_unity_project() {
    let repo = repo(false);
    git(&repo.dir, &["switch", "-c", "feat/GH-1"]);
    write(&repo.dir, "Assets/logo.png.meta", "from the branch");
    git(&repo.dir, &["add", "."]);
    git(&repo.dir, &["commit", "-m", "meta"]);
    git(&repo.dir, &["switch", "main"]);
    write(&repo.dir, "Assets/logo.png.meta", "mine");
    assert!(switch(&repo.ctx, &repo.dir, &[GIT_SWITCH, "feat/GH-1"]).is_err());
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("Assets/logo.png.meta")).unwrap(),
        "mine"
    );
}

#[test]
fn status_parser_drops_the_origin_of_a_rename() {
    let parsed = parse_status("R  new.txt\0old.txt\0?? x.meta\0 M y.cs\0");
    assert_eq!(
        parsed,
        [
            ("R ".to_string(), "new.txt".to_string()),
            ("??".to_string(), "x.meta".to_string()),
            (" M".to_string(), "y.cs".to_string()),
        ]
    );
}

#[test]
fn overwritten_metas_are_read_from_git_stderr() {
    let stderr = "error: The following untracked working tree files would be overwritten by checkout:\n\tAssets/a.png.meta\n\tAssets/b.cs\nPlease move or remove them before you switch branches.\nAborting\n";
    assert_eq!(overwritten_metas(stderr), ["Assets/a.png.meta"]);
}
