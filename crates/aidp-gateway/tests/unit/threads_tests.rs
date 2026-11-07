use super::{
    enter_loop, guarded, snapshot, KEY_LOOP_PANICS, KEY_RESIDENT_LOOPS, KEY_RESIDENT_TOTAL,
};

const GUARDED_RESULT: u32 = 42;
const PANIC_MESSAGE: &str = "iteration failed on purpose";

/// A loop name no other test uses, so parallel tests never share a counter.
fn unique_loop_name(label: &str) -> &'static str {
    Box::leak(format!("test.{label}.{}", nanos_since_epoch()).into_boxed_str())
}

fn nanos_since_epoch() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default()
}

fn live_count(name: &str) -> u64 {
    snapshot()[KEY_RESIDENT_LOOPS][name].as_u64().unwrap_or(0)
}

fn loop_panics() -> u64 {
    snapshot()[KEY_LOOP_PANICS].as_u64().unwrap_or(0)
}

#[test]
fn enter_loop_counts_while_the_guard_lives() {
    let name = unique_loop_name("lifetime");
    let guard = enter_loop(name);
    assert_eq!(live_count(name), 1);
    drop(guard);
    assert_eq!(live_count(name), 0);
}

#[test]
fn nested_guards_of_one_loop_add_up() {
    let name = unique_loop_name("nested");
    let first = enter_loop(name);
    let second = enter_loop(name);
    assert_eq!(live_count(name), 2);
    drop(first);
    assert_eq!(live_count(name), 1);
    drop(second);
    assert_eq!(live_count(name), 0);
}

#[test]
fn guard_dropped_during_unwinding_is_uncounted() {
    let name = unique_loop_name("unwind");
    let outcome = std::panic::catch_unwind(|| {
        let _guard = enter_loop(name);
        assert_eq!(live_count(name), 1);
        panic!("{PANIC_MESSAGE}");
    });
    assert!(outcome.is_err());
    assert_eq!(live_count(name), 0);
}

#[test]
fn guarded_returns_the_body_result_on_success() {
    let name = unique_loop_name("success");
    assert_eq!(guarded(name, || GUARDED_RESULT), Some(GUARDED_RESULT));
}

#[test]
fn guarded_turns_a_panic_into_none_and_counts_it() {
    let name = unique_loop_name("panic");
    let panics_before = loop_panics();
    let result: Option<u32> = guarded(name, || panic!("{PANIC_MESSAGE}"));
    assert_eq!(result, None);
    assert!(loop_panics() > panics_before);
}

#[test]
fn guarded_survives_a_str_panic_payload() {
    let name = unique_loop_name("str-payload");
    let result: Option<()> = guarded(name, || std::panic::panic_any(PANIC_MESSAGE));
    assert_eq!(result, None);
}

#[test]
fn snapshot_exposes_the_three_keys() {
    let name = unique_loop_name("snapshot");
    let _guard = enter_loop(name);
    let health = snapshot();
    assert!(health[KEY_RESIDENT_LOOPS].is_object());
    assert!(health[KEY_RESIDENT_TOTAL].as_u64().unwrap_or(0) >= 1);
    assert!(health[KEY_LOOP_PANICS].is_u64());
}

mod poisoned_lock {
    use crate::sync::lock;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::Level;

    const POISONED_VALUE: u32 = 7;
    const RECOVERED_MESSAGE: &str = "recovered poisoned mutex";
    const MUTEX_TYPE_NAME: &str = "u32";
    const LEVEL_WARN: &str = "WARN";
    const POISON_PANIC: &str = "poison the mutex";

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Runs `body` under a thread-local WARN subscriber and returns what it wrote.
    fn captured_output(body: impl FnOnce()) -> String {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::WARN)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = capture.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// Locking a poisoned mutex logs a warning and still returns the value.
    #[test]
    fn recovered_poisoned_mutex_is_logged_and_value_returned() {
        let mutex = Arc::new(Mutex::new(POISONED_VALUE));
        let poisoner = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("{POISON_PANIC}");
        })
        .join();
        assert!(mutex.is_poisoned());

        let mut value = 0;
        let output = captured_output(|| value = *lock(&mutex));

        assert_eq!(value, POISONED_VALUE);
        assert!(output.contains(LEVEL_WARN), "{output}");
        assert!(output.contains(RECOVERED_MESSAGE), "{output}");
        assert!(output.contains(MUTEX_TYPE_NAME), "{output}");
    }
}
