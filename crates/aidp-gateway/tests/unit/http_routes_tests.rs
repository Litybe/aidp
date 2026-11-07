//! The `/api/flows` handlers driven in-process against a real engine with
//! fake sessions, plus the read-only gate of the HTTP server.

use super::{handle, FlowReply};
use crate::bus::Bus;
use crate::flows::agent_sessions_fake::{self, AgentSessions};
use crate::flows::executor::Ports;
use crate::flows::lock::LocalLock;
use crate::flows::registry::{
    ActionContext, BudgetGuard, FlowAction, PipelineLaunch, ProjectRef, Registry, SessionPort,
    SessionStart, SessionState,
};
use crate::flows::{FlowEngine, FlowEngineConfig};
use crate::http::{serve_on, HttpConfig};
use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_flows::ActionOutcome;
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_http::Method;

const MANUAL_PIPELINE: &str = "pr-comments";
const INVALID_PUBLISH_TEXT: &str = "true";
const REPO: &str = "api-repo";
const SERVICE: &str = "svc";
const FLOW: &str = "test-flow";
const COUNT_ACTION: &str = "count";
const STEP_PREPARE: &str = "prepare";
const STEP_APPROVE: &str = "approve";
/// `approve` with its first letter percent-encoded, as a browser may send any step id.
const ENCODED_STEP_APPROVE: &str = "%61pprove";
const ACTOR: &str = "alice";
const WAIT_LIMIT: Duration = Duration::from_secs(10);
const WAIT_POLL: Duration = Duration::from_millis(10);
const LOCK_WAIT: Duration = Duration::from_millis(60);
const FLOWS_PATH: &str = "/api/flows";
const STATUS_OK: u16 = 200;
const STATUS_CREATED: u16 = 201;
const STATUS_CONFLICT: u16 = 409;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_FORBIDDEN: u16 = 403;
const WAITING_HUMAN: &str = "waiting_human";
const DONE: &str = "done";

/// Does nothing and succeeds.
struct CountAction;

impl FlowAction for CountAction {
    fn writes(&self) -> bool {
        false
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        ActionOutcome::Ok(BTreeMap::new())
    }
}

struct AllowAll;

impl BudgetGuard for AllowAll {
    fn allows(&self, _template: &str) -> bool {
        true
    }
}

/// Sessions the test flow never reaches, since its pipeline step is switched off.
struct NoSessions(PathBuf);

impl SessionPort for NoSessions {
    fn project_for(&self, _ticket: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: "p1".to_string(),
            path: self.0.clone(),
            repo: REPO.to_string(),
        })
    }

    fn start(&self, _launch: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        Ok(SessionStart::Busy)
    }

    fn state(&self, _session_id: &str) -> SessionState {
        SessionState::Gone
    }

    fn cancel(&self, _session_id: &str) {}
}

struct Api {
    _workspace: TempWorkspace,
    engine: Arc<FlowEngine>,
}

impl Api {
    fn new(tag: &str) -> Self {
        Self::build(tag, |vault| Arc::new(NoSessions(vault)))
    }

    /// An api whose sessions hold one run with a pending agent job.
    fn with_agents(tag: &str) -> (Self, Arc<AgentSessions>) {
        let slot = std::sync::Mutex::new(None);
        let api = Self::build(tag, |vault| {
            let sessions = Arc::new(AgentSessions::new(vault, REPO));
            *slot.lock().unwrap() = Some(sessions.clone());
            sessions
        });
        let sessions = slot.lock().unwrap().take().unwrap();
        (api, sessions)
    }

