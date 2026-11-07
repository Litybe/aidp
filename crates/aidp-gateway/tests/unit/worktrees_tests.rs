use super::*;
use std::io;
use std::process::{ExitStatus, Output};
use std::sync::Mutex;

use aidp_core::process_runner::SpawnedChild;

struct Rule {
    needle: &'static str,
    code: i32,
    stdout: String,
}

#[derive(Default)]
struct FakeRunner {
    rules: Vec<Rule>,
    calls: Mutex<Vec<String>>,
}

impl FakeRunner {
    fn rule(mut self, needle: &'static str, code: i32, stdout: &str) -> Self {
        self.rules.push(Rule {
            needle,
            code,
            stdout: stdout.to_string(),
        });
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, needle: &str) -> usize {
        self.calls()
            .iter()
            .filter(|call| call.contains(needle))
            .count()
    }
}

fn status(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }
}

impl ProcessRunner for FakeRunner {
    fn output(&self, spec: &ProcessSpec) -> io::Result<Output> {
        let line = spec.args.join(" ");
        self.calls.lock().unwrap().push(line.clone());
        let rule = self.rules.iter().find(|rule| line.contains(rule.needle));
        let (code, stdout) =
            rule.map_or((0, String::new()), |rule| (rule.code, rule.stdout.clone()));
        Ok(Output {
            status: status(code),
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        })
    }

    fn spawn(&self, _spec: &ProcessSpec) -> io::Result<Box<dyn SpawnedChild>> {
        Err(io::Error::other("spawn unused"))
    }
}

const PORCELAIN: &str = "worktree C:/CODE/repo\nHEAD aaa111\nbranch refs/heads/master\n\n\
worktree C:/CODE/AIDP/Pipelines/repo/K-1/.worktree\nHEAD bbb222\nbranch refs/heads/feat/K-1\n\n\
worktree C:/tmp/detached\nHEAD ccc333\ndetached\nlocked reason\n\n\
worktree C:/CODE/AIDP/Pipelines/repo/K-2/.worktree\nHEAD ddd444\nbranch refs/heads/feat/K-2\nprunable gone\n";

fn nowhere() -> PathBuf {
    PathBuf::from("/nowhere")
}

fn no_sleep(_: Duration) {}

#[test]
fn worktree_path_joins_the_ticket_layout() {
    let path = worktree_path(Path::new("/v"), "repo", "K-1");
    assert_eq!(path, Path::new("/v/Pipelines/repo/K-1/.worktree"));
}

#[test]
fn parse_worktree_list_reads_every_field() {
    let entries = parse_worktree_list(PORCELAIN);
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[1].branch.as_deref(), Some("feat/K-1"));
    assert_eq!(entries[1].head, "bbb222");
    assert!(entries[2].detached && entries[2].locked && entries[2].branch.is_none());
    assert!(entries[3].prunable);
    assert!(!entries[0].prunable && !entries[0].locked);
}

#[test]
fn under_pipelines_keeps_only_vault_worktrees() {
    let entries = parse_worktree_list(PORCELAIN);
    let kept = under_pipelines(&entries, Path::new("C:/CODE/AIDP"));
    let branches: Vec<_> = kept.iter().filter_map(|e| e.branch.as_deref()).collect();
    assert_eq!(branches, ["feat/K-1", "feat/K-2"]);
}

#[test]
fn needs_salvage_counts_tracked_and_untracked_not_ignored() {
    assert!(needs_salvage(" M src/a.rs\n"));
    assert!(needs_salvage("?? new.txt\n"));
    assert!(!needs_salvage("!! target/\n"));
    assert!(!needs_salvage(""));
    assert!(needs_salvage("!! target/\n?? new.txt\n"));
}

#[test]
fn default_branch_prefers_origin_head_then_master_then_main() {
    assert_eq!(
        default_branch(Some("refs/remotes/origin/develop\n"), true, true).as_deref(),
        Some("develop")
    );
    assert_eq!(default_branch(None, true, true).as_deref(), Some("master"));
    assert_eq!(default_branch(None, false, true).as_deref(), Some("main"));
    assert_eq!(default_branch(None, false, false), None);
}

#[test]
fn workspace_key_ignores_separator_style() {
    let slash = Workspace {
        kind: WorkspaceKind::Ticket,
        path: PathBuf::from("a/b/"),
        branch: None,
        name: None,
    };
    let back = Workspace {
        path: PathBuf::from("a\\b"),
        ..slash.clone()
    };
    assert_eq!(slash.key(), back.key());
}

