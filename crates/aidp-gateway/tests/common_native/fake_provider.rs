//! Scripted agent provider for native run tests: reads the stage from the prompt and plays a behaviour.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aidp_agents::{AgentError, AgentEvent, AgentHandle, AgentProvider, AgentSpec, Capabilities};
use aidp_testkit::fake_provider::FakeProvider;
use aidp_testkit::fake_provider_script::FakeScript;
use serde_json::Value;

pub const PROVIDER_ID: &str = "fake";
pub const REVIEWER_ROLE: &str = "reviewer";
pub const BLOCKING_SIGNAL: &str = "has_blocking_findings";
pub const UNEVALUATED_SIGNAL: &str = "unevaluated_checks";

const STAGE_MARKER: &str = "AIDP_STAGE=";
const ROLE_MARKER: &str = "**Agent:** `";
const OUTPUTS_HEADING: &str = "## Outputs (write into `";
const SUB_SESSION_MARKER: &str = "## Reply location";
const TASK_MARKER: &str = "## Plan task\n\n`";
const REPLY_FILE_SUFFIX: &str = "reply.md`";
const REPLY_FILE_NAME: &str = "reply.md";
const BULLET: &str = "- `";
const BACKTICK: char = '`';
const TASK_SEPARATOR: char = '#';
const PLAN_FILE: &str = "plan.json";
const COMPLETED: &str = "completed";
const STREAM: &str = include_str!("../fixtures/fake_provider/stream.jsonl");
const SUMMARY: &str = "Scripted reply.";
const OUTPUT_CONTENT: &str = "scripted output\n";
const HANG: Duration = Duration::from_secs(600);
const BYPASS_MODE: &str = "bypassPermissions";
const PERMISSION_MODE_FIELD: &str = "permissionMode";

#[path = "behaviour.rs"]
mod behaviour;
#[path = "call.rs"]
mod call;
#[path = "refusal.rs"]
mod refusal;
#[path = "reply_body.rs"]
mod reply_body;

pub use behaviour::Behaviour;
pub use call::Call;
pub use refusal::Refusal;
use reply_body::ReplyBody;

type Decide = dyn Fn(&Call) -> Behaviour + Send + Sync;

/// Provider that plays [`Behaviour`] per launch and records every [`Call`].
pub struct ScriptedProvider {
    decide: Box<Decide>,
    plan_json: Option<String>,
    plans_by_label: BTreeMap<String, String>,
    calls: Mutex<Vec<Call>>,
    refusal: Mutex<Option<Refusal>>,
}

impl ScriptedProvider {
    /// `plan_json` is the content written for a declared `plan.json`.
    pub fn new(
        plan_json: Option<&str>,
        decide: impl Fn(&Call) -> Behaviour + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(ScriptedProvider {
            decide: Box::new(decide),
            plan_json: plan_json.map(str::to_string),
            plans_by_label: BTreeMap::new(),
            calls: Mutex::new(Vec::new()),
            refusal: Mutex::new(None),
        })
    }

    /// From now on, claude refuses the modes `refusal` describes: it reports `refusal.shown` and a
    /// session asked for bypass then never ends by itself.
    #[allow(dead_code)]
    pub fn refuse(&self, refusal: Refusal) {
        *self.refusal.lock().expect("refusal lock") = Some(refusal);
    }

    #[allow(dead_code)]
    /// Like [`ScriptedProvider::new`], with a distinct `plan.json` content for each listed stage label.
    pub fn with_plans(
        plan_json: Option<&str>,
        plans_by_label: &[(&str, &str)],
        decide: impl Fn(&Call) -> Behaviour + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(ScriptedProvider {
            decide: Box::new(decide),
            plan_json: plan_json.map(str::to_string),
            plans_by_label: plans_by_label
                .iter()
                .map(|(label, plan)| (label.to_string(), plan.to_string()))
                .collect(),
            calls: Mutex::new(Vec::new()),
            refusal: Mutex::new(None),
        })
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls lock").clone()
    }

