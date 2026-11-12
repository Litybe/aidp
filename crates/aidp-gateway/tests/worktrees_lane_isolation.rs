//! `worktrees::ensure` of two tickets (two lanes) of one project never wait on each other: the
//! `git fetch` runs outside any lock, and the creation lock is per ticket.

use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_gateway::worktrees;
use aidp_testkit::TempWorkspace;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REPO: &str = "svc";

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// Real git, except that the first `git fetch` blocks until released.
struct StallingFirstFetch {
    stalled: AtomicBool,
    entered: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
}

impl ProcessRunner for StallingFirstFetch {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        if spec.args.iter().any(|arg| arg == "fetch") && !self.stalled.swap(true, Ordering::SeqCst)
        {
            self.entered.lock().unwrap().send(()).unwrap();
            let _ = self
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(60));
        }
        OsProcessRunner.output(spec)
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        OsProcessRunner.spawn(spec)
    }
}

#[test]
fn a_slow_fetch_of_one_ticket_does_not_block_another_ticket() {
    let root = TempWorkspace::new("worktrees_lane_isolation");
    let project = root.child("svc");
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-b", "master"]);
    git(&project, &["config", "user.name", "tester"]);
    git(&project, &["config", "user.email", "tester@example.com"]);
    git(&project, &["config", "commit.gpgsign", "false"]);
    fs::write(project.join("README.md"), "seed").unwrap();
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-m", "seed"]);
    git(
        &project,
        &["update-ref", "refs/remotes/origin/master", "HEAD"],
    );
    let vault = root.child("vault");
    fs::create_dir_all(&vault).unwrap();

    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let runner = Arc::new(StallingFirstFetch {
        stalled: AtomicBool::new(false),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let slow = {
        let (runner, project, vault) = (runner.clone(), project.clone(), vault.clone());
        std::thread::spawn(move || {
            worktrees::ensure(runner.as_ref(), &project, &vault, REPO, "LOC-1", &[]).is_ok()
        })
    };
    entered_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the first ensure reached its fetch");

    let (done_tx, done_rx) = channel();
    let other = {
        let (runner, project, vault) = (runner.clone(), project.clone(), vault.clone());
        std::thread::spawn(move || {
            let ok =
                worktrees::ensure(runner.as_ref(), &project, &vault, REPO, "LOC-2", &[]).is_ok();
            done_tx.send(ok).unwrap();
        })
    };
    let finished = done_rx.recv_timeout(Duration::from_secs(30));
    release_tx.send(()).unwrap();
    assert_eq!(
        finished,
        Ok(true),
        "the second ticket waited on the first one's fetch"
    );
    assert!(slow.join().unwrap());
    other.join().unwrap();
}
