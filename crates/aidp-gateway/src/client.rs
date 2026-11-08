//! One-shot CLI verbs: thin HTTP clients for the running daemon.

use crate::cli_api::TextView;
use crate::flows::http_routes::{
    FlowVerb, BODY_FLOW, BODY_OK, BODY_REPORT, BODY_SETTINGS, BODY_TICKET, STEPS_SEGMENT,
};
use crate::models::enums::PrReference;
use crate::models::gateway_http_error::GatewayHttpError;
use crate::models::gateway_unreachable::GatewayUnreachable;
use crate::models::{AdoptRequest, StartBody, StartOrigin, WorkspaceRequest};
use crate::routes::{DISPATCHER_BUDGET, DISPATCHER_PLAN, DISPATCHER_TICK, PROJECTS, TICKETS};
use anyhow::anyhow;
use serde_json::{json, Value};

const UNAUTHORIZED_STATUS: u16 = 401;
const ENGINE_PIPELINE: &str = "v7";
const FIELD_ENGINE: &str = "engine";
const ENGINE_PROMPT: &str = "prompt";
const ORIGIN_KIND_API: &str = "api";
const ORIGIN_LABEL_CLI: &str = "cli";

fn base(url: &Option<String>) -> String {
    url.clone()
        .or_else(|| {
            std::env::var("AIDP_GATEWAY_URL")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "http://127.0.0.1:3001".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// The `Authorization` header value for a UI/API token, or `None` when the
/// token is empty/blank (never send `Authorization: Bearer ` with nothing
/// after it — that reads as a malformed credential rather than none).
fn auth_header_value(token: &str) -> Option<String> {
    let t = token.trim();
    if t.is_empty() {
        None
    } else {
        Some(format!("Bearer {t}"))
    }
}

/// The UI/API token this client presents, from `AIDP_UI_TOKEN` (empty = none).
fn ui_token() -> String {
    std::env::var("AIDP_UI_TOKEN").unwrap_or_default()
}

/// Attach the token the same way the daemon reads it: `Authorization: Bearer <t>`.
fn with_auth(req: ureq::Request) -> ureq::Request {
    match auth_header_value(&ui_token()) {
        Some(v) => req.set("Authorization", &v),
        None => req,
    }
}

fn parse(r: ureq::Response) -> anyhow::Result<Value> {
    let s = r.into_string()?;
    if s.trim().is_empty() {
        Ok(json!({}))
    } else {
        Ok(serde_json::from_str(&s)?)
    }
}

fn call(req: ureq::Request, body: Option<Value>) -> anyhow::Result<Value> {
    let resp = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match resp {
        Ok(r) => parse(r),
        Err(ureq::Error::Status(UNAUTHORIZED_STATUS, r)) => {
            let v = parse(r).unwrap_or_else(|_| json!({}));
            Err(GatewayHttpError {
                status: UNAUTHORIZED_STATUS,
                message: format!(
                    "HTTP {UNAUTHORIZED_STATUS}: {} — set AIDP_UI_TOKEN to the daemon's token (off-loopback callers must present it)",
                    v["error"].as_str().unwrap_or("unauthorized")
                ),
            }
            .into())
        }
        Err(ureq::Error::Status(code, r)) => {
            let v = parse(r).unwrap_or_else(|_| json!({}));
            Err(GatewayHttpError {
                status: code,
                message: format!("HTTP {code}: {}", v["error"].as_str().unwrap_or("error")),
            }
            .into())
        }
        Err(e) => Err(GatewayUnreachable {
            message: format!("gateway unreachable ({e}) — is `aidp gateway serve` running?"),
        }
        .into()),
    }
}

pub(crate) fn get(url: &Option<String>, path: &str) -> anyhow::Result<Value> {
    call(
        with_auth(crate::net::agent().get(&format!("{}{}", base(url), path))),
        None,
    )
}
pub(crate) fn post(url: &Option<String>, path: &str, body: Value) -> anyhow::Result<Value> {
    call(
        with_auth(crate::net::agent().post(&format!("{}{}", base(url), path))),
        Some(body),
    )
}
/// `post` with the long read timeout, for daemon calls that do slow git work.
pub(crate) fn post_long(url: &Option<String>, path: &str, body: Value) -> anyhow::Result<Value> {
    call(
        with_auth(crate::net::long_agent().post(&format!("{}{}", base(url), path))),
        Some(body),
    )
}
pub(crate) fn put(url: &Option<String>, path: &str, body: Value) -> anyhow::Result<Value> {
    call(
        with_auth(crate::net::agent().put(&format!("{}{}", base(url), path))),
        Some(body),
    )
}
pub(crate) fn delete(url: &Option<String>, path: &str) -> anyhow::Result<Value> {
    call(
        with_auth(crate::net::agent().delete(&format!("{}{}", base(url), path))),
        None,
    )
}

pub(crate) fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

fn pct(v: &Value) -> String {
    v.as_f64()
        .map(|u| format!("{}%", (u * 100.0).round()))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn text_status(v: &Value) -> String {
    format!(
        "sessions {} running / {} total · automations {}/{} enabled · webhooks {} · 5h {} · 7d {}\nskill {} · vault {} · state {}",
        s(&v["sessions"]["running"]),
        s(&v["sessions"]["total"]),
        s(&v["automations"]["enabled"]),
        s(&v["automations"]["total"]),
        if v["hooksEnabled"].as_bool().unwrap_or(false) { "on" } else { "off (AIDP_HOOK_TOKEN)" },
        pct(&v["rateLimit"]["fiveHour"]["utilization"]),
        pct(&v["rateLimit"]["sevenDay"]["utilization"]),
        s(&v["skillRoot"]),
        s(&v["vault"]),
        s(&v["stateDir"]),
    )
}

pub(crate) fn text_sessions(v: &Value) -> String {
    let rows: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    let tok = x["usage"]
                        .as_object()
                        .map(|u| {
                            let t: u64 = ["input", "output", "cacheRead", "cacheCreate"]
                                .iter()
                                .map(|k| u[*k].as_u64().unwrap_or(0))
                                .sum();
                            format!("{}k tok", t / 1000)
                        })
                        .unwrap_or_else(|| "-".to_string());
                    format!(
                        "{:<9} {:<8} {:<22} {:<22} {:<6} {:>8}  {}",
                        s(&x["status"]),
                        s(&x["origin"]["kind"]),
                        head(&s(&x["templateId"]), 22),
                        head(&s(&x["projectName"]), 22),
                        s(&x["turns"]),
                        tok,
                        s(&x["id"])
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        "no sessions".to_string()
    } else {
        rows.join("\n")
    }
}

pub(crate) fn text_automations(v: &Value) -> String {
    let rows: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    format!(
                        "{} {:<28} {:<8} → {:<8} runs={:<3} last={:<8} next={}  {}",
                        if x["enabled"].as_bool().unwrap_or(false) {
                            "on "
                        } else {
                            "off"
                        },
                        head(&s(&x["name"]), 28),
                        s(&x["trigger"]["kind"]),
                        s(&x["action"]["kind"]),
                        s(&x["runCount"]),
                        s(&x["lastStatus"]),
                        s(&x["nextRunAt"]),
                        s(&x["id"])
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        "no automations".to_string()
    } else {
        rows.join("\n")
    }
}

pub(crate) fn text_presets(v: &Value) -> String {
    let rows: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    format!(
                        "{:<28} {:<22} {:<12} {:<12}  {}",
                        head(&s(&x["name"]), 28),
                        head(&s(&x["templateId"]), 22),
                        s(&x["target"]),
                        s(&x["projectId"]),
                        s(&x["id"])
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        "no presets".to_string()
    } else {
        rows.join("\n")
    }
}

pub(crate) fn text_queue(v: &Value) -> String {
    let rows: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| {
                    format!(
                        "{:<12} {:<22} {:<16} {:<8} prio={}",
                        s(&x["id"]),
                        head(&s(&x["projectName"]), 22),
                        head(&s(&x["templateId"]), 16),
                        s(&x["target"]),
                        s(&x["priority"]),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        "no queued launches".to_string()
    } else {
        rows.join("\n")
    }
}

/// Renders `GET /api/dispatcher` (mode, slots, last tick, one line per
/// running ticket and one per skip reason). Reused for `dispatcher tick` /
/// `dispatcher tick --dry-run`, whose reports are shaped a little
/// differently (`launched`/`launches` instead of `running`, a flat
/// `skipped` instead of `lastPlan.skipped`) — every lookup below falls back
/// across those shapes rather than assuming one, so no field ever panics.
pub(crate) fn text_dispatcher(v: &Value) -> String {
    let slots_used = if v["slots"]["used"].is_null() {
        &v["slotsUsed"]
    } else {
        &v["slots"]["used"]
    };
    let slots_max = if v["slots"]["max"].is_null() {
        &v["slotsMax"]
    } else {
        &v["slots"]["max"]
    };
    let mut lines = vec![format!(
        "mode {} · slots {}/{} · last tick {} ({})",
        s(&v["mode"]),
        s(slots_used),
        s(slots_max),
        s(&v["lastTickAt"]),
        s(&v["lastReason"]),
    )];

    for r in v["running"].as_array().into_iter().flatten() {
        lines.push(format!(
            "running  {:<14} {:<22} session={}",
            s(&r["key"]),
            head(&s(&r["project"]), 22),
            s(&r["sessionId"]),
        ));
    }

    let launched = v["lastPlan"]["launched"]
        .as_array()
        .or_else(|| v["launched"].as_array());
    for k in launched.into_iter().flatten() {
        lines.push(format!("launched {}", s(k)));
    }
    for l in v["launches"].as_array().into_iter().flatten() {
        lines.push(format!("would-launch {}", s(&l["key"])));
    }

    let skipped = v["lastPlan"]["skipped"]
        .as_array()
        .or_else(|| v["skipped"].as_array());
    for sk in skipped.into_iter().flatten() {
        match sk.as_array() {
            // plan_only()'s skipped entries serialize as `[key, reason]`
            // tuples rather than `{key, reason}` objects.
            Some(pair) if pair.len() == 2 => {
                lines.push(format!("skip     {:<14} {}", s(&pair[0]), s(&pair[1])))
            }
            _ => lines.push(format!(
                "skip     {:<14} {}",
                s(&sk["key"]),
                s(&sk["reason"])
            )),
        }
    }

    lines.join("\n")
}

fn head(x: &str, n: usize) -> String {
    x.chars().take(n).collect()
}

pub(crate) fn resolve_project(url: &Option<String>, id_or_name: &str) -> anyhow::Result<String> {
    let projects = get(url, PROJECTS)?;
    projects
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|p| p["id"] == id_or_name || p["name"] == id_or_name)
        })
        .and_then(|p| p["id"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("unknown project {id_or_name:?} (see `aidp gateway projects list`)"))
}

/// Arguments for [`start_body`] — grouped into a struct because the
/// `POST /api/sessions` body carries this many independent optional fields.
pub(crate) struct StartBodyArgs {
    pub(crate) pid: String,
    pub(crate) template: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) inputs_map: serde_json::Map<String, Value>,
    pub(crate) prompt: Option<String>,
    pub(crate) permission_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) max_budget_usd: Option<f64>,
    pub(crate) context_from: Option<String>,
    pub(crate) keep_open: bool,
    pub(crate) workspace: Option<WorkspaceRequest>,
}

/// Build the `POST /api/sessions` request body for `aidp gateway start`.
///
/// Every optional field is included even when absent (as `null`) rather than
/// omitted — the daemon ignores nulls, and this keeps `contextFrom` in step
/// with the other optional keys (`permissionMode`, `model`, `effort`,
/// `maxBudgetUsd`) already built this way.
pub(crate) fn start_body(args: StartBodyArgs) -> Value {
    let engine = if args.template.is_some() {
        ENGINE_PIPELINE
    } else {
        ENGINE_PROMPT
    };
    let body = StartBody {
        project_id: args.pid,
        engine: engine.to_string(),
        template_id: args.template,
        target: args.target,
        inputs: args.inputs_map,
        prompt: args.prompt,
        permission_mode: args.permission_mode,
        model: args.model,
        effort: args.effort,
        provider: None,
        max_budget_usd: args.max_budget_usd,
        context_from: args.context_from,
        workspace: args.workspace,
        origin: StartOrigin {
            kind: ORIGIN_KIND_API.to_string(),
            label: ORIGIN_LABEL_CLI.to_string(),
        },
        auto_close: !args.keep_open,
    };
    serde_json::to_value(body).unwrap_or(Value::Null)
}

/// Sets the requested engine on a template start body, or drops the default so the daemon picks native.
pub(crate) fn with_template_engine(mut body: Value, engine: Option<String>) -> Value {
    if let Some(fields) = body.as_object_mut() {
        match engine {
            Some(engine) => fields.insert(FIELD_ENGINE.to_string(), Value::String(engine)),
            None => fields.remove(FIELD_ENGINE),
        };
    }
    body
}

/// Start a ticket's session directly (`aidp ticket start <key>`), bypassing
/// the dispatcher's own candidate selection. `force` is always sent; an
/// unset `service`/`template`/`track` is omitted and a set one is persisted.
pub fn ticket_start(
    url: &Option<String>,
    key: &str,
    force: bool,
    service: Option<&str>,
    template: Option<&str>,
    track: Option<&str>,
) -> anyhow::Result<Value> {
    let mut body = json!({ "force": force });
    if let Some(v) = service {
        body["service"] = json!(v);
    }
    if let Some(v) = template {
        body["template"] = json!(v);
    }
    if let Some(v) = track {
        body["track"] = json!(v);
    }
    post(url, &format!("{TICKETS}/{key}/start"), body)
}

/// Adopt an existing pull request (url or number) for a ticket (`aidp ticket adopt`).
pub fn adopt_ticket(url: &Option<String>, key: &str, pr: &str) -> anyhow::Result<Value> {
    let body = serde_json::to_value(AdoptRequest {
        pr: PrReference::Url(pr.to_string()),
    })?;
    post(url, &format!("{TICKETS}/{key}/adopt"), body)
}

/// Run (or plan) one dispatcher cycle (`aidp ticket dispatch`), reusing the
/// same `base`/`with_auth`/`call` plumbing as every other verb — so
/// `AIDP_GATEWAY_URL` and `AIDP_UI_TOKEN` keep working unchanged from
/// `aidp-cli`.
pub fn dispatcher_tick(url: &Option<String>, dry_run: bool) -> anyhow::Result<Value> {
    if dry_run {
        get(url, DISPATCHER_PLAN)
    } else {
        post(url, DISPATCHER_TICK, json!({}))
    }
}

/// Explain the daemon's current token-budget verdict (`aidp ticket budget`),
/// reusing the same `base`/`with_auth`/`call` plumbing as every other verb so
/// `AIDP_GATEWAY_URL` and `AIDP_UI_TOKEN` keep working unchanged.
pub fn dispatcher_budget(url: &Option<String>) -> anyhow::Result<Value> {
    get(url, DISPATCHER_BUDGET)
}

/// The cost report of one ticket (`aidp cost ticket`).
pub fn ticket_cost(url: &Option<String>, key: &str) -> anyhow::Result<Value> {
    get(
        url,
        &format!("{TICKETS}/{key}/{}", crate::routes::TICKET_COST_SEGMENT),
    )
}

const PR_FIX_PATH: &str = "/api/prs/fix";
const WORKTREES_PATH: &str = "/api/worktrees";
const WORKTREES_PRUNE_PATH: &str = "/api/worktrees/prune";
const WORKTREES_ENSURE_PATH: &str = "/api/worktrees/ensure";
const WORKTREES_REMOVE_PATH: &str = "/api/worktrees/remove";
const PROJECT_QUERY_KEY: &str = "project";
const PRUNABLE_KEY: &str = "prunable";

/// The `GET /api/worktrees` path, filtered on one project when given.
pub(crate) fn worktrees_path(project: Option<&str>) -> String {
    match project {
        Some(project) => format!("{WORKTREES_PATH}?{PROJECT_QUERY_KEY}={project}"),
        None => WORKTREES_PATH.to_string(),
    }
}

/// Keeps only the entries flagged prunable in a worktree listing.
pub(crate) fn prunable_entries(listing: &Value) -> Value {
    let entries: Vec<Value> = listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry[PRUNABLE_KEY].as_bool().unwrap_or(false))
        .cloned()
        .collect();
    Value::Array(entries)
}

