use super::*;
use aidp_core::process_runner::SpawnedChild;
use std::process::Output;
use std::sync::Mutex;

const TASK: &str = "T1";
const HEAD_SHA: &str = "aaa111";
const BASE_SHA: &str = "bbb222";
const SRC_FILE: &str = "src/a.cs";
const PATCH_TEXT: &str = "diff --git a/src/a.cs b/src/a.cs\n";
const DIRTY_DIFF: &str = "diff --git a/x b/x\n";

/// Records every command and answers from a script keyed on the joined arguments.
#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<(String, Option<PathBuf>)>>,
    replies: Mutex<Vec<(String, i32, String, String)>>,
}

impl RecordingRunner {
    fn reply(self, key: &str, code: i32, stdout: &str, stderr: &str) -> Self {
        self.replies.lock().unwrap().push((
            key.to_string(),
            code,
            stdout.to_string(),
            stderr.to_string(),
        ));
        self
    }

    fn commands(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(line, _)| line.clone())
            .collect()
    }

    fn position(&self, needle: &str) -> Option<usize> {
        self.commands()
            .iter()
            .position(|line| line.contains(needle))
    }

    fn count(&self, needle: &str) -> usize {
        self.commands()
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        let line = spec.args.join(" ");
        self.calls
            .lock()
            .unwrap()
            .push((line.clone(), spec.cwd.clone()));
        let replies = self.replies.lock().unwrap();
        let (code, stdout, stderr) = replies
            .iter()
            .find(|(key, ..)| line.contains(key.as_str()))
            .map(|(_, code, stdout, stderr)| (*code, stdout.clone(), stderr.clone()))
            .unwrap_or((0, String::new(), String::new()));
        Ok(Output {
            status: exit_status(code),
            stdout: stdout.into_bytes(),
            stderr: stderr.into_bytes(),
        })
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::other("not scripted"))
    }
}

#[cfg(unix)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code as u32)
}

fn tree(root: &Path) -> TaskWorktree {
    TaskWorktree {
        path: root.join(TASK),
        base: BASE_SHA.to_string(),
    }
}

#[test]
fn isolation_is_chosen_only_for_parallel_plans() {
    assert!(!should_isolate(true, 3));
    assert!(!should_isolate(false, 1));
    assert!(should_isolate(false, 2));
    assert!(should_isolate(false, 3));
}

#[test]
fn prepare_creates_a_detached_worktree_under_the_run_dir() {
    let dir = tempfile::tempdir().unwrap();
    let root = isolation_root(dir.path());
    let runner = RecordingRunner::default().reply("rev-parse", 0, &format!("{HEAD_SHA}\n"), "");
    let prepared = prepare(dir.path(), &root, TASK, None, &runner).unwrap();
    let commands = runner.commands();
    assert!(commands[0].contains("--show-toplevel"));
    assert!(commands[1].starts_with("worktree remove --force"));
    assert_eq!(commands[2], "worktree prune");
    assert!(commands[3].starts_with("worktree add --detach"));
    assert!(commands[3].ends_with("HEAD"));
    assert_eq!(prepared.path, root.join(TASK));
    assert!(!commands[3].contains("-b"));
}

#[test]
fn prepare_skips_the_base_commit_for_a_clean_ticket_tree() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default().reply("rev-parse", 0, &format!("{HEAD_SHA}\n"), "");
    let prepared = prepare(dir.path(), &isolation_root(dir.path()), TASK, None, &runner).unwrap();
    assert_eq!(runner.count("commit"), 0);
    assert_eq!(runner.count("apply"), 0);
    assert_eq!(prepared.base, HEAD_SHA);
}

#[test]
fn prepare_carries_the_dirty_state_into_a_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("new.txt"), "new").unwrap();
    let root = isolation_root(dir.path());
    let runner = RecordingRunner::default()
        .reply("diff HEAD", 0, DIRTY_DIFF, "")
        .reply("ls-files", 0, "new.txt\0", "")
        .reply("rev-parse", 0, &format!("{BASE_SHA}\n"), "");
    // The isolated tree is a fake, so its folder is created by hand.
    std::fs::create_dir_all(root.join(TASK)).unwrap();
    let prepared = prepare(dir.path(), &root, TASK, None, &runner).unwrap();
    assert!(runner.position(" apply ").unwrap() < runner.position("commit").unwrap());
    assert_eq!(runner.count("commit --no-verify --allow-empty"), 1);
    assert_eq!(prepared.base, BASE_SHA);
    assert!(root.join(TASK).join("new.txt").exists());
}

