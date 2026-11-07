use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative stop signal shared by the daemon's detached loops.
/// Cloned as `Arc<Shutdown>`; every long-lived loop checks `is_set()` at
/// the TOP of each iteration and returns when it is true.
#[derive(Debug, Default)]
pub struct Shutdown(AtomicBool);

impl Shutdown {
    pub fn new() -> Shutdown {
        Shutdown(AtomicBool::new(false))
    }
    /// Idempotent.
    pub fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fresh_flag_is_not_set() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_set());
    }

    #[test]
    fn set_makes_is_set_true() {
        let shutdown = Shutdown::new();
        shutdown.set();
        assert!(shutdown.is_set());
    }

    #[test]
    fn set_is_idempotent() {
        let shutdown = Shutdown::new();
        shutdown.set();
        shutdown.set();
        assert!(shutdown.is_set());
    }

    #[test]
    fn arc_clone_observes_set() {
        let shutdown = Arc::new(Shutdown::new());
        let shutdown_clone = Arc::clone(&shutdown);

        assert!(!shutdown_clone.is_set());

        shutdown.set();
        assert!(shutdown_clone.is_set());
    }
}
