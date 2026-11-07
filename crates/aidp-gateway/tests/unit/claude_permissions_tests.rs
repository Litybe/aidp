use super::*;
use crate::models::constants::{claude_permissions, launch_policy, session_profile};
use crate::models::enums::PermissionMode;
use crate::recording_writer::RecordingWriter;

const REQUEST: &str = include_str!("../fixtures/claude_permissions/request.json");
const REQUEST_ID: &str = "permission-one";
const UNKNOWN_ID: &str = "unknown-permission";
const SHELL_PREFIX: &str = "git push";
const EXPECTED_SHELL_RULE: &str = "Bash(git push:*)";
const EXPECTED_RTK_RULE: &str = "Bash(rtk git push:*)";
const INVALID_PREFIX: &str = "git push)*";
const MODE: &str = "default";
const RESPONSE_FIELD: &str = "response";
const UPDATED_INPUT_FIELD: &str = "updatedInput";
const BEHAVIOR_FIELD: &str = "behavior";
const ALLOW_BEHAVIOR: &str = "allow";
const PERMISSION_MODE_FLAG: &str = "--permission-mode";

/// A launch spec in `mode` under `policy`, denying [`SHELL_PREFIX`] with ambient MCP disabled.
fn restricted_spec(mode: PermissionMode, policy: PermissionPolicy) -> AgentSpec {
    AgentSpec {
        extra_args: claude_launch_tail(mode.as_str(), None, None, None, None),
        permission_policy: policy,
        strict_mcp_config: Some(true),
        denied_shell_commands: vec![SHELL_PREFIX.to_string()],
        ..Default::default()
    }
}

/// The value following `flag` in `args`.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let position = args.iter().position(|argument| argument == flag)?;
    args.get(position + 1).map(String::as_str)
}

/// A headless launch runs `dontAsk`, keeps its deny rules and strict MCP, relays no prompt.
#[test]
fn headless_sessions_run_dont_ask_without_a_prompt_relay() {
    let spec = restricted_spec(PermissionMode::HEADLESS_DEFAULT, PermissionPolicy::Headless);
    let args = claude_session_args(&spec).unwrap();
    assert_eq!(
        flag_value(&args, PERMISSION_MODE_FLAG),
        Some(PermissionMode::DontAsk.as_str())
    );
    assert!(
        !args.contains(&claude_permissions::PROMPT_TOOL_FLAG.to_string()),
        "{args:?}"
    );
    for expected in [
        EXPECTED_SHELL_RULE,
        EXPECTED_RTK_RULE,
        launch_policy::STRICT_MCP_FLAG,
    ] {
        assert!(args.contains(&expected.to_string()), "{args:?}");
    }
}

/// A headless launch also denies, plain and through rtk, the aidp verbs that publish, merge or deploy.
#[test]
fn headless_sessions_deny_the_aidp_publishing_verbs() {
    let spec = restricted_spec(PermissionMode::HEADLESS_DEFAULT, PermissionPolicy::Headless);
    let args = claude_session_args(&spec).unwrap();
    for command in launch_policy::HEADLESS_DENIED_COMMANDS {
        for expected in [
            format!("Bash({command}:*)"),
            format!("Bash(rtk {command}:*)"),
        ] {
            assert!(args.contains(&expected), "{expected} missing from {args:?}");
        }
    }
}

/// A launch that may ask gets only the host's deny rules.
#[test]
fn asking_sessions_keep_only_the_host_deny_rules() {
    let spec = restricted_spec(PermissionMode::Default, PermissionPolicy::Default);
    let args = claude_session_args(&spec).unwrap();
    for command in launch_policy::HEADLESS_DENIED_COMMANDS {
        assert!(
            !args.contains(&format!("Bash({command}:*)")),
            "{command} denied in {args:?}"
        );
    }
}

/// A launch that may ask still relays its prompts over stdio, deny rules included.
#[test]
fn default_sessions_still_relay_prompts_over_stdio() {
    let spec = restricted_spec(PermissionMode::Default, PermissionPolicy::Default);
    let args = claude_session_args(&spec).unwrap();
    assert_eq!(flag_value(&args, PERMISSION_MODE_FLAG), Some(MODE));
    assert_eq!(
        flag_value(&args, claude_permissions::PROMPT_TOOL_FLAG),
        Some(claude_permissions::STDIO)
    );
    assert!(args.contains(&EXPECTED_SHELL_RULE.to_string()), "{args:?}");
}

