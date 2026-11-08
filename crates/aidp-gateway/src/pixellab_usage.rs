//! Pixellab usage, read from the `pixellab-usage.json` files the art stages write in their stage
//! directory: generations and credits per run and per ticket, and the last known balance.
//! The daemon never calls Pixellab: it only reads what the stages recorded.

use crate::models::constants::pixellab::{
    BALANCE_KEYS, BALANCE_MAX_DEPTH, BALANCE_TTL, MAX_RUNS_PER_TICKET, MAX_STAGE_DEPTH,
    PIPELINES_DIR, USAGE_FILE,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// One `pixellab-usage.json`, as the art stages write it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PixellabUsageFile {
    #[serde(default)]
    pub balance_before: Option<Value>,
    #[serde(default)]
    pub balance_after: Option<Value>,
    #[serde(default)]
    pub generations: u32,
    #[serde(default)]
    pub jobs: Vec<String>,
    #[serde(default)]
    pub recorded_at: Option<String>,
}

/// What the art stages of one run used.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunPixellabUsage {
    pub run_id: String,
    pub generations: u32,
    pub jobs: u32,
    pub balance_before: Option<f64>,
    pub balance_after: Option<f64>,
    pub spent: Option<f64>,
    pub recorded_at: Option<String>,
    pub stages: u32,
}

/// What the runs of one ticket used.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TicketPixellabUsage {
    pub key: String,
    pub generations: u32,
    pub spent: f64,
    pub runs: Vec<RunPixellabUsage>,
}

/// The last balance an art stage recorded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSnapshot {
    pub amount: Option<f64>,
    pub raw: Value,
    pub recorded_at: String,
    pub run_id: String,
}

fn number_in(text: &str) -> Option<f64> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let tail = &text[start..];
    let end = tail
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(tail.len());
    tail[..end].parse().ok()
}

fn amount_at(value: &Value, depth: usize) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => number_in(text),
        Value::Object(map) if depth < BALANCE_MAX_DEPTH => {
            for key in BALANCE_KEYS {
                let found = map
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .and_then(|(_, inner)| amount_at(inner, depth + 1));
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        _ => None,
    }
}

/// The credit amount of a raw `get_balance` answer: a number, or the first of the usual keys
/// (`credits`, `balance`, `usd`, ...) holding one, or the first number of a text answer.
pub fn balance_amount(raw: &Value) -> Option<f64> {
    amount_at(raw, 0)
}

/// Reads one usage file; `None` when it is missing or not the expected JSON.
pub fn read_usage_file(path: &Path) -> Option<PixellabUsageFile> {
    serde_json::from_str(&aidp_core::text::read_text_lenient(path).ok()?).ok()
}

fn collect_files(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < MAX_STAGE_DEPTH {
            collect_files(&path, depth + 1, found);
        } else if path.file_name().is_some_and(|name| name == USAGE_FILE) {
            found.push(path);
        }
    }
}

/// Every usage file under a run directory (one per art stage).
pub fn usage_files(run_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_files(run_dir, 0, &mut found);
    found.sort();
    found
}

/// The usage of one run: generations summed over its stages, balance from the first
/// `balanceBefore` to the last `balanceAfter`.
pub fn aggregate_run(run_id: &str, run_dir: &Path) -> RunPixellabUsage {
    let mut files: Vec<PixellabUsageFile> = usage_files(run_dir)
        .iter()
        .filter_map(|path| read_usage_file(path))
        .collect();
    files.sort_by(|a, b| a.recorded_at.cmp(&b.recorded_at));
    let before = files
        .iter()
        .find_map(|file| file.balance_before.as_ref().and_then(balance_amount));
    let after = files
        .iter()
        .rev()
        .find_map(|file| file.balance_after.as_ref().and_then(balance_amount));
    RunPixellabUsage {
        run_id: run_id.to_string(),
        generations: files.iter().map(|file| file.generations).sum(),
        jobs: files.iter().map(|file| file.jobs.len() as u32).sum(),
        balance_before: before,
        balance_after: after,
        spent: before.zip(after).map(|(before, after)| before - after),
        recorded_at: files.last().and_then(|file| file.recorded_at.clone()),
        stages: files.len() as u32,
    }
}

