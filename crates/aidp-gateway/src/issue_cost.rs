//! `GET /api/tickets/:key/cost/detail`: what one issue cost, in tokens and dollars, with the
//! breakdown per pipeline run, per stage and per agent, plus the Pixellab credits its art stages
//! recorded. Sources: the run states under `<vault>/Pipelines/*/<key>/<run>/` (stage usage, priced
//! when the stage carries no `cost_usd`), the `pixellab-usage.json` files, and the ticket's sessions.

use crate::models::constants::pixellab::{MAX_RUNS_PER_TICKET, PIPELINES_DIR};
use crate::pixellab_usage::{aggregate_run, RunPixellabUsage};
use crate::sessions::SessionRecord;
use crate::ticket_cost::{session_tokens, ticket_of, TokenTotals};
use aidp_core::cost::repricing::reprice_counters;
use aidp_core::cost::{tier, PriceTable};
use aidp_core::models::run_state::{RunState, StageUsage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Path segment after `/cost` that selects this report.
pub const DETAIL_SEGMENT: &str = "detail";

/// Bucket key of a stage without an agent type.
const UNKNOWN_AGENT: &str = "unknown";

/// Tokens and dollars of one slice of work.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostAmount {
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

impl CostAmount {
    fn new(tokens: TokenTotals, cost_usd: f64) -> Self {
        CostAmount {
            tokens,
            total_tokens: tokens.total(),
            cost_usd,
        }
    }

    fn add(&mut self, other: &CostAmount) {
        self.tokens.input += other.tokens.input;
        self.tokens.output += other.tokens.output;
        self.tokens.cache_read += other.tokens.cache_read;
        self.tokens.cache_create += other.tokens.cache_create;
        self.total_tokens = self.tokens.total();
        self.cost_usd += other.cost_usd;
    }
}

/// One stage of a run.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StageCost {
    pub stage: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    #[serde(flatten)]
    pub amount: CostAmount,
}

/// One pipeline run of the issue.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunCost {
    pub run_id: String,
    pub pipeline_id: String,
    pub started_at: String,
    #[serde(flatten)]
    pub amount: CostAmount,
    pub stages: Vec<StageCost>,
    pub pixellab: Option<RunPixellabUsage>,
}

/// A row of the per-stage or per-agent breakdown.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostGroup {
    pub key: String,
    #[serde(flatten)]
    pub amount: CostAmount,
    pub count: u64,
}

/// Pixellab credits of the whole issue.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PixellabTotals {
    pub generations: u32,
    /// Credits spent, `None` when no run recorded a balance before and after.
    pub credits: Option<f64>,
}

/// `GET /api/tickets/:key/cost/detail`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IssueCost {
    pub key: String,
    /// Sum of the runs' stages.
    pub totals: CostAmount,
    pub runs: Vec<RunCost>,
    pub by_stage: Vec<CostGroup>,
    pub by_agent: Vec<CostGroup>,
    /// Interactive and flow sessions of the issue (transcript usage), not split by run.
    pub sessions: CostAmount,
    pub session_count: u64,
    pub pixellab: Option<PixellabTotals>,
}

fn usage_tokens(usage: &StageUsage) -> TokenTotals {
    TokenTotals {
        input: usage.input,
        output: usage.output,
        cache_read: usage.cache_read,
        cache_create: usage.cache_creation,
    }
}

/// The recorded dollars of a stage, else its counters priced at the model's tier.
fn stage_cost_usd(usage: &StageUsage, table: &PriceTable) -> f64 {
    if let Some(cost) = usage.cost_usd {
        return cost;
    }
    let Some(model) = usage.model.as_deref() else {
        return 0.0;
    };
    let price = table.price(tier(model).as_str());
    reprice_counters(
        usage.input,
        usage.output,
        usage.cache_creation,
        usage.cache_read,
        &price,
    )
}

fn sorted_groups(groups: BTreeMap<String, CostGroup>) -> Vec<CostGroup> {
    let mut groups: Vec<CostGroup> = groups.into_values().collect();
    groups.sort_by(|a, b| {
        b.amount
            .cost_usd
            .total_cmp(&a.amount.cost_usd)
            .then_with(|| b.amount.total_tokens.cmp(&a.amount.total_tokens))
            .then_with(|| a.key.cmp(&b.key))
    });
    groups
}

