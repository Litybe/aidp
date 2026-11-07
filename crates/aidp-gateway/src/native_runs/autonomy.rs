//! Autonomous runs: a question becomes an assumption, a second block fails the stage.

use crate::models::constants::autonomy::{
    ANSWER_FIELD_SEPARATOR, ANSWER_LINE_PREFIX, ANSWER_LINE_SEPARATOR, NO_OPTION_CHOICE,
    RETRY_HEADING, RETRY_INTRO,
};
use crate::models::native_question::NativeQuestion;
use crate::models::native_reply_envelope::NativeReplyEnvelope;
use aidp_core::models::assumption::Assumption;
use aidp_core::models::constants::autonomy::ESCALATIONS_SIGNAL_KEY;
use aidp_core::models::enums::auto_answer_reason::AutoAnswerReason;
use aidp_core::models::escalation::Escalation;
use aidp_core::run::EnvelopeStatus;

/// True when the reply asks for a human: `blocked`, or questions still open.
pub fn needs_auto_answer(status: EnvelopeStatus, questions: &[NativeQuestion]) -> bool {
    status == EnvelopeStatus::Blocked || !questions.is_empty()
}

/// One assumption per question: the recommended option, else the first, else a prudent default.
pub fn auto_answers(questions: &[NativeQuestion]) -> Vec<Assumption> {
    questions.iter().map(auto_answer).collect()
}

fn auto_answer(question: &NativeQuestion) -> Assumption {
    let recommended = question
        .recommended
        .as_ref()
        .filter(|choice| !choice.is_empty());
    let (choice, reason) = match (recommended, question.options.first()) {
        (Some(choice), _) => (choice.clone(), AutoAnswerReason::Recommended),
        (None, Some(first)) => (first.clone(), AutoAnswerReason::FirstOption),
        (None, None) => (NO_OPTION_CHOICE.to_string(), AutoAnswerReason::NoOption),
    };
    let rejected_alternative = question.options.iter().find(|option| **option != choice);
    Assumption {
        stage: String::new(),
        choice,
        reason: reason.as_str().to_string(),
        rejected_alternative: rejected_alternative.cloned(),
        auto_answered: true,
        question_id: Some(question.id.clone()),
        question: Some(question.question.clone()),
    }
}

/// The text appended to the original prompt to relaunch a blocked stage.
pub fn retry_note(answers: &[Assumption]) -> String {
    let lines: String = answers
        .iter()
        .map(|answer| {
            format!(
                "{ANSWER_LINE_PREFIX}{}{ANSWER_FIELD_SEPARATOR}{}{ANSWER_LINE_SEPARATOR}{}\n",
                answer.question_id.as_deref().unwrap_or_default(),
                answer.question.as_deref().unwrap_or_default(),
                answer.choice
            )
        })
        .collect();
    format!("{RETRY_HEADING}{RETRY_INTRO}{lines}")
}

/// What a blocked first reply journals: its own declared assumptions, then the auto-answers.
pub fn blocked_journal(
    stage: &str,
    envelope: &NativeReplyEnvelope,
    answers: Vec<Assumption>,
) -> Vec<Assumption> {
    let mut journal = reply_assumptions(stage, envelope);
    journal.extend(answers);
    journal
}

/// The assumptions a reply journaled, tagged with `stage`.
pub fn reply_assumptions(stage: &str, envelope: &NativeReplyEnvelope) -> Vec<Assumption> {
    envelope
        .assumptions
        .iter()
        .map(|entry| Assumption {
            stage: stage.to_string(),
            auto_answered: false,
            ..entry.clone()
        })
        .collect()
}

/// The escalations a reply reported in `signals`, tagged with `stage`; malformed entries are skipped.
pub fn reply_escalations(stage: &str, envelope: &NativeReplyEnvelope) -> Vec<Escalation> {
    let entries = envelope
        .signals
        .as_ref()
        .and_then(|signals| signals.get(ESCALATIONS_SIGNAL_KEY))
        .and_then(|value| value.as_array());
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| serde_json::from_value::<Escalation>(entry.clone()).ok())
        .map(|entry| Escalation {
            stage: stage.to_string(),
            ..entry
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/autonomy_tests.rs"]
mod tests;
