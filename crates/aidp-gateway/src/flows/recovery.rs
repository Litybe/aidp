//! Restart and wake-ups: session end on the bus, GitHub webhook, 60 s timer.

use super::FlowEngine;
pub use crate::models::enums::FlowWorker;
use crate::sessions::EVENT_SESSION_ENDED;
use crate::shutdown::Shutdown;
use crate::threads;
use serde_json::Value;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Interval of the timer that polls every active run.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);
const EVENT_TYPE_KEY: &str = "type";
const SLICE: Duration = Duration::from_millis(200);
const BUS_RECV_TIMEOUT: Duration = Duration::from_millis(500);

/// Reloads every active run and re-attaches its sessions and waits.
/// Returns how many runs were examined. Call once, when the daemon starts.
pub fn resume_all(engine: &Arc<FlowEngine>) -> usize {
    engine.executor().reconcile_all()
}

/// One poll of every active run: finished sessions, waits, budget retries.
pub fn tick(engine: &Arc<FlowEngine>) -> usize {
    engine.executor().reconcile_all()
}

/// The hook point for the GitHub webhook: any delivery re-polls the waits.
/// `payload` is the webhook body; its content is not needed to decide.
pub fn wake_from_webhook(engine: &Arc<FlowEngine>, _payload: &Value) -> usize {
    tick(engine)
}

/// The resident threads of the flow engine.
pub struct Background {
    handles: Vec<JoinHandle<()>>,
}

impl Background {
    /// Waits for the threads to exit; set the shutdown flag first.
    pub fn join(self) {
        for handle in self.handles {
            let _ = handle.join();
        }
    }
}

/// Starts the timer thread and the bus listener thread. Both stop at `shutdown`.
pub fn spawn_background(
    engine: &Arc<FlowEngine>,
    shutdown: Arc<Shutdown>,
    interval: Duration,
) -> Background {
    let timer = {
        let (engine, shutdown) = (engine.clone(), shutdown.clone());
        std::thread::spawn(move || run_timer(&engine, &shutdown, interval))
    };
    let listener = {
        let (engine, shutdown) = (engine.clone(), shutdown);
        std::thread::spawn(move || run_bus_listener(&engine, &shutdown))
    };
    Background {
        handles: vec![timer, listener],
    }
}

fn run_timer(engine: &Arc<FlowEngine>, shutdown: &Shutdown, interval: Duration) {
    let _loop = threads::enter_loop(FlowWorker::Timer.as_str());
    let mut elapsed = Duration::ZERO;
    while !shutdown.is_set() {
        std::thread::sleep(SLICE);
        elapsed += SLICE;
        if elapsed < interval {
            continue;
        }
        elapsed = Duration::ZERO;
        threads::guarded(FlowWorker::Timer.as_str(), || tick(engine));
    }
}

fn run_bus_listener(engine: &Arc<FlowEngine>, shutdown: &Shutdown) {
    let _loop = threads::enter_loop(FlowWorker::Bus.as_str());
    let receiver = engine.executor().ports.bus.subscribe();
    while !shutdown.is_set() {
        match receiver.recv_timeout(BUS_RECV_TIMEOUT) {
            Ok(event) => {
                if event[EVENT_TYPE_KEY].as_str() == Some(EVENT_SESSION_ENDED) {
                    threads::guarded(FlowWorker::Bus.as_str(), || tick(engine));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}
