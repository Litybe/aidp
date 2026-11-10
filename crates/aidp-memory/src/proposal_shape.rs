//! Folds the undocumented proposal shapes agents write into the documented one.

use crate::models::constants::limits::{TEXT_MAX_CHARS, TITLE_MAX_CHARS};
use crate::models::constants::proposal_fields::{
    DETAIL_FIELDS, FIELD_KIND, FIELD_LEGACY_TYPE, FIELD_PROPOSALS, FIELD_TEXT, FIELD_TITLE,
    LEGACY_DEFAULT_KIND, LEGACY_MARKERS, LEGACY_TYPE_KINDS, TEXT_ALIASES, TITLE_ALIASES,
    TRUNCATION_MARK,
};
use crate::models::enums::MemoryKind;
use serde_json::{Map, Value};

/// Rewrites every undocumented proposal; a documented one is left to the strict parser.
pub(crate) fn canonicalize_document(document: &mut Value) {
    let Some(proposals) = document
        .get_mut(FIELD_PROPOSALS)
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for proposal in proposals.iter_mut().filter_map(Value::as_object_mut) {
        if LEGACY_MARKERS
            .iter()
            .any(|field| proposal.contains_key(*field))
        {
            canonicalize(proposal);
        }
    }
}

fn canonicalize(proposal: &mut Map<String, Value>) {
    let kind = kind_of(proposal);
    let title = take_first(proposal, &TITLE_ALIASES);
    let text = take_first(proposal, &TEXT_ALIASES);
    let details = take_details(proposal);
    proposal.insert(FIELD_KIND.to_string(), Value::String(kind));
    if let Some(title) = title {
        proposal.insert(
            FIELD_TITLE.to_string(),
            Value::String(shortened(title, TITLE_MAX_CHARS)),
        );
    }
    if let Some(text) = text {
        proposal.insert(
            FIELD_TEXT.to_string(),
            Value::String(shortened(text + &details, TEXT_MAX_CHARS)),
        );
    }
}

fn kind_of(proposal: &mut Map<String, Value>) -> String {
    let declared = proposal.remove(FIELD_LEGACY_TYPE);
    if let Some(Value::String(kind)) = proposal.remove(FIELD_KIND) {
        return kind;
    }
    let declared = declared
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default();
    if MemoryKind::parse(declared).is_ok() {
        return declared.to_string();
    }
    LEGACY_TYPE_KINDS
        .iter()
        .find(|(legacy, _)| *legacy == declared)
        .map_or(LEGACY_DEFAULT_KIND, |(_, kind)| *kind)
        .as_str()
        .to_string()
}

/// Removes every alias and returns the first non-empty value.
fn take_first(proposal: &mut Map<String, Value>, aliases: &[&str]) -> Option<String> {
    let values: Vec<Value> = aliases
        .iter()
        .filter_map(|field| proposal.remove(*field))
        .collect();
    values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

fn take_details(proposal: &mut Map<String, Value>) -> String {
    DETAIL_FIELDS
        .iter()
        .filter_map(|(field, label)| {
            let value = proposal.remove(*field)?;
            let detail = value.as_str()?.trim();
            (!detail.is_empty()).then(|| format!("\n{label}: {detail}"))
        })
        .collect()
}

fn shortened(value: String, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value;
    }
    let kept: String = value
        .chars()
        .take(limit - TRUNCATION_MARK.chars().count())
        .collect();
    format!("{}{TRUNCATION_MARK}", kept.trim_end())
}
