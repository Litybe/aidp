//! Non-regression test for the `aidp mock` subcommand.
//!
//! `aidp_integrations::mock::serve` was rewritten as a thin wrapper around the new
//! race-free `serve_on` function. This test pins the observable behaviour of the
//! binary: the stderr banner (including the two-space padding before `data=`) and
//! the HTTP response of the state route.
//!
//! The internal bind error type changed from `Box<dyn Error>` to `io::Error`, so only
//! the error prefix `bind <addr>: ` is stable and the localized OS message is not asserted.
//! This test focuses on the banner and the responses, which are unchanged.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Guard to ensure the child process is killed even if a test panics.
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child(&mut self) -> &mut Child {
        self.child.as_mut().expect("child already taken")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn mock_subcommand_prints_its_banner_and_answers_a_state_read() {
    // Step 1: Bind an ephemeral port, drop the listener to free it for the child.
    // This is the one place drop/rebind is unavoidable because the port must be passed
    // to a separate process on the command line. We handle the residual flake by
    // retrying the connect for a bounded time in step 3.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port for mock subcommand test");
    let port = listener
        .local_addr()
        .expect("read port from ephemeral listener")
        .port();
    drop(listener);

    // Step 2: Resolve the fixtures directory and spawn the aidp binary.
    let data_dir = aidp_testkit::fixtures::mocks_dir();

    let mut child_guard = ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_aidp"))
            .arg("mock")
            .arg("--addr")
            .arg(format!("127.0.0.1:{}", port))
            .arg("--data")
            .arg(&data_dir)
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn aidp mock subcommand"),
    );

    let stderr = child_guard.child().stderr.take().expect("piped stderr");
    let reader = BufReader::new(stderr);

    // Step 3: Read stderr lines on a background thread until we get the banner or timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    let reader_thread = thread::spawn(move || {
        let mut reader = reader;
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok() && !line.is_empty() {
            tx.send(line.clone()).ok();
            line.clear();
        }
    });

    // Build the expected banner string.
    let expected_banner = format!(
        "aidp mock listening on http://127.0.0.1:{}  data={}",
        port,
        data_dir.display()
    );

    // Wait for the banner with a ~10s deadline.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut banner_found = false;
    loop {
        match rx.try_recv() {
            Ok(line) => {
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                if trimmed == expected_banner {
                    banner_found = true;
                    break;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if Instant::now() > deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break;
            }
        }
    }

    assert!(
        banner_found,
        "did not find banner within 10s. expected:\n  {}\n",
        expected_banner
    );

    // Step 4: Open a TcpStream and write a raw HTTP request for the state route.
    // Retry for a bounded time to handle the drop/rebind flake.
    let addr = format!("127.0.0.1:{}", port);
    let max_attempts = 20;
    let mut stream_result = Err("no stream yet".to_string());

    for attempt in 0..max_attempts {
        match TcpStream::connect(&addr) {
            Ok(s) => {
                stream_result = Ok(s);
                break;
            }
            Err(_e) if attempt < max_attempts - 1 => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                stream_result = Err(format!(
                    "failed to connect after {} attempts: {}",
                    max_attempts, e
                ));
            }
        }
    }

    let mut stream = stream_result.expect("connect to mock server within bounded time");

    // Make a raw HTTP GET request on the state route, expect 200.
    let request_with_auth = "GET /__state HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream
        .write_all(request_with_auth.as_bytes())
        .expect("write HTTP request with auth");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read response with auth");
    let response_str = String::from_utf8_lossy(&response);

    assert!(
        response_str.contains("200"),
        "expected 200 in response with auth, got:\n{}",
        response_str
    );
    assert!(
        response_str.contains("gh_prs"),
        "expected the state document in the response body, got:\n{}",
        response_str
    );

    // Step 5: The ChildGuard Drop impl will kill the child process and join the thread,
    // even if an assertion above has failed. This guarantees no orphaned aidp mock process.
    drop(child_guard);

    // Give the thread a moment to finish.
    thread::sleep(Duration::from_millis(100));
    reader_thread.join().ok();
}
