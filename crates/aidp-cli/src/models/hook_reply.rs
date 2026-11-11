use serde::Serialize;

/// What a hook prints on stdout: context Claude Code adds to the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookReply {
    pub hook_specific_output: HookSpecificOutput,
}

/// The event name and the context text of a [`HookReply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    pub hook_event_name: String,
    pub additional_context: String,
}

impl HookReply {
    pub fn new(event: &str, context: String) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: event.to_string(),
                additional_context: context,
            },
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/hook_reply_tests.rs"]
mod tests;
