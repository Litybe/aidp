use super::PrFixRequest;
use crate::models::enums::origin_kind::OriginKind;
use crate::models::pr_fix_summary::PrFixSummary;
use aidp_integrations::models::pr_mark::PrMark;

const REQUEST_MINIMAL: &str = r#"{"repo":"acme/api","number":7,"threads":["T1"]}"#;
const SUMMARY_FULL: &str = r#"{"fixed":["T1"],"needs_reviewer":["T2"],"not_relevant":["T3"],"pushed_sha":"abc","assumptions":["a"]}"#;
const SUMMARY_EMPTY: &str = "{}";
const MARK_LEGACY: &str = r#"{"seen_sha":"abc","seen_at":"now","ignored":false}"#;
const MARK_WITH_THREADS: &str =
    r#"{"threads":{"T1":{"fix_session":"s-1","at":"2026-10-08T10:00:00Z"}}}"#;
const THREADS_KEY: &str = "threads";
const THREAD_ID: &str = "T1";
const FIX_SESSION: &str = "s-1";
const FIX_AT: &str = "2026-10-08T10:00:00Z";
const WIRE_UI: &str = "\"ui\"";
const WIRE_CI: &str = "\"ci\"";

#[test]
fn request_defaults_optional_fields() {
    let request: PrFixRequest = serde_json::from_str(REQUEST_MINIMAL).unwrap();
    assert_eq!(request.threads, vec!["T1".to_string()]);
    assert!(request.points.is_empty());
    assert_eq!(request.note, None);
    assert_eq!(request.model, None);
    assert_eq!(request.effort, None);
}

#[test]
fn summary_parses_full_and_empty() {
    let full: PrFixSummary = serde_json::from_str(SUMMARY_FULL).unwrap();
    assert_eq!(full.fixed, vec!["T1".to_string()]);
    assert_eq!(full.pushed_sha.as_deref(), Some("abc"));
    let empty: PrFixSummary = serde_json::from_str(SUMMARY_EMPTY).unwrap();
    assert_eq!(empty, PrFixSummary::default());
}

#[test]
fn origin_pr_fix_round_trips_and_others_keep_names() {
    let wire = serde_json::to_string(&OriginKind::PrFix).unwrap();
    assert_eq!(wire, format!("\"{}\"", OriginKind::PrFix.as_str()));
    assert_eq!(
        serde_json::from_str::<OriginKind>(&wire).unwrap(),
        OriginKind::PrFix
    );
    assert_eq!(serde_json::to_string(&OriginKind::Ui).unwrap(), WIRE_UI);
    assert_eq!(serde_json::to_string(&OriginKind::Ci).unwrap(), WIRE_CI);
}

#[test]
fn legacy_pr_mark_still_loads() {
    let mark: PrMark = serde_json::from_str(MARK_LEGACY).unwrap();
    assert_eq!(mark.fix_session, None);
    assert_eq!(mark.fix_at, None);
    assert!(mark.threads.is_empty());
}

#[test]
fn default_pr_mark_has_no_thread_marks_and_omits_them_when_serialized() {
    let mark = PrMark::default();
    assert!(mark.threads.is_empty());
    let wire = serde_json::to_string(&mark).unwrap();
    assert!(!wire.contains(THREADS_KEY));
}

#[test]
fn pr_mark_thread_marks_round_trip() {
    let mark: PrMark = serde_json::from_str(MARK_WITH_THREADS).unwrap();
    assert_eq!(mark.threads[THREAD_ID].fix_session, FIX_SESSION);
    assert_eq!(mark.threads[THREAD_ID].at, FIX_AT);
    let wire = serde_json::to_string(&mark).unwrap();
    assert_eq!(serde_json::from_str::<PrMark>(&wire).unwrap(), mark);
}
