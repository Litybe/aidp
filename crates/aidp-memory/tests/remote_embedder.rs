//! The embedder that asks the daemon, against a tiny_http stub of the model routes.

use aidp_memory::models::constants::remote::EMBED_MAX_TEXTS;
use aidp_memory::models::embed_request::EmbedRequest;
use aidp_memory::models::embed_response::EmbedResponse;
use aidp_memory::models::enums::model_state::ModelState;
use aidp_memory::models::model_chunks::ModelChunks;
use aidp_memory::models::model_status_body::ModelStatusBody;
use aidp_memory::{Embedder, RemoteEmbedder};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Response, Server};

const MODEL: &str = "stub-model";
const DIM: usize = 2;
const EMBED_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_PATH: &str = "/api/memory/model";
const EMBED_PATH: &str = "/api/embed";
const STALL: Duration = Duration::from_millis(1500);
const PATIENCE: Duration = Duration::from_millis(1400);

#[derive(Serialize)]
struct StubError {
    error: &'static str,
    code: &'static str,
}

struct Stub {
    base_url: String,
    embed_calls: Arc<AtomicUsize>,
    refuse_embed: Arc<AtomicBool>,
    stalled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn status_body(state: ModelState) -> ModelStatusBody {
    ModelStatusBody {
        model: MODEL.to_string(),
        installed: true,
        state,
        autostart: true,
        dim: Some(DIM),
        ram_bytes: None,
        loaded_at: None,
        error: None,
        chunks: ModelChunks::default(),
        path: String::new(),
    }
}

fn json_response(status: u16, body: &impl Serialize) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", "application/json").unwrap();
    Response::from_string(serde_json::to_string(body).unwrap())
        .with_status_code(status)
        .with_header(header)
}

fn stub(state: ModelState) -> Stub {
    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let stub = Stub {
        base_url: format!("http://127.0.0.1:{port}"),
        embed_calls: Arc::new(AtomicUsize::new(0)),
        refuse_embed: Arc::new(AtomicBool::new(false)),
        stalled: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let calls = stub.embed_calls.clone();
    let refuse = stub.refuse_embed.clone();
    let stalled = stub.stalled.clone();
    let stop = stub.stop.clone();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(20)) else {
                continue;
            };
            if stalled.load(Ordering::SeqCst) {
                std::thread::sleep(STALL);
            }
            let reply = match (request.method(), request.url()) {
                (Method::Get, STATUS_PATH) => json_response(200, &status_body(state)),
                (Method::Post, EMBED_PATH) if refuse.load(Ordering::SeqCst) => json_response(
                    409,
                    &StubError {
                        error: "stopped",
                        code: "model_stopped",
                    },
                ),
                (Method::Post, EMBED_PATH) => {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let mut raw = String::new();
                    request.as_reader().read_to_string(&mut raw).unwrap();
                    let body: EmbedRequest = serde_json::from_str(&raw).unwrap();
                    let vectors = body
                        .texts
                        .iter()
                        .map(|text| vec![text.len() as f32, 1.0])
                        .collect();
                    json_response(
                        200,
                        &EmbedResponse {
                            model: MODEL.to_string(),
                            dim: DIM,
                            vectors,
                        },
                    )
                }
                _ => json_response(
                    404,
                    &StubError {
                        error: "no",
                        code: "not_found",
                    },
                ),
            };
            let _ = request.respond(reply);
        }
    });
    stub
}

#[test]
fn it_reads_the_model_from_the_daemon_and_embeds_through_it() {
    let daemon = stub(ModelState::Running);
    let embedder = RemoteEmbedder::connect(&daemon.base_url, EMBED_TIMEOUT).unwrap();

    assert_eq!(embedder.model_id(), MODEL);
    assert_eq!(embedder.dim(), DIM);
    let vectors = embedder.embed(&["coyote", "saut de grace"]).unwrap();
    assert_eq!(vectors, vec![vec![6.0, 1.0], vec![13.0, 1.0]]);
}

#[test]
fn it_splits_a_large_batch_into_calls_the_daemon_accepts() {
    let daemon = stub(ModelState::Running);
    let embedder = RemoteEmbedder::connect(&daemon.base_url, EMBED_TIMEOUT).unwrap();
    let texts: Vec<String> = (0..EMBED_MAX_TEXTS + 44).map(|n| n.to_string()).collect();
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();

    let vectors = embedder.embed(&borrowed).unwrap();

    assert_eq!(vectors.len(), texts.len());
    assert_eq!(daemon.embed_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn it_refuses_a_daemon_whose_model_is_not_running() {
    for state in [ModelState::Stopped, ModelState::Loading, ModelState::Error] {
        let daemon = stub(state);
        let error = RemoteEmbedder::connect(&daemon.base_url, EMBED_TIMEOUT)
            .err()
            .expect("no embedder");
        assert!(error.to_string().contains(state.as_str()), "{error}");
    }
}

#[test]
fn it_refuses_an_absent_daemon() {
    let dead_port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let started = Instant::now();

    let result = RemoteEmbedder::connect(&format!("http://127.0.0.1:{dead_port}"), EMBED_TIMEOUT);

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_model_stopped_after_the_probe_makes_embedding_fail_instead_of_hanging() {
    let daemon = stub(ModelState::Running);
    let embedder = RemoteEmbedder::connect(&daemon.base_url, EMBED_TIMEOUT).unwrap();
    daemon.refuse_embed.store(true, Ordering::SeqCst);

    let error = embedder.embed(&["coyote"]).unwrap_err();

    assert!(error.to_string().contains("409"), "{error}");
}

#[test]
fn a_hung_daemon_costs_the_caller_at_most_its_timeout() {
    let daemon = stub(ModelState::Running);
    let embedder = RemoteEmbedder::connect(&daemon.base_url, Duration::from_millis(300)).unwrap();
    daemon.stalled.store(true, Ordering::SeqCst);
    let started = Instant::now();

    let outcome = embedder.embed(&["coyote"]);

    assert!(outcome.is_err());
    assert!(started.elapsed() < PATIENCE, "{:?}", started.elapsed());
}

#[test]
fn a_hung_daemon_fails_the_probe_quickly() {
    let daemon = stub(ModelState::Running);
    daemon.stalled.store(true, Ordering::SeqCst);
    let started = Instant::now();

    let outcome = RemoteEmbedder::connect(&daemon.base_url, EMBED_TIMEOUT);

    assert!(outcome.is_err());
    assert!(started.elapsed() < PATIENCE, "{:?}", started.elapsed());
}