const FLOWS_PATH: &str = "/api/flows";

/// Lists every flow run known to the daemon (`GET /api/flows`).
pub fn list_flows(url: &Option<String>) -> anyhow::Result<Value> {
    get(url, FLOWS_PATH)
}

/// Reads one flow run (`GET /api/flows/:key`).
pub fn get_flow(url: &Option<String>, key: &str) -> anyhow::Result<Value> {
    get(url, &format!("{FLOWS_PATH}/{key}"))
}

/// Starts a flow for a ticket (`POST /api/flows`). An unset `flow` or
/// `settings` is omitted so the daemon applies its defaults.
pub fn start_flow(
    url: &Option<String>,
    ticket: &str,
    flow: Option<&str>,
    settings: Option<Value>,
) -> anyhow::Result<Value> {
    let mut body = json!({ BODY_TICKET: ticket });
    if let Some(name) = flow {
        body[BODY_FLOW] = json!(name);
    }
    if let Some(values) = settings {
        body[BODY_SETTINGS] = values;
    }
    post(url, FLOWS_PATH, body)
}

/// Applies a run-level verb: `pause`, `resume` or `abort`.
pub fn flow_action(url: &Option<String>, key: &str, verb: FlowVerb) -> anyhow::Result<Value> {
    post(
        url,
        &format!("{FLOWS_PATH}/{key}/{}", verb.as_str()),
        json!({}),
    )
}