    fn build(tag: &str, sessions: impl FnOnce(PathBuf) -> Arc<dyn SessionPort>) -> Self {
        let workspace = TempWorkspace::new(tag);
        let vault = workspace.child("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let mut registry = Registry::with_builtins();
        registry.register_action(COUNT_ACTION, Arc::new(CountAction));
        let ports = Ports {
            sessions: sessions(vault.clone()),
            budget: Arc::new(AllowAll),
            lock: Arc::new(LocalLock::default()),
            bus: Arc::new(Bus::new()),
            clock: Arc::new(SystemClock),
            ids: Arc::new(SystemIdGen),
            runner: Arc::new(ScriptedProcessRunner::new()),
            lock_wait: LOCK_WAIT,
        };
        let config = FlowEngineConfig {
            flow_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flows")],
            ..FlowEngineConfig::default()
        };
        let engine = FlowEngine::new(vault, workspace.child("state"), registry, ports, config);
        Api {
            _workspace: workspace,
            engine,
        }
    }

    fn call(&self, method: Method, path: &str, body: Value) -> FlowReply {
        self.call_with_query(method, path, &HashMap::new(), body)
    }

    fn call_with_query(
        &self,
        method: Method,
        path: &str,
        query: &HashMap<String, String>,
        body: Value,
    ) -> FlowReply {
        handle(&self.engine, &method, path, query, &body).expect("a flow route")
    }

    fn get(&self, path: &str) -> FlowReply {
        self.call(Method::Get, path, Value::Null)
    }

    fn post(&self, path: &str, body: Value) -> FlowReply {
        self.call(Method::Post, path, body)
    }

    fn start(&self, key: &str) -> FlowReply {
        let ticket = json!({ "key": key, "service": SERVICE });
        let settings = json!({ "with_pipeline": false, "with_wait": false, "with_human": true });
        self.post(
            FLOWS_PATH,
            json!({ "ticket": ticket, "flow": FLOW, "settings": settings }),
        )
    }

    /// Starts `key` on the external executor with agent sessions run by the host, and attaches the fake run.
    fn start_external_agents(&self, key: &str) {
        let ticket = json!({ "key": key, "service": SERVICE });
        let settings = json!({ "with_pipeline": false, "with_wait": false, "with_human": true });
        let policy = json!({ "executor": "external", "externalAgents": true });
        let started = self.post(
            FLOWS_PATH,
            json!({ "ticket": ticket, "flow": FLOW, "settings": settings, "policy": policy }),
        );
        assert_eq!(started.status, STATUS_CREATED);
        let store = &self.engine.executor().store;
        let mut runtime = store.load_runtime(REPO, key);
        runtime.sessions.insert(
            "dev:1:run_pipeline".to_string(),
            agent_sessions_fake::RUN_ID.to_string(),
        );
        store.save_runtime(REPO, key, &runtime).unwrap();
    }

    fn wait_for_run_status(&self, key: &str, wanted: &str) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while self.get(&run_path(key)).body["status"] != wanted {
            assert!(Instant::now() < deadline, "timed out waiting for {wanted}");
            std::thread::sleep(WAIT_POLL);
        }
    }
}

fn run_path(key: &str) -> String {
    format!("{FLOWS_PATH}/{key}")
}

fn step_path(key: &str, step: &str, verb: &str) -> String {
    format!("{FLOWS_PATH}/{key}/steps/{step}/{verb}")
}

#[test]
fn it_full_run_through_the_api() {
    let api = Api::new("flows_api_full");
    let started = api.start("LOC-1");
    assert_eq!(started.status, STATUS_CREATED);

    api.wait_for_run_status("LOC-1", WAITING_HUMAN);
    let approved = api.post(
        &step_path("LOC-1", STEP_APPROVE, "approve"),
        json!({ "actor": ACTOR }),
    );
    assert_eq!(approved.status, STATUS_OK);
    api.wait_for_run_status("LOC-1", DONE);

    let listed = api.get(FLOWS_PATH);
    assert_eq!(listed.body["entries"][0]["key"], "LOC-1");
    let journal = api.get(&format!("{}/journal", run_path("LOC-1")));
    assert_eq!(journal.status, STATUS_OK);
    let lines = journal.body["entries"].as_array().unwrap();
    assert!(lines.iter().any(|line| {
        let text = line["line"].as_str().unwrap();
        text.contains("human decision `approve`") && text.contains(&format!("by {ACTOR} at 20"))
    }));
}

#[test]
fn it_an_encoded_step_id_is_decoded_before_the_decision() {
    let api = Api::new("flows_api_encoded_step");
    api.start("LOC-9");
    api.wait_for_run_status("LOC-9", WAITING_HUMAN);

    let approved = api.post(
        &step_path("LOC-9", ENCODED_STEP_APPROVE, "approve"),
        json!({ "actor": ACTOR }),
    );

    assert_eq!(approved.status, STATUS_OK);
    api.wait_for_run_status("LOC-9", DONE);
}

#[test]
fn it_second_start_answers_conflict_with_the_existing_run() {
    let api = Api::new("flows_api_conflict");
    assert_eq!(api.start("LOC-2").status, STATUS_CREATED);

    let second = api.start("LOC-2");

    assert_eq!(second.status, STATUS_CONFLICT);
    assert_eq!(second.body["code"], "conflict");
    assert_eq!(second.body["existing"]["key"], "LOC-2");
}

