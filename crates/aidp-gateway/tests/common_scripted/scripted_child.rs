use super::recording_writer::RecordingWriter;
use aidp_core::process_runner::SpawnedChild;
use std::io::{Cursor, Read, Write};

pub struct ScriptedChild {
    pub stdout: Vec<u8>,
    pub stdin: RecordingWriter,
}

impl SpawnedChild for ScriptedChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(Cursor::new(std::mem::take(&mut self.stdout))))
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(self.stdin.clone()))
    }
    fn id(&self) -> Option<u32> {
        None
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(0)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(0))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
