//! Integration test: exercises the mock server's entire routing table
//! without opening a socket.
//!
//! The `aidp_integrations::mock::handle` function is a pure router taking
//! method, URL, body and state — and returning (status, JSON).
//! This test drives it directly on an empty state, asserting that every routed
//! arm (every declared route plus the _ 404 fallback) behaves as the code
//! implements. Coverage happens without spawning a thread or a listener.

use serde_json::json;
use std::sync::Mutex;

use aidp_integrations::mock::{handle, State};
use tiny_http::Method;

/// An empty mock state wrapped in a Mutex for the tests.
fn state() -> Mutex<State> {
    Mutex::new(State::default())
}

#[test]
fn route_table_covers_every_arm() {
    let state = state();

    // 1. GET /__state -> 200 with no pull request yet.
    let (code, body) = handle(&Method::Get, "/__state", "", &state).expect("__state must succeed");
    assert_eq!(code, 200, "__state returns 200");
    assert_eq!(body.get("gh_prs"), Some(&json!({})), "gh_prs starts empty");

    // 2. POST /github/repos/o/r/pulls -> 201, number == 1.
    let req_body = json!({"title": "p"}).to_string();
    let (code, body) = handle(&Method::Post, "/github/repos/o/r/pulls", &req_body, &state)
        .expect("POST /github/repos/o/r/pulls must succeed");
    assert_eq!(code, 201, "POST /github/repos/o/r/pulls returns 201");
    assert_eq!(body.get("number"), Some(&json!(1)), "first PR number is 1");

    // 3. GET /github/repos/o/r/pulls/1 -> 200.
    let (code, _body) = handle(&Method::Get, "/github/repos/o/r/pulls/1", "", &state)
        .expect("GET /github/repos/o/r/pulls/1 must succeed");
    assert_eq!(code, 200, "GET /github/repos/o/r/pulls/1 returns 200");

    let (code, body) = handle(&Method::Get, "/__state", "", &state).expect("__state must succeed");
    assert_eq!(code, 200, "__state returns 200 after writes");
    assert!(
        body.pointer("/gh_prs/o~1r~11").is_some(),
        "the created pull request is recorded"
    );

    // 4. POST /__reset -> 200 with {"ok": true}.
    let (code, body) = handle(&Method::Post, "/__reset", "", &state).expect("__reset must succeed");
    assert_eq!(code, 200, "__reset returns 200");
    assert_eq!(body, json!({"ok": true}), "__reset body must be ok: true");
}

#[test]
fn state_and_reset_answer_without_authorization() {
    let state = state();

    let (code, _body) =
        handle(&Method::Get, "/__state", "", &state).expect("__state without auth must succeed");
    assert_eq!(code, 200, "/__state requires no Authorization");

    let (code, _body) =
        handle(&Method::Post, "/__reset", "", &state).expect("__reset without auth must succeed");
    assert_eq!(code, 200, "/__reset requires no Authorization");
}

#[test]
fn unknown_path_falls_through_to_404() {
    let state = state();

    // GET /nope/deep/path -> 404, exact error body.
    let (code, body) = handle(&Method::Get, "/nope/deep/path", "", &state)
        .expect("unknown path must still return (code, body)");
    assert_eq!(code, 404, "unknown path returns 404");
    assert_eq!(
        body,
        json!({"error": "no route", "url": "/nope/deep/path"}),
        "404 body is exact"
    );

    // Query strings are included in the url field for debug, but routing splits on ?.
    let (code, body) =
        handle(&Method::Get, "/nope?x=1", "", &state).expect("unknown path with query string");
    assert_eq!(code, 404);
    assert_eq!(
        body.get("url"),
        Some(&json!("/nope?x=1")),
        "url field carries the raw request url including query"
    );
}

#[test]
fn poisoned_state_does_not_kill_the_double() {
    use std::sync::Arc;

    let state = Arc::new(Mutex::new(State::default()));

    // Deliberately poison the mutex by locking and panicking on another thread.
    let state_clone = Arc::clone(&state);
    let poison_thread = std::thread::spawn(move || {
        let _lock = state_clone.lock();
        panic!("intentional poison");
    });

    // Silence the panic output.
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let _ = poison_thread.join();
    std::panic::set_hook(old_hook);

    // The mutex is poisoned, but handle uses unwrap_or_else(|e| e.into_inner()),
    // which recovers the guard even after a panic. Verify it still works.
    let (code, body) = handle(&Method::Get, "/__state", "", &state)
        .expect("__state after poisoning must still succeed");
    assert_eq!(code, 200, "after poison: __state still returns 200");
    assert!(
        body.is_object(),
        "after poison: __state still returns a valid state object"
    );
}