    pub fn labels(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.label).collect()
    }

    pub fn count(&self, label: &str) -> usize {
        self.calls()
            .iter()
            .filter(|call| call.label == label)
            .count()
    }

    fn record(&self, prompt: &str) -> Call {
        let mut calls = self.calls.lock().expect("calls lock");
        let mut call = parse_call(prompt);
        call.nth = calls.iter().filter(|seen| seen.label == call.label).count() + 1;
        calls.push(call.clone());
        call
    }

    fn contents_of(&self, label: &str, file_name: &str) -> String {
        if file_name != PLAN_FILE {
            return OUTPUT_CONTENT.to_string();
        }
        self.plans_by_label
            .get(label)
            .or(self.plan_json.as_ref())
            .cloned()
            .unwrap_or_else(|| OUTPUT_CONTENT.to_string())
    }

    fn script_for(&self, call: &Call, behaviour: &Behaviour, reported_mode: &str) -> FakeScript {
        let mut script = stream_script(reported_mode);
        if behaviour.hang {
            return script.delay(HANG);
        }
        for (relative_path, contents) in &behaviour.workspace_writes {
            script = script.write_file(relative_path, contents);
        }
        if let Some(hold) = behaviour.hold {
            script = script.delay(hold);
        }
        let mut outputs = BTreeMap::new();
        if !call.sub_session {
            for name in &call.declared_outputs {
                let path = path_text(&call.stage_dir.join(name));
                script = script.write_file(&path, &self.contents_of(&call.label, name));
                outputs.entry(output_key(name)).or_insert(path);
            }
        }
        let reply = behaviour
            .raw_reply
            .clone()
            .unwrap_or_else(|| reply_text(call, behaviour, outputs));
        script.write_file(&path_text(&call.reply_path), &reply)
    }
}

impl AgentProvider for ScriptedProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError> {
        let call = self.record(&spec.prompt);
        let mut behaviour = (self.decide)(&call);
        let asked = spec.extra_args.first().cloned().unwrap_or_default();
        let refusal = self
            .refusal
            .lock()
            .expect("refusal lock")
            .clone()
            .filter(|refusal| !refusal.bypass_only || asked == BYPASS_MODE);
        let reported = refusal.map_or(asked.clone(), |refusal| refusal.shown);
        behaviour.hang |= reported != asked && asked == BYPASS_MODE;
        FakeProvider::new(self.script_for(&call, &behaviour, &reported)).start(spec)
    }
}

/// The init, assistant and result messages of a Claude session, which the session manager reads.
/// The init message reports `reported_mode` as the permission mode of the session.
fn stream_script(reported_mode: &str) -> FakeScript {
    STREAM.lines().fold(FakeScript::new(), |script, line| {
        let mut message: Value = serde_json::from_str(line).expect("stream line is json");
        if message.get("subtype").and_then(Value::as_str) == Some("init") {
            message[PERMISSION_MODE_FIELD] = Value::from(reported_mode);
        }
        script.emit(AgentEvent::Raw(message))
    })
}

fn reply_text(call: &Call, behaviour: &Behaviour, outputs: BTreeMap<String, String>) -> String {
    let mut signals = behaviour.signals.clone();
    if call.role == REVIEWER_ROLE {
        signals
            .entry(BLOCKING_SIGNAL.to_string())
            .or_insert(Value::Bool(false));
        signals
            .entry(UNEVALUATED_SIGNAL.to_string())
            .or_insert(Value::from(0));
    }
    let body = ReplyBody {
        status: COMPLETED.to_string(),
        summary: SUMMARY.to_string(),
        outputs,
        signals,
    };
    serde_json::to_string_pretty(&body).expect("reply serializes")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// `changed-files.json` becomes `changed-files`, the key a reply uses.
fn output_key(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .to_string()
}

fn text_between<'a>(text: &'a str, marker: &str, terminator: char) -> Option<&'a str> {
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(terminator).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// The last path the prompt names for `reply.md`: a task override wins over the stage directory.
fn reply_path_of(prompt: &str) -> PathBuf {
    let end = prompt
        .rfind(REPLY_FILE_SUFFIX)
        .expect("the prompt names reply.md")
        + REPLY_FILE_NAME.len();
    let start = prompt[..end].rfind(BACKTICK).expect("the path is quoted") + 1;
    PathBuf::from(&prompt[start..end])
}

fn declared_outputs_of(prompt: &str) -> Vec<String> {
    let Some(start) = prompt.find(OUTPUTS_HEADING) else {
        return Vec::new();
    };
    prompt[start..]
        .lines()
        .skip(1)
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| line.starts_with(BULLET))
        .map(|line| {
            line.trim_start_matches(BULLET)
                .trim_end_matches(BACKTICK)
                .to_string()
        })
        .collect()
}

fn parse_call(prompt: &str) -> Call {
    let stage = text_between(prompt, STAGE_MARKER, '\n')
        .expect("the prompt names its stage")
        .to_string();
    let role = text_between(prompt, ROLE_MARKER, BACKTICK)
        .unwrap_or_default()
        .to_string();
    let sub_session = prompt.contains(SUB_SESSION_MARKER);
    let task = text_between(prompt, TASK_MARKER, BACKTICK).filter(|_| sub_session);
    let label = match task {
        Some(id) => format!("{stage}{TASK_SEPARATOR}{id}"),
        None => stage.clone(),
    };
    let reply_path = reply_path_of(prompt);
    Call {
        label,
        role,
        nth: 0,
        stage_dir: reply_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        reply_path,
        declared_outputs: declared_outputs_of(prompt),
        sub_session,
        prompt: prompt.to_string(),
    }
}
