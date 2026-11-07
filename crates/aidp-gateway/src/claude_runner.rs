//! The production [`aidp_core::process_runner::ProcessRunner`] for `claude` sessions.
//! Providers consume child stderr without forwarding confidential diagnostics.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};
use anyhow::anyhow;

use crate::sessions::{claude_command, kill_tree, wait_for_exit};
use crate::sync::lock;

const SPAWN_ERROR_PREFIX: &str = "cannot start claude";
const DEFAULT_CWD: &str = ".";

/// Spawn `claude` with the given args in `cwd` (see [`claude_command`]).
fn spawn_claude(
    args: &[String],
    cwd: &str,
    extra_env: &[(String, String)],
) -> anyhow::Result<Child> {
    claude_command(args, cwd, extra_env)?
        .spawn()
        .map_err(|e| anyhow!("{SPAWN_ERROR_PREFIX}: {e}"))
}

/// Wraps the real `claude` child as a [`SpawnedChild`], reusing `sessions.rs` kill/wait.
/// Providers take ownership of piped stderr.
struct ClaudeSpawnedChild(Mutex<Child>);

impl ClaudeSpawnedChild {
    fn new(child: Child) -> Self {
        Self(Mutex::new(child))
    }
}

impl SpawnedChild for ClaudeSpawnedChild {
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        lock(&self.0)
            .stderr
            .take()
            .map(|stderr| Box::new(stderr) as Box<dyn Read + Send>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        lock(&self.0)
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        lock(&self.0)
            .stdin
            .take()
            .map(|s| Box::new(s) as Box<dyn Write + Send>)
    }

    fn id(&self) -> Option<u32> {
        Some(lock(&self.0).id())
    }

    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(wait_for_exit(&self.0).unwrap_or(-1))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(lock(&self.0)
            .try_wait()?
            .map(|status| status.code().unwrap_or(-1)))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        kill_tree(&mut lock(&self.0));
        Ok(())
    }
}

/// The production [`ProcessRunner`] `SessionManager` spawns `claude` sessions through.
/// `output` delegates to [`OsProcessRunner`]; `spawn` is the sole caller of [`spawn_claude`].
pub struct ClaudeProcessRunner;

impl ProcessRunner for ClaudeProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        OsProcessRunner.output(spec)
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        if spec.program != crate::claude_provider::CLAUDE_PROGRAM {
            let mut command = Command::new(&spec.program);
            command.args(&spec.args).envs(&spec.envs);
            if let Some(cwd) = &spec.cwd {
                command.current_dir(cwd);
            }
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(crate::models::constants::process::CREATE_NO_WINDOW);
            }
            return Ok(Box::new(ClaudeSpawnedChild::new(command.spawn()?)));
        }
        let cwd = spec
            .cwd
            .as_deref()
            .and_then(|p| p.to_str())
            .unwrap_or(DEFAULT_CWD)
            .to_string();
        let extra_env: Vec<(String, String)> = spec
            .envs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let child = spawn_claude(&spec.args, &cwd, &extra_env)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Box::new(ClaudeSpawnedChild::new(child)))
    }
}
