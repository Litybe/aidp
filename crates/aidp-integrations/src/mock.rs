//! Local HTTP mock server emulating the GitHub REST routes the flows call. State is held in
//! memory and wiped by `POST /__reset`.
//!
//! Started by `aidp mock --addr 127.0.0.1:8765 --data fixtures/mocks`.
//!
//! Programmatic entry point: [`serve_on`] accepts a bound [`std::net::TcpListener`]
//! and returns a [`MockHandle`] carrying the really-bound port and address.
//!
//! Endpoints:
//!   POST /github/repos/<owner>/<repo>/pulls          (PR create)
//!   GET  /github/repos/<owner>/<repo>/pulls/<n>
//!   GET  /github/repos/<owner>/<repo>/pulls/<n>/requested_reviewers
//!   POST /github/repos/<owner>/<repo>/pulls/<n>/requested_reviewers
//!   GET  /github/repos/<owner>/<repo>/contents/<path>
//!   GET  /github/repos/<owner>/<repo>/issues?state=open|closed|all
//!   GET  /github/repos/<owner>/<repo>/issues/<n>
//!   GET  /github/repos/<owner>/<repo>/issues/<n>/dependencies/blocked_by
//!   POST /github/repos/<owner>/<repo>/issues/<n>/labels        ({"labels": [..]})
//!   DELETE /github/repos/<owner>/<repo>/issues/<n>/labels/<name>
//!   GET  /github/repos/<owner>/<repo>/issues/<n>/comments      (posted comments, oldest first)
//!   POST /github/repos/<owner>/<repo>/issues/<n>/comments      ({"body": ".."})
//!   POST /__seed_issues                              ({"repo": "o/r", "issues": [..]})
//!   GET  /__state                                    (debug: full state dump)
//!   POST /__reset                                    (test isolation)
//!   POST /__seed, POST /__inject_failure             (test setup)

use crate::models::constants::mock::{
    BASE64_ALPHABET, BASE64_ENCODING, FILE_TYPE, INJECTED_FAILURE_MESSAGE, INTERNAL_ROUTE_PREFIX,
    NOT_FOUND_MESSAGE,
};
use crate::models::mock_error::MockError;
use crate::models::mock_error_messages::MockErrorMessages;
use crate::models::mock_file_contents::FileContents;
use crate::models::mock_flow_state::FlowState;
use crate::models::mock_injected_failure::InjectedFailure;
use crate::models::mock_login::Login;
use crate::models::mock_message::MockMessage;
use crate::models::mock_ok::MockOk;
use crate::models::mock_recorded_request::RecordedRequest;
use crate::models::mock_request_reviewers_body::RequestReviewersBody;
use crate::models::mock_requested_reviewers_response::RequestedReviewersResponse;
use crate::models::mock_reviewers_view::ReviewersView;
use crate::models::mock_seed_request::SeedRequest;
use crate::models::mock_team_slug::TeamSlug;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tiny_http::{Method, Response, Server, StatusCode};