#[test]
fn workspace_serializes_camel_case_lowercase_kind() {
    let workspace = Workspace {
        kind: WorkspaceKind::Project,
        path: PathBuf::from("p"),
        branch: Some("b".into()),
        name: None,
    };
    let json = serde_json::to_value(&workspace).unwrap();
    assert_eq!(json["kind"], "project");
    assert_eq!(json["branch"], "b");
}

#[test]
fn ensure_creates_from_origin_default_branch() {
    let vault = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default()
        .rule("symbolic-ref", 0, "refs/remotes/origin/master\n")
        .rule("show-ref --verify --quiet refs/heads/feat/K-1", 1, "")
        .rule(
            "worktree list",
            0,
            "worktree C:/CODE/repo\nHEAD a\nbranch refs/heads/master\n",
        );
    let ensured = ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap();
    assert!(!ensured.reused);
    assert_eq!(ensured.workspace.branch.as_deref(), Some("feat/K-1"));
    assert_eq!(runner.count("worktree add -b feat/K-1"), 1);
    assert!(runner
        .calls()
        .iter()
        .any(|call| call.contains("worktree add -b feat/K-1") && call.ends_with(" origin/master")));
}

#[test]
fn ensure_reuses_a_registered_worktree() {
    let vault = tempfile::tempdir().unwrap();
    let path = worktree_path(vault.path(), "repo", "K-1");
    let listing = format!(
        "worktree {}\nHEAD a\nbranch refs/heads/feat/K-1\n",
        path.display()
    );
    let runner = FakeRunner::default().rule("worktree list", 0, &listing);
    let ensured = ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap();
    assert!(ensured.reused);
    assert_eq!(runner.count("worktree add"), 0);
}

/// A checkout the host prepared at the ticket path on the ticket branch is reused, not re-added.
#[test]
fn ensure_reuses_a_host_checkout_on_the_ticket_branch() {
    let vault = tempfile::tempdir().unwrap();
    let path = worktree_path(vault.path(), "repo", "K-1");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join(".git"), "gitdir: elsewhere").unwrap();
    let runner = FakeRunner::default().rule("rev-parse --abbrev-ref HEAD", 0, "feat/K-1\n");
    let ensured = ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap();
    assert!(ensured.reused);
    assert_eq!(runner.count("worktree add"), 0);
}

/// A checkout at the ticket path on another branch is not taken for the ticket's.
#[test]
fn ensure_ignores_a_host_checkout_on_another_branch() {
    let vault = tempfile::tempdir().unwrap();
    let path = worktree_path(vault.path(), "repo", "K-1");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join(".git"), "gitdir: elsewhere").unwrap();
    let runner = FakeRunner::default().rule("rev-parse --abbrev-ref HEAD", 0, "master\n");
    let ensured = ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap();
    assert!(!ensured.reused);
    assert_eq!(runner.count("worktree add"), 1);
}

#[test]
fn ensure_refuses_a_branch_checked_out_elsewhere_without_creating() {
    let vault = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default().rule(
        "worktree list",
        0,
        "worktree C:/CODE/repo\nHEAD a\nbranch refs/heads/feat/K-1\n",
    );
    let error = ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap_err();
    assert_eq!(error.slug(), "branch-checked-out-elsewhere");
    assert_eq!(runner.count("worktree add"), 0);
    assert!(!worktree_path(vault.path(), "repo", "K-1").exists());
}

#[test]
fn ensure_attaches_an_existing_local_branch() {
    let vault = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default().rule("show-ref --verify --quiet refs/heads/feat/K-1", 0, "");
    ensure(&runner, &nowhere(), vault.path(), "repo", "K-1", &[]).unwrap();
    assert_eq!(runner.count("-b feat/K-1"), 0);
    assert_eq!(runner.count("worktree add"), 1);
}

#[test]
fn ensure_rejects_an_unsafe_key() {
    let vault = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    let error = ensure(&runner, &nowhere(), vault.path(), "repo", "../x", &[]).unwrap_err();
    assert_eq!(error.slug(), "invalid-key");
    assert!(runner.calls().is_empty());
}

