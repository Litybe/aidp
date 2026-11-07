use std::sync::{Mutex, MutexGuard};

/// Lock that survives a poisoned mutex (a panicking request thread must not take the daemon down).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| {
        tracing::warn!(
            mutex = std::any::type_name::<T>(),
            "recovered poisoned mutex"
        );
        poisoned.into_inner()
    })
}
