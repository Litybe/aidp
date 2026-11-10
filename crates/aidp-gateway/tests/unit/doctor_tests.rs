use super::*;
use crate::models::enums::{DoctorAction, DoctorCategory, DoctorTrigger};
use crate::models::DoctorActionOutcome;
use aidp_testkit::TempWorkspace;

const ID: &str = "GH-5/implement@1";
const CAUSE: &str = "the report stage answered needs_review while the PR exists";
const REPORT: &str = "Diagnostic : faux échec.";
const AT: &str = "2026-11-09T12:00:00Z";

fn incident() -> DoctorIncident {
    DoctorIncident {
        candidate: DoctorCandidate {
            id: ID.to_string(),
            key: "GH-5".to_string(),
            ticket: "GH-5".to_string(),
            trigger: DoctorTrigger::FlowFailed,
            step: Some("implement".to_string()),
            iteration: Some(1),
            run_id: None,
            project_id: None,
            detail: "implement failed".to_string(),
            live: false,
            activity_ms: 0,
        },
        status: DoctorIncidentStatus::Fixed,
        detected_at: AT.to_string(),
        updated_at: AT.to_string(),
        investigation_run_id: None,
        investigated_ms: None,
        bundle: None,
        diagnosis: Some(DoctorDiagnosis {
            category: DoctorCategory::FalseFailure,
            cause: CAUSE.to_string(),
            ..DoctorDiagnosis::default()
        }),
        outcome: Some(DoctorActionOutcome {
            action: DoctorAction::RetryStep,
            idempotency_key: store::action_key(ID, 1),
            ok: true,
            detail: "retried `implement`".to_string(),
            at: AT.to_string(),
        }),
        actions: Vec::new(),
        next_attempt_ms: None,
        pr_url: None,
        note: Some("retry_step : retried `implement`".to_string()),
        commented: false,
        shared_pattern: None,
        affected: Vec::new(),
        rearmed_ms: None,
    }
}

#[test]
fn the_comment_states_cause_action_and_result() {
    let body = comment_body(&incident(), Some(REPORT));
    assert!(body.contains(&format!("Cause : {CAUSE}")), "{body}");
    assert!(
        body.contains("Action : `retry_step` : retried `implement`"),
        "{body}"
    );
    assert!(body.contains("Résultat : fixed"), "{body}");
    assert!(body.trim_end().ends_with(REPORT), "{body}");
}

#[test]
fn the_comment_falls_back_on_what_detection_saw() {
    let mut bare = incident();
    bare.diagnosis = None;
    bare.outcome = None;
    let body = comment_body(&bare, None);
    assert!(body.contains("Cause : implement failed"), "{body}");
    assert!(body.contains("Action : -"), "{body}");
}

#[test]
fn the_diagnosis_is_read_from_a_stage_directory_and_unknown_categories_are_other() {
    let root = TempWorkspace::new("doctor-read-diagnosis");
    let stage = root.child("diagnose");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(
        stage.join(DIAGNOSIS_FILE),
        r#"{"category":"cosmic_rays","cause":"x","confidence":0.8,"action":"retry_step","params":{"step":"implement"},"needs_human":false,"evidence":["a"],"summary":"s"}"#,
    )
    .unwrap();
    let diagnosis = read_diagnosis(root.path()).unwrap();
    assert_eq!(diagnosis.category, DoctorCategory::Other);
    assert_eq!(diagnosis.action, DoctorAction::RetryStep.as_str());
    assert!(read_diagnosis(&root.child("nothing")).is_err());
}

#[test]
fn a_diagnosis_written_with_a_byte_order_mark_is_read() {
    let root = TempWorkspace::new("doctor-read-diagnosis-bom");
    let stage = root.child("diagnose");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(
        stage.join(DIAGNOSIS_FILE),
        "\u{feff}{\"category\":\"other\",\"cause\":\"x\",\"confidence\":0.8,\"action\":\"none\",\"params\":{},\"needs_human\":false,\"evidence\":[],\"summary\":\"s\"}",
    )
    .unwrap();
    assert_eq!(read_diagnosis(root.path()).unwrap().cause, "x");
}

#[test]
fn a_bundle_name_is_file_safe() {
    assert_eq!(bundle_name("GH-5/run:abc@1"), "GH-5_run_abc_1");
}
