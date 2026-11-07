//! Shared `{{ ... }}` expression engine for prompt rendering and `when:`/`gate:` evaluation.
//! Grammar: literals, refs, `==`/`!=`/`in`, `| default(..)`, `| length`, `and`/`or`/`not`.

use crate::models::constants::expr::{
    UNPARSED_KIND_SUFFIX, UNPARSED_MID, UNPARSED_PREFIX, UNPARSED_SUFFIX,
};
use crate::models::enums::expression_kind::ExpressionKind;
use crate::pipeline::{Pipeline, Placement, Stage};
use crate::run::RunState;
use serde_json::Value as Json;

pub use crate::models::expr::EvalCtx;

/// Evaluate a stage's `when:`; absent or unparseable means run (fail open).
/// Discards the unparsed warning; use `eval_when_collect` to keep it.
pub fn eval_when(stage: &Stage, state: &RunState, pipe: &Pipeline) -> bool {
    let mut warnings = Vec::new();
    eval_when_collect(stage, state, pipe, &mut warnings)
}

/// Same as `eval_when`, pushing a message into `warnings` when the `when:`
/// expression could not be parsed, instead of printing it.
pub fn eval_when_collect(
    stage: &Stage,
    state: &RunState,
    pipe: &Pipeline,
    warnings: &mut Vec<String>,
) -> bool {
    let ctx = EvalCtx::new(state, pipe);
    match &stage.when {
        None => true,
        Some(v) => match eval_condition(v, &ctx) {
            Some(b) => b,
            None => {
                warnings.push(unparsed_message(ExpressionKind::When, &stage.name, v));
                true
            }
        },
    }
}

/// Build the warning for an unparseable `when:`/`gate:` expression.
/// Does not change the returned decision.
fn unparsed_message(kind: ExpressionKind, stage: &str, v: &serde_yaml::Value) -> String {
    let shown = match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    };
    let kind = kind.as_str();
    format!("{UNPARSED_PREFIX}{stage}{UNPARSED_MID}{kind}{UNPARSED_KIND_SUFFIX}{shown}{UNPARSED_SUFFIX}")
}

/// Evaluate a loop container's `when:`, redirecting refs to the loop body's artifact.
/// Discards the unparsed warning; use `eval_loop_when_collect` to keep it.
pub fn eval_loop_when(container: &Stage, state: &RunState, pipe: &Pipeline) -> bool {
    let mut warnings = Vec::new();
    eval_loop_when_collect(container, state, pipe, &mut warnings)
}

/// Same as `eval_loop_when`, pushing a message into `warnings` when the
/// `when:` expression could not be parsed, instead of printing it.
pub fn eval_loop_when_collect(
    container: &Stage,
    state: &RunState,
    pipe: &Pipeline,
    warnings: &mut Vec<String>,
) -> bool {
    let ctx = EvalCtx::new(state, pipe).with_loop_body(Some(&container.body));
    match &container.when {
        None => true,
        Some(v) => match eval_condition(v, &ctx) {
            Some(b) => b,
            None => {
                warnings.push(unparsed_message(
                    ExpressionKind::WhenLoop,
                    &container.name,
                    v,
                ));
                true
            }
        },
    }
}

/// Evaluate a `gate:`; unevaluable expressions fail safe and count as a gate.
/// Discards the unparsed warning; use `eval_gate_collect` to keep it.
pub fn eval_gate(stage: &Stage, state: &RunState, pipe: &Pipeline) -> bool {
    let mut warnings = Vec::new();
    eval_gate_collect(stage, state, pipe, &mut warnings)
}

/// Same as `eval_gate`, pushing a message into `warnings` when the `gate:`
/// expression could not be parsed, instead of printing it.
pub fn eval_gate_collect(
    stage: &Stage,
    state: &RunState,
    pipe: &Pipeline,
    warnings: &mut Vec<String>,
) -> bool {
    let ctx = EvalCtx::new(state, pipe);
    match &stage.gate {
        None | Some(serde_yaml::Value::Null) => false,
        Some(serde_yaml::Value::Bool(b)) => *b,
        Some(other) => match eval_condition(other, &ctx) {
            Some(b) => b,
            None => {
                warnings.push(unparsed_message(ExpressionKind::Gate, &stage.name, other));
                true
            } // fail safe
        },
    }
}

