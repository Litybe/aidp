//! Tests for how [`refresh`] reports each generated index file it writes,
//! leaves unchanged, or fails to write.

use super::*;
use crate::model::Source;
use aidp_testkit::TempWorkspace;

#[path = "test_support.rs"]
#[allow(clippy::duplicate_mod)]
mod test_support;

/// Key of the ticket that pins the boards' `updated` stamp, since an empty board stamps `Utc::now()`.
const TICKET_KEY: &str = "LOC-1";

/// Service folder the round-trip test creates under `TODO/`.
const SERVICE_DIR: &str = "svc";

/// Service index file generated for [`SERVICE_DIR`].
const SERVICE_INDEX_FILE: &str = "_svc.md";

/// Fragment every warning about the board must carry, on every OS.
const BOARD_WARNING_FRAGMENT: &str = "TODO/_TODO.md";

#[test]
fn refresh_reports_a_board_it_cannot_write() {
    let workspace = TempWorkspace::new("refresh-unwritable-board");
    let vault = workspace.path();
    let board_path = vault.join(TODO_DIR).join(BOARD_FILE);
    fs::create_dir_all(&board_path).unwrap();

    let report = refresh(vault, &[]);

    assert_eq!(
        report.failed.len(),
        1,
        "only the board should fail: {report:?}"
    );
    assert_eq!(report.failed[0].path, board_path);
    let warnings = report.warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(BOARD_WARNING_FRAGMENT)),
        "a warning should name the board: {warnings:?}"
    );
}

#[test]
fn refresh_reports_written_then_unchanged() {
    let workspace = TempWorkspace::new("refresh-written-unchanged");
    let vault = workspace.path();
    let service_dir = vault.join(TODO_DIR).join(SERVICE_DIR);
    fs::create_dir_all(&service_dir).unwrap();
    let mut expected = vec![
        vault.join(TODO_DIR).join(BOARD_FILE),
        service_dir.join(SERVICE_INDEX_FILE),
    ];
    expected.sort();

    let tickets = [test_support::ticket(TICKET_KEY, Source::Local)];

    let mut first = refresh(vault, &tickets);
    first.written.sort();
    assert_eq!(first.written, expected, "first pass writes every index");
    assert!(first.unchanged.is_empty() && first.failed.is_empty());

    let mut second = refresh(vault, &tickets);
    second.unchanged.sort();
    assert_eq!(
        second.unchanged, expected,
        "second pass leaves every index untouched"
    );
    assert!(second.written.is_empty() && second.failed.is_empty());
    assert!(second.warnings().is_empty());
}