fn add_group(groups: &mut BTreeMap<String, CostGroup>, key: &str, amount: &CostAmount) {
    let group = groups.entry(key.to_string()).or_insert_with(|| CostGroup {
        key: key.to_string(),
        ..CostGroup::default()
    });
    group.amount.add(amount);
    group.count += 1;
}

/// The cost of one run state: its stages that carry usage.
pub fn run_cost(state: &RunState, table: &PriceTable) -> RunCost {
    let stages: Vec<StageCost> = state
        .stages
        .iter()
        .filter_map(|stage| {
            let usage = stage.usage.as_ref()?;
            Some(StageCost {
                stage: stage.name.clone(),
                agent: usage.agent_type.clone(),
                model: usage.model.clone(),
                amount: CostAmount::new(usage_tokens(usage), stage_cost_usd(usage, table)),
            })
        })
        .collect();
    let mut amount = CostAmount::default();
    for stage in &stages {
        amount.add(&stage.amount);
    }
    RunCost {
        run_id: state.run_id.clone(),
        pipeline_id: state.pipeline_id.clone(),
        started_at: state.started_at.clone(),
        amount,
        stages,
        pixellab: None,
    }
}

/// Folds the runs and the sessions of issue `key` into one report.
pub fn aggregate(key: &str, runs: Vec<RunCost>, records: &[SessionRecord]) -> IssueCost {
    let mut totals = CostAmount::default();
    let mut by_stage = BTreeMap::new();
    let mut by_agent = BTreeMap::new();
    for run in &runs {
        totals.add(&run.amount);
        for stage in &run.stages {
            add_group(&mut by_stage, &stage.stage, &stage.amount);
            add_group(
                &mut by_agent,
                stage.agent.as_deref().unwrap_or(UNKNOWN_AGENT),
                &stage.amount,
            );
        }
    }
    let mut sessions = CostAmount::default();
    let mut session_count = 0;
    for record in records.iter().filter(|r| ticket_of(r) == Some(key)) {
        sessions.add(&CostAmount::new(
            session_tokens(record),
            record.cost_usd.unwrap_or(0.0),
        ));
        session_count += 1;
    }
    let recorded: Vec<&RunPixellabUsage> = runs
        .iter()
        .filter_map(|run| run.pixellab.as_ref())
        .filter(|usage| usage.generations > 0 || usage.spent.is_some())
        .collect();
    let pixellab = (!recorded.is_empty()).then(|| PixellabTotals {
        generations: recorded.iter().map(|usage| usage.generations).sum(),
        credits: recorded
            .iter()
            .filter_map(|usage| usage.spent)
            .fold(None, |sum, spent| Some(sum.unwrap_or(0.0) + spent)),
    });
    IssueCost {
        key: key.to_string(),
        totals,
        runs,
        by_stage: sorted_groups(by_stage),
        by_agent: sorted_groups(by_agent),
        sessions,
        session_count,
        pixellab,
    }
}

/// Reads every run of `key` under the vault and aggregates them with the sessions.
pub fn issue_cost_report(vault: &Path, key: &str, records: &[SessionRecord]) -> IssueCost {
    let table = PriceTable::from_env_or_embedded();
    let mut run_dirs = Vec::new();
    if !vault.as_os_str().is_empty() {
        if let Ok(repos) = std::fs::read_dir(vault.join(PIPELINES_DIR)) {
            for repo in repos.flatten() {
                if let Ok(runs) = std::fs::read_dir(repo.path().join(key)) {
                    run_dirs.extend(runs.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
                }
            }
        }
    }
    run_dirs.sort_by(|a, b| b.cmp(a));
    run_dirs.truncate(MAX_RUNS_PER_TICKET);
    let runs = run_dirs
        .iter()
        .filter_map(|dir| {
            let state = crate::runs::read_run_state(dir)?;
            let mut run = run_cost(&state, &table);
            let usage = aggregate_run(&run.run_id, dir);
            run.pixellab = (usage.stages > 0).then_some(usage);
            Some(run)
        })
        .collect();
    aggregate(key, runs, records)
}

#[cfg(test)]
#[path = "../tests/unit/issue_cost_tests.rs"]
mod tests;