fn run_dirs_of(vault: &Path, key: &str) -> Vec<(String, PathBuf)> {
    let Ok(repos) = std::fs::read_dir(vault.join(PIPELINES_DIR)) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for repo in repos.flatten() {
        let Ok(entries) = std::fs::read_dir(repo.path().join(key)) else {
            continue;
        };
        runs.extend(
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())),
        );
    }
    runs.sort_by(|a, b| b.0.cmp(&a.0));
    runs.truncate(MAX_RUNS_PER_TICKET);
    runs
}

/// The usage of ticket `key`: every run of `<vault>/Pipelines/*/<key>/` that recorded some.
pub fn aggregate_ticket(vault: &Path, key: &str) -> TicketPixellabUsage {
    let runs: Vec<RunPixellabUsage> = run_dirs_of(vault, key)
        .iter()
        .map(|(run_id, dir)| aggregate_run(run_id, dir))
        .filter(|run| run.stages > 0)
        .collect();
    TicketPixellabUsage {
        key: key.to_string(),
        generations: runs.iter().map(|run| run.generations).sum(),
        spent: runs.iter().filter_map(|run| run.spent).sum(),
        runs,
    }
}

fn all_run_dirs(vault: &Path) -> Vec<(String, PathBuf)> {
    let Ok(repos) = std::fs::read_dir(vault.join(PIPELINES_DIR)) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for repo in repos.flatten() {
        let Ok(features) = std::fs::read_dir(repo.path()) else {
            continue;
        };
        for feature in features.flatten().filter(|e| e.path().is_dir()) {
            let Ok(entries) = std::fs::read_dir(feature.path()) else {
                continue;
            };
            runs.extend(
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())),
            );
        }
    }
    runs
}

/// Scans every run of the vault for the newest recorded balance (`balanceAfter`, else `balanceBefore`).
pub fn scan_latest_balance(vault: &Path) -> Option<BalanceSnapshot> {
    let mut best: Option<BalanceSnapshot> = None;
    for (run_id, dir) in all_run_dirs(vault) {
        for path in usage_files(&dir) {
            let Some(file) = read_usage_file(&path) else {
                continue;
            };
            let Some(raw) = file.balance_after.or(file.balance_before) else {
                continue;
            };
            let recorded_at = file.recorded_at.unwrap_or_default();
            if best.as_ref().is_some_and(|b| b.recorded_at >= recorded_at) {
                continue;
            }
            best = Some(BalanceSnapshot {
                amount: balance_amount(&raw),
                raw,
                recorded_at,
                run_id: run_id.clone(),
            });
        }
    }
    best
}

type BalanceCache = Mutex<HashMap<PathBuf, (Instant, Option<BalanceSnapshot>)>>;

fn balance_cache() -> &'static BalanceCache {
    static CACHE: OnceLock<BalanceCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// [`scan_latest_balance`], remembered for a short while so a dispatcher tick does not rescan the vault.
pub fn latest_balance(vault: &Path) -> Option<BalanceSnapshot> {
    let mut cache = balance_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((scanned_at, snapshot)) = cache.get(vault) {
        if scanned_at.elapsed() < BALANCE_TTL {
            return snapshot.clone();
        }
    }
    let snapshot = scan_latest_balance(vault);
    cache.insert(vault.to_path_buf(), (Instant::now(), snapshot.clone()));
    snapshot
}

/// `GET /api/pixellab/balance`: the last known balance, its date and the configured reserve.
/// `balance` is `null` until an art stage has recorded one; the daemon never asks Pixellab itself.
pub fn balance_report(vault: &Path) -> Value {
    let reserve = if vault.as_os_str().is_empty() {
        0.0
    } else {
        aidp_tickets::config::load(vault)
            .budget
            .pixellab
            .min_reserve
    };
    let snapshot = (!vault.as_os_str().is_empty())
        .then(|| latest_balance(vault))
        .flatten();
    match snapshot {
        Some(found) => serde_json::json!({
            "balance": found.amount,
            "raw": found.raw,
            "recordedAt": found.recorded_at,
            "runId": found.run_id,
            "minReserve": reserve,
            "belowReserve": found.amount.is_some_and(|amount| reserve > 0.0 && amount < reserve),
        }),
        None => serde_json::json!({
            "balance": null,
            "raw": null,
            "recordedAt": null,
            "runId": null,
            "minReserve": reserve,
            "belowReserve": false,
        }),
    }
}

/// Forgets the remembered balance of `vault`, so the next read rescans.
pub fn forget_balance(vault: &Path) {
    balance_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(vault);
}

#[cfg(test)]
#[path = "../tests/unit/pixellab_usage_tests.rs"]
mod tests;
