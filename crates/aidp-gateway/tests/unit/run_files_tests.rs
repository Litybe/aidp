//! Run directory files: listing order and kinds, MIME types, traversal and link refusal, size cap.

use super::{
    content_type_of, kind_of, list_files, read_file, FileKind, RunFilesReply, MAX_ENTRIES,
};
use aidp_testkit::TempWorkspace;
use std::fs;

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn fixture() -> TempWorkspace {
    let ws = TempWorkspace::new("run-files");
    fs::create_dir_all(ws.child("run/plan")).unwrap();
    fs::create_dir_all(ws.child("run/implement/shots")).unwrap();
    fs::write(ws.child("run/run-state.json"), "{}").unwrap();
    fs::write(ws.child("run/plan/plan.md"), "# plan").unwrap();
    fs::write(ws.child("run/plan/reply.json"), "{\"ok\":true}").unwrap();
    fs::write(ws.child("run/implement/shots/a.png"), PNG_MAGIC).unwrap();
    fs::write(ws.child("run/implement/loop.gif"), b"GIF89a").unwrap();
    fs::write(ws.child("run/implement/notes.bin"), b"x").unwrap();
    fs::write(ws.child("secret.txt"), "outside").unwrap();
    ws
}

#[test]
fn listing_is_recursive_sorted_and_typed() {
    let ws = fixture();
    let files = list_files(&ws.child("run"));
    let rows: Vec<(&str, &str, FileKind)> = files
        .iter()
        .map(|f| (f.stage.as_str(), f.path.as_str(), f.kind))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("", "run-state.json", FileKind::Json),
            ("implement", "implement/loop.gif", FileKind::Gif),
            ("implement", "implement/notes.bin", FileKind::Other),
            ("implement", "implement/shots/a.png", FileKind::Png),
            ("plan", "plan/plan.md", FileKind::Md),
            ("plan", "plan/reply.json", FileKind::Json),
        ]
    );
    assert_eq!(files[3].size, PNG_MAGIC.len() as u64);
}

#[test]
fn listing_is_capped() {
    let ws = TempWorkspace::new("run-files-cap");
    fs::create_dir_all(ws.child("run/s")).unwrap();
    for index in 0..MAX_ENTRIES + 20 {
        fs::write(ws.child(&format!("run/s/f{index:05}.md")), "x").unwrap();
    }
    assert_eq!(list_files(&ws.child("run")).len(), MAX_ENTRIES);
}

#[test]
fn kinds_and_mime_types() {
    assert_eq!(kind_of("A.PNG"), FileKind::Png);
    assert_eq!(kind_of("noext"), FileKind::Other);
    assert_eq!(content_type_of("a.png"), "image/png");
    assert_eq!(content_type_of("a.gif"), "image/gif");
    assert_eq!(content_type_of("a.json"), "application/json");
    assert_eq!(content_type_of("a.md"), "text/markdown; charset=utf-8");
    assert_eq!(content_type_of("a.bin"), "application/octet-stream");
}

#[test]
fn a_png_is_served_with_its_type_and_no_cache() {
    let ws = fixture();
    let RunFilesReply::File(file) = read_file(&ws.child("run"), "implement/shots/a.png") else {
        panic!("expected a file");
    };
    assert_eq!(file.content_type, "image/png");
    assert_eq!(file.cache_control, "no-cache");
    assert_eq!(file.bytes, PNG_MAGIC);
}

#[test]
fn traversal_and_missing_files_are_404() {
    let ws = fixture();
    let run = ws.child("run");
    for path in [
        "../secret.txt",
        "plan/../../secret.txt",
        "%2e%2e/secret.txt",
        "plan%2f..%2f..%2fsecret.txt",
        "..%5csecret.txt",
        "C:/Windows/win.ini",
        "plan",
        "plan/missing.md",
        "",
    ] {
        assert!(
            matches!(read_file(&run, path), RunFilesReply::Error(404, _)),
            "{path}"
        );
    }
}

#[test]
fn a_file_over_the_cap_is_413() {
    let ws = fixture();
    let big = fs::File::create(ws.child("run/plan/big.bin")).unwrap();
    big.set_len(super::MAX_FILE_BYTES + 1).unwrap();
    assert!(matches!(
        read_file(&ws.child("run"), "plan/big.bin"),
        RunFilesReply::Error(413, _)
    ));
}

#[test]
fn symbolic_links_are_refused() {
    let ws = fixture();
    let link = ws.child("run/plan/link.txt");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(ws.child("secret.txt"), &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(ws.child("secret.txt"), &link).is_ok();
    if !made {
        eprintln!("symlink not permitted here, skipped");
        return;
    }
    assert!(matches!(
        read_file(&ws.child("run"), "plan/link.txt"),
        RunFilesReply::Error(404, _)
    ));
    assert!(list_files(&ws.child("run"))
        .iter()
        .all(|f| f.path != "plan/link.txt"));
}
