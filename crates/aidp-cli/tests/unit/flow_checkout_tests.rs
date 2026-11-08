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

/// `origin` (a dev checkout with the ticket branch pushed) and a separate clone of it where the
/// branch exists only as `origin/<branch>`.
fn clone_of(repo: &Repo) -> Repo {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("test-checkout");
    let origin = repo.dir.to_string_lossy().to_string();
    git(root.path(), &["clone", &origin, &dir.to_string_lossy()]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "t"]);
    let mut ctx = Ctx::for_test(dir.clone(), None);
    ctx.runner = Box::new(OsProcessRunner);
    Repo {
        _root: root,
        dir,
        ctx,
    }
}

#[test]
fn checkout_takes_a_branch_that_exists_only_on_origin() {
    let origin = repo(true);
    let clone = clone_of(&origin);
    git(&origin.dir, &["switch", "-c", "feat/GH-9"]);
    write(&origin.dir, "Assets/new.txt", "from the dev worktree");
    git(&origin.dir, &["add", "."]);
    git(&origin.dir, &["commit", "-m", "work"]);
    git(&origin.dir, &["switch", "main"]);
    assert!(git(&clone.dir, &["branch", "--list", "feat/GH-9"]).is_empty());

    let output = git_checkout(&clone.ctx, "GH-9").unwrap();
    assert!(
        matches!(output.payload, Payload::Text(ref text) if text.contains("\"blocked\":false"))
    );
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "feat/GH-9");
    assert!(clone.dir.join("Assets/new.txt").is_file());
}

#[test]
fn checkout_reuses_a_local_branch_and_fast_forwards_it_to_origin() {
    let origin = repo(true);
    git(&origin.dir, &["switch", "-c", "feat/GH-9"]);
    let clone = clone_of(&origin);
    git(&clone.dir, &["switch", "main"]);
    write(&origin.dir, "Assets/fix.txt", "pushed fix");
    git(&origin.dir, &["add", "."]);
    git(&origin.dir, &["commit", "-m", "fix"]);
    git(&origin.dir, &["switch", "main"]);

    git_checkout(&clone.ctx, "GH-9").unwrap();
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "feat/GH-9");
    assert!(clone.dir.join("Assets/fix.txt").is_file());
}

#[test]
fn checkout_creates_a_missing_branch_from_the_up_to_date_origin_base() {
    let origin = repo(true);
    let clone = clone_of(&origin);
    write(&origin.dir, "Assets/later.txt", "merged after the clone");
    git(&origin.dir, &["add", "."]);
    git(&origin.dir, &["commit", "-m", "later"]);

    let output = git_checkout(&clone.ctx, "GH-10").unwrap();
    assert!(matches!(output.payload, Payload::Text(ref text) if text.contains("\"created\":true")));
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "feat/GH-10");
    assert!(clone.dir.join("Assets/later.txt").is_file());
}

#[test]
fn restore_goes_back_to_base_and_pulls_it() {
    let origin = repo(true);
    let clone = clone_of(&origin);
    git(&clone.dir, &["switch", "-c", "feat/GH-9"]);
    write(&origin.dir, "Assets/merged.txt", "merged on main");
    git(&origin.dir, &["add", "."]);
    git(&origin.dir, &["commit", "-m", "merged"]);

    let output = git_restore(&clone.ctx, "GH-9").unwrap();
    assert!(matches!(output.payload, Payload::Text(ref text) if text.contains("\"pulled\":true")));
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "main");
    assert!(clone.dir.join("Assets/merged.txt").is_file());
}

#[test]
fn forced_checkout_discards_local_changes_but_keeps_the_editor_version() {
    let origin = repo(true);
    let clone = clone_of(&origin);
    write(&clone.dir, "Assets/a.txt", "rewritten by the editor");
    write(
        &clone.dir,
        "ProjectSettings/ProjectVersion.txt",
        "m_EditorVersion: 6000.6.8f1",
    );
    // Without force the checkout is blocked on the dirty tracked file.
    let blocked = checkout_in(&clone.ctx, "GH-12", false).unwrap();
    assert!(
        matches!(blocked.payload, Payload::Text(ref text) if text.contains("\"blocked\":true"))
    );

    let output =
        keeping_editor_version(&clone.ctx, true, || checkout_in(&clone.ctx, "GH-12", true))
            .unwrap();
    assert!(
        matches!(output.payload, Payload::Text(ref text) if text.contains("\"blocked\":false"))
    );
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "feat/GH-12");
    assert_eq!(
        std::fs::read_to_string(clone.dir.join("Assets/a.txt")).unwrap(),
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(clone.dir.join("ProjectSettings/ProjectVersion.txt")).unwrap(),
        "m_EditorVersion: 6000.6.8f1"
    );
    assert!(git(&clone.dir, &["ls-files", "-v"]).contains("S ProjectSettings/ProjectVersion.txt"));
    assert_eq!(git(&clone.dir, &["status", "--porcelain"]), "");

    let restored =
        keeping_editor_version(&clone.ctx, true, || restore_in(&clone.ctx, "GH-12", true)).unwrap();
    assert!(
        matches!(restored.payload, Payload::Text(ref text) if text.contains("\"restored\":true"))
    );
    assert_eq!(git(&clone.dir, &["branch", "--show-current"]), "main");
    assert_eq!(
        std::fs::read_to_string(clone.dir.join("ProjectSettings/ProjectVersion.txt")).unwrap(),
        "m_EditorVersion: 6000.6.8f1"
    );
}
