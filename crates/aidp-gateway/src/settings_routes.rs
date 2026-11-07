//! `GET` and `PUT` of `/api/settings/routing|budget|models`: `TODO/routing.yaml` and
//! `TODO/config.yaml` as JSON. A `PUT` is checked against the daemon's own types before the file
//! is written (temporary file, then rename); the dispatcher rereads both files on every tick, so
//! no restart is ever needed.

use crate::http::App;
use crate::models::budget_settings::BudgetSettings;
use crate::models::constants::settings::{
    BUDGET_FIELDS, CONFIG_FILE, CRITERIA_LIST_KEYS, CRITERIA_REGEX_KEY, CRITERIA_TEXT_KEYS,
    DEFAULTS_KEYS, EVENT_SECTION_KEY, EVENT_SETTINGS_CHANGED, KEY_CHANGED, KEY_DEFAULTS, KEY_MATCH,
    KEY_MODELS, KEY_RELOAD_REQUIRED, KEY_REPOS, KEY_ROUTE, KEY_RULES, KEY_SERVICE, KEY_TRACK,
    MAX_PERCENT, REPO_KEYS, ROUTING_FILE, ROUTING_SECTIONS, SETTINGS_SEGMENT, TEMP_SUFFIX,
    TODO_DIR, TRACK_NAMES,
};
use crate::models::enums::{ApiStatus, FieldKind, SettingsFailure, SettingsSection};
use crate::models::models_settings::ModelsSettings;
use crate::models::setting_issue::SettingIssue;
use crate::settings_yaml::{apply_to_value, patch_leaves, replace_section, LeafEdit};
use crate::sync::lock;
use aidp_core::model_policy::{load_policy, ModelPolicy};
use aidp_core::models::enums::ModelTier;
use aidp_tickets::config::{self, Config, RepoRoute, Routing};
use aidp_tickets::models::enums::BudgetRoute;
use aidp_tickets::models::token_amount::TokenAmount;
use aidp_tickets::models::track::Track;
use aidp_tickets::night_mode::parse_hhmm;
use aidp_tickets::routing::{summary_regex_error, Rule};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tiny_http::Method;

const STATUS_OK: u16 = 200;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

type Outcome = Result<(Value, bool), SettingsFailure>;