#[test]
fn prepare_does_not_copy_the_exempt_run_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = isolation_root(dir.path());
    let runner = RecordingRunner::default()
        .reply("ls-files", 0, ".aidp/runs/state.json\0", "")
        .reply("rev-parse", 0, &format!("{BASE_SHA}\n"), "");
    prepare(dir.path(), &root, TASK, Some(".aidp"), &runner).unwrap();
    assert_eq!(runner.count("commit"), 0);
}

#[test]
fn prepare_runs_every_git_command_in_the_ticket_worktree_or_iso() {
    let dir = tempfile::tempdir().unwrap();
    let root = isolation_root(dir.path());
    let runner = RecordingRunner::default()
        .reply("diff HEAD", 0, DIRTY_DIFF, "")
        .reply("rev-parse", 0, &format!("{BASE_SHA}\n"), "");
    std::fs::create_dir_all(root.join(TASK)).unwrap();
    prepare(dir.path(), &root, TASK, None, &runner).unwrap();
    let iso = root.join(TASK);
    for (line, cwd) in runner.calls.lock().unwrap().iter() {
        let cwd = cwd.as_ref().expect("every command has a cwd");
        assert!(
            cwd == dir.path() || *cwd == iso,
            "{line} ran in {}",
            cwd.display()
        );
    }
}

#[test]
fn prepare_failure_returns_the_error_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default().reply("worktree add", 128, "", "fatal: nope");
    let error = prepare(dir.path(), &isolation_root(dir.path()), TASK, None, &runner)
        .unwrap_err()
        .to_string();
    assert!(error.contains("fatal: nope"));
    assert_eq!(runner.count("worktree remove"), 2);
    assert_eq!(runner.count("worktree prune"), 2);
}

#[test]
fn apply_back_with_an_empty_patch_does_not_call_apply() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default();
    let verdict = apply_back(dir.path(), &tree(dir.path()), &[], &runner).unwrap();
    assert_eq!(verdict, ApplyVerdict::Applied);
    assert_eq!(runner.count("apply"), 0);
    assert!(runner.count("diff --cached --binary --full-index bbb222") == 1);
}

#[test]
fn apply_back_checks_before_applying() {
    let dir = tempfile::tempdir().unwrap();
    let iso = tree(dir.path());
    let runner = RecordingRunner::default().reply("diff --cached", 0, PATCH_TEXT, "");
    let verdict = apply_back(dir.path(), &iso, &[SRC_FILE.to_string()], &runner).unwrap();
    assert_eq!(verdict, ApplyVerdict::Applied);
    let check = runner.position("apply --check").unwrap();
    let apply = runner
        .commands()
        .iter()
        .position(|line| line.contains(" apply ") && !line.contains("--check"))
        .unwrap();
    assert!(check < apply);
    assert_eq!(
        std::fs::read_to_string(patch_file(&iso.path)).unwrap(),
        PATCH_TEXT
    );
}

#[test]
fn apply_back_conflict_returns_the_paths_git_names() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default()
        .reply("diff --cached", 0, PATCH_TEXT, "")
        .reply(
            "apply --check",
            1,
            "",
            "error: patch failed: src/a.cs:12\nerror: src/a.cs: patch does not apply\n",
        );
    let verdict = apply_back(dir.path(), &tree(dir.path()), &[], &runner).unwrap();
    assert_eq!(verdict, ApplyVerdict::Conflict(vec![SRC_FILE.to_string()]));
    assert_eq!(runner.count("apply"), 1);
}

#[test]
fn apply_back_conflict_without_paths_falls_back_to_written() {
    let dir = tempfile::tempdir().unwrap();
    let written = vec!["src/b.cs".to_string()];
    let runner = RecordingRunner::default()
        .reply("diff --cached", 0, PATCH_TEXT, "")
        .reply("apply --check", 1, "", "something unexpected");
    let verdict = apply_back(dir.path(), &tree(dir.path()), &written, &runner).unwrap();
    assert_eq!(verdict, ApplyVerdict::Conflict(written));
}

