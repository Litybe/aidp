use super::*;
use aidp_testkit::TempWorkspace;
use serde::Deserialize;
use std::io::BufWriter;
use std::sync::Arc;
use std::thread;

#[test]
fn dir_size_counts_bytes_and_files_exactly() {
    let _ws = TempWorkspace::new("store-dirsize");
    let dir = _ws.path();
    fs::write(dir.join("a.txt"), "hello").unwrap(); // 5 bytes
    fs::write(dir.join("b.txt"), "world!").unwrap(); // 6 bytes

    let (bytes, files) = dir_size(dir);
    assert_eq!(bytes, 11);
    assert_eq!(files, 2);
}

#[test]
fn dir_size_on_missing_dir_is_zero() {
    let _ws = TempWorkspace::new("store-dirsize-missing");
    let missing = _ws.path().join("does-not-exist");
    assert_eq!(dir_size(&missing), (0, 0));
}

const WRITES_PER_THREAD: usize = 1_000;
const WRITER_THREADS: usize = 2;
const STATE_FILE: &str = "state.json";
const LOG_FILE: &str = "log.jsonl";
const TMP_SUFFIX: &str = ".tmp";
const LARGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const WRITER_CAPACITY: usize = 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Record {
    seq: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Padded {
    seq: u64,
    pad: String,
}

/// One JSONL line for sequence `seq`, space-padded so every line has the same length.
fn fixed_width_line(seq: u64) -> String {
    format!("{{\"seq\":{seq:>10}}}\n")
}

fn tmp_residue(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(TMP_SUFFIX))
        .collect()
}

#[test]
fn concurrent_writes_to_one_path_keep_the_file_and_leave_no_tmp() {
    let workspace = TempWorkspace::new("store-concurrent");
    let path = Arc::new(workspace.child(STATE_FILE));
    let writers: Vec<_> = (0..WRITER_THREADS)
        .map(|thread_index| {
            let path = Arc::clone(&path);
            thread::spawn(move || {
                for write_index in 0..WRITES_PER_THREAD {
                    let payload = Record {
                        seq: (thread_index * WRITES_PER_THREAD + write_index) as u64,
                    };
                    let _ = write_json(&path, &payload);
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }

    let content = fs::read_to_string(path.as_ref()).expect("state file must still exist");
    let _: Record = serde_json::from_str(&content).expect("state file must deserialize");
    assert_eq!(tmp_residue(workspace.path()), Vec::<String>::new());
}

#[test]
fn large_log_is_read_within_the_scan_ceiling_and_reports_truncation() {
    let workspace = TempWorkspace::new("store-large-jsonl");
    let path = workspace.child(LOG_FILE);
    let line_len = fixed_width_line(0).len() as u64;
    let line_count = LARGE_FILE_BYTES / line_len;
    {
        let file = fs::File::create(&path).unwrap();
        let mut writer = BufWriter::with_capacity(WRITER_CAPACITY, file);
        for seq in 0..line_count {
            writer.write_all(fixed_width_line(seq).as_bytes()).unwrap();
        }
        writer.flush().unwrap();
    }

    let JsonlRead { items, truncated } = read_jsonl_bounded::<Record>(&path);

    assert!(truncated);
    assert!(!items.is_empty());
    assert!(items.len() as u64 * line_len <= MAX_SCAN_BYTES);
    assert_eq!(items.last().unwrap().seq, line_count - 1);
    assert!(items.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1));
}

#[test]
fn small_log_is_read_whole_in_order_skipping_bad_lines() {
    let workspace = TempWorkspace::new("store-small-jsonl");
    let path = workspace.child(LOG_FILE);
    let content = "{\"seq\":1}\n\n   \nnot json\n{\"seq\":2}\r\n{\"seq\":3}";
    fs::write(&path, content).unwrap();

    let read = read_jsonl_bounded::<Record>(&path);

    assert!(!read.truncated);
    let seqs: Vec<u64> = read.items.iter().map(|record| record.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    assert_eq!(read_jsonl::<Record>(&path), read.items);
}

#[test]
fn missing_log_yields_nothing_and_is_not_truncated() {
    let workspace = TempWorkspace::new("store-missing-jsonl");
    let read = read_jsonl_bounded::<Record>(&workspace.child(LOG_FILE));
    assert!(read.items.is_empty());
    assert!(!read.truncated);
}

#[test]
fn line_straddling_a_chunk_boundary_is_reconstructed() {
    let workspace = TempWorkspace::new("store-straddle-jsonl");
    let path = workspace.child(LOG_FILE);
    let tail = serde_json::to_string(&Padded {
        seq: 2,
        pad: "b".repeat(CHUNK_BYTES / 2),
    })
    .unwrap();
    let straddling = Padded {
        seq: 1,
        pad: "a".repeat(CHUNK_BYTES),
    };
    let first = serde_json::to_string(&Padded {
        seq: 0,
        pad: String::new(),
    })
    .unwrap();
    let body = format!(
        "{first}\n{}\n{tail}\n",
        serde_json::to_string(&straddling).unwrap()
    );
    fs::write(&path, body).unwrap();

    let read = read_jsonl_bounded::<Padded>(&path);

    assert!(!read.truncated);
    assert_eq!(read.items.len(), 3);
    assert_eq!(read.items[1], straddling);
    let seqs: Vec<u64> = read.items.iter().map(|item| item.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
}

mod append_failure {
    use crate::store::{append_jsonl, APPEND_FAILED_MESSAGE};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::Level;

    const LEVEL_WARN: &str = "WARN";
    const BLOCKER_FILE: &str = "regular-file";
    const TARGET_FILE: &str = "log.jsonl";
    const APPENDED_VALUE: u32 = 7;

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

    /// Appending below a regular file fails and the failure is logged as a warning.
    #[test]
    fn append_jsonl_failure_is_logged() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join(BLOCKER_FILE);
        std::fs::write(&blocker, "x").unwrap();
        let target = blocker.join(TARGET_FILE);

        let output = captured_output(|| append_jsonl(&target, &APPENDED_VALUE));

        assert!(output.contains(LEVEL_WARN), "{output}");
        assert!(output.contains(APPEND_FAILED_MESSAGE), "{output}");
    }
}