#[test]
fn it_approving_a_step_that_is_not_waiting_is_a_conflict() {
    let api = Api::new("flows_api_not_waiting");
    api.start("LOC-3");
    api.wait_for_run_status("LOC-3", WAITING_HUMAN);

    let reply = api.post(
        &step_path("LOC-3", STEP_PREPARE, "approve"),
        json!({ "actor": ACTOR }),
    );

    assert_eq!(reply.status, STATUS_CONFLICT);
    assert_eq!(reply.body["code"], "conflict");
}

#[test]
fn it_run_view_carries_definition_steps_and_current_step() {
    let api = Api::new("flows_api_shape");
    api.start("LOC-4");
    api.wait_for_run_status("LOC-4", WAITING_HUMAN);

    let body = api.get(&run_path("LOC-4")).body;

    assert_eq!(body["key"], "LOC-4");
    assert_eq!(body["def"]["id"], FLOW);
    assert_eq!(body["current"], STEP_APPROVE);
    assert_eq!(body["repo"], REPO);
    assert_eq!(body["steps"][STEP_PREPARE]["status"], "done");
    assert!(body["steps"][STEP_PREPARE]["started_at"].is_string());
    assert!(body["inbox"].is_array());
}

#[test]
fn it_list_filters_by_status_and_service() {
    let api = Api::new("flows_api_filters");
    api.start("LOC-5");
    api.wait_for_run_status("LOC-5", WAITING_HUMAN);
    let matching = HashMap::from([
        ("status".to_string(), WAITING_HUMAN.to_string()),
        ("service".to_string(), SERVICE.to_string()),
    ]);
    let other_service = HashMap::from([("service".to_string(), "other".to_string())]);

    let hit = api.call_with_query(Method::Get, FLOWS_PATH, &matching, Value::Null);
    let miss = api.call_with_query(Method::Get, FLOWS_PATH, &other_service, Value::Null);

    assert_eq!(hit.body["entries"].as_array().unwrap().len(), 1);
    assert!(miss.body["entries"].as_array().unwrap().is_empty());
}

#[test]
fn it_unknown_run_unknown_flow_and_bad_bodies_map_to_typed_errors() {
    let api = Api::new("flows_api_errors");

    assert_eq!(api.get(&run_path("NOPE-1")).status, STATUS_NOT_FOUND);
    assert_eq!(api.post(FLOWS_PATH, json!({})).status, STATUS_BAD_REQUEST);
    let unknown_flow = api.post(
        FLOWS_PATH,
        json!({ "ticket": { "key": "LOC-6" }, "flow": "missing" }),
    );
    assert_eq!(unknown_flow.status, STATUS_NOT_FOUND);
}

#[test]
fn it_pause_resume_and_abort_drive_the_run() {
    let api = Api::new("flows_api_control");
    api.start("LOC-7");
    api.wait_for_run_status("LOC-7", WAITING_HUMAN);

    let aborted = api.post(
        &format!("{}/abort", run_path("LOC-7")),
        json!({ "actor": ACTOR }),
    );

    assert_eq!(aborted.status, STATUS_OK);
    api.wait_for_run_status("LOC-7", "aborted");
}

#[test]
fn it_verdict_requires_a_boolean_and_manual_requires_a_target() {
    let api = Api::new("flows_api_verdict");
    api.start("LOC-8");

    let verdict = api.post(&step_path("LOC-8", STEP_APPROVE, "verdict"), json!({}));
    let manual = api.post(&format!("{}/manual", run_path("LOC-8")), json!({}));

    assert_eq!(verdict.status, STATUS_BAD_REQUEST);
    assert_eq!(manual.status, STATUS_BAD_REQUEST);
}

#[test]
fn it_other_paths_are_not_flow_routes() {
    let api = Api::new("flows_api_other");

    let reply = handle(
        &api.engine,
        &Method::Get,
        "/api/sessions",
        &HashMap::new(),
        &Value::Null,
    );

    assert!(reply.is_none());
}

#[test]
fn it_read_only_mode_refuses_every_post_and_serves_gets() {
    let workspace = TempWorkspace::new("flows_api_read_only");
    let state_dir = workspace.path().to_path_buf();
    std::fs::write(state_dir.join(".defaults-seeded"), "").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir,
        vault_root: PathBuf::new(),
        hook_token: "hook-secret".to_string(),
        ui_token: "ui-secret".to_string(),
        read_only: true,
        allowed_origins: vec![],
    };
    let handle = serve_on(listener, config).unwrap();
    let base = format!("http://127.0.0.1:{port}{FLOWS_PATH}");
    let posts = [
        base.clone(),
        format!("{base}/LOC-1/pause"),
        format!("{base}/LOC-1/resume"),
        format!("{base}/LOC-1/abort"),
        format!("{base}/LOC-1/manual"),
        format!("{base}/LOC-1/steps/approve/approve"),
        format!("{base}/LOC-1/steps/approve/verdict"),
    ];

    for url in &posts {
        let status = ureq::post(url)
            .send_json(json!({}))
            .unwrap_or_else(|e| e.into_response().unwrap())
            .status();
        assert_eq!(status, STATUS_FORBIDDEN, "{url}");
    }
    let read = ureq::get(&base)
        .call()
        .unwrap_or_else(|e| e.into_response().unwrap())
        .status();
    assert_ne!(read, STATUS_FORBIDDEN);
    handle.stop();
}