fn failure(status: ApiStatus, message: impl ToString) -> SettingsFailure {
    SettingsFailure::Api(status, message.to_string())
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> SettingIssue {
    SettingIssue::new(path, message)
}

/// Answers `/api/settings/<section>`; `None` for any other route.
pub fn handle(app: &App, method: &Method, path: &str, body: &Value) -> Option<(u16, Value)> {
    let segments: Vec<&str> = path
        .trim_start_matches("/api")
        .trim_matches('/')
        .split('/')
        .collect();
    let [SETTINGS_SEGMENT, name] = segments.as_slice() else {
        return None;
    };
    let section = SettingsSection::parse(name)?;
    let vault = app.vault_root.as_path();
    let outcome = match method {
        Method::Get => read(vault, section).map(|value| (value, false)),
        Method::Put => write(vault, section, body),
        _ => return None,
    };
    Some(match outcome {
        Ok((value, changed)) => {
            if changed {
                app.bus.emit(
                    EVENT_SETTINGS_CHANGED,
                    json!({ EVENT_SECTION_KEY: section.as_str() }),
                );
            }
            (STATUS_OK, value)
        }
        Err(failure) => failure.into_reply(),
    })
}

fn read(vault: &Path, section: SettingsSection) -> Result<Value, SettingsFailure> {
    ensure_vault(vault)?;
    match section {
        SettingsSection::Routing => Ok(routing_value(&config::load_routing(vault))),
        SettingsSection::Budget => to_json(&BudgetSettings::from(&config::load(vault))),
        SettingsSection::Models => to_json(&ModelsSettings::new(load_policy(vault))),
    }
}

fn write(vault: &Path, section: SettingsSection, body: &Value) -> Outcome {
    ensure_vault(vault)?;
    let (mut reply, changed) = match section {
        SettingsSection::Routing => write_routing(vault, body)?,
        SettingsSection::Budget => write_budget(vault, body)?,
        SettingsSection::Models => write_models(vault, body)?,
    };
    if let Some(object) = reply.as_object_mut() {
        object.insert(KEY_RELOAD_REQUIRED.to_string(), Value::Bool(false));
        object.insert(KEY_CHANGED.to_string(), Value::Bool(changed));
    }
    Ok((reply, changed))
}

fn ensure_vault(vault: &Path) -> Result<(), SettingsFailure> {
    if vault.as_os_str().is_empty() {
        return Err(failure(ApiStatus::Config, "no vault configured"));
    }
    Ok(())
}

fn to_json(value: &impl serde::Serialize) -> Result<Value, SettingsFailure> {
    serde_json::to_value(value).map_err(|error| failure(ApiStatus::Internal, error))
}

fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter(|(_, child)| !child.is_null())
                .map(|(key, child)| (key, strip_nulls(child)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

fn routing_value(routing: &Routing) -> Value {
    serde_json::to_value(routing)
        .map(strip_nulls)
        .unwrap_or_else(|_| Value::Object(Map::new()))
}

fn todo_file(vault: &Path, name: &str) -> PathBuf {
    vault.join(TODO_DIR).join(name)
}

fn read_text(file: &Path, fallback: &str) -> Result<String, SettingsFailure> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(fallback.to_string()),
        Err(error) => Err(failure(ApiStatus::Io, error)),
    }
}

fn parse_document(file: &Path, text: &str) -> Result<serde_yaml::Value, SettingsFailure> {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().to_string());
    let unreadable = |detail: String| {
        failure(
            ApiStatus::Conflict,
            format!(
                "{} cannot be edited: {detail}",
                name.clone().unwrap_or_default()
            ),
        )
    };
    match serde_yaml::from_str::<serde_yaml::Value>(text) {
        Ok(serde_yaml::Value::Null) => Ok(serde_yaml::Value::Mapping(Default::default())),
        Ok(document) if document.is_mapping() => Ok(document),
        Ok(_) => Err(unreadable("the top level is not a mapping".to_string())),
        Err(error) => Err(unreadable(error.to_string())),
    }
}

fn persist(file: &Path, text: &str) -> Result<(), SettingsFailure> {
    let io = |error: std::io::Error| failure(ApiStatus::Io, error);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(format!(".{TEMP_SUFFIX}"));
    let temporary = PathBuf::from(temporary);
    let written = std::fs::File::create(&temporary)
        .and_then(|mut handle| {
            handle.write_all(text.as_bytes())?;
            handle.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, file));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(io(error));
    }
    Ok(())
}

/// The patched text when it parses back to `expected`, else `expected` serialized whole.
fn verified_text(
    patched: Option<String>,
    expected: &serde_yaml::Value,
) -> Result<String, SettingsFailure> {
    if let Some(text) = patched {
        let same = serde_yaml::from_str::<serde_yaml::Value>(&text)
            .map(|parsed| &parsed == expected)
            .unwrap_or(false);
        if same {
            return Ok(text);
        }
    }
    serde_yaml::to_string(expected).map_err(|error| failure(ApiStatus::Internal, error))
}

fn write_budget(vault: &Path, body: &Value) -> Outcome {
    let (edits, issues) = budget_edits(body);
    if !issues.is_empty() {
        return Err(SettingsFailure::Issues(issues));
    }
    let file = todo_file(vault, CONFIG_FILE);
    let _guard = lock(&WRITE_LOCK);
    let text = read_text(&file, config::default_yaml())?;
    let original = parse_document(&file, &text)?;
    let mut document = original.clone();
    for edit in &edits {
        apply_to_value(&mut document, &edit.path, edit.value.as_ref());
    }
    let changed = document != original;
    if changed {
        let updated = verified_text(patch_leaves(&text, &edits), &document)?;
        serde_yaml::from_str::<Config>(&updated).map_err(|error| {
            failure(
                ApiStatus::Conflict,
                format!("{CONFIG_FILE} would not load: {error}"),
            )
        })?;
        persist(&file, &updated)?;
    }
    Ok((
        to_json(&BudgetSettings::from(&config::load(vault)))?,
        changed,
    ))
}

