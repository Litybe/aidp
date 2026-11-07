//! Plain data describing processes for [`crate::process_runner::ProcessRunner`].

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A process to run, as plain data; the runner decides how to execute it.
#[derive(Debug, Clone, Default)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub envs: BTreeMap<String, String>,
    pub piped_stdin: bool,
    pub piped_stdout: bool,
    pub piped_stderr: bool,
    pub owned_process_group: bool,
}

impl ProcessSpec {
    /// `program` with no arguments, no piped stdio, inherited cwd and env.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// A canned reply a scripted runner hands back for one call.
#[derive(Debug, Clone, Default)]
pub struct ScriptedOutput {
    pub status_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