fn base64_encode(bytes: &[u8]) -> String {
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let packed = (first << 16) | (second << 8) | third;
        for sextet_index in 0..4usize {
            if sextet_index <= chunk.len() {
                let sextet = (packed >> (18 - 6 * sextet_index)) & 0x3f;
                encoded.push(BASE64_ALPHABET[sextet as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

fn to_body<T: Serialize>(status: u16, payload: &T) -> anyhow::Result<(u16, Value)> {
    Ok((status, serde_json::to_value(payload)?))
}

/// A 404 reply with a GitHub-style message.
fn not_found_message() -> anyhow::Result<(u16, Value)> {
    to_body(
        404,
        &MockMessage {
            message: NOT_FOUND_MESSAGE.to_string(),
        },
    )
}

/// A 400 reply carrying the parse error text.
fn bad_request(error: serde_json::Error) -> anyhow::Result<(u16, Value)> {
    to_body(
        400,
        &MockError {
            error: error.to_string(),
        },
    )
}

/// The 200 acknowledgement of the internal routes.
fn acknowledged() -> anyhow::Result<(u16, Value)> {
    to_body(200, &MockOk { ok: true })
}

fn seed_flow(state: &mut State, seed: SeedRequest) {
    for file in seed.workflow_files {
        let key = format!("{}/{}/{}", file.owner, file.repo, file.path);
        state.flow.workflow_files.insert(key, file.content);
    }
}

/// The value of query parameter `name` in `url`.
fn query_value(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// Decodes the `%XX` escapes of one path segment.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Seeds issues: `{"repo": "o/r", "issues": [{number, title, body, state, labels: [names],
/// milestone, blocked_by}]}`; labels become `{name}` objects like the REST API shows them.
fn seed_issues(state: &mut State, seed: &Value) {
    let repo = seed["repo"].as_str().unwrap_or_default();
    for issue in seed["issues"].as_array().into_iter().flatten() {
        let number = issue["number"].as_u64().unwrap_or(0);
        let mut shown = issue.clone();
        let names: Vec<Value> = issue["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|name| json!({ "name": name }))
            .collect();
        shown["labels"] = json!(names);
        if shown.get("state").is_none() {
            shown["state"] = json!("open");
        }
        shown["html_url"] = json!(format!("http://mock/github/{repo}/issues/{number}"));
        state.flow.issues.insert(format!("{repo}/{number}"), shown);
    }
}

/// Routes the flow-action endpoints; `None` means the URL belongs to the main router.
fn handle_flow(
    method: &Method,
    url: &str,
    parts: &[&str],
    body: &str,
    state: &mut State,
) -> Option<anyhow::Result<(u16, Value)>> {
    let result = match (method, parts) {
        (Method::Post, ["__seed"]) => match serde_json::from_str::<SeedRequest>(body) {
            Ok(seed) => {
                seed_flow(state, seed);
                acknowledged()
            }
            Err(error) => bad_request(error),
        },
        (Method::Post, ["__inject_failure"]) => {
            match serde_json::from_str::<InjectedFailure>(body) {
                Ok(failure) => {
                    state.flow.failures.push(failure);
                    acknowledged()
                }
                Err(error) => bad_request(error),
            }
        }
        (Method::Post, ["__seed_issues"]) => match serde_json::from_str::<Value>(body) {
            Ok(seed) => {
                seed_issues(state, &seed);
                acknowledged()
            }
            Err(error) => bad_request(error),
        },
        (Method::Get, ["github", "repos", owner, repo, "issues"]) => {
            let wanted = query_value(url, "state").unwrap_or_else(|| "open".to_string());
            let prefix = format!("{owner}/{repo}/");
            let mut found: Vec<(u64, Value)> = state
                .flow
                .issues
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .filter(|(_, issue)| {
                    wanted == "all" || issue.get("state").and_then(Value::as_str) == Some(&wanted)
                })
                .map(|(_, issue)| (issue["number"].as_u64().unwrap_or(0), issue.clone()))
                .collect();
            found.sort_by_key(|(number, _)| std::cmp::Reverse(*number));
            to_body(200, &found.into_iter().map(|(_, v)| v).collect::<Vec<_>>())
        }
        (Method::Get, ["github", "repos", owner, repo, "issues", number]) => {
            match state.flow.issues.get(&format!("{owner}/{repo}/{number}")) {
                Some(issue) => to_body(200, issue),
                None => not_found_message(),
            }
        }
        (
            Method::Get,
            ["github", "repos", owner, repo, "issues", number, "dependencies", "blocked_by"],
        ) => match state.flow.issues.get(&format!("{owner}/{repo}/{number}")) {
            Some(issue) => to_body(200, &issue.get("blocked_by").cloned().unwrap_or(json!([]))),
            None => not_found_message(),
        },
        (Method::Post, ["github", "repos", owner, repo, "issues", number, "labels"]) => {
            let request: Value = serde_json::from_str(body).unwrap_or_default();
            let Some(issue) = state
                .flow
                .issues
                .get_mut(&format!("{owner}/{repo}/{number}"))
            else {
                return Some(not_found_message());
            };
            let labels = issue["labels"].as_array_mut().expect("seeded labels");
            for name in request["labels"].as_array().into_iter().flatten() {
                if !labels.iter().any(|label| label["name"] == *name) {
                    labels.push(json!({ "name": name }));
                }
            }
            to_body(200, &labels.clone())
        }
        (Method::Delete, ["github", "repos", owner, repo, "issues", number, "labels", name]) => {
            let name = percent_decode(name);
            let Some(issue) = state
                .flow
                .issues
                .get_mut(&format!("{owner}/{repo}/{number}"))
            else {
                return Some(not_found_message());
            };
            let labels = issue["labels"].as_array_mut().expect("seeded labels");
            let before = labels.len();
            labels.retain(|label| label["name"].as_str() != Some(name.as_str()));
            if labels.len() == before {
                return Some(not_found_message());
            }
            to_body(200, &labels.clone())
        }
        (Method::Get, ["github", "repos", owner, repo, "issues", number, "comments"]) => {
            let key = format!("{owner}/{repo}/{number}");
            let listed: Vec<Value> = state
                .flow
                .issue_comments
                .get(&key)
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, body)| json!({ "id": index + 1, "body": body }))
                .collect();
            to_body(200, &listed)
        }
        (Method::Post, ["github", "repos", owner, repo, "issues", number, "comments"]) => {
            let request: Value = serde_json::from_str(body).unwrap_or_default();
            let key = format!("{owner}/{repo}/{number}");
            if !state.flow.issues.contains_key(&key) {
                return Some(not_found_message());
            }
            let text = request["body"].as_str().unwrap_or_default().to_string();
            let comments = state.flow.issue_comments.entry(key).or_default();
            comments.push(text.clone());
            let id = comments.len();
            to_body(
                201,
                &json!({
                    "id": id,
                    "body": text,
                    "html_url": format!("http://mock/github/{owner}/{repo}/issues/{number}#comment-{id}")
                }),
            )
        }
        (Method::Get, ["github", "repos", owner, repo, "pulls", number, "requested_reviewers"]) => {
            let set = state
                .flow
                .reviewers
                .get(&format!("{owner}/{repo}/{number}"))
                .cloned()
                .unwrap_or_default();
            to_body(
                200,
                &ReviewersView {
                    users: set.users.into_iter().map(|login| Login { login }).collect(),
                    teams: set
                        .teams
                        .into_iter()
                        .map(|slug| TeamSlug { slug })
                        .collect(),
                },
            )
        }
        (
            Method::Post,
            ["github", "repos", owner, repo, "pulls", number, "requested_reviewers"],
        ) => {
            let request: RequestReviewersBody = serde_json::from_str(body).unwrap_or_default();
            let set = state
                .flow
                .reviewers
                .entry(format!("{owner}/{repo}/{number}"))
                .or_default();
            for user in request.reviewers {
                if !set.users.contains(&user) {
                    set.users.push(user);
                }
            }
            for team in request.team_reviewers {
                if !set.teams.contains(&team) {
                    set.teams.push(team);
                }
            }
            let response = RequestedReviewersResponse {
                requested_reviewers: set
                    .users
                    .iter()
                    .map(|login| Login {
                        login: login.clone(),
                    })
                    .collect(),
                requested_teams: set
                    .teams
                    .iter()
                    .map(|slug| TeamSlug { slug: slug.clone() })
                    .collect(),
            };
            state.flow.reviewer_posts += 1;
            to_body(201, &response)
        }
        (Method::Get, ["github", "repos", owner, repo, "contents", path @ ..]) => {
            let key = format!("{owner}/{repo}/{}", path.join("/"));
            match state.flow.workflow_files.get(&key) {
                Some(content) => to_body(
                    200,
                    &FileContents {
                        kind: FILE_TYPE,
                        encoding: BASE64_ENCODING,
                        content: base64_encode(content.as_bytes()),
                    },
                ),
                None => not_found_message(),
            }
        }
        _ => return None,
    };
    Some(result)
}

/// The 500 body for an error that escaped a route.
fn error_value(error: &anyhow::Error) -> Value {
    let body = MockError {
        error: error.to_string(),
    };
    serde_json::to_value(body).unwrap_or(Value::Null)
}

/// Handle to a running mock HTTP server, holding the bound address and server instance.
///
/// Can be dropped to cleanly shut down the server, or use [`stop`](Self::stop) explicitly.
pub struct MockHandle {
    server: Arc<Server>,
    addr: SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MockHandle {
    /// Returns the port the server is listening on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Returns the address as a string (e.g., `127.0.0.1:8765`).
    pub fn addr(&self) -> String {
        format!("{}", self.addr)
    }

    /// Returns the base URL for the mock server (e.g., `http://127.0.0.1:8765`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Stops the server gracefully by unblocking and joining the accept thread.
    pub fn stop(mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Block until the server thread exits (for use in `serve()`).
    fn block_until_done(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MockHandle {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Internal server state for all mock endpoints.
///
/// Exposed for the crate's own integration tests; not a stable API.
#[doc(hidden)]
#[derive(Default)]
pub struct State {
    gh_prs: HashMap<String, Value>, // key = "owner/repo/number"
    gh_next_pr: HashMap<String, u64>,
    flow: FlowState,
}

/// Start the mock server on a pre-bound listener and return a handle.
///
/// The server begins accepting connections immediately; this function returns
/// as soon as the listening thread is spawned, not when the first request arrives.
pub fn serve_on(listener: std::net::TcpListener, data: &Path) -> anyhow::Result<MockHandle> {
    std::fs::create_dir_all(data).ok();
    let state = Arc::new(Mutex::new(State::default()));
    let addr = listener.local_addr()?;
    let server = Arc::new(
        Server::from_listener(listener, None).map_err(|e| anyhow::anyhow!("from_listener: {e}"))?,
    );

    let server_clone = Arc::clone(&server);
    let state_clone = Arc::clone(&state);
    let thread = std::thread::spawn(move || {
        for mut req in server_clone.incoming_requests() {
            let url = req.url().to_string();
            let method = req.method().clone();
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let resp = handle(&method, &url, &body, &state_clone);
            let (code, payload) = resp.unwrap_or_else(|e| (500, error_value(&e)));
            let body = serde_json::to_string(&payload).unwrap_or_default();
            let _ = req.respond(
                Response::from_string(body)
                    .with_status_code(StatusCode(code))
                    .with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    )
                    // Defense-in-depth (LOC-49): never leave a client socket
                    // parked on a keep-alive connection the mock won't serve.
                    .with_header("Connection: close".parse::<tiny_http::Header>().unwrap()),
            );
        }
    });

    Ok(MockHandle {
        server,
        addr,
        thread: Some(thread),
    })
}

pub fn serve(addr: &str, data: &Path) -> anyhow::Result<()> {
    let listener =
        std::net::TcpListener::bind(addr).map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
    eprintln!(
        "aidp mock listening on http://{addr}  data={}",
        data.display()
    );
    let mut handle = serve_on(listener, data)?;
    // Block forever by joining the server thread; it runs until unblock() is called.
    handle.block_until_done();
    Ok(())
}

/// Route a request and return (status_code, json_body).
///
/// Exposed for the crate's own integration tests; not a stable API.
#[doc(hidden)]
pub fn handle(
    method: &Method,
    url: &str,
    body: &str,
    state: &Mutex<State>,
) -> anyhow::Result<(u16, Value)> {
    let parts: Vec<&str> = url
        .split('?')
        .next()
        .unwrap_or("")
        .trim_start_matches('/')
        .split('/')
        .collect();

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());

    let is_internal = parts
        .first()
        .is_some_and(|prefix| prefix.starts_with(INTERNAL_ROUTE_PREFIX));
    if !is_internal {
        s.flow.recorded.push(RecordedRequest {
            method: method.to_string(),
            url: url.to_string(),
        });
        if let Some(status) = s.flow.take_failure(url) {
            return to_body(status, &MockErrorMessages::single(INJECTED_FAILURE_MESSAGE));
        }
    }
    if let Some(result) = handle_flow(method, url, parts.as_slice(), body, &mut s) {
        return result;
    }

    match (method, parts.as_slice()) {
        (Method::Get, ["__state"]) => Ok((
            200,
            json!({
                "gh_prs": s.gh_prs,
                "gh_writes": { "requested_reviewers": s.flow.reviewer_posts },
                "requests": s.flow.recorded
            }),
        )),
        (Method::Post, ["__reset"]) => {
            *s = State::default();
            acknowledged()
        }

        // GitHub
        (Method::Post, ["github", "repos", owner, repo, "pulls"]) => {
            let req: Value = serde_json::from_str(body).unwrap_or(json!({}));
            let key = format!("{owner}/{repo}");
            let n = *s
                .gh_next_pr
                .entry(key.clone())
                .and_modify(|x| *x += 1)
                .or_insert(1);
            let pr = json!({
                "number": n, "title": req.get("title"), "body": req.get("body"),
                "head": req.get("head"), "base": req.get("base"),
                "state": "open",
                "html_url": format!("http://mock/github/{owner}/{repo}/pull/{n}")
            });
            s.gh_prs.insert(format!("{owner}/{repo}/{n}"), pr.clone());
            Ok((201, pr))
        }
        (Method::Get, ["github", "repos", owner, repo, "pulls", n]) => {
            match s.gh_prs.get(&format!("{owner}/{repo}/{n}")) {
                Some(pr) => Ok((200, pr.clone())),
                None => Ok((404, json!({}))),
            }
        }

        _ => Ok((404, json!({"error":"no route", "url": url}))),
    }
}

#[cfg(test)]
mod tests {}

#[cfg(test)]
#[path = "tests/units/mock_flow_tests.rs"]
mod mock_flow_tests;