#[test]
fn conflict_paths_reads_the_three_git_messages() {
    let stderr = "error: patch failed: a.cs:3\nerror: b.cs: already exists in working directory\nerror: c.cs: does not exist in index\nwarning: ignored\n";
    assert_eq!(conflict_paths(stderr), vec!["a.cs", "b.cs", "c.cs"]);
}

#[test]
fn release_removes_the_tree_and_the_patch_file() {
    let dir = tempfile::tempdir().unwrap();
    let iso = tree(dir.path());
    std::fs::create_dir_all(&iso.path).unwrap();
    std::fs::write(patch_file(&iso.path), "p").unwrap();
    let runner = RecordingRunner::default().reply("worktree remove", 1, "", "locked");
    release(dir.path(), &iso.path, &runner);
    assert_eq!(runner.count("worktree remove --force"), 1);
    assert_eq!(runner.count("worktree prune"), 1);
    assert!(!patch_file(&iso.path).exists());
    assert!(!iso.path.exists());
}

#[test]
fn sweep_removes_every_leftover_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = isolation_root(dir.path());
    std::fs::create_dir_all(root.join("T1")).unwrap();
    std::fs::create_dir_all(root.join("T2")).unwrap();
    let runner = RecordingRunner::default();
    sweep(dir.path(), &root, &runner);
    assert_eq!(runner.count("worktree remove"), 2);
    assert_eq!(runner.count("worktree prune"), 1);
}

#[test]
fn sweep_without_an_iso_folder_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default();
    sweep(dir.path(), &isolation_root(dir.path()), &runner);
    assert!(runner.commands().is_empty());
}

#[test]
fn isolation_guard_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default();
    let lock = Mutex::new(());
    {
        let _guard = IsolatedTask::new(dir.path(), tree(dir.path()), &runner, &lock);
    }
    assert_eq!(runner.count("worktree remove"), 1);
    assert_eq!(runner.count("worktree prune"), 1);
}

#[test]
fn isolation_guard_releases_under_the_ticket_lock() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Mutex::new(());
    let runner = LockProbe {
        lock: &lock,
        held: Mutex::new(Vec::new()),
    };
    drop(IsolatedTask::new(
        dir.path(),
        tree(dir.path()),
        &runner,
        &lock,
    ));
    let held = runner.held.lock().unwrap();
    assert!(!held.is_empty());
    assert!(held.iter().all(|locked| *locked));
    assert!(lock.try_lock().is_ok());
}

/// Records whether the ticket lock was held at each command.
struct LockProbe<'a> {
    lock: &'a Mutex<()>,
    held: Mutex<Vec<bool>>,
}

impl ProcessRunner for LockProbe<'_> {
    fn output(&self, _spec: &ProcessSpec) -> std::io::Result<Output> {
        self.held
            .lock()
            .unwrap()
            .push(self.lock.try_lock().is_err());
        Ok(Output {
            status: exit_status(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::other("not scripted"))
    }
}

#[test]
fn apply_runs_without_autocrlf_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RecordingRunner::default().reply("diff --cached", 0, PATCH_TEXT, "");
    apply_back(dir.path(), &tree(dir.path()), &[], &runner).unwrap();
    assert_eq!(runner.count("-c core.autocrlf=false apply --check"), 1);
    assert_eq!(runner.count("-c core.autocrlf=false apply "), 2);
}

#[test]
fn apply_runs_from_the_checkout_root_of_a_subfolder_ticket() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let runner = RecordingRunner::default()
        .reply(
            "--show-toplevel",
            0,
            &format!(
                "{}
",
                dir.path().display()
            ),
            "",
        )
        .reply("diff --cached", 0, PATCH_TEXT, "");
    apply_back(&sub, &tree(dir.path()), &[], &runner).unwrap();
    for (line, cwd) in runner.calls.lock().unwrap().iter() {
        if line.contains(" apply ") {
            assert_eq!(cwd.as_deref(), Some(dir.path()), "{line}");
        }
    }
}