/// Manual publishing is optional and only explicit booleans opt a pipeline in.
#[test]
fn manual_request_parses_optional_publish_boolean() {
    use super::{manual_request, BODY_PIPELINE, BODY_PUBLISH};
    for publish in [None, Some(false), Some(true)] {
        let mut body = json!({ BODY_PIPELINE: MANUAL_PIPELINE });
        if let Some(value) = publish {
            body[BODY_PUBLISH] = json!(value);
        }
        let request = manual_request(&body).unwrap();
        assert_eq!(
            request,
            aidp_flows::ManualRequest::Pipeline {
                template: MANUAL_PIPELINE.to_string(),
                with: BTreeMap::new(),
                publish: publish.unwrap_or(false),
            }
        );
    }
}

/// Invalid publishing values are rejected as typed HTTP 400 responses before dispatch.
#[test]
fn manual_request_rejects_non_boolean_publish() {
    use super::{
        manual_request, BODY_CODE, BODY_ERROR, BODY_PIPELINE, BODY_PUBLISH, INVALID_PUBLISH,
    };
    use crate::models::enums::ApiStatus;
    for publish in [
        Value::Null,
        json!(INVALID_PUBLISH_TEXT),
        json!(1),
        json!([]),
        json!({}),
    ] {
        let reply = manual_request(&json!({BODY_PIPELINE: MANUAL_PIPELINE, BODY_PUBLISH: publish}))
            .unwrap_err();
        assert_eq!(reply.status, ApiStatus::BadRequest.status());
        assert_eq!(reply.body[BODY_CODE], ApiStatus::BadRequest.code());
        assert_eq!(reply.body[BODY_ERROR], INVALID_PUBLISH);
    }
}

const ENCODED_JOB_ID: &str = "ci_fix%2Fci_recheck:1:run_action";

#[test]
fn external_routes_answer_unknown_run_and_invalid_result_with_typed_errors() {
    let api = Api::new("flows_api_external_errors");

    let jobs = api.get(&format!("{FLOWS_PATH}/NOPE-1/external"));
    assert_eq!(jobs.status, STATUS_NOT_FOUND);
    let result_path = format!("{FLOWS_PATH}/NOPE-1/external/{ENCODED_JOB_ID}/result");
    let missing = api.post(&result_path, json!({ "outcome": "ok" }));
    assert_eq!(missing.status, STATUS_NOT_FOUND);
    let invalid = api.post(&result_path, json!({ "outcome": "exploded" }));
    assert_eq!(invalid.status, STATUS_BAD_REQUEST);
}

#[test]
fn external_result_route_accepts_an_unencoded_job_id_spanning_segments() {
    let api = Api::new("flows_api_external_segments");

    let reply = api.post(
        &format!("{FLOWS_PATH}/NOPE-2/external/ci_fix/ci_recheck:1:run_action/result"),
        json!({ "outcome": "ok" }),
    );

    assert_eq!(reply.status, STATUS_NOT_FOUND);
}

#[test]
fn external_jobs_of_an_internal_run_are_empty() {
    let api = Api::new("flows_api_external_internal");
    api.start("LOC-9");

    let jobs = api.get(&format!("{FLOWS_PATH}/LOC-9/external"));

    assert_eq!(jobs.status, STATUS_OK);
    assert_eq!(jobs.body["key"], "LOC-9");
    assert_eq!(jobs.body["jobs"], json!([]));
}

/// The job id as a client sends it: the `:` are percent-encoded.
fn encoded_job_path(key: &str, verb: &str) -> String {
    let encoded = agent_sessions_fake::job_id().replace(':', "%3A");
    format!("{FLOWS_PATH}/{key}/external/{encoded}/{verb}")
}

