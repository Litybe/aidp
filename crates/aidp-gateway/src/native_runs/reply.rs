//! Reads and validates the `reply.md` a native stage agent writes.

use crate::models::enums::native_reply_error::NativeReplyError;
use crate::models::native_reply_envelope::NativeReplyEnvelope;
use aidp_core::models::enums::agent_role::AgentRole;
use serde_json::Value;
use std::path::Path;

/// Name of the reply file a stage agent writes.
pub const REPLY_FILE: &str = "reply.md";
const FENCE_OPEN: &str = "```json";
const FENCE_CLOSE: &str = "```";
const SIGNALS_KEY: &str = "signals";
/// Signal key set by reviewers when findings block.
pub const HAS_BLOCKING_KEY: &str = "has_blocking_findings";
/// Signal key counting checks a reviewer could not run.
pub const UNEVALUATED_KEY: &str = "unevaluated_checks";
const BACKSLASH: char = '\\';
const JSON_ESCAPES: &str = "\"\\/bfnrtu";
const URL_MARKER: &str = "://";
/// The control characters a JSON escape such as `\f` decodes to, with the letter that wrote it.
const DECODED_ESCAPES: [(char, char); 5] = [
    ('\u{8}', 'b'),
    ('\u{c}', 'f'),
    ('\n', 'n'),
    ('\r', 'r'),
    ('\t', 't'),
];
const PATH_SEPARATORS: [char; 2] = ['/', '\\'];
const REPLY_SCHEMA: &str = include_str!("../../../../schema/agent-reply.v4.schema.json");

/// Reads `<stage_dir>/reply.md` and validates it for `role` and `declared_outputs`.
pub fn read_reply(
    stage_dir: &Path,
    role: AgentRole,
    declared_outputs: &[String],
) -> Result<NativeReplyEnvelope, NativeReplyError> {
    let reply_path = stage_dir.join(REPLY_FILE);
    if !reply_path.is_file() {
        return Err(NativeReplyError::Missing(reply_path));
    }
    let text = aidp_core::text::read_text_lenient(&reply_path)
        .map_err(|e| NativeReplyError::Unreadable(e.to_string()))?;
    let value = extract_json(&text)?;
    validate_schema(&value)?;
    if role == AgentRole::Reviewer {
        validate_reviewer_signals(&value)?;
    }
    let mut envelope: NativeReplyEnvelope = serde_json::from_value(value)
        .map_err(|e| NativeReplyError::SchemaViolation(e.to_string()))?;
    name_outputs_as_declared(&mut envelope, declared_outputs);
    adopt_outputs_found_on_disk(&mut envelope, declared_outputs, stage_dir);
    restore_escaped_backslashes(&mut envelope);
    validate_outputs(&envelope, declared_outputs)?;
    Ok(envelope)
}

/// Output key used to compare a reply key with a declared output: file stem,
/// lowercase, `_` and `-` equivalent (`memory_context` == `memory-context.md`).
fn normalize_output_key(key: &str) -> String {
    Path::new(key)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(key)
        .to_lowercase()
        .replace('_', "-")
}

/// A reply key answers for a declared output when both normalize to the same name
/// (`study.md` for `study`, `memory_context` for `memory-context`). The declared name
/// stays the canonical key. Agents rarely echo the declared name byte for byte;
/// rejecting such a reply cost a second session that only renamed the key.
fn name_outputs_as_declared(envelope: &mut NativeReplyEnvelope, declared_outputs: &[String]) {
    for name in declared_outputs {
        if envelope.outputs.contains_key(name) {
            continue;
        }
        let wanted = normalize_output_key(name);
        let file_key = envelope
            .outputs
            .keys()
            .find(|key| normalize_output_key(key) == wanted)
            .cloned();
        if let Some(value) = file_key.and_then(|key| envelope.outputs.remove(&key)) {
            envelope.outputs.insert(name.clone(), value);
        }
    }
}

/// A declared output the envelope omits is accepted when its file sits non-empty in the stage dir.
fn adopt_outputs_found_on_disk(
    envelope: &mut NativeReplyEnvelope,
    declared_outputs: &[String],
    stage_dir: &Path,
) {
    for name in declared_outputs {
        if envelope.outputs.contains_key(name) {
            continue;
        }
        let wanted = normalize_output_key(name);
        let Ok(entries) = std::fs::read_dir(stage_dir) else {
            return;
        };
        let found = entries.flatten().map(|e| e.path()).find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f != REPLY_FILE && normalize_output_key(f) == wanted)
                && std::fs::metadata(path).is_ok_and(|m| m.len() > 0)
        });
        if let Some(path) = found {
            tracing::warn!(
                output = %name,
                file = %path.display(),
                "declared output absent from the reply envelope; accepted from the stage directory"
            );
            envelope
                .outputs
                .insert(name.clone(), path.to_string_lossy().into_owned());
        }
    }
}