/// Resolve `max_iterations` to a concrete count. Falls back to
/// `settings.max_correction_iterations` when absent or unparseable.
pub fn eval_max_iterations(container: &Stage, state: &RunState, pipe: &Pipeline) -> u32 {
    let fallback = pipe.settings.max_correction_iterations;
    let ctx = EvalCtx::new(state, pipe);
    match &container.max_iterations {
        None => fallback,
        Some(serde_yaml::Value::Number(n)) => n.as_u64().map(|v| v as u32).unwrap_or(fallback),
        Some(serde_yaml::Value::String(s)) => {
            let t = s.trim();
            let inner = strip_braces(t);
            match eval_value(inner, &ctx) {
                Json::Number(n) => n.as_u64().map(|v| v as u32).unwrap_or(fallback),
                Json::String(s) => s.trim().parse::<u32>().unwrap_or(fallback),
                _ => fallback,
            }
        }
        _ => fallback,
    }
}

/// Render one `{{ ... }}` token: output refs become a path or JSON scalar.
/// Unknown refs pass through as `{{ expr }}`.
pub fn render_token(expr: &str, ctx: &EvalCtx) -> String {
    let parts: Vec<&str> = expr.split('.').map(str::trim).collect();
    if let ["stages", stage, "outputs", name] = parts.as_slice() {
        return output_path(ctx, stage, name).to_string_lossy().into_owned();
    }
    match resolve_ref(&parts, ctx) {
        Json::Null => format!("{{{{ {expr} }}}}"),
        Json::String(s) => s,
        other => other.to_string(),
    }
}

/// Evaluate a YAML `when:`/`gate:` value to a bool. `None` => could not parse.
pub fn eval_condition(v: &serde_yaml::Value, ctx: &EvalCtx) -> Option<bool> {
    match v {
        serde_yaml::Value::Bool(b) => Some(*b),
        serde_yaml::Value::Null => Some(false),
        serde_yaml::Value::Number(n) => Some(n.as_f64().map(|f| f != 0.0).unwrap_or(false)),
        serde_yaml::Value::String(s) => {
            let inner = strip_braces(s.trim());
            eval_bool_expr(inner, ctx)
        }
        _ => None,
    }
}

fn eval_bool_expr(expr: &str, ctx: &EvalCtx) -> Option<bool> {
    let e = expr.trim();
    if e.is_empty() {
        return None;
    }
    if let Some(idx) = e.find(" or ") {
        let l = eval_bool_expr(&e[..idx], ctx)?;
        let r = eval_bool_expr(&e[idx + 4..], ctx)?;
        return Some(l || r);
    }
    if let Some(idx) = e.find(" and ") {
        let l = eval_bool_expr(&e[..idx], ctx)?;
        let r = eval_bool_expr(&e[idx + 5..], ctx)?;
        return Some(l && r);
    }
    if let Some(rest) = e.strip_prefix("not ") {
        return Some(!eval_bool_expr(rest.trim(), ctx)?);
    }
    if let Some(idx) = e.find(" in ") {
        let lhs = &e[..idx];
        let rhs = &e[idx + 4..];
        let val = eval_value(lhs, ctx);
        let list = parse_list(rhs);
        return Some(list.iter().any(|item| json_eq(&val, item)));
    }
    if let Some(idx) = e.find("!=") {
        let lhs = eval_value(&e[..idx], ctx);
        let rhs = parse_literal(&e[idx + 2..]);
        return Some(!json_eq(&lhs, &rhs));
    }
    if let Some(idx) = e.find("==") {
        let lhs = eval_value(&e[..idx], ctx);
        let rhs = parse_literal(&e[idx + 2..]);
        return Some(json_eq(&lhs, &rhs));
    }
    for (op, len) in [(">=", 2), ("<=", 2), (">", 1), ("<", 1)] {
        if let Some(idx) = e.find(op) {
            let l = as_f64(&eval_value(&e[..idx], ctx));
            let r = as_f64(&eval_value(&e[idx + len..], ctx));
            if let (Some(l), Some(r)) = (l, r) {
                return Some(match op {
                    ">=" => l >= r,
                    "<=" => l <= r,
                    ">" => l > r,
                    _ => l < r,
                });
            }
            return None; // operator present but operands aren't numeric
        }
    }
    Some(truthy(&eval_value(e, ctx)))
}

