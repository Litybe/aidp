//! In-process event bus: every session, gate, CI and cron event flows through here.
//! A ring buffer lets late subscribers catch up.

use crate::sync::lock;
use crate::time::now_ms;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;

const CAPACITY: usize = 500;
const EVENT_ID_KEY: &str = "id";
const EVENT_TYPE_KEY: &str = "type";
const EVENT_TS_KEY: &str = "ts";
const EVENT_DATA_KEY: &str = "data";

/// Events a subscriber may have queued before further events are dropped for it.
pub const SUBSCRIBER_CAPACITY: usize = 1024;

pub struct Bus {
    inner: Mutex<Inner>,
}

struct Inner {
    seq: u64,
    buffer: VecDeque<Value>,
    subs: Vec<SyncSender<Value>>,
    dropped_events: u64,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        Bus {
            inner: Mutex::new(Inner {
                seq: 0,
                buffer: VecDeque::new(),
                subs: Vec::new(),
                dropped_events: 0,
            }),
        }
    }

    pub fn emit(&self, ty: &str, data: Value) -> Value {
        let mut g = lock(&self.inner);
        g.seq += 1;
        let ev = json!({ EVENT_ID_KEY: g.seq, EVENT_TYPE_KEY: ty, EVENT_TS_KEY: now_ms(), EVENT_DATA_KEY: data });
        g.buffer.push_back(ev.clone());
        while g.buffer.len() > CAPACITY {
            g.buffer.pop_front();
        }
        g.fan_out(&ev);
        ev
    }

    /// Total events dropped because a subscriber's queue was full.
    pub fn dropped_events(&self) -> u64 {
        lock(&self.inner).dropped_events
    }

    /// Subscribers still registered (a disconnected one is removed at the next emit).
    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner).subs.len()
    }

    pub fn recent(&self, limit: usize, since: u64) -> Vec<Value> {
        let g = lock(&self.inner);
        let all: Vec<Value> = g
            .buffer
            .iter()
            .filter(|e| e[EVENT_ID_KEY].as_u64().unwrap_or(0) > since)
            .cloned()
            .collect();
        let skip = all.len().saturating_sub(limit);
        all.into_iter().skip(skip).collect()
    }

    /// Registers a subscriber whose queue holds at most `SUBSCRIBER_CAPACITY` events.
    pub fn subscribe(&self) -> Receiver<Value> {
        let (tx, rx) = sync_channel(SUBSCRIBER_CAPACITY);
        lock(&self.inner).subs.push(tx);
        rx
    }
}

impl Inner {
    /// Sends `event` to each subscriber without blocking; full queues drop and count it.
    /// Disconnected receivers are removed. Runs under the bus lock, so it must not emit.
    fn fan_out(&mut self, event: &Value) {
        let mut dropped = 0u64;
        self.subs
            .retain(|subscriber| match subscriber.try_send(event.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    dropped += 1;
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            });
        self.dropped_events += dropped;
    }
}

#[cfg(test)]
#[path = "../tests/unit/bus_tests.rs"]
mod bus_tests;