/// `"C:\\a\forge"` is valid JSON whose `\f` decodes to a form feed, so the path no longer exists.
/// A missing output path gets its decoded escapes back as backslashes when that path exists.
fn restore_escaped_backslashes(envelope: &mut NativeReplyEnvelope) {
    for value in envelope.outputs.values_mut() {
        if !looks_like_path(value) || Path::new(value.as_str()).exists() {
            continue;
        }
        let mut restored = String::with_capacity(value.len());
        for c in value.chars() {
            match DECODED_ESCAPES.iter().find(|(decoded, _)| *decoded == c) {
                Some((_, letter)) => {
                    restored.push(BACKSLASH);
                    restored.push(*letter);
                }
                None => restored.push(c),
            }
        }
        if restored != *value && Path::new(&restored).exists() {
            *value = restored;
        }
    }
}

/// Builds the relaunch text that tells the agent what to fix in `reply.md`.
pub fn repair_instruction(err: &NativeReplyError) -> String {
    format!(
        "Your previous reply was rejected: {err}.\nRewrite {REPLY_FILE} in your stage directory as a single JSON envelope \
         (status, summary, outputs, signals) that fixes this. Do not redo the stage work."
    )
}

/// Parses the whole text as JSON, else the first fenced json block.
/// A text only rejected for its Windows paths is read again with those backslashes escaped.
fn extract_json(text: &str) -> Result<Value, NativeReplyError> {
    parse_json(text)
        .or_else(|first_error| parse_json(&escape_stray_backslashes(text)).map_err(|_| first_error))
}

/// Doubles every backslash that does not start a JSON escape, as in `C:\\Users\\a`.
fn escape_stray_backslashes(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(current) = chars.next() {
        escaped.push(current);
        if current != BACKSLASH {
            continue;
        }
        match chars.peek() {
            Some(next) if JSON_ESCAPES.contains(*next) => {
                escaped.push(*next);
                chars.next();
            }
            _ => escaped.push(BACKSLASH),
        }
    }
    escaped
}

fn parse_json(text: &str) -> Result<Value, NativeReplyError> {
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(value) => Ok(value),
        Err(whole_err) => match fenced_block(text) {
            Some(block) => serde_json::from_str(block)
                .map_err(|e| NativeReplyError::InvalidJson(e.to_string())),
            None => Err(NativeReplyError::InvalidJson(whole_err.to_string())),
        },
    }
}

/// Returns the body of the first json fenced block.
fn fenced_block(text: &str) -> Option<&str> {
    let start = text.find(FENCE_OPEN)? + FENCE_OPEN.len();
    let rest = &text[start..];
    let end = rest.find(FENCE_CLOSE)?;
    Some(rest[..end].trim())
}

/// Drops the relative `$id`, which the validator cannot resolve without a base URL.
fn strip_relative_id(schema: &mut Value) {
    if let Some(object) = schema.as_object_mut() {
        object.remove("$id");
    }
}

fn validate_schema(value: &Value) -> Result<(), NativeReplyError> {
    let mut schema: Value = serde_json::from_str(REPLY_SCHEMA)
        .map_err(|e| NativeReplyError::SchemaViolation(e.to_string()))?;
    strip_relative_id(&mut schema);
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .map_err(|e| NativeReplyError::SchemaViolation(e.to_string()))?;
    if let Err(errors) = compiled.validate(value) {
        let messages: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        return Err(NativeReplyError::SchemaViolation(messages.join("; ")));
    }
    Ok(())
}

/// Mirrors the reviewer reply schema: both signals are required and typed.
fn validate_reviewer_signals(value: &Value) -> Result<(), NativeReplyError> {
    let signals = value.get(SIGNALS_KEY);
    let has_blocking = signals
        .and_then(|s| s.get(HAS_BLOCKING_KEY))
        .is_some_and(Value::is_boolean);
    let unevaluated = signals
        .and_then(|s| s.get(UNEVALUATED_KEY))
        .is_some_and(Value::is_u64);
    if has_blocking && unevaluated {
        Ok(())
    } else {
        Err(NativeReplyError::ReviewerSignals)
    }
}

fn validate_outputs(
    envelope: &NativeReplyEnvelope,
    declared_outputs: &[String],
) -> Result<(), NativeReplyError> {
    for name in declared_outputs {
        let path = envelope
            .outputs
            .get(name)
            .ok_or_else(|| NativeReplyError::MissingOutput(name.clone()))?;
        if looks_like_path(path) && !Path::new(path).exists() {
            return Err(NativeReplyError::OutputNotOnDisk {
                name: name.clone(),
                path: path.clone(),
            });
        }
    }
    Ok(())
}

/// Output values may be external ids such as a PR url; only filesystem paths are checked.
fn looks_like_path(value: &str) -> bool {
    !value.contains(URL_MARKER)
        && (Path::new(value).is_absolute() || value.contains(PATH_SEPARATORS))
}

#[cfg(test)]
#[path = "../../tests/unit/reply_tests.rs"]
mod tests;
