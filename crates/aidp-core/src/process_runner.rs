//! The one substitutable point of contact with child processes:
//! [`OsProcessRunner`] spawns them, [`ScriptedProcessRunner`] replays canned
//! output for tests.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;

use crate::models::constants::process::SCRIPTED_RUNNER_POISONED;
use crate::models::port::SharedPort;

pub use crate::models::process::{ProcessSpec, ScriptedOutput};

/// A live or already-finished child process, abstracted just enough that a
/// fake can hand back an in-memory stdout stream.
pub trait SpawnedChild: Send {
    /// Take the child's stdout reader, if one was requested and not already
    /// taken.
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>>;

    /// Take the child's stdin writer, if one was requested and not already
    /// taken.
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>>;

    /// Take stderr when the runner exposes a captured error stream.
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        None
    }

    /// The child's process id, where one exists.
    fn id(&self) -> Option<u32>;

    /// Block until the child exits, returning its exit status as an integer
    /// code (platform-dependent on signal termination).
    fn wait(&mut self) -> std::io::Result<i32>;

    /// Return the exit code if the child has already exited, without blocking.
    fn try_wait(&mut self) -> std::io::Result<Option<i32>>;

    /// Terminate the child immediately.
    fn kill(&mut self) -> std::io::Result<()>;

    /// Stops an owned process tree; fake or custom children default to their own kill operation.
    fn kill_tree(&mut self) -> std::io::Result<()> {
        self.kill()
    }
}

/// A source of child processes.
pub trait ProcessRunner: SharedPort {
    /// Run `spec` to completion, capturing its output — the equivalent of
    /// [`std::process::Command::output`].
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output>;

    /// Start `spec` without waiting for it, returning a handle to interact
    /// with it while it runs.
    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>>;
}

fn build_command(spec: &ProcessSpec) -> Command {
    let mut cmd = Command::new(&spec.program);
    #[cfg(windows)]
    if spec.owned_process_group {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(crate::models::constants::process_tree::HIDDEN_WINDOW);
    }
    #[cfg(unix)]
    if spec.owned_process_group {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &spec.envs {
        cmd.env(k, v);
    }
    cmd
}

struct OsSpawnedChild {
    child: std::process::Child,
    owned_process_group: bool,
}

impl SpawnedChild for OsSpawnedChild {
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>)
    }
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.child
            .stdin
            .take()
            .map(|s| Box::new(s) as Box<dyn Write + Send>)
    }

    fn id(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(self.child.wait()?.code().unwrap_or(-1))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(self
            .child
            .try_wait()?
            .map(|status| status.code().unwrap_or(-1)))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    fn kill_tree(&mut self) -> std::io::Result<()> {
        if !self.owned_process_group {
            return self.child.kill();
        }
        terminate_owned_tree(self.child.id())?;
        if self.child.try_wait()?.is_none() {
            if let Err(error) = self.child.kill() {
                if self.child.try_wait()?.is_none() {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

/// The real runner: every method spawns via [`std::process::Command`].
pub struct OsProcessRunner;

impl ProcessRunner for OsProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        build_command(spec).output()
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let mut cmd = build_command(spec);
        cmd.stdin(stdio_for(spec.piped_stdin));
        cmd.stdout(stdio_for(spec.piped_stdout));
        cmd.stderr(stdio_for(spec.piped_stderr));
        let child = cmd.spawn()?;
        Ok(Box::new(OsSpawnedChild {
            child,
            owned_process_group: spec.owned_process_group,
        }))
    }
}

fn stdio_for(piped: bool) -> Stdio {
    if piped {
        Stdio::piped()
    } else {
        Stdio::inherit()
    }
}

fn terminate_owned_tree(process_id: u32) -> std::io::Result<()> {
    use crate::models::constants::process_tree::*;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = Command::new(TASKKILL)
            .args(TASKKILL_FLAGS)
            .arg(process_id.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(HIDDEN_WINDOW)
            .status()?;
        if status.success() {
            return Ok(());
        }
        let status = Command::new(POWERSHELL)
            .args(POWERSHELL_FLAGS)
            .arg(format!("{WINDOWS_DESCENDANTS}{process_id}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(HIDDEN_WINDOW)
            .status()?;
        if status.success() {
            return Ok(());
        }
    }
    #[cfg(unix)]
    {
        let pid = i32::try_from(process_id)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or_else(|| std::io::Error::other(TREE_UNCONFIRMED))?;
        match rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(windows)]
    return Err(std::io::Error::other(format!(
        "{TREE_UNCONFIRMED}: {process_id}"
    )));
}

struct ScriptedChild {
    stdout: Option<std::io::Cursor<Vec<u8>>>,
    status_code: i32,
}

impl SpawnedChild for ScriptedChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout
            .take()
            .map(|c| Box::new(c) as Box<dyn Read + Send>)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        None
    }

    fn id(&self) -> Option<u32> {
        None
    }

    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(self.status_code)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(self.status_code))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A fake [`ProcessRunner`] replaying canned output instead of spawning real
/// processes. Public so other crates' tests can use it too.
#[derive(Default)]
pub struct ScriptedProcessRunner {
    pub scripted_output: Mutex<ScriptedOutput>,
    pub spawn_stdout: Mutex<Vec<u8>>,
}

impl ScriptedProcessRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// What [`ProcessRunner::output`] returns for every call.
    pub fn with_output(self, out: ScriptedOutput) -> Self {
        *self.scripted_output.lock().expect(SCRIPTED_RUNNER_POISONED) = out;
        self
    }

    /// What a spawned child's stdout reader yields.
    pub fn with_spawn_stdout(self, bytes: Vec<u8>) -> Self {
        *self.spawn_stdout.lock().expect(SCRIPTED_RUNNER_POISONED) = bytes;
        self
    }
}

impl ProcessRunner for ScriptedProcessRunner {
    fn output(&self, _spec: &ProcessSpec) -> std::io::Result<Output> {
        let scripted = self.scripted_output.lock().expect(SCRIPTED_RUNNER_POISONED);
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(scripted.status_code)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(scripted.status_code as u32)
        };
        Ok(Output {
            status,
            stdout: scripted.stdout.clone(),
            stderr: scripted.stderr.clone(),
        })
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let stdout = self
            .spawn_stdout
            .lock()
            .expect(SCRIPTED_RUNNER_POISONED)
            .clone();
        let status_code = self
            .scripted_output
            .lock()
            .expect(SCRIPTED_RUNNER_POISONED)
            .status_code;
        Ok(Box::new(ScriptedChild {
            stdout: Some(std::io::Cursor::new(stdout)),
            status_code,
        }))
    }
}

#[cfg(test)]
#[path = "../tests/unit/process_runner_tests.rs"]
mod tests;
