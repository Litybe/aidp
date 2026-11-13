//! An in-memory `git` for the integration tests: repositories live in memory, worktrees are real
//! directories on disk, and no process is spawned. It serves as the `ProcessRunner` of the
//! gateway and as its `Git` port.

mod commands;
mod constants;
mod merge;
mod state;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

use aidp_core::git::Git;
use aidp_core::gitlog::Commit;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};

use commands::Reply;
use constants::{GIT_PROGRAM, WORKTREE_LINE};
use state::{Files, State};

#[cfg(windows)]
fn status_of(code: i32) -> std::process::ExitStatus {
    std::os::windows::process::ExitStatusExt::from_raw(code as u32)
}

#[cfg(unix)]
fn status_of(code: i32) -> std::process::ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(code << 8)
}

/// Every `git` command is answered from memory; any other program goes to the real runner.
#[derive(Default)]
pub struct FakeGit {
    state: Mutex<State>,
    commands: Mutex<Vec<Vec<String>>>,
    unsupported: Mutex<Vec<String>>,
}

impl FakeGit {
    pub fn new() -> Self {
        Self::default()
    }

    /// A repository at `workspace` holding `files` on `branch`, with a pushed bare remote at
    /// `origin` when given.
    pub fn seed(
        &self,
        workspace: &Path,
        origin: Option<&Path>,
        branch: &str,
        files: &[(&str, &str)],
    ) {
        let files: Files = files
            .iter()
            .map(|(path, contents)| (path.to_string(), contents.as_bytes().to_vec()))
            .collect();
        self.state
            .lock()
            .unwrap()
            .seed(workspace, origin, branch, &files);
    }

    /// Runs `git args` in `cwd`; the trimmed stdout, or the failure.
    pub fn run(&self, cwd: &Path, args: &[&str]) -> Result<String, String> {
        let reply = self.execute(
            cwd,
            &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
        );
        if reply.code == 0 {
            Ok(reply.stdout.trim().to_string())
        } else {
            Err(format!("git {args:?}: {}{}", reply.stdout, reply.stderr))
        }
    }

    /// The bytes of `spec` (`revision:path`) in the repository at `cwd`.
    pub fn show(&self, cwd: &Path, spec: &str) -> Vec<u8> {
        let reply = self.execute(cwd, &[constants::CMD_SHOW.to_string(), spec.to_string()]);
        assert_eq!(reply.code, 0, "git show {spec}: {}", reply.stderr);
        reply.stdout.into_bytes()
    }

    /// The commands the fake did not know how to answer.
    pub fn unsupported(&self) -> Vec<String> {
        self.unsupported.lock().unwrap().clone()
    }

    fn execute(&self, cwd: &Path, args: &[String]) -> Reply {
        self.commands.lock().unwrap().push(args.to_vec());
        let reply = self.state.lock().unwrap().run(cwd, args);
        if reply.stderr.starts_with(constants::UNSUPPORTED) {
            self.unsupported.lock().unwrap().push(reply.stderr.clone());
        }
        reply
    }
}

impl ProcessRunner for FakeGit {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        if spec.program != GIT_PROGRAM {
            return OsProcessRunner.output(spec);
        }
        let cwd = spec.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
        let reply = self.execute(&cwd, &spec.args);
        Ok(Output {
            status: status_of(reply.code),
            stdout: reply.stdout.into_bytes(),
            stderr: reply.stderr.into_bytes(),
        })
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        OsProcessRunner.spawn(spec)
    }
}

impl Git for FakeGit {
    fn head(&self, workspace: &Path) -> Option<String> {
        self.run(workspace, &["rev-parse", "HEAD"]).ok()
    }

    fn branch(&self, workspace: &Path) -> Option<String> {
        self.run(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
    }

    fn log(
        &self,
        _workspace: &Path,
        _n: usize,
        _since: Option<&str>,
    ) -> anyhow::Result<Vec<Commit>> {
        Ok(Vec::new())
    }

    fn changed(&self, workspace: &Path, base: &str) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        if let Ok(names) = self.run(workspace, &["diff", "--name-only", base, "--"]) {
            paths.extend(names.lines().map(str::to_string));
        }
        if let Ok(status) = self.run(
            workspace,
            &["status", "--porcelain", "--untracked-files=all"],
        ) {
            paths.extend(
                status
                    .lines()
                    .filter_map(|line| line.get(3..))
                    .map(str::to_string),
            );
        }
        paths.sort();
        paths.dedup();
        paths
    }

    fn last_commit_date(&self, _workspace: &Path, _file: &str) -> Option<String> {
        None
    }

    fn is_usable(&self, workspace: &Path) -> bool {
        self.run(workspace, &["rev-parse", "--is-inside-work-tree"])
            .is_ok()
    }

    fn worktree_list(&self, workspace: &Path) -> Vec<PathBuf> {
        let listing = self
            .run(workspace, &["worktree", "list", "--porcelain"])
            .unwrap_or_default();
        let root = state::key(workspace);
        listing
            .lines()
            .filter_map(|line| line.strip_prefix(WORKTREE_LINE))
            .map(PathBuf::from)
            .filter(|path| state::key(path) != root)
            .collect()
    }
}
