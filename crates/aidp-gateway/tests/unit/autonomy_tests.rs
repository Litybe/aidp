use super::*;

const ESCALATION_ID: &str = "d1";
const ESCALATION_DECISION: &str = "drop the endpoint";
const ESCALATION_REASON: &str = "breaking change";
const QUESTION_ID: &str = "q1";
const OPTION_A: &str = "option-a";
const OPTION_B: &str = "option-b";

const QUESTION_TEXT: &str = "which store?";
const STAGE_NAME: &str = "plan";
const RECOMMENDED_VALUE: &str = "recommended";
const FIRST_OPTION_VALUE: &str = "first-option";
const NO_OPTION_VALUE: &str = "no-option";
const EXPECTED_LINE: &str = "- q1 which store? -> option-a
";

fn question(options: &[&str], recommended: Option<&str>) -> NativeQuestion {
    NativeQuestion {
        id: QUESTION_ID.to_string(),
        question: QUESTION_TEXT.to_string(),
        options: options.iter().map(|option| option.to_string()).collect(),
        recommended: recommended.map(str::to_string),
        ..NativeQuestion::default()
    }
}

#[test]
fn blocked_status_needs_an_answer() {
    assert!(needs_auto_answer(EnvelopeStatus::Blocked, &[]));
}

#[test]
fn open_questions_need_an_answer() {
    assert!(needs_auto_answer(
        EnvelopeStatus::NeedsReview,
        &[question(&[OPTION_A], None)]
    ));
}

#[test]
fn clean_completed_reply_needs_no_answer() {
    assert!(!needs_auto_answer(EnvelopeStatus::Completed, &[]));
}

#[test]
fn recommended_option_wins_over_the_first() {
    let answers = auto_answers(&[question(&[OPTION_A, OPTION_B], Some(OPTION_B))]);
    assert_eq!(answers[0].choice, OPTION_B);
    assert_eq!(answers[0].rejected_alternative.as_deref(), Some(OPTION_A));
    assert!(answers[0].auto_answered);
}

#[test]
fn first_option_is_taken_without_recommendation() {
    let answers = auto_answers(&[question(&[OPTION_A, OPTION_B], None)]);
    assert_eq!(answers[0].choice, OPTION_A);
    assert_eq!(answers[0].rejected_alternative.as_deref(), Some(OPTION_B));
}

#[test]
fn question_without_options_still_gets_an_answer() {
    let answers = auto_answers(&[question(&[], None)]);
    assert_eq!(answers[0].choice, NO_OPTION_CHOICE);
    assert_eq!(answers[0].rejected_alternative, None);
}

#[test]
fn retry_note_lists_every_answer() {
    let answers = auto_answers(&[question(&[OPTION_A], None), question(&[OPTION_B], None)]);
    let note = retry_note(&answers);
    assert!(note.starts_with(RETRY_HEADING));
    assert!(note.contains(OPTION_A));
    assert!(note.contains(OPTION_B));
}

/// The retry line carries the question id and text before the choice.
#[test]
fn retry_line_names_the_question_and_its_choice() {
    let note = retry_note(&auto_answers(&[question(&[OPTION_A], None)]));
    assert!(note.ends_with(EXPECTED_LINE));
}

/// The auto-answer copies the question id and text into the assumption.
#[test]
fn question_is_copied_into_the_assumption() {
    let answers = auto_answers(&[question(&[OPTION_A], None)]);
    assert_eq!(answers[0].question_id.as_deref(), Some(QUESTION_ID));
    assert_eq!(answers[0].question.as_deref(), Some(QUESTION_TEXT));
}

/// Each way of answering has its own stable kebab-case reason.
#[test]
fn reasons_use_the_enum_wire_values() {
    let recommended = auto_answers(&[question(&[OPTION_A], Some(OPTION_A))]);
    let first = auto_answers(&[question(&[OPTION_A], None)]);
    let none = auto_answers(&[question(&[], None)]);
    assert_eq!(recommended[0].reason, RECOMMENDED_VALUE);
    assert_eq!(first[0].reason, FIRST_OPTION_VALUE);
    assert_eq!(none[0].reason, NO_OPTION_VALUE);
}

/// A reply's own assumptions are tagged with the stage and never marked auto-answered.
#[test]
fn reply_assumptions_are_tagged_with_the_stage() {
    let envelope = NativeReplyEnvelope {
        status: String::new(),
        summary: String::new(),
        outputs: Default::default(),
        signals: None,
        follow_ups: Vec::new(),
        assumptions: vec![Assumption {
            stage: String::new(),
            choice: OPTION_A.to_string(),
            reason: OPTION_B.to_string(),
            rejected_alternative: None,
            auto_answered: true,
            question_id: None,
            question: None,
        }],
        ask_user_questions: Vec::new(),
    };
    let journal = reply_assumptions(STAGE_NAME, &envelope);
    assert_eq!(journal[0].stage, STAGE_NAME);
    assert!(!journal[0].auto_answered);
    let combined = blocked_journal(STAGE_NAME, &envelope, auto_answers(&[question(&[], None)]));
    assert_eq!(combined.len(), 2);
    assert!(!combined[0].auto_answered);
    assert!(combined[1].auto_answered);
}

fn envelope_with_signals(signals: Option<serde_json::Value>) -> NativeReplyEnvelope {
    NativeReplyEnvelope {
        status: String::new(),
        summary: String::new(),
        outputs: Default::default(),
        signals,
        follow_ups: Vec::new(),
        assumptions: Vec::new(),
        ask_user_questions: Vec::new(),
    }
}

/// A reply's escalations are read from `signals` and tagged with the stage.
#[test]
fn reply_escalations_are_read_and_tagged_with_the_stage() {
    let signals = serde_json::json!({
        ESCALATIONS_SIGNAL_KEY: [{"id": ESCALATION_ID, "decision": ESCALATION_DECISION, "reason": ESCALATION_REASON}]
    });
    let escalations = reply_escalations(STAGE_NAME, &envelope_with_signals(Some(signals)));
    assert_eq!(
        escalations,
        vec![Escalation {
            stage: STAGE_NAME.to_string(),
            id: ESCALATION_ID.to_string(),
            decision: ESCALATION_DECISION.to_string(),
            reason: ESCALATION_REASON.to_string(),
        }]
    );
}

/// A reply without signals or without the key reports no escalation.
#[test]
fn reply_escalations_are_empty_when_absent() {
    assert!(reply_escalations(STAGE_NAME, &envelope_with_signals(None)).is_empty());
    let other = serde_json::json!({ QUESTION_ID: [] });
    assert!(reply_escalations(STAGE_NAME, &envelope_with_signals(Some(other))).is_empty());
}

/// Malformed escalation entries are skipped, valid ones kept, and nothing errors.
#[test]
fn reply_escalations_skip_malformed_entries() {
    let signals = serde_json::json!({
        ESCALATIONS_SIGNAL_KEY: [QUESTION_TEXT, {"id": ESCALATION_ID}, {"id": ESCALATION_ID, "decision": ESCALATION_DECISION, "reason": ESCALATION_REASON}]
    });
    let escalations = reply_escalations(STAGE_NAME, &envelope_with_signals(Some(signals)));
    assert_eq!(escalations.len(), 1);
    assert_eq!(escalations[0].id, ESCALATION_ID);
    let not_an_array = serde_json::json!({ ESCALATIONS_SIGNAL_KEY: QUESTION_TEXT });
    assert!(reply_escalations(STAGE_NAME, &envelope_with_signals(Some(not_an_array))).is_empty());
}