/// Applies a step-level verb: `retry`, `skip`, `approve`, `reject` or `click`.
pub fn step_action(
    url: &Option<String>,
    key: &str,
    step: &str,
    verb: FlowVerb,
) -> anyhow::Result<Value> {
    post(
        url,
        &format!(
            "{FLOWS_PATH}/{key}/{STEPS_SEGMENT}/{step}/{}",
            verb.as_str()
        ),
        json!({}),
    )
}

/// Reports the outcome of a step that waits on an external verdict.
pub fn verdict(
    url: &Option<String>,
    key: &str,
    step: &str,
    ok: bool,
    report: Option<&str>,
) -> anyhow::Result<Value> {
    let mut body = json!({ BODY_OK: ok });
    if let Some(text) = report {
        body[BODY_REPORT] = json!(text);
    }
    post(
        url,
        &format!(
            "{FLOWS_PATH}/{key}/{STEPS_SEGMENT}/{step}/{}",
            FlowVerb::Verdict.as_str()
        ),
        body,
    )
}

/// Lists the worktrees the daemon tracks, optionally of one project.
pub fn worktrees_list(url: &Option<String>, project: Option<&str>) -> anyhow::Result<Value> {
    get(url, &worktrees_path(project))
}

/// Removes the prunable worktrees, or with `dry_run` only reports them.
pub fn worktrees_prune(url: &Option<String>, dry_run: bool) -> anyhow::Result<Value> {
    if dry_run {
        Ok(prunable_entries(&get(url, WORKTREES_PATH)?))
    } else {
        post(url, WORKTREES_PRUNE_PATH, json!({}))
    }
}