fn budget_edits(body: &Value) -> (Vec<LeafEdit>, Vec<SettingIssue>) {
    let (mut edits, mut issues) = (Vec::new(), Vec::new());
    walk_budget(&mut Vec::new(), body, &mut edits, &mut issues);
    (edits, issues)
}

fn walk_budget(
    prefix: &mut Vec<String>,
    value: &Value,
    edits: &mut Vec<LeafEdit>,
    issues: &mut Vec<SettingIssue>,
) {
    let Some(object) = value.as_object() else {
        issues.push(issue(prefix.join("."), "must be an object"));
        return;
    };
    for (key, child) in object {
        prefix.push(key.clone());
        let same_path = |path: &[&str]| {
            path.iter()
                .map(|part| part.to_string())
                .eq(prefix.iter().cloned())
        };
        let is_parent = |path: &[&str]| {
            path.len() > prefix.len()
                && path
                    .iter()
                    .zip(prefix.iter())
                    .all(|(left, right)| left == right)
        };
        if let Some(field) = BUDGET_FIELDS.iter().find(|field| same_path(field.path)) {
            match check_field(field.kind, child) {
                Ok(checked) => edits.push(LeafEdit {
                    path: prefix.clone(),
                    value: checked,
                }),
                Err(message) => issues.push(issue(field.dotted(), message)),
            }
        } else if BUDGET_FIELDS.iter().any(|field| is_parent(field.path)) {
            walk_budget(prefix, child, edits, issues);
        } else {
            issues.push(issue(prefix.join("."), "unknown setting"));
        }
        prefix.pop();
    }
}

fn yaml_of(value: &Value) -> Result<Option<serde_yaml::Value>, String> {
    serde_yaml::to_value(value)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn check_field(kind: FieldKind, value: &Value) -> Result<Option<serde_yaml::Value>, String> {
    let number = value.as_f64();
    match kind {
        FieldKind::OptionalPercent if value.is_null() => Ok(None),
        FieldKind::OptionalPercent => match number {
            Some(percent) if percent > 0.0 && percent <= MAX_PERCENT => yaml_of(value),
            _ => Err(format!(
                "must be a number above 0 and at most {MAX_PERCENT}, or null"
            )),
        },
        _ if value.is_null() => Err("must not be null".to_string()),
        FieldKind::Bool => value
            .as_bool()
            .map(|flag| Some(serde_yaml::Value::Bool(flag)))
            .ok_or_else(|| "must be true or false".to_string()),
        FieldKind::Fraction => match number {
            Some(fraction) if fraction > 0.0 && fraction <= 1.0 => yaml_of(value),
            _ => Err("must be a fraction above 0 and at most 1 (0.8 = 80 %)".to_string()),
        },
        FieldKind::UnitInterval => match number {
            Some(fraction) if (0.0..=1.0).contains(&fraction) => yaml_of(value),
            _ => Err("must be a fraction between 0 and 1".to_string()),
        },
        FieldKind::NonNegative => match number {
            Some(amount) if amount >= 0.0 => yaml_of(value),
            _ => Err("must be a number of 0 or more".to_string()),
        },
        FieldKind::Count(min, max) => match value.as_u64() {
            Some(count) if (min..=max).contains(&count) => yaml_of(value),
            _ => Err(format!("must be a whole number between {min} and {max}")),
        },
        FieldKind::Tokens => check_tokens(value),
        FieldKind::Choice(words) => {
            match value.as_str().map(|text| text.trim().to_ascii_lowercase()) {
                Some(word) if words.contains(&word.as_str()) => {
                    Ok(Some(serde_yaml::Value::String(word)))
                }
                _ => Err(format!("must be one of {}", words.join(", "))),
            }
        }
        FieldKind::ClockTime => match value.as_str().map(str::trim) {
            Some(time) if parse_hhmm(time).is_some() => {
                Ok(Some(serde_yaml::Value::String(time.to_string())))
            }
            _ => Err("must be a local time written HH:MM".to_string()),
        },
    }
}

fn check_tokens(value: &Value) -> Result<Option<serde_yaml::Value>, String> {
    let refused = || "must be a token count: a number, or text such as 3M or 500k".to_string();
    let amount = match value {
        Value::Number(_) => value
            .as_f64()
            .filter(|count| count.is_finite() && *count >= 0.0)
            .map(|count| TokenAmount(count.round() as u64)),
        Value::String(text) => TokenAmount::parse(text),
        _ => None,
    }
    .ok_or_else(refused)?;
    match value.as_str().map(str::trim) {
        Some(text)
            if text
                .chars()
                .any(|character| character.is_ascii_alphabetic()) =>
        {
            Ok(Some(serde_yaml::Value::String(text.to_string())))
        }
        _ => Ok(Some(serde_yaml::Value::Number(amount.0.into()))),
    }
}

fn string_list(value: &Value) -> Option<Vec<&str>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().filter(|text| !text.trim().is_empty()))
        .collect()
}

