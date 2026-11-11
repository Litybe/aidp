//! `aidp memory model` and the callers that embed through the one model of the daemon.

use crate::common::*;
use aidp_gateway::http::{serve_on_with_environment, HttpConfig, ServeHandle};
use aidp_gateway::model_service::ModelLoader;
use aidp_gateway::models::runtime_environment::RuntimeEnvironment;
use aidp_memory::models::constants::embedding::DEFAULT_MODEL_NAME;
use aidp_memory::Embedder;
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAKE_DIM: usize = 3;
const WAIT: Duration = Duration::from_secs(30);
const NOTE: &str = "---\ntype: note\ndomain: unity\ntags: [jump]\nagents: [unity-issue-implementer]\nstatus: active\nimportance: 0.9\n---\n\n# Coyote time\n\nLe coyote window laisse sauter apres avoir quitte le bord.\n";
const EXIT_UPSTREAM: i32 = 5;
const EXIT_POLICY: i32 = 4;

struct FakeEmbedder;

impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        DEFAULT_MODEL_NAME
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> aidp_memory::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![1.0; FAKE_DIM]).collect())
    }
}

struct FakeLoader {
    installed: AtomicBool,
}

impl ModelLoader for FakeLoader {
    fn installed(&self, _: &Path) -> bool {
        self.installed.load(Ordering::SeqCst)
    }

    fn weights_bytes(&self, _: &Path) -> Option<u64> {
        Some(2 * 1024 * 1024)
    }

    fn load(&self, _: &Path) -> Result<Arc<dyn Embedder>, String> {
        Ok(Arc::new(FakeEmbedder))
    }
}

struct Daemon {
    _handle: ServeHandle,
    url: String,
}

fn daemon(ws: &TempWorkspace, installed: bool) -> Daemon {
    let state = ws.child("gateway");
    std::fs::create_dir_all(state.join("memory")).unwrap();
    std::fs::write(state.join(".defaults-seeded"), "").unwrap();
    std::fs::write(state.join("memory/model.json"), r#"{ "autostart": false }"#).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = HttpConfig {
        state_dir: state,
        vault_root: ws.child("vault"),
        hook_token: String::new(),
        ui_token: String::new(),
        read_only: false,
        allowed_origins: vec![],
    };
    let environment = RuntimeEnvironment {
        model_loader: Some(Arc::new(FakeLoader {
            installed: AtomicBool::new(installed),
        })),
        ..Default::default()
    };
    Daemon {
        _handle: serve_on_with_environment(listener, config, environment).unwrap(),
        url: format!("http://127.0.0.1:{port}"),
    }
}

fn model_command(ws: &TempWorkspace, url: Option<&str>, args: &[&str]) -> Output {
    let mut command = aidp(ws);
    command.env("AIDP_GATEWAY_DIR", ws.child("gateway"));
    if let Some(url) = url {
        command.env("AIDP_GATEWAY_URL", url);
    }
    command
        .args(["--format", "json", "memory", "model"])
        .args(args)
        .output()
        .unwrap()
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}

fn wait_running(ws: &TempWorkspace, url: &str) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let status = json_of(&model_command(ws, Some(url), &["status"]));
        if status["state"] == "running" {
            return status;
        }
        assert!(Instant::now() < deadline, "never running: {status}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn status_without_the_daemon_reads_the_disk_and_says_so() {
    let ws = workspace();

    let output = model_command(&ws, None, &["status"]);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let status = json_of(&output);
    assert_eq!(status["daemonRunning"], false);
    assert_eq!(status["model"], DEFAULT_MODEL_NAME);
    assert_eq!(status["present"], false);
}

#[test]
fn start_without_the_daemon_is_an_upstream_failure() {
    let ws = workspace();
    for verb in [
        &["start"][..],
        &["stop"],
        &["embed-pending"],
        &["autostart", "off"],
    ] {
        let output = model_command(&ws, None, verb);
        assert_eq!(
            output.status.code(),
            Some(EXIT_UPSTREAM),
            "{verb:?}: {output:?}"
        );
    }
}

#[test]
fn start_of_a_model_that_is_not_installed_is_a_policy_refusal() {
    let ws = workspace();
    let daemon = daemon(&ws, false);

    let output = model_command(&ws, Some(&daemon.url), &["start"]);

    assert_eq!(output.status.code(), Some(EXIT_POLICY), "{output:?}");
}

#[test]
fn the_daemon_model_is_started_used_by_the_index_and_stopped_from_the_cli() {
    let ws = workspace();
    let daemon = daemon(&ws, true);
    let project = ws.child("game");
    std::fs::create_dir_all(&project).unwrap();
    let note = ws.child("vault/Knowledge/_global/unity/coyote.md");
    std::fs::create_dir_all(note.parent().unwrap()).unwrap();
    std::fs::write(&note, NOTE).unwrap();

    let started = model_command(&ws, Some(&daemon.url), &["start"]);
    assert_eq!(started.status.code(), Some(0), "{started:?}");
    let running = wait_running(&ws, &daemon.url);
    assert_eq!(running["dim"], FAKE_DIM);
    assert_eq!(running["autostart"], false);

    let mut index = aidp(&ws);
    let indexed = index
        .env("AIDP_GATEWAY_DIR", ws.child("gateway"))
        .env("AIDP_GATEWAY_URL", &daemon.url)
        .args([
            "rag",
            "index",
            "--embed",
            "--source",
            "knowledge",
            "--project",
        ])
        .arg(&project)
        .output()
        .unwrap();
    assert_eq!(indexed.status.code(), Some(0), "{indexed:?}");
    let report = json_of(&indexed);
    assert!(report["embedded"].as_u64().unwrap() > 0, "{report}");

    let pending = model_command(&ws, Some(&daemon.url), &["embed-pending"]);
    assert_eq!(pending.status.code(), Some(0), "{pending:?}");
    assert_eq!(json_of(&pending)["queued"], true);

    let autostart = model_command(&ws, Some(&daemon.url), &["autostart", "on"]);
    assert_eq!(json_of(&autostart)["autostart"], true);

    let stopped = model_command(&ws, Some(&daemon.url), &["stop"]);
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");
    assert_eq!(json_of(&stopped)["state"], "stopped");

    let mut again = aidp(&ws);
    let refused = again
        .env("AIDP_GATEWAY_DIR", ws.child("gateway"))
        .env("AIDP_GATEWAY_URL", &daemon.url)
        .args([
            "rag",
            "index",
            "--embed",
            "--source",
            "knowledge",
            "--project",
        ])
        .arg(&project)
        .output()
        .unwrap();
    assert_ne!(
        refused.status.code(),
        Some(0),
        "--embed needs the running model"
    );
}

#[test]
fn the_text_status_lists_the_state_in_french_like_the_other_memory_verbs() {
    let ws = workspace();
    let daemon = daemon(&ws, true);
    model_command(&ws, Some(&daemon.url), &["start"]);
    wait_running(&ws, &daemon.url);

    let mut command = aidp(&ws);
    let output = command
        .env("AIDP_GATEWAY_DIR", ws.child("gateway"))
        .env("AIDP_GATEWAY_URL", &daemon.url)
        .args(["--format", "text", "memory", "model", "status"])
        .output()
        .unwrap();

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("etat: running"), "{text}");
    assert!(text.contains("memoire (estimation): 2 Mo"), "{text}");
}