#[test]
fn the_agent_job_routes_list_claim_renew_and_settle_a_job() {
    let (api, sessions) = Api::with_agents("flows_api_agent_jobs");
    api.start_external_agents("LOC-A");

    let listed = api.get(&format!("{FLOWS_PATH}/LOC-A/external"));
    let agent = listed.body["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["kind"] == "agent")
        .expect("an agent job")
        .clone();
    assert_eq!(agent["id"], json!(agent_sessions_fake::job_id()));
    assert_eq!(agent["agent"]["runId"], json!(agent_sessions_fake::RUN_ID));

    let claim = api.post(
        &encoded_job_path("LOC-A", "claim"),
        json!({ "actor": "ada", "hostId": "worker-1" }),
    );
    assert_eq!(claim.status, STATUS_OK, "{}", claim.body);
    assert_eq!(
        claim.body["claimToken"],
        json!(agent_sessions_fake::CLAIM_TOKEN)
    );
    assert_eq!(
        claim.body["job"]["id"],
        json!(agent_sessions_fake::job_id())
    );

    let beat = api.post(
        &encoded_job_path("LOC-A", "heartbeat"),
        json!({ "claimToken": agent_sessions_fake::CLAIM_TOKEN }),
    );
    assert_eq!(beat.status, STATUS_OK, "{}", beat.body);
    assert_eq!(beat.body["cancelRequested"], json!(false));

    let result = api.post(
        &encoded_job_path("LOC-A", "result"),
        json!({
            "outcome": "completed",
            "actor": "ada",
            "agent": { "claimToken": agent_sessions_fake::CLAIM_TOKEN },
        }),
    );
    assert_eq!(result.status, STATUS_OK, "{}", result.body);
    assert_eq!(result.body["accepted"], json!(true));
    assert_eq!(result.body["next"], json!("settled"));
    let job = agent_sessions_fake::job_id();
    assert_eq!(
        *sessions.calls.lock().unwrap(),
        vec![
            format!("claim {job}"),
            format!("heartbeat {job} {}", agent_sessions_fake::CLAIM_TOKEN),
            format!("result {job} completed"),
        ]
    );
}

#[test]
fn an_unencoded_agent_job_id_reaches_the_claim_route_too() {
    let (api, _) = Api::with_agents("flows_api_agent_plain_id");
    api.start_external_agents("LOC-B");

    let claim = api.post(
        &format!(
            "{FLOWS_PATH}/LOC-B/external/{}/claim",
            agent_sessions_fake::job_id()
        ),
        Value::Null,
    );

    assert_eq!(claim.status, STATUS_OK, "{}", claim.body);
}

#[test]
fn the_agent_job_routes_answer_not_found_for_a_foreign_job_and_an_unknown_flow() {
    let (api, sessions) = Api::with_agents("flows_api_agent_not_found");
    api.start_external_agents("LOC-C");

    for verb in ["claim", "heartbeat", "result"] {
        let foreign = api.post(
            &format!("{FLOWS_PATH}/LOC-C/external/agent%3Aother-run%3A1/{verb}"),
            json!({ "outcome": "completed" }),
        );
        assert_eq!(foreign.status, STATUS_NOT_FOUND, "{verb}: {}", foreign.body);
        let unknown_flow = api.post(&encoded_job_path("NOPE-3", verb), json!({}));
        assert_eq!(unknown_flow.status, STATUS_NOT_FOUND, "{verb}");
    }
    assert!(sessions.calls.lock().unwrap().is_empty());
}

#[test]
fn a_claim_on_a_paused_flow_or_an_already_claimed_job_answers_conflict() {
    let (api, _) = Api::with_agents("flows_api_agent_conflict");
    api.start_external_agents("LOC-D");
    let first = api.post(&encoded_job_path("LOC-D", "claim"), json!({}));
    assert_eq!(first.status, STATUS_OK);

    let second = api.post(&encoded_job_path("LOC-D", "claim"), json!({}));
    assert_eq!(second.status, STATUS_CONFLICT);

    let paused = api.post(
        &format!("{FLOWS_PATH}/LOC-D/pause"),
        json!({ "actor": ACTOR }),
    );
    assert_eq!(paused.status, STATUS_OK, "{}", paused.body);
    let refused = api.post(&encoded_job_path("LOC-D", "claim"), json!({}));
    assert_eq!(refused.status, STATUS_CONFLICT);
}

#[test]
fn external_agents_without_the_external_executor_answer_bad_request() {
    let api = Api::new("flows_api_agents_need_executor");
    let ticket = json!({ "key": "LOC-E", "service": SERVICE });

    let reply = api.post(
        FLOWS_PATH,
        json!({ "ticket": ticket, "flow": FLOW, "policy": { "externalAgents": true } }),
    );

    assert_eq!(reply.status, STATUS_BAD_REQUEST, "{}", reply.body);
}
