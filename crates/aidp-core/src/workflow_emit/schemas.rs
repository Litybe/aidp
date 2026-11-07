/// Reviewer stages must populate `signals.has_blocking_findings` (the correction loop reads
/// `signals`, not the artifact file).
pub(super) fn reply_schema_reviewer_json() -> String {
    let mut v = reply_schema_value();
    v["required"] = serde_json::json!(["status", "summary", "signals"]);
    v["properties"]["signals"] = serde_json::json!({
        "type": "object",
        "required": ["has_blocking_findings", "unevaluated_checks"],
        "additionalProperties": true,
        "properties": {
            "has_blocking_findings": { "type": "boolean" },
            "unevaluated_checks": { "type": "integer", "minimum": 0 },
            "has_harness_gap": { "type": "boolean" }
        },
        "description": "Booleans/counters the orchestrator branches on. has_blocking_findings MUST mirror findings.json's has_blocking_findings — the correction loop reads it from here. unevaluated_checks MUST mirror the count of checks[] entries with status \"skip\" — 0 means every declared control actually ran. has_harness_gap mirrors unevaluated_checks > 0 and is what a stage's on_harness_gap: pause reads."
    });
    serde_json::to_string(&v).unwrap()
}

/// `schema` (a serialized reply schema) with a required `skills_loaded`, the proof a stage
/// declaring skills must return.
pub(super) fn with_skills_loaded(schema: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(schema).unwrap();
    v["properties"]["skills_loaded"] = serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "required": ["name", "applied_rule"],
            "properties": {
                "name": { "type": "string" },
                "applied_rule": { "type": "string", "minLength": 1 }
            }
        },
        "description": "One entry per skill the prompt's Skills section declares: the skill name, and one rule of it actually applied in this stage."
    });
    if let Some(req) = v["required"].as_array_mut() {
        req.push(serde_json::json!("skills_loaded"));
    }
    serde_json::to_string(&v).unwrap()
}

/// The v7 reply envelope schema (agent-reply.v4 + an optional free-form
/// `signals` object the orchestrator branches on).
pub(super) fn reply_schema_json() -> String {
    serde_json::to_string(&reply_schema_value()).unwrap()
}

pub(super) fn reply_schema_value() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["status", "summary"],
        "properties": {
            "status": { "enum": ["completed", "blocked", "needs_review", "failed"] },
            "summary": { "type": "string" },
            "outputs": { "type": "object", "additionalProperties": { "type": "string" } },
            "signals": {
                "type": "object",
                "additionalProperties": true,
                "description": "Booleans/counters the orchestrator branches on, e.g. has_blocking_findings."
            },
            "follow_ups": { "type": "array", "items": { "type": "string" } },
            "ask_user_questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "answer"],
                    "properties": {
                        "id": { "type": "string" },
                        "question": { "type": "string" },
                        "answer": { "type": "string" }
                    }
                }
            },
            "scope_violations": { "type": "array", "items": { "type": "string" } },
            "metrics": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "tool_calls": { "type": "integer" },
                    "duration_ms": { "type": "integer" },
                    "tokens_used": { "type": "integer" }
                }
            }
        }
    })
}
