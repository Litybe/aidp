//! Bounded subscriber channels of the in-process event bus.

use super::{Bus, SUBSCRIBER_CAPACITY};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const EVENT_COUNT: usize = 10_000;
const RING_CAPACITY: usize = 500;
const MAX_EMIT_DURATION: Duration = Duration::from_secs(5);
const EVENT_TYPE: &str = "test.event";

#[test]
fn stuck_subscriber_queue_is_bounded_and_emit_never_blocks() {
    let bus = Bus::new();
    let stuck = bus.subscribe();

    let started = Instant::now();
    for index in 0..EVENT_COUNT {
        bus.emit(EVENT_TYPE, json!({ "index": index }));
    }
    let elapsed = started.elapsed();

    assert!(elapsed < MAX_EMIT_DURATION, "emit loop took {elapsed:?}");
    let queued = stuck.try_iter().count();
    assert!(queued <= SUBSCRIBER_CAPACITY, "queued {queued}");
    assert!(bus.dropped_events() >= (EVENT_COUNT - SUBSCRIBER_CAPACITY) as u64);
    assert_eq!(bus.subscriber_count(), 1);
}

#[test]
fn reading_subscriber_receives_every_event_in_order_beside_a_stuck_one() {
    let bus = Bus::new();
    let _stuck = bus.subscribe();
    let reader = bus.subscribe();
    let received = Arc::new(AtomicUsize::new(0));
    let received_by_drain = Arc::clone(&received);

    let drain = thread::spawn(move || {
        let mut indexes = Vec::with_capacity(EVENT_COUNT);
        while indexes.len() < EVENT_COUNT {
            let event = reader.recv().expect("bus still alive");
            indexes.push(event["data"]["index"].as_u64().expect("index"));
            received_by_drain.fetch_add(1, Ordering::SeqCst);
        }
        indexes
    });

    for index in 0..EVENT_COUNT {
        wait_for_reader_headroom(&received, index);
        bus.emit(EVENT_TYPE, json!({ "index": index }));
    }

    let indexes = drain.join().expect("drain thread");
    let expected: Vec<u64> = (0..EVENT_COUNT as u64).collect();
    assert_eq!(indexes, expected);
}

/// Holds the emitter until the reader lags by less than half its queue, so a
/// slow test machine never makes the reader itself drop events.
fn wait_for_reader_headroom(received: &AtomicUsize, emitted: usize) {
    let deadline = Instant::now() + MAX_EMIT_DURATION;
    while emitted - received.load(Ordering::SeqCst).min(emitted) >= SUBSCRIBER_CAPACITY / 2 {
        assert!(Instant::now() < deadline, "reader stopped draining");
        thread::yield_now();
    }
}

#[test]
fn dropped_receiver_is_removed_at_next_emit() {
    let bus = Bus::new();
    let kept = bus.subscribe();
    let gone = bus.subscribe();
    assert_eq!(bus.subscriber_count(), 2);

    drop(gone);
    bus.emit(EVENT_TYPE, json!({}));

    assert_eq!(bus.subscriber_count(), 1);
    assert_eq!(kept.try_iter().count(), 1);
}

#[test]
fn recent_is_capped_by_the_ring_buffer() {
    let bus = Bus::new();
    for index in 0..EVENT_COUNT {
        bus.emit(EVENT_TYPE, json!({ "index": index }));
    }

    let recent = bus.recent(usize::MAX, 0);

    assert_eq!(recent.len(), RING_CAPACITY);
    assert_eq!(
        recent.last().expect("last")["data"]["index"],
        json!(EVENT_COUNT - 1)
    );
}
