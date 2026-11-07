use super::recording_writer::RecordingWriter;
use super::scripted_child::ScriptedChild;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use std::sync::Mutex;

const UNEXPECTED_OUTPUT: &str = "unexpected output call in scripted fixture";

pub struct ScriptedRunner {
    pub stdout: Vec<u8>,
    pub stdin: RecordingWriter,
    pub specs: Mutex<Vec<ProcessSpec>>,
}

impl ScriptedRunner {
    pub fn new(stdout: &str) -> Self {
        Self {
            stdout: stdout.as_bytes().to_vec(),
            stdin: RecordingWriter::default(),
            specs: Mutex::new(Vec::new()),
        }
    }
}

impl ProcessRunner for ScriptedRunner {
    fn output(&self, _spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        Err(std::io::Error::other(UNEXPECTED_OUTPUT))
    }
    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        self.specs.lock().unwrap().push(spec.clone());
        Ok(Box::new(ScriptedChild {
            stdout: self.stdout.clone(),
            stdin: self.stdin.clone(),
        }))
    }
}
