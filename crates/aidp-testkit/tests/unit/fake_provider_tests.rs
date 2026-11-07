use std::time::Duration;

use crate::fake_provider::FakeProvider;
use crate::fake_provider_script::FakeScript;
use aidp_agents::{AgentEvent, AgentProvider, AgentSpec, ExitReason, TokenUsage};

const OUTPUT_FILE: &str = "reply.md";
const OUTPUT_BODY: &str = "done";
const USAGE: TokenUsage = TokenUsage {
    input: 10,
    output: 5,
    cache_creation: 0,
    cache_read: 0,
};

fn text(value: &str) -> AgentEvent {
    AgentEvent::Text {
        text: value.to_string(),
    }
}

fn drain_until_exit(provider: &FakeProvider, spec: AgentSpec) -> Vec<AgentEvent> {
    let handle = provider.start(spec).expect("start");
    let mut events = Vec::new();
    while let Ok(event) = handle.events().recv() {
        let exited = matches!(event, AgentEvent::Exited { .. });
        events.push(event);
        if exited {
            break;
        }
    }
    events
}

#[test]
fn plays_three_turns_usage_output_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = FakeScript::new()
        .text("one")
        .text("two")
        .write_file(OUTPUT_FILE, OUTPUT_BODY)
        .text("three")
        .usage(USAGE);
    let provider = FakeProvider::new(script);
    let spec = AgentSpec {
        work_dir: dir.path().to_path_buf(),
        prompt: "go".to_string(),
        ..AgentSpec::default()
    };

    let events = drain_until_exit(&provider, spec);

    assert_eq!(
        events,
        vec![
            text("one"),
            text("two"),
            text("three"),
            AgentEvent::Usage(USAGE),
            AgentEvent::Exited {
                code: Some(0),
                reason: ExitReason::Completed
            },
        ]
    );
    let written = std::fs::read_to_string(dir.path().join(OUTPUT_FILE)).expect("output file");
    assert_eq!(written, OUTPUT_BODY);
    assert_eq!(provider.id(), "fake");
    assert_eq!(provider.launched().len(), 1);
    assert_eq!(provider.launched()[0].prompt, "go");
}

#[test]
fn cancel_mid_script_exits_cancelled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = FakeScript::new()
        .text("first")
        .delay(Duration::from_secs(30))
        .text("never");
    let provider = FakeProvider::new(script);
    let spec = AgentSpec {
        work_dir: dir.path().to_path_buf(),
        ..AgentSpec::default()
    };
    let handle = provider.start(spec).expect("start");

    assert_eq!(handle.events().recv().expect("first"), text("first"));
    handle.cancel().expect("cancel");
    let last = handle.events().recv().expect("exit");

    assert_eq!(
        last,
        AgentEvent::Exited {
            code: None,
            reason: ExitReason::Cancelled
        }
    );
    assert_eq!(handle.wait().expect("wait"), ExitReason::Cancelled);
}
