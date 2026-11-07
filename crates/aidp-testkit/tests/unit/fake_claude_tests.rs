use super::*;
use std::time::{Duration, Instant};

/// Stem every test installs the fake under.
const CLAUDE_STEM: &str = "claude";
/// Filler size asserted by the output test.
const EXPECTED_STDOUT_BYTES: usize = 4096;
/// Sleep long enough that the process is still running when probed.
const LONG_SLEEP_SECS: u64 = 30;
/// Exit code asserted by the exit-code test.
const CUSTOM_EXIT_CODE: i32 = 3;
/// Upper bound for the PID file to appear after spawn.
const PID_WAIT: Duration = Duration::from_secs(10);
/// Pause between two reads of the PID file.
const PID_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Threads installing and running fakes at once in the concurrency test.
const CONCURRENT_THREADS: usize = 8;
/// Installs run by each thread of the concurrency test.
const INSTALLS_PER_THREAD: usize = 10;

/// Poll until the fake records its first PID, panicking after `PID_WAIT`.
fn wait_for_first_pid(fake: &FakeClaude) -> u32 {
    let deadline = Instant::now() + PID_WAIT;
    loop {
        if let Some(pid) = fake.pids().first() {
            return *pid;
        }
        assert!(
            Instant::now() < deadline,
            "fake claude never recorded a pid"
        );
        std::thread::sleep(PID_POLL_INTERVAL);
    }
}

#[test]
fn it_install_writes_stdout_and_records_one_pid() {
    let ws = TempWorkspace::new("fake_claude_stdout");
    let config = FakeClaudeConfig {
        stdout_bytes: EXPECTED_STDOUT_BYTES,
        ..FakeClaudeConfig::default()
    };
    let fake = FakeClaude::install(&ws, CLAUDE_STEM, &config);
    let output = spawn_retrying_busy(
        std::process::Command::new(fake.exe_path()).stdout(std::process::Stdio::piped()),
    )
    .and_then(std::process::Child::wait_with_output)
    .expect("run fake claude");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.len() >= EXPECTED_STDOUT_BYTES);
    assert_eq!(fake.pids().len(), 1);
    assert_eq!(fake.dir(), ws.path());
}

#[test]
fn it_sleeping_fake_is_alive_until_killed() {
    let ws = TempWorkspace::new("fake_claude_sleep");
    let config = FakeClaudeConfig {
        sleep_secs: LONG_SLEEP_SECS,
        ..FakeClaudeConfig::default()
    };
    let fake = FakeClaude::install(&ws, CLAUDE_STEM, &config);
    let mut child = spawn_retrying_busy(&mut std::process::Command::new(fake.exe_path()))
        .expect("spawn fake claude");
    let pid = wait_for_first_pid(&fake);
    assert_eq!(pid, child.id());
    assert!(pid_alive(pid), "sleeping fake claude should be alive");
    child.kill().expect("kill fake claude");
    child.wait().expect("wait fake claude");
    assert!(!pid_alive(pid), "killed fake claude should be gone");
}

#[test]
fn it_configured_exit_code_is_returned() {
    let ws = TempWorkspace::new("fake_claude_exit");
    let config = FakeClaudeConfig {
        exit_code: CUSTOM_EXIT_CODE,
        ..FakeClaudeConfig::default()
    };
    let fake = FakeClaude::install(&ws, CLAUDE_STEM, &config);
    let status = spawn_retrying_busy(&mut std::process::Command::new(fake.exe_path()))
        .and_then(|mut child| child.wait())
        .expect("run fake claude");
    assert_eq!(status.code(), Some(CUSTOM_EXIT_CODE));
}

#[test]
fn it_concurrent_installs_all_run() {
    let handles: Vec<_> = (0..CONCURRENT_THREADS)
        .map(|thread_index| {
            std::thread::spawn(move || {
                for install_index in 0..INSTALLS_PER_THREAD {
                    let ws = TempWorkspace::new(&format!(
                        "fake_claude_concurrent_{thread_index}_{install_index}"
                    ));
                    let fake = FakeClaude::install(&ws, CLAUDE_STEM, &FakeClaudeConfig::default());
                    let status =
                        spawn_retrying_busy(&mut std::process::Command::new(fake.exe_path()))
                            .and_then(|mut child| child.wait())
                            .expect("run concurrently installed fake claude");
                    assert_eq!(status.code(), Some(0));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("concurrent install thread");
    }
}