#[test]
fn restrictions_preserve_existing_denies_and_disable_ambient_mcp() {
    let spec = AgentSpec {
        extra_args: vec![
            MODE.to_string(),
            session_profile::DISALLOWED_TOOLS_FLAG.to_string(),
            session_profile::ASK_USER_QUESTION_TOOL.to_string(),
        ],
        strict_mcp_config: Some(true),
        mcp_servers: Some(Vec::new()),
        denied_shell_commands: vec![SHELL_PREFIX.to_string()],
        ..Default::default()
    };
    let args = restricted_claude_args(&spec).unwrap();
    assert_eq!(
        args.iter()
            .filter(|argument| *argument == session_profile::DISALLOWED_TOOLS_FLAG)
            .count(),
        1
    );
    for expected in [
        session_profile::ASK_USER_QUESTION_TOOL,
        EXPECTED_SHELL_RULE,
        EXPECTED_RTK_RULE,
        launch_policy::STRICT_MCP_FLAG,
    ] {
        assert!(args.contains(&expected.to_string()));
    }
}

#[test]
fn permission_pattern_injection_is_rejected_before_launch() {
    let spec = AgentSpec {
        denied_shell_commands: vec![INVALID_PREFIX.to_string()],
        ..Default::default()
    };
    assert!(
        matches!(restricted_claude_args(&spec), Err(AgentError::Config(message)) if message == launch_policy::INVALID_DENY_RULE)
    );
}

#[test]
fn permission_control_preserves_original_tool_input_and_rejects_stale_ids() {
    let request: Value = serde_json::from_str(REQUEST).unwrap();
    let input = request[FIELD_REQUEST][FIELD_INPUT].clone();
    let writer = RecordingWriter::default();
    let permissions: PermissionCell = Arc::new(Mutex::new(BTreeMap::from([(
        REQUEST_ID.to_string(),
        input.clone(),
    )])));
    let control = ClaudeControl {
        child: Arc::new(Mutex::new(None)),
        stdin: Arc::new(Mutex::new(Some(Box::new(writer.clone())))),
        cancelled: Arc::new(AtomicBool::new(false)),
        exit: Arc::new(ExitSignal::new()),
        permissions: permissions.clone(),
    };
    assert!(
        matches!(control.reply_permission(UNKNOWN_ID, true), Err(AgentError::Config(message)) if message == claude_permissions::INVALID_REQUEST)
    );
    assert!(writer.0.lock().unwrap().is_empty());
    control.reply_permission(REQUEST_ID, true).unwrap();
    assert!(
        matches!(control.reply_permission(REQUEST_ID, true), Err(AgentError::Config(message)) if message == claude_permissions::INVALID_REQUEST)
    );
    let bytes = writer.0.lock().unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(response[RESPONSE_FIELD][FIELD_REQUEST_ID], REQUEST_ID);
    assert_eq!(
        response[RESPONSE_FIELD][RESPONSE_FIELD][BEHAVIOR_FIELD],
        ALLOW_BEHAVIOR
    );
    assert_eq!(
        response[RESPONSE_FIELD][RESPONSE_FIELD][UPDATED_INPUT_FIELD],
        input
    );
    assert!(permissions.lock().unwrap().is_empty());
}

fn persona_argument(args: &[String]) -> Option<&String> {
    let position = args
        .iter()
        .position(|argument| argument == "--append-system-prompt")?;
    args.get(position + 1)
}

#[test]
fn headless_sessions_get_the_denial_guidance_after_their_persona() {
    let mut spec = restricted_spec(PermissionMode::Default, PermissionPolicy::Headless);
    let args = restricted_claude_args(&spec).unwrap();
    assert_eq!(
        persona_argument(&args).map(String::as_str),
        Some(launch_policy::HEADLESS_DENIAL_GUIDANCE)
    );
    spec.persona = Some("Be terse.".to_string());
    let args = restricted_claude_args(&spec).unwrap();
    let persona = persona_argument(&args).unwrap();
    assert!(persona.starts_with("Be terse."), "{persona}");
    assert!(persona.ends_with(launch_policy::HEADLESS_DENIAL_GUIDANCE));
}

#[test]
fn non_headless_sessions_get_no_denial_guidance() {
    let mut spec = restricted_spec(PermissionMode::Default, PermissionPolicy::Stdio);
    assert!(persona_argument(&restricted_claude_args(&spec).unwrap()).is_none());
    spec.persona = Some("Be terse.".to_string());
    let args = restricted_claude_args(&spec).unwrap();
    assert_eq!(
        persona_argument(&args).map(String::as_str),
        Some("Be terse.")
    );
}