/// Creates or reuses the worktree of a ticket.
pub fn worktrees_ensure(url: &Option<String>, key: &str) -> anyhow::Result<Value> {
    post_long(url, WORKTREES_ENSURE_PATH, key_body(key)?)
}

/// Saves the unsaved work of a ticket's worktree, then deletes it.
pub fn worktrees_remove(url: &Option<String>, key: &str) -> anyhow::Result<Value> {
    post_long(url, WORKTREES_REMOVE_PATH, key_body(key)?)
}

fn key_body(key: &str) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(
        crate::models::worktree_key_request::WorktreeKeyRequest {
            key: key.to_string(),
        },
    )?)
}

/// Renders `value` for `--format text` according to `view`.
pub(crate) fn render_view(view: TextView, value: &Value) -> String {
    match view {
        TextView::Status => text_status(value),
        TextView::Sessions => text_sessions(value),
        TextView::Automations => text_automations(value),
        TextView::Presets => text_presets(value),
        TextView::Queue => text_queue(value),
        TextView::Dispatcher => text_dispatcher(value),
        TextView::StartedSession => format!(
            "session {} started ({} on {})",
            s(&value["id"]),
            s(&value["templateId"]),
            s(&value["projectName"])
        ),
        TextView::Raw => s(value),
    }
}

#[cfg(test)]
#[path = "../tests/unit/client_tests.rs"]
mod client_tests;

/// Starts a native run on the daemon (`POST /api/runs`).
pub fn start_native_run(url: &Option<String>, body: Value) -> anyhow::Result<Value> {
    post(url, "/api/runs", body)
}

/// POST a PR fix request to the daemon.
pub fn pr_fix(
    url: &Option<String>,
    request: &crate::models::pr_fix_request::PrFixRequest,
) -> anyhow::Result<Value> {
    post(url, PR_FIX_PATH, serde_json::to_value(request)?)
}
