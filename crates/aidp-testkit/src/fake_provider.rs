use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use aidp_agents::{
    AgentError, AgentEvent, AgentHandle, AgentProvider, AgentSpec, Capabilities, ExitReason,
};

use crate::fake_provider_script::FakeScript;
use crate::fake_provider_script::FakeStep;
use crate::models::fake_control::FakeControl;
pub use crate::models::fake_provider::FakeProvider;

const PROVIDER_ID: &str = "fake";
const CANCEL_POLL: Duration = Duration::from_millis(5);

/// Sleeps in short slices; returns false when cancelled meanwhile.
fn sleep_unless_cancelled(duration: Duration, cancelled: &AtomicBool) -> bool {
    let mut remaining = duration;
    while !remaining.is_zero() {
        if cancelled.load(Ordering::SeqCst) {
            return false;
        }
        let slice = remaining.min(CANCEL_POLL);
        thread::sleep(slice);
        remaining -= slice;
    }
    !cancelled.load(Ordering::SeqCst)
}

fn write_file(spec: &AgentSpec, path: &str, contents: &str) -> std::io::Result<()> {
    let target = spec.work_dir.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, contents)
}

/// Plays the script and returns the exit reason it sent.
fn play(
    script: FakeScript,
    spec: AgentSpec,
    sender: Sender<AgentEvent>,
    cancelled: Arc<AtomicBool>,
) -> ExitReason {
    for step in script.steps {
        if cancelled.load(Ordering::SeqCst) {
            return finish(&sender, None, ExitReason::Cancelled);
        }
        match step {
            FakeStep::Emit(event) => {
                let _ = sender.send(event);
            }
            FakeStep::WriteFile { path, contents } => {
                if write_file(&spec, &path, &contents).is_err() {
                    return finish(&sender, None, ExitReason::Failed);
                }
            }
            FakeStep::Delay(duration) => {
                if !sleep_unless_cancelled(duration, &cancelled) {
                    return finish(&sender, None, ExitReason::Cancelled);
                }
            }
        }
    }
    let reason = if script.exit_code == 0 {
        ExitReason::Completed
    } else {
        ExitReason::Failed
    };
    finish(&sender, Some(script.exit_code), reason)
}

fn finish(sender: &Sender<AgentEvent>, code: Option<i32>, reason: ExitReason) -> ExitReason {
    let _ = sender.send(AgentEvent::Exited { code, reason });
    reason
}

impl AgentProvider for FakeProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError> {
        self.launched
            .lock()
            .expect("launched lock")
            .push(spec.clone());
        let (sender, receiver) = channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = {
            let script = self.script.clone();
            let cancelled = Arc::clone(&cancelled);
            thread::spawn(move || play(script, spec, sender, cancelled))
        };
        let control = FakeControl {
            cancelled,
            worker: Mutex::new(Some(worker)),
            outcome: Mutex::new(None),
        };
        Ok(AgentHandle::new(receiver, Box::new(control)))
    }
}

#[cfg(test)]
#[path = "../tests/unit/fake_provider_tests.rs"]
mod fake_provider_tests;