fn blank(value: &Value) -> bool {
    value.as_str().is_none_or(|text| text.trim().is_empty())
}

fn normalize_defaults(value: &Value, issues: &mut Vec<SettingIssue>) -> Value {
    let Some(object) = value.as_object() else {
        issues.push(issue(KEY_DEFAULTS, "must be an object"));
        return Value::Null;
    };
    let mut result = Map::new();
    for (key, child) in object {
        let path = format!("{KEY_DEFAULTS}.{key}");
        if child.is_null() && DEFAULTS_KEYS.contains(&key.as_str()) {
            continue;
        }
        if !DEFAULTS_KEYS.contains(&key.as_str()) {
            issues.push(issue(path, "unknown setting"));
        } else if serde_json::from_value::<Track>(child.clone()).is_err() {
            issues.push(issue(path, format!("must be one of {TRACK_NAMES}")));
        } else {
            result.insert(key.clone(), child.clone());
        }
    }
    Value::Object(result)
}

fn normalize_rule(index: usize, entry: &Value, issues: &mut Vec<SettingIssue>) -> Option<Value> {
    let prefix = format!("{KEY_RULES}[{index}]");
    let Some(object) = entry.as_object() else {
        issues.push(issue(prefix, "must be an object"));
        return None;
    };
    let before = issues.len();
    for key in object.keys() {
        if ![KEY_MATCH, KEY_SERVICE, KEY_ROUTE].contains(&key.as_str()) {
            issues.push(issue(format!("{prefix}.{key}"), "unknown setting"));
        }
    }
    let service = match (object.get(KEY_SERVICE), object.get(KEY_ROUTE)) {
        (Some(_), Some(_)) => {
            issues.push(issue(
                format!("{prefix}.{KEY_ROUTE}"),
                "give either service or route, not both",
            ));
            None
        }
        (Some(name), None) | (None, Some(name)) if !blank(name) => name.as_str().map(str::trim),
        _ => {
            issues.push(issue(
                format!("{prefix}.{KEY_SERVICE}"),
                "must name a route target",
            ));
            None
        }
    };
    let criteria = normalize_criteria(&prefix, object.get(KEY_MATCH), issues);
    if issues.len() > before {
        return None;
    }
    let rule = serde_json::from_value::<Rule>(json!({ KEY_MATCH: criteria, KEY_SERVICE: service }));
    match rule {
        Ok(rule) => serde_json::to_value(rule).ok().map(strip_nulls),
        Err(error) => {
            issues.push(issue(prefix, error.to_string()));
            None
        }
    }
}

fn normalize_criteria(
    prefix: &str,
    value: Option<&Value>,
    issues: &mut Vec<SettingIssue>,
) -> Value {
    let path = format!("{prefix}.{KEY_MATCH}");
    let Some(object) = value.and_then(Value::as_object) else {
        issues.push(issue(path, "must be an object"));
        return Value::Null;
    };
    let mut result = Map::new();
    for (key, child) in object {
        let here = format!("{path}.{key}");
        if child.is_null() {
            continue;
        }
        if CRITERIA_TEXT_KEYS.contains(&key.as_str()) {
            match child
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                Some(text) if key == CRITERIA_REGEX_KEY && summary_regex_error(text).is_some() => {
                    issues.push(issue(here, "not a valid regular expression"));
                }
                Some(text) => {
                    result.insert(key.clone(), Value::String(text.to_string()));
                }
                None => issues.push(issue(here, "must be a non-empty text")),
            }
        } else if CRITERIA_LIST_KEYS.contains(&key.as_str()) {
            match string_list(child) {
                Some(items) if !items.is_empty() => {
                    let trimmed: Vec<Value> = items.iter().map(|item| json!(item.trim())).collect();
                    result.insert(key.clone(), Value::Array(trimmed));
                }
                _ => issues.push(issue(here, "must be a non-empty list of non-empty texts")),
            }
        } else {
            issues.push(issue(here, "unknown criterion"));
        }
    }
    if result.is_empty() && !issues.iter().any(|known| known.path.starts_with(&path)) {
        issues.push(issue(path, "must set at least one criterion"));
    }
    Value::Object(result)
}

