use crate::models::constants::flow_shutdown::*;
use crate::models::flow_action_child::FlowActionChild;
use crate::models::flow_action_process::FlowActionProcess;
pub use crate::models::flow_action_processes::FlowActionProcesses;
use crate::sync::lock;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use std::io::{Read, Write};
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

impl FlowActionProcesses {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            stopping: AtomicBool::new(false),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Closes spawn admission and confirms every owned child has exited before returning.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        self.stopping.store(true, Ordering::SeqCst);
        let children = lock(&self.children).clone();
        let mut failures = Vec::new();
        for process in children {
            if let Err(error) = process.stop() {
                failures.push(error.to_string());
            }
        }
        anyhow::ensure!(failures.is_empty(), "{}", failures.join("; "));
        Ok(())
    }
}

impl ProcessRunner for FlowActionProcesses {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        let mut spec = spec.clone();
        spec.piped_stdin = true;
        spec.piped_stdout = true;
        spec.piped_stderr = true;
        let mut child = self.spawn(&spec)?;
        drop(child.take_stdin());
        let stdout = capture_stream(child.take_stdout());
        let stderr = capture_stream(child.take_stderr());
        let code = child.wait()?;
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code as u32)
        };
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(if code < 0 { code } else { code << 8 })
        };
        Ok(Output {
            status,
            stdout: joined_stream(stdout)?,
            stderr: joined_stream(stderr)?,
        })
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let mut children = lock(&self.children);
        if self.stopping.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                STOPPED,
            ));
        }
        let mut owned_spec = spec.clone();
        owned_spec.owned_process_group = true;
        let process = Arc::new(FlowActionProcess {
            child: Mutex::new(self.runner.spawn(&owned_spec)?),
            process_group: true,
            stop_failure: Mutex::new(None),
        });
        children.push(process.clone());
        Ok(Box::new(FlowActionChild {
            process,
            children: self.children.clone(),
        }))
    }
}

fn capture_stream(
    stream: Option<Box<dyn Read + Send>>,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stream {
            stream.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    })
}

fn joined_stream(
    worker: std::thread::JoinHandle<std::io::Result<Vec<u8>>>,
) -> std::io::Result<Vec<u8>> {
    worker
        .join()
        .map_err(|_| std::io::Error::other(JOIN_FAILED))?
}

impl FlowActionProcess {
    fn stop(&self) -> std::io::Result<()> {
        let mut child = lock(&self.child);
        let result = self.stop_owned(&mut **child);
        let mut previous = lock(&self.stop_failure);
        match result {
            Ok(()) => {
                *previous = None;
                Ok(())
            }
            Err(error) => {
                let primary = previous.get_or_insert_with(|| error.to_string());
                Err(std::io::Error::other(primary.clone()))
            }
        }
    }

    fn stop_owned(&self, child: &mut dyn SpawnedChild) -> std::io::Result<()> {
        if self.process_group {
            child.kill_tree()?;
        } else {
            child.kill()?;
        }
        let deadline = Instant::now() + EXIT_LIMIT;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    EXIT_UNCONFIRMED,
                ));
            }
            std::thread::sleep(PROCESS_POLL);
        }
        Ok(())
    }
}

impl SpawnedChild for FlowActionChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        lock(&self.process.child).take_stdout()
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        lock(&self.process.child).take_stdin()
    }
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        lock(&self.process.child).take_stderr()
    }
    fn id(&self) -> Option<u32> {
        lock(&self.process.child).id()
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        lock(&self.process.child).try_wait()
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        loop {
            if let Some(code) = self.try_wait()? {
                return Ok(code);
            }
            std::thread::sleep(PROCESS_POLL);
        }
    }
    fn kill(&mut self) -> std::io::Result<()> {
        self.process.stop()
    }
}

impl Drop for FlowActionChild {
    fn drop(&mut self) {
        let mut child = lock(&self.process.child);
        if lock(&self.process.stop_failure).is_some() {
            return;
        }
        if child.try_wait().ok().flatten().is_some() {
            lock(&self.children).retain(|process| !Arc::ptr_eq(process, &self.process));
        }
    }
}
