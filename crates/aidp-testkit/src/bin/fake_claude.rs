//! Configurable stand-in for the `claude` executable, used by daemon-level tests.
//!
//! Reads `<own path with extension conf>`, plain `key=value` lines, all optional.
//! A missing file means exit 0 at once. The steps run in a fixed order: record the
//! PID, write stdout filler, read one stdin line, sleep, exit with the configured code.

use aidp_testkit::fake_claude::{
    CONFIG_EXTENSION, KEY_EXIT_CODE, KEY_PID_FILE, KEY_READ_STDIN_LINE, KEY_SLEEP_SECS,
    KEY_STDOUT_BYTES, KEY_VALUE_SEPARATOR, MAX_FILLER_LINE_BYTES,
};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

/// Byte repeated to build a filler line; never valid JSON on its own.
const FILLER_BYTE: u8 = b'x';
/// Byte ending every filler line.
const NEWLINE_BYTE: u8 = b'\n';

/// Behaviour read from the sidecar config file.
#[derive(Default)]
struct Behaviour {
    pid_file: Option<String>,
    stdout_bytes: usize,
    read_stdin_line: bool,
    sleep_secs: u64,
    exit_code: i32,
}

fn main() -> std::process::ExitCode {
    let own_path = std::env::current_exe().expect("fake_claude: resolve own path");
    let Some(behaviour) = read_behaviour(&own_path.with_extension(CONFIG_EXTENSION)) else {
        return std::process::ExitCode::SUCCESS;
    };
    if let Some(pid_file) = &behaviour.pid_file {
        append_pid(Path::new(pid_file));
    }
    write_filler(behaviour.stdout_bytes);
    if behaviour.read_stdin_line {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
    }
    if behaviour.sleep_secs > 0 {
        std::thread::sleep(Duration::from_secs(behaviour.sleep_secs));
    }
    std::process::ExitCode::from(u8::try_from(behaviour.exit_code).unwrap_or(1))
}

/// Parse the config file; `None` when it does not exist or cannot be read.
fn read_behaviour(config_path: &Path) -> Option<Behaviour> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let mut behaviour = Behaviour::default();
    for line in content.lines() {
        let Some((key, value)) = line.split_once(KEY_VALUE_SEPARATOR) else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            KEY_PID_FILE => behaviour.pid_file = Some(value.to_string()),
            KEY_STDOUT_BYTES => behaviour.stdout_bytes = value.parse().unwrap_or_default(),
            KEY_READ_STDIN_LINE => behaviour.read_stdin_line = value.parse().unwrap_or_default(),
            KEY_SLEEP_SECS => behaviour.sleep_secs = value.parse().unwrap_or_default(),
            KEY_EXIT_CODE => behaviour.exit_code = value.parse().unwrap_or_default(),
            _ => {}
        }
    }
    Some(behaviour)
}

/// Append this process's PID as one line, so several launches are all recorded.
fn append_pid(pid_file: &Path) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(pid_file)
        .expect("fake_claude: open pid file");
    writeln!(file, "{}", std::process::id()).expect("fake_claude: write pid");
}

/// Write `total_bytes` of newline-terminated filler lines to stdout, then flush.
fn write_filler(total_bytes: usize) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut remaining = total_bytes;
    while remaining > 0 {
        let line_len = remaining.min(MAX_FILLER_LINE_BYTES);
        let mut line = vec![FILLER_BYTE; line_len];
        line[line_len - 1] = NEWLINE_BYTE;
        if out.write_all(&line).is_err() {
            return;
        }
        remaining -= line_len;
    }
    let _ = out.flush();
}