fn normalize_repo(name: &str, entry: &Value, issues: &mut Vec<SettingIssue>) -> Option<Value> {
    let prefix = format!("{KEY_REPOS}.{name}");
    let Some(object) = entry.as_object() else {
        issues.push(issue(prefix, "must be an object"));
        return None;
    };
    let before = issues.len();
    let mut result = Map::new();
    for (key, child) in object {
        let path = format!("{prefix}.{key}");
        if child.is_null() && REPO_KEYS.contains(&key.as_str()) {
            continue;
        }
        if !REPO_KEYS.contains(&key.as_str()) {
            issues.push(issue(path, "unknown setting"));
        } else if key == KEY_TRACK {
            if serde_json::from_value::<Track>(child.clone()).is_ok() {
                result.insert(key.clone(), child.clone());
            } else {
                issues.push(issue(path, format!("must be one of {TRACK_NAMES}")));
            }
        } else if key == KEY_ROUTE {
            match child.as_str().and_then(BudgetRoute::parse) {
                Some(route) => {
                    result.insert(key.clone(), json!(route.as_str()));
                }
                None => issues.push(issue(path, "must be code, art, map or doc")),
            }
        } else if blank(child) {
            issues.push(issue(path, "must be a non-empty text"));
        } else if let Some(text) = child.as_str() {
            result.insert(key.clone(), json!(text.trim()));
        }
    }
    if issues.len() > before {
        return None;
    }
    let route = serde_json::from_value::<RepoRoute>(Value::Object(result));
    route
        .ok()
        .and_then(|route| serde_json::to_value(route).ok())
        .map(strip_nulls)
}

fn normalize_routing(body: &Value) -> Result<Vec<(&'static str, Value)>, Vec<SettingIssue>> {
    let mut issues = Vec::new();
    let Some(object) = body.as_object() else {
        return Err(vec![issue("", "the body must be a JSON object")]);
    };
    for key in object.keys() {
        if !ROUTING_SECTIONS.contains(&key.as_str()) {
            issues.push(issue(key.clone(), "unknown section"));
        }
    }
    let mut sections = Vec::new();
    if let Some(value) = object.get(KEY_DEFAULTS) {
        sections.push((KEY_DEFAULTS, normalize_defaults(value, &mut issues)));
    }
    if let Some(value) = object.get(KEY_RULES) {
        match value.as_array() {
            Some(entries) => {
                let rules = entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| normalize_rule(index, entry, &mut issues))
                    .collect();
                sections.push((KEY_RULES, Value::Array(rules)));
            }
            None => issues.push(issue(KEY_RULES, "must be a list")),
        }
    }
    if let Some(value) = object.get(KEY_REPOS) {
        match value.as_object() {
            Some(entries) => {
                let mut repos = Map::new();
                for (name, entry) in entries {
                    if name.trim().is_empty() {
                        issues.push(issue(KEY_REPOS, "a route target needs a name"));
                    } else if let Some(normalized) = normalize_repo(name.trim(), entry, &mut issues)
                    {
                        repos.insert(name.trim().to_string(), normalized);
                    }
                }
                sections.push((KEY_REPOS, Value::Object(repos)));
            }
            None => issues.push(issue(KEY_REPOS, "must be an object")),
        }
    }
    if issues.is_empty() {
        Ok(sections)
    } else {
        Err(issues)
    }
}

