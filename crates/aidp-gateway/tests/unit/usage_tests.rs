use crate::models::enums::SessionStatus;
use crate::sessions::{Origin, SessionRecord};
use crate::usage::aggregate;

const MULTIBYTE_STARTED_AT: &str = "日本語テストです";
const SHORT_STARTED_AT: &str = "court";
const GROUP_BY_DAY: &str = "day";
const PROJECT_ID: &str = "p1";
const PROJECT_NAME: &str = "Project One";
const CWD: &str = "/repo";
const ENGINE: &str = "v7";
const PROMPT: &str = "do stuff";
const PERMISSION_MODE: &str = "default";
const ORIGIN_KIND: &str = "cli";

/// A finished session whose only varying field is its `started_at` timestamp.
fn session_started_at(started_at: &str) -> SessionRecord {
    SessionRecord {
        mcp_config_id: None,
        provider: None,
        provider_session_id: None,
        id: started_at.to_string(),
        project_id: PROJECT_ID.to_string(),
        project_name: PROJECT_NAME.to_string(),
        cwd: CWD.to_string(),
        engine: ENGINE.to_string(),
        template_id: None,
        target: None,
        inputs: None,
        prompt: PROMPT.to_string(),
        permission_mode: PERMISSION_MODE.to_string(),
        effective_permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        status: SessionStatus::Done,
        started_at: started_at.to_string(),
        ended_at: None,
        start_sha: None,
        exit_code: None,
        claude_session_id: None,
        resumed_from: None,
        cost_usd: None,
        turns: 0,
        last_text: None,
        pending_question: None,
        origin: Origin::new(ORIGIN_KIND),
        automation_id: None,
        automation_run_id: None,
        workflow_run_id: None,
        event_count: 0,
        auto_close: false,
        usage: None,
        title: None,
        pinned: false,
        archived: false,
        unread: false,
        context_from: None,
        chain_depth: None,
        workspace: None,
        warnings: Vec::new(),
        headless: false,
    }
}

/// A timestamp with no char boundary at byte 10, or shorter than 10 bytes,
/// is kept whole as its day bucket key instead of being byte-sliced.
#[test]
fn day_bucket_keeps_a_multibyte_timestamp_whole() {
    let records = vec![
        session_started_at(MULTIBYTE_STARTED_AT),
        session_started_at(SHORT_STARTED_AT),
    ];

    let report = aggregate(&records, None, None, GROUP_BY_DAY).expect("aggregate succeeds");

    let mut keys: Vec<&str> = report
        .buckets
        .iter()
        .map(|bucket| bucket.key.as_str())
        .collect();
    keys.sort_unstable();
    let mut expected = vec![MULTIBYTE_STARTED_AT, SHORT_STARTED_AT];
    expected.sort_unstable();
    assert_eq!(keys, expected);
}