fn as_f64(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Resolve one operand to a JSON value, honouring a trailing `| default(LIT)`.
fn eval_value(s: &str, ctx: &EvalCtx) -> Json {
    let s = s.trim();
    if let Some(pipe_idx) = s.find('|') {
        let lhs = s[..pipe_idx].trim();
        let filter = s[pipe_idx + 1..].trim();
        let base = eval_value(lhs, ctx);
        if let Some(arg) = filter
            .strip_prefix("default(")
            .and_then(|r| r.strip_suffix(')'))
        {
            return if base.is_null() {
                parse_literal(arg)
            } else {
                base
            };
        }
        if filter == "length" || filter == "count" {
            return match &base {
                Json::Array(a) => Json::from(a.len()),
                Json::String(s) => Json::from(s.chars().count()),
                Json::Object(m) => Json::from(m.len()),
                _ => Json::Null,
            };
        }
        return base;
    }
    if is_literal(s) {
        return parse_literal(s);
    }
    let parts: Vec<&str> = s.split('.').map(str::trim).collect();
    resolve_ref(&parts, ctx)
}

/// Resolve a dotted reference to a JSON value.
pub fn resolve_ref(parts: &[&str], ctx: &EvalCtx) -> Json {
    match parts {
        ["run_id"] => Json::String(ctx.state.run_id.clone()),

        ["feature"] => crate::run::feature_key(&ctx.state.inputs)
            .map(Json::String)
            .unwrap_or(Json::Null),

        ["inputs", k] => input_value(ctx, k),
        ["inputs", k, rest @ ..] => index_json(input_value(ctx, k), rest),

        ["settings", k] => setting_value(ctx, k),

        ["stages", stage, "outputs", name] => {
            Json::String(output_path(ctx, stage, name).to_string_lossy().into_owned())
        }
        ["stages", stage, "outputs", name, rest @ ..] => {
            let path = output_path(ctx, stage, name);
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Json>(&t).ok())
            {
                Some(doc) => index_json(doc, rest),
                None => Json::Null,
            }
        }
        _ => Json::Null,
    }
}

fn input_value(ctx: &EvalCtx, key: &str) -> Json {
    if let Some(v) = ctx.state.inputs.get(key) {
        return v.clone();
    }
    ctx.pipe
        .inputs
        .get(key)
        .and_then(|spec| spec.default.as_ref())
        .map(yaml_to_json)
        .unwrap_or(Json::Null)
}

fn setting_value(ctx: &EvalCtx, key: &str) -> Json {
    match key {
        "max_correction_iterations" => Json::from(ctx.pipe.settings.max_correction_iterations),
        other => ctx
            .pipe
            .settings
            .extra
            .get(other)
            .map(yaml_to_json)
            .unwrap_or(Json::Null),
    }
}

/// Resolve the on-disk path of `stages.<stage>.outputs.<name>`.
/// Prefers an existing artifact and honours loop-body redirection.
fn output_path(ctx: &EvalCtx, stage: &str, name: &str) -> std::path::PathBuf {
    if let Some(body) = ctx.loop_body {
        for b in body.iter().rev() {
            if declares_output(b, name) {
                if let Some(p) = existing_artifact(ctx, &b.name, b, name) {
                    return p;
                }
            }
        }
    }
    let producer = find_stage_in_pipe(ctx.pipe, stage);
    if let Some(p) = producer.and_then(|st| existing_artifact(ctx, stage, st, name)) {
        return p;
    }
    let filename = producer
        .and_then(|st| declared_filename(st, name))
        .unwrap_or_else(|| name.to_string());
    let placement = producer.map(|st| st.placement()).unwrap_or(Placement::Run);
    ctx.state.stage_dir(stage, placement).join(filename)
}

