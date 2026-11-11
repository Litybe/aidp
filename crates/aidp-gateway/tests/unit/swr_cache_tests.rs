use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

const KEY: &str = "key";
const SECOND_KEY: &str = "second";
const DOWN: &str = "down";
const LONG_TTL: Duration = Duration::from_secs(600);
const WAIT_LIMIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(5);
const SLOW_COMPUTE: Duration = Duration::from_millis(500);
const FAST_ANSWER: Duration = Duration::from_millis(300);

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    while !condition() && Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
}

fn counted(
    calls: &Arc<AtomicUsize>,
    value: u32,
) -> impl FnOnce() -> Result<u32, String> + Send + 'static {
    let calls = Arc::clone(calls);
    move || {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(value)
    }
}

fn value_of(cache: &SwrCache<&'static str, u32>, key: &'static str) -> Option<u32> {
    cache.cached(&key)
}

#[test]
fn a_fresh_value_is_served_without_computing_again() {
    let cache = SwrCache::new(LONG_TTL);
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(cache.get(KEY, counted(&calls, 1)), Ok(1));
    assert_eq!(cache.get(KEY, counted(&calls, 2)), Ok(1));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_stale_value_is_served_at_once_then_replaced_in_the_background() {
    let cache = SwrCache::new(Duration::ZERO);
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(cache.get(KEY, counted(&calls, 1)), Ok(1));
    assert_eq!(cache.get(KEY, counted(&calls, 2)), Ok(1));
    wait_until(|| value_of(&cache, KEY) == Some(2));
    assert_eq!(value_of(&cache, KEY), Some(2));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_stale_read_never_waits_for_the_refresh() {
    let cache = SwrCache::new(Duration::ZERO);
    cache.get(KEY, || Ok::<_, String>(1)).unwrap();
    let started = Instant::now();
    let served = cache.get(KEY, || {
        std::thread::sleep(SLOW_COMPUTE);
        Ok::<_, String>(2)
    });
    assert_eq!(served, Ok(1));
    assert!(started.elapsed() < FAST_ANSWER);
}

#[test]
fn a_failed_refresh_keeps_the_last_value() {
    let cache = SwrCache::new(Duration::ZERO);
    cache.get(KEY, || Ok::<_, String>(1)).unwrap();
    assert_eq!(cache.get(KEY, || Err(DOWN.to_string())), Ok(1));
    wait_until(|| !lock(&cache.shared.entries)[KEY].refreshing);
    assert_eq!(value_of(&cache, KEY), Some(1));
}

#[test]
fn the_first_read_of_a_key_returns_its_error_and_caches_nothing() {
    let cache: SwrCache<&'static str, u32> = SwrCache::new(LONG_TTL);
    assert_eq!(
        cache.get(KEY, || Err(DOWN.to_string())),
        Err(DOWN.to_string())
    );
    assert_eq!(value_of(&cache, KEY), None);
}

#[test]
fn keys_are_cached_apart() {
    let cache = SwrCache::new(LONG_TTL);
    assert_eq!(cache.get(KEY, || Ok::<_, String>(1)), Ok(1));
    assert_eq!(cache.get(SECOND_KEY, || Ok::<_, String>(2)), Ok(2));
    assert_eq!(cache.get(KEY, || Ok::<_, String>(9)), Ok(1));
}

#[test]
fn refresh_computes_now_and_stores() {
    let cache = SwrCache::new(LONG_TTL);
    cache.get(KEY, || Ok::<_, String>(1)).unwrap();
    assert_eq!(cache.refresh(KEY, || Ok::<_, String>(5)), Ok(5));
    assert_eq!(cache.get(KEY, || Ok::<_, String>(9)), Ok(5));
}

#[test]
fn clear_forgets_values_and_drops_a_refresh_running_meanwhile() {
    let cache = SwrCache::new(Duration::ZERO);
    cache.get(KEY, || Ok::<_, String>(1)).unwrap();
    cache
        .get(KEY, || {
            std::thread::sleep(Duration::from_millis(200));
            Ok::<_, String>(2)
        })
        .unwrap();
    cache.clear();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(value_of(&cache, KEY), None);
}

#[test]
fn concurrent_first_reads_of_a_key_compute_once() {
    let cache = SwrCache::new(LONG_TTL);
    let calls = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let (cache, calls) = (cache.clone(), Arc::clone(&calls));
            scope.spawn(move || {
                cache
                    .get(KEY, move || {
                        std::thread::sleep(Duration::from_millis(100));
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(1)
                    })
                    .unwrap();
            });
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
