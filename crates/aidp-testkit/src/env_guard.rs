use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

thread_local! {
    static HELD_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A guard that manages temporary environment variable changes.
///
/// `EnvGuard` allows modifying environment variables for the duration of its lifetime,
/// automatically restoring the original values when dropped. It uses a single process-wide
/// lock to serialize threads; the owning thread may nest guards, which must drop in LIFO order.
pub struct EnvGuard {
    prior_values: HashMap<String, Option<OsString>>,
    lock: Option<MutexGuard<'static, ()>>,
}

impl EnvGuard {
    /// Creates a guard that sets a single environment variable.
    pub fn set(key: &str, value: &str) -> EnvGuard {
        EnvGuard::with(&[(key, Some(value))])
    }

    /// Creates a guard that unsets a single environment variable.
    pub fn unset(key: &str) -> EnvGuard {
        EnvGuard::with(&[(key, None)])
    }

    /// Creates a guard that modifies multiple environment variables at once.
    ///
    /// # Arguments
    ///
    /// * `pairs` - A slice of `(key, Option<value>)` tuples.
    ///   - `Some(value)` sets the variable
    ///   - `None` removes the variable
    pub fn with(pairs: &[(&str, Option<&str>)]) -> EnvGuard {
        let lock = Self::acquire_if_outermost();
        let mut prior_values = HashMap::new();

        // Record prior values for all keys
        for (key, _) in pairs {
            prior_values.insert(key.to_string(), std::env::var_os(key));
        }

        // Apply new values
        for (key, value) in pairs {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }

        EnvGuard { prior_values, lock }
    }

    /// Takes the process-wide lock unless this thread already holds it, then counts the guard.
    fn acquire_if_outermost() -> Option<MutexGuard<'static, ()>> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let already_held = HELD_DEPTH.with(|depth| depth.get() > 0);
        let lock = if already_held {
            None
        } else {
            Some(
                LOCK.get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            )
        };
        HELD_DEPTH.with(|depth| depth.set(depth.get() + 1));
        lock
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prior_value) in &self.prior_values {
            match prior_value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        HELD_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        self.lock.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    #[test]
    fn env_guard_restores_after_panic() {
        const TEST_KEY: &str = "AIDP_TESTKIT_PROBE";

        // Record the prior value
        let prior = std::env::var_os(TEST_KEY);

        // Set a value, panic inside the guard, and verify restoration
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = EnvGuard::set(TEST_KEY, "test_value");

            // Verify the value was set
            assert_eq!(std::env::var(TEST_KEY).unwrap(), "test_value");

            panic!("Intentional panic for testing");
        }));

        assert!(result.is_err(), "Expected panic to occur");

        // After the guard is dropped, the prior value should be restored
        let after = std::env::var_os(TEST_KEY);
        assert_eq!(
            prior, after,
            "Environment variable should be restored to prior value after panic"
        );
    }

    #[test]
    #[should_panic(expected = "Intentional panic for testing")]
    fn env_guard_poisons_nothing_a() {
        const TEST_KEY: &str = "AIDP_TESTKIT_POISON_TEST";

        // Take a guard on the key and panic while holding it
        let _guard = EnvGuard::set(TEST_KEY, "value_a");

        panic!("Intentional panic for testing");
    }

    #[test]
    fn env_guard_poisons_nothing_b() {
        const TEST_KEY: &str = "AIDP_TESTKIT_POISON_TEST";

        // This test should run after env_guard_poisons_nothing_a panicked
        // and successfully take the guard without PoisonError
        let _guard = EnvGuard::set(TEST_KEY, "value_b");

        // If we get here, the lock was not poisoned
        assert_eq!(std::env::var(TEST_KEY).unwrap(), "value_b");
    }

    #[test]
    fn nested_env_guards_on_one_thread_do_not_deadlock() {
        const OUTER_KEY: &str = "AIDP_TESTKIT_NESTED_OUTER";
        const INNER_KEY: &str = "AIDP_TESTKIT_NESTED_INNER";
        let outer = EnvGuard::set(OUTER_KEY, "outer");
        let inner = EnvGuard::set(INNER_KEY, "inner");
        assert_eq!(std::env::var(OUTER_KEY).unwrap(), "outer");
        assert_eq!(std::env::var(INNER_KEY).unwrap(), "inner");
        drop(inner);
        assert!(std::env::var_os(INNER_KEY).is_none());
        assert_eq!(std::env::var(OUTER_KEY).unwrap(), "outer");
        drop(outer);
        assert!(std::env::var_os(OUTER_KEY).is_none());
    }
}
