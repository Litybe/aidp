pub const PROMPT_TOOL_FLAG: &str = "--permission-prompt-tool";
pub const STDIO: &str = "stdio";
pub const QUESTION: &str = "Allow this Claude action?";
pub const HEADER: &str = "Claude permission";
pub const ALLOW: &str = "Allow";
pub const DENY: &str = "Deny";
pub const INVALID_ANSWER: &str =
    "Claude permission reply requires an Allow or Deny answer to the pending request";
pub const INVALID_REQUEST: &str = "Claude permission request is no longer pending";