#[test]
fn remove_retries_three_times_then_reports_locked() {
    let runner = FakeRunner::default()
        .rule("status --porcelain", 0, "")
        .rule("worktree remove", 1, "");
    let sleeps = Mutex::new(0);
    let sleeper = |_: Duration| *sleeps.lock().unwrap() += 1;
    let error = remove(&runner, &sleeper, &nowhere(), Path::new("/wt"), "K-1").unwrap_err();
    assert!(matches!(error, RemoveError::Locked { attempts: 3, .. }));
    assert_eq!(runner.count("worktree remove"), 3);
    assert_eq!(*sleeps.lock().unwrap(), 2);
    assert_eq!(runner.count("branch -D"), 0);
}

#[test]
fn remove_cleans_branch_and_registration_when_clean() {
    let runner = FakeRunner::default().rule("status --porcelain", 0, "");
    let removed = remove(&runner, &no_sleep, &nowhere(), Path::new("/wt"), "K-1").unwrap();
    assert_eq!(removed.salvaged_ref, None);
    assert_eq!(runner.count("branch -D feat/K-1"), 1);
    assert_eq!(runner.count("worktree prune"), 1);
}

#[test]
fn salvage_names_the_ref_with_twelve_sha_chars() {
    let runner = FakeRunner::default()
        .rule("status --porcelain", 0, " M a.rs\n")
        .rule(
            "rev-parse HEAD",
            0,
            "0123456789abcdef0123456789abcdef01234567\n",
        );
    let reference = salvage(&runner, &nowhere(), Path::new("/wt"), "K-1").unwrap();
    assert_eq!(reference.as_deref(), Some("refs/aidp-wip/K-1-0123456789ab"));
    assert_eq!(
        runner.count("commit --no-verify -m wip(K-1): sauvegarde avant suppression"),
        1
    );
    assert_eq!(runner.count("push"), 0);
}

#[test]
fn salvage_does_nothing_for_a_clean_worktree() {
    let runner = FakeRunner::default().rule("status --porcelain", 0, "!! target/\n");
    assert_eq!(
        salvage(&runner, &nowhere(), Path::new("/wt"), "K-1").unwrap(),
        None
    );
    assert_eq!(runner.count("commit"), 0);
}

#[test]
fn existing_for_branch_ignores_non_ticket_branches() {
    let vault = tempfile::tempdir().unwrap();
    let runner = FakeRunner::default();
    assert!(existing_for_branch(&runner, &nowhere(), vault.path(), "repo", "main").is_none());
}

#[test]
fn wildcard_matches_prefix_and_suffix() {
    assert!(wildcard_matches("*.json", "dev-target.json"));
    assert!(!wildcard_matches("*.json", "dev.txt"));
    assert!(wildcard_matches("a*c", "abbc"));
}

/// A path shaped `<KEY>/.worktree` yields its ticket key, any other path yields nothing.
#[test]
fn worktree_owner_returns_the_ticket_key_of_a_worktree_path() {
    let owned = Path::new("/vault/Pipelines/repo/LOC-9").join(WORKTREE_DIR);
    assert_eq!(worktree_owner(&owned), Some("LOC-9".to_string()));
    assert_eq!(
        worktree_owner(Path::new("/vault/Pipelines/repo/LOC-9")),
        None
    );
}

#[test]
fn ticket_kind_serializes_as_ticket() {
    let json = serde_json::to_value(WorkspaceKind::Ticket).unwrap();
    assert_eq!(json, "ticket");
}

#[test]
fn it_legacy_worktree_kind_still_loads() {
    let legacy = r#"{"kind":"worktree","path":"p","branch":"feat/X"}"#;
    let workspace: Workspace = serde_json::from_str(legacy).unwrap();
    assert_eq!(workspace.kind, WorkspaceKind::Ticket);
    assert_eq!(workspace.name, None);
}

#[test]
fn scratch_workspace_round_trips() {
    let workspace = Workspace {
        kind: WorkspaceKind::Scratch,
        path: PathBuf::from("p"),
        branch: Some("scratch/x".into()),
        name: Some("x".into()),
    };
    let text = serde_json::to_string(&workspace).unwrap();
    let back: Workspace = serde_json::from_str(&text).unwrap();
    assert_eq!(back, workspace);
}

#[test]
fn scratch_paths_and_branches_are_named_after_the_scratch() {
    let path = scratch_path(Path::new("v"), "r", "idea");
    assert!(path.ends_with("Pipelines/r/scratch-idea/.worktree"));
    assert_eq!(scratch_branch_for("idea"), "scratch/idea");
}
