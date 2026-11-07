//! Per-stage usage and run-level cost rollup.

use super::{RunState, StageState};
use crate::cost::PriceTable;
use crate::models::constants::cost::{CLAUDE_DIR, PROJECTS_DIR, UNKNOWN_LABEL};
use crate::models::run_state::{RunCost, RunPorts, StageUsage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The pricing-grid version for a [`PriceTable`], `None` when it failed to load.
fn table_pricing_version(prices: &PriceTable) -> Option<String> {
    match prices.load_error() {
        Some(_) => None,
        None => Some(prices.pricing_version().to_string()),
    }
}

/// Where a stage's usage counters came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Jsonl,
    Budget,
    #[default]
    Unavailable,
}

impl std::fmt::Display for UsageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl UsageSource {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::Budget => "budget",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Deterministic complexity bucket of a stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityBucket {
    Small,
    #[default]
    Medium,
    Large,
}

impl std::fmt::Display for ComplexityBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ComplexityBucket {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// Attach per-stage [`StageUsage`] and a run-level [`RunCost`] rollup from the transcript
/// ledger (`crate::tokens::scan_run_agents` / `crate::cost`).
pub(super) fn attach_cost(state: &mut RunState, ports: &RunPorts) {
    let now = ports.clock.now_rfc3339();

    if let Some(reason) = ports.prices.load_error() {
        tracing::warn!(%reason, "every stage's usage is reported as unavailable");
        mark_every_stage_unavailable(state);
        return;
    }

    let Some(projects_dir) = claude_projects_dir() else {
        for stage in state.stages.iter_mut() {
            attach_budget_fallback(stage);
        }
        return;
    };

    let agents = crate::tokens::scan_run_agents_since(
        &projects_dir,
        &state.workspace_root,
        &state.run_id,
        Some(&state.started_at),
    );

    let mut by_model: BTreeMap<String, StageUsage> = BTreeMap::new();
    let mut tokens_billable: u64 = 0;
    let mut total_usd = 0.0f64;
    let mut any_cost = false;

    for stage in state.stages.iter_mut() {
        let matches: Vec<&crate::tokens::AgentUsage> = agents
            .iter()
            .filter(|a| a.stage.as_deref() == Some(stage.name.as_str()))
            .collect();
        if matches.is_empty() {
            attach_budget_fallback(stage);
            continue;
        }

        let mut u = StageUsage {
            source: UsageSource::Jsonl,
            bucket: ComplexityBucket::Medium,
            ..Default::default()
        };
        let mut stage_cost = 0.0f64;
        let mut first_ts: Option<String> = None;
        let mut last_ts: Option<String> = None;

        for a in &matches {
            u.input += a.usage.input_tokens;
            u.output += a.usage.output_tokens;
            u.cache_creation += a.usage.cache_creation_input_tokens;
            u.cache_read += a.usage.cache_read_input_tokens;
            if u.model.is_none() {
                u.model = a.model.clone();
            }
            if u.agent_type.is_none() {
                u.agent_type = a.agent_type.clone();
            }
            let tier = a
                .model
                .as_deref()
                .map_or(crate::cost::ModelTier::Unknown, crate::cost::tier);
            let cost_here = ports.prices.cost_usd(tier.as_str(), &a.usage, None);
            stage_cost += cost_here;

            if let Some(ts) = &a.first_ts {
                if first_ts.as_deref().map(|f| ts.as_str() < f).unwrap_or(true) {
                    first_ts = Some(ts.clone());
                }
            }
            if let Some(ts) = &a.last_ts {
                if last_ts.as_deref().map(|l| ts.as_str() > l).unwrap_or(true) {
                    last_ts = Some(ts.clone());
                }
            }

            let model_key = a.model.clone().unwrap_or_else(|| UNKNOWN_LABEL.to_string());
            let entry = by_model
                .entry(model_key.clone())
                .or_insert_with(|| StageUsage {
                    source: UsageSource::Jsonl,
                    bucket: ComplexityBucket::Medium,
                    model: Some(model_key.clone()),
                    model_pinned: Some(model_key.clone()),
                    agent_type: a.agent_type.clone(),
                    pricing_version: table_pricing_version(ports.prices),
                    cost_usd: Some(0.0),
                    ..Default::default()
                });
            entry.input += a.usage.input_tokens;
            entry.output += a.usage.output_tokens;
            entry.cache_creation += a.usage.cache_creation_input_tokens;
            entry.cache_read += a.usage.cache_read_input_tokens;
            entry.cost_usd = Some(entry.cost_usd.unwrap_or(0.0) + cost_here);
        }

        u.model_pinned = u.model.clone();
        u.cost_usd = Some(stage_cost);
        u.pricing_version = table_pricing_version(ports.prices);

        if let (Some(f), Some(l)) = (&first_ts, &last_ts) {
            if let (Ok(fd), Ok(ld)) = (
                chrono::DateTime::parse_from_rfc3339(f),
                chrono::DateTime::parse_from_rfc3339(l),
            ) {
                let ms = (ld - fd).num_milliseconds();
                if ms >= 0 {
                    stage.duration_ms = Some(ms as u64);
                }
            }
        }

        total_usd += stage_cost;
        any_cost = true;
        tokens_billable += u.input + u.output + u.cache_creation + u.cache_read;
        stage.usage = Some(u);
    }

    state.cost = Some(RunCost {
        total_usd: any_cost.then_some(total_usd),
        tokens_billable,
        pricing_version: table_pricing_version(ports.prices),
        captured_at: now,
        by_model,
    });
}

/// Report every stage usage as `unavailable` and clear `state.cost`.
fn mark_every_stage_unavailable(state: &mut RunState) {
    for stage in state.stages.iter_mut() {
        stage.usage = Some(unavailable_usage());
    }
    state.cost = None;
}

fn unavailable_usage() -> StageUsage {
    StageUsage {
        source: UsageSource::Unavailable,
        bucket: ComplexityBucket::Medium,
        ..Default::default()
    }
}

/// A stage the ledger could not attribute any transcript to (no Claude projects dir found, or
/// genuinely no matching agent JSONL).
fn attach_budget_fallback(stage: &mut StageState) {
    stage.usage = Some(StageUsage {
        output: stage.tokens_used.unwrap_or(0),
        bucket: ComplexityBucket::Medium,
        source: UsageSource::Budget,
        ..Default::default()
    });
}

/// Locate the Claude projects directory, honouring `AIDP_CLAUDE_PROJECTS_DIR`.
fn claude_projects_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AIDP_CLAUDE_PROJECTS_DIR") {
        let pb = PathBuf::from(p);
        return pb.is_dir().then_some(pb);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let dir = PathBuf::from(home).join(CLAUDE_DIR).join(PROJECTS_DIR);
    dir.is_dir().then_some(dir)
}
