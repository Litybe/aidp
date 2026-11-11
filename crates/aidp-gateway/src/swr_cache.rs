//! A keyed cache that answers the last value at once and refreshes it on a background thread
//! once it is older than its TTL. Only the first read of a key waits for the computation.

use crate::models::constants::swr_cache::REFRESH_THREAD_NAME;
use crate::sync::lock;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Entry<V> {
    value: V,
    stored_at: Instant,
    refreshing: bool,
}

enum Lookup<V> {
    Fresh(V),
    Stale(V),
    Refresh(V),
    Missing,
}

struct Shared<K, V> {
    ttl: Duration,
    entries: Mutex<HashMap<K, Entry<V>>>,
    flights: Mutex<HashMap<K, Arc<Mutex<()>>>>,
    generation: AtomicU64,
}

/// Stale-while-revalidate cache; cloning shares the entries.
pub struct SwrCache<K, V> {
    shared: Arc<Shared<K, V>>,
}

impl<K, V> Clone for SwrCache<K, V> {
    fn clone(&self) -> Self {
        SwrCache {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<K, V> SwrCache<K, V>
where
    K: Hash + Eq + Clone + Send + 'static,
    V: Clone + Send + 'static,
{
    pub fn new(ttl: Duration) -> Self {
        SwrCache {
            shared: Arc::new(Shared {
                ttl,
                entries: Mutex::new(HashMap::new()),
                flights: Mutex::new(HashMap::new()),
                generation: AtomicU64::new(0),
            }),
        }
    }

    fn lookup(&self, key: &K) -> Lookup<V> {
        let mut entries = lock(&self.shared.entries);
        match entries.get_mut(key) {
            None => Lookup::Missing,
            Some(entry) if entry.stored_at.elapsed() < self.shared.ttl => {
                Lookup::Fresh(entry.value.clone())
            }
            Some(entry) if entry.refreshing => Lookup::Stale(entry.value.clone()),
            Some(entry) => {
                entry.refreshing = true;
                Lookup::Refresh(entry.value.clone())
            }
        }
    }

    fn store(&self, key: K, value: V) {
        lock(&self.shared.entries).insert(
            key,
            Entry {
                value,
                stored_at: Instant::now(),
                refreshing: false,
            },
        );
    }

    fn end_refresh(&self, key: &K, generation: u64, result: Option<V>) {
        if self.shared.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match result {
            Some(value) => self.store(key.clone(), value),
            None => {
                if let Some(entry) = lock(&self.shared.entries).get_mut(key) {
                    entry.refreshing = false;
                    entry.stored_at = Instant::now();
                }
            }
        }
    }

    fn spawn_refresh<E>(
        &self,
        key: K,
        compute: impl FnOnce() -> Result<V, E> + Send + 'static,
    ) -> bool {
        let cache = self.clone();
        let generation = self.shared.generation.load(Ordering::SeqCst);
        let thread_key = key.clone();
        std::thread::Builder::new()
            .name(REFRESH_THREAD_NAME.to_string())
            .spawn(move || {
                let result = compute().ok();
                cache.end_refresh(&thread_key, generation, result);
            })
            .is_ok()
    }

    fn flight(&self, key: &K) -> Arc<Mutex<()>> {
        Arc::clone(
            lock(&self.shared.flights)
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn cached(&self, key: &K) -> Option<V> {
        lock(&self.shared.entries)
            .get(key)
            .map(|entry| entry.value.clone())
    }

    /// The cached value, whatever its age (a stale one starts a background refresh through
    /// `compute`). Only a key never computed waits for `compute`; its error is returned, not cached.
    pub fn get<E>(
        &self,
        key: K,
        compute: impl FnOnce() -> Result<V, E> + Send + 'static,
    ) -> Result<V, E> {
        match self.lookup(&key) {
            Lookup::Fresh(value) | Lookup::Stale(value) => Ok(value),
            Lookup::Refresh(value) => {
                if !self.spawn_refresh(key.clone(), compute) {
                    self.end_refresh(&key, self.shared.generation.load(Ordering::SeqCst), None);
                }
                Ok(value)
            }
            Lookup::Missing => {
                let flight = self.flight(&key);
                let _turn = lock(&flight);
                if let Some(value) = self.cached(&key) {
                    return Ok(value);
                }
                let value = compute()?;
                self.store(key, value.clone());
                Ok(value)
            }
        }
    }

    /// Computes now, stores the value and returns it, whatever is cached.
    pub fn refresh<E>(&self, key: K, compute: impl FnOnce() -> Result<V, E>) -> Result<V, E> {
        let value = compute()?;
        self.store(key, value.clone());
        Ok(value)
    }

    /// Forgets every entry; a refresh running meanwhile does not bring one back.
    pub fn clear(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        lock(&self.shared.entries).clear();
    }
}

#[cfg(test)]
#[path = "../tests/unit/swr_cache_tests.rs"]
mod tests;