/// First existing candidate artifact for `name` produced by `stage_def`,
/// written under `dir_name`'s stage directory.
fn existing_artifact(
    ctx: &EvalCtx,
    dir_name: &str,
    stage_def: &Stage,
    name: &str,
) -> Option<std::path::PathBuf> {
    let dir = ctx.state.stage_dir(dir_name, stage_def.placement());
    if let Some(filename) = declared_filename(stage_def, name) {
        let p = dir.join(&filename);
        if p.exists() {
            return Some(p);
        }
    }
    let bare = dir.join(name);
    if bare.exists() {
        return Some(bare);
    }
    None
}

/// The raw declared output string (with extension) whose stripped name == `name`.
fn declared_filename(stage: &Stage, name: &str) -> Option<String> {
    for o in &stage.outputs {
        let raw = match o {
            serde_yaml::Value::String(s) => s.clone(),
            serde_yaml::Value::Mapping(m) => m
                .get(serde_yaml::Value::String("name".into()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())?,
            _ => continue,
        };
        if strip_ext(&raw) == name {
            return Some(raw);
        }
    }
    None
}

fn declares_output(stage: &Stage, name: &str) -> bool {
    stage.output_names().iter().any(|o| o == name)
}

fn find_stage_in_pipe<'a>(pipe: &'a Pipeline, name: &str) -> Option<&'a Stage> {
    for s in &pipe.stages {
        if s.name == name {
            return Some(s);
        }
        for b in &s.body {
            if b.name == name {
                return Some(b);
            }
        }
    }
    None
}

fn strip_braces(s: &str) -> &str {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
        inner.trim()
    } else {
        t
    }
}

fn index_json(mut v: Json, props: &[&str]) -> Json {
    for p in props {
        v = match v {
            Json::Object(mut m) => m.remove(*p).unwrap_or(Json::Null),
            Json::Array(a) => match p.parse::<usize>() {
                Ok(i) => a.get(i).cloned().unwrap_or(Json::Null),
                Err(_) => return Json::Null,
            },
            _ => return Json::Null,
        };
    }
    v
}

fn is_literal(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    matches!(s, "true" | "false")
        || s.starts_with('\'')
        || s.starts_with('"')
        || s.parse::<f64>().is_ok()
}

fn parse_literal(s: &str) -> Json {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
        || (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
    {
        return Json::String(s[1..s.len() - 1].to_string());
    }
    match s {
        "true" => Json::Bool(true),
        "false" => Json::Bool(false),
        "null" => Json::Null,
        _ => {
            if let Ok(i) = s.parse::<i64>() {
                Json::from(i)
            } else if let Ok(f) = s.parse::<f64>() {
                Json::from(f)
            } else {
                Json::String(s.to_string())
            }
        }
    }
}

fn parse_list(s: &str) -> Vec<Json> {
    let t = s.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(t);
    inner
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(parse_literal)
        .collect()
}

fn json_eq(a: &Json, b: &Json) -> bool {
    if a == b {
        return true;
    }
    json_scalar_string(a) == json_scalar_string(b)
}

fn json_scalar_string(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

/// The engine's single notion of truth, shared by `when:` guards and alias coercion.
pub(crate) fn truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Json::String(s) => !s.is_empty() && s != "false" && s != "0",
        Json::Array(a) => !a.is_empty(),
        Json::Object(m) => !m.is_empty(),
    }
}

fn yaml_to_json(v: &serde_yaml::Value) -> Json {
    serde_json::to_value(v).unwrap_or(Json::Null)
}

fn strip_ext(s: &str) -> String {
    match s.rsplit_once('.') {
        Some((n, _)) => n.to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
#[path = "../tests/unit/expr_eval_tests.rs"]
mod expr_eval_tests;

#[cfg(test)]
#[path = "../tests/unit/expr_tests.rs"]
mod expr_collect_tests;
