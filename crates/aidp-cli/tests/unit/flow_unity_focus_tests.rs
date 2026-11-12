use super::*;

const FOCUSED_OUTPUT: &str = "PID=7 title=PixelHood\nWIN32DIALOG title='Modified' buttons=[Reload|Ignore] action=BM_CLICK Reload\nFOCUS=True\n";
const NOT_FOCUSED_OUTPUT: &str = "PID=7 title=PixelHood\nFOCUS=False\n";

#[test]
fn a_focused_editor_is_reported_with_its_dialogs() {
    let report = parse(FOCUSED_OUTPUT);
    assert!(report.focused);
    assert!(report.editor_found);
    assert_eq!(report.dialogs.len(), 1);
}

#[test]
fn a_refused_foreground_is_not_focused() {
    let report = parse(NOT_FOCUSED_OUTPUT);
    assert!(!report.focused);
    assert!(report.editor_found);
}

#[test]
fn an_absent_editor_is_not_found() {
    let report = parse("ABSENT\n");
    assert!(!report.focused);
    assert!(!report.editor_found);
}

#[test]
fn a_window_less_editor_is_not_found() {
    let report = parse("PID=7 title=\nNOWINDOW\n");
    assert!(!report.editor_found);
}