/// Rewrites top-level sections of a YAML file: `None` drops a section.
fn rewrite_sections(
    file: &Path,
    default_text: &str,
    changes: &[(&str, Option<serde_yaml::Value>)],
) -> Result<(), SettingsFailure> {
    let text = read_text(file, default_text)?;
    let mut document = parse_document(file, &text)?;
    let mut updated = text;
    for (key, value) in changes {
        let rendered = value
            .as_ref()
            .map(|value| {
                let mut single = serde_yaml::Mapping::new();
                single.insert(serde_yaml::Value::String((*key).to_string()), value.clone());
                serde_yaml::to_string(&single)
            })
            .transpose()
            .map_err(|error| failure(ApiStatus::Internal, error))?;
        updated = replace_section(&updated, key, rendered.as_deref());
        apply_to_value(&mut document, &[(*key).to_string()], value.as_ref());
    }
    persist(file, &verified_text(Some(updated), &document)?)
}

fn write_routing(vault: &Path, body: &Value) -> Outcome {
    let sections = normalize_routing(body).map_err(SettingsFailure::Issues)?;
    let file = todo_file(vault, ROUTING_FILE);
    let _guard = lock(&WRITE_LOCK);
    let current = routing_value(&config::load_routing(vault));
    let mut changes = Vec::new();
    for (key, normalized) in sections {
        if current.get(key) != Some(&normalized) {
            let value = serde_yaml::to_value(&normalized)
                .map_err(|error| failure(ApiStatus::Internal, error))?;
            changes.push((key, Some(value)));
        }
    }
    let changed = !changes.is_empty();
    if changed {
        rewrite_sections(&file, "", &changes)?;
    }
    Ok((routing_value(&config::load_routing(vault)), changed))
}

fn normalize_models(body: &Value) -> Result<ModelPolicy, Vec<SettingIssue>> {
    let mut issues = Vec::new();
    let mut policy = ModelPolicy::new();
    let Some(object) = body
        .as_object()
        .filter(|object| object.contains_key(KEY_MODELS))
    else {
        return Err(vec![issue(
            KEY_MODELS,
            "the body must be an object with a models key",
        )]);
    };
    for key in object.keys().filter(|key| key.as_str() != KEY_MODELS) {
        issues.push(issue(key.clone(), "unknown setting"));
    }
    let routes = &object[KEY_MODELS];
    if routes.is_null() {
        return if issues.is_empty() {
            Ok(policy)
        } else {
            Err(issues)
        };
    }
    let Some(routes) = routes.as_object() else {
        issues.push(issue(KEY_MODELS, "must be an object of routes"));
        return Err(issues);
    };
    for (route, stages) in routes {
        let route_path = format!("{KEY_MODELS}.{route}");
        let Some(known) = BudgetRoute::parse(route) else {
            issues.push(issue(
                route_path,
                "unknown route, expected code, art, map or doc",
            ));
            continue;
        };
        let Some(stages) = stages.as_object() else {
            issues.push(issue(route_path, "must be an object of stages"));
            continue;
        };
        for (stage, tier) in stages {
            let path = format!("{route_path}.{stage}");
            let valid_stage = !stage.is_empty()
                && stage
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid_stage {
                issues.push(issue(path, "not a stage id"));
                continue;
            }
            match serde_json::from_value::<ModelTier>(tier.clone()) {
                Ok(tier) => {
                    policy
                        .entry(known.as_str().to_string())
                        .or_default()
                        .insert(stage.clone(), tier);
                }
                Err(_) => issues.push(issue(path, "must be haiku, sonnet, opus or fable")),
            }
        }
    }
    if issues.is_empty() {
        Ok(policy)
    } else {
        Err(issues)
    }
}

fn write_models(vault: &Path, body: &Value) -> Outcome {
    let policy = normalize_models(body).map_err(SettingsFailure::Issues)?;
    let file = todo_file(vault, CONFIG_FILE);
    let _guard = lock(&WRITE_LOCK);
    let changed = load_policy(vault) != policy;
    if changed {
        let value = (!policy.is_empty())
            .then(|| serde_yaml::to_value(&policy))
            .transpose()
            .map_err(|error| failure(ApiStatus::Internal, error))?;
        rewrite_sections(&file, config::default_yaml(), &[(KEY_MODELS, value)])?;
    }
    Ok((to_json(&ModelsSettings::new(load_policy(vault)))?, changed))
}

#[cfg(test)]
#[path = "../tests/unit/settings_routes_tests.rs"]
mod tests;
