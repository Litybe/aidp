//! Token cost per ticket: the pure report behind `GET /api/tickets/:key/cost`, the ticket-aware
//! `GET /api/usage` groupings, `GET /api/cost/anomalies` and the report footer.
//! The key of a ticket is the `origin.ref` of its flow sessions.

use crate::models::constants::ticket_cost::{
    ANOMALY_INSUFFICIENT, ANOMALY_MIN_POINTS, ANOMALY_OK, FIVE_HOUR_SECONDS, FOOTER_PREFIX,
    FOOTER_UNIT_MILLION, FOOTER_UNIT_THOUSAND, FOOTER_WINDOW_SUFFIX, GROUP_MODEL, GROUP_ROUTE,
    GROUP_STAGE, GROUP_TICKET, MAX_RUNS_PER_TICKET, MILLION, PIPELINES_DIR, SEVEN_DAY_SECONDS,
    THOUSAND, TOTAL_KEY, UNKNOWN_KEY,
};
use crate::sessions::{OriginKind, RateLimitInfo, RateLimitWindow, SessionRecord, TokenUsage};
use crate::usage::{in_range, UsageBucket, UsageReport};
use aidp_core::cost::repricing::reprice_counters;
use aidp_core::cost::{tier, PriceTable};
use aidp_core::cost::anomaly::{is_anomaly, median_and_mad, modified_z_scores};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Token counters of one slice of work.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

impl TokenTotals {
    /// Every counter added up.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_create
    }

    /// Cache reads and cache writes together.
    pub fn cache(&self) -> u64 {
        self.cache_read + self.cache_create
    }

    fn add(&mut self, other: &TokenTotals) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_create += other.cache_create;
    }
}

impl From<&TokenUsage> for TokenTotals {
    fn from(usage: &TokenUsage) -> Self {
        TokenTotals {
            input: usage.input,
            output: usage.output,
            cache_read: usage.cache_read,
            cache_create: usage.cache_create,
        }
    }
}

/// One row of a breakdown: its key, its counters and how many sessions fed it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostLine {
    pub key: String,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub total_tokens: u64,
    pub sessions: u64,
}

impl CostLine {
    fn new(key: &str) -> Self {
        CostLine {
            key: key.to_string(),
            ..CostLine::default()
        }
    }

    fn add(&mut self, tokens: &TokenTotals, sessions: u64) {
        self.tokens.add(tokens);
        self.total_tokens = self.tokens.total();
        self.sessions += sessions;
    }
}

/// Tokens a stage of a pipeline run burnt, read from the run state.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StageRow {
    pub run_id: String,
    pub stage: String,
    pub model: Option<String>,
    pub tokens: TokenTotals,
}

/// The part of the 5-hour and 7-day windows a ticket used, as fractions of a whole window.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowShare {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

/// `GET /api/tickets/:key/cost`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TicketCost {
    pub key: String,
    pub route: Option<String>,
    pub totals: CostLine,
    pub by_flow: Vec<CostLine>,
    pub by_step: Vec<CostLine>,
    pub by_stage: Vec<CostLine>,
    pub by_model: Vec<CostLine>,
    pub window_share: WindowShare,
    /// Dollars of the ticket's sessions: recorded `cost_usd`, else tokens priced at the model's tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixellab: Option<serde_json::Value>,
}

fn started_ms(record: &SessionRecord) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&record.started_at)
        .ok()
        .map(|moment| moment.timestamp_millis())
}

/// The ticket a session belongs to. A stage session of a pipeline run (`origin.kind == "run"`)
/// carries the run id in `origin.ref` and the ticket in `target`; any other origin (the
/// dispatcher, a flow) carries the ticket key in `origin.ref`.
pub fn ticket_of(record: &SessionRecord) -> Option<&str> {
    if record.origin.is(OriginKind::Run) {
        return record
            .target
            .as_deref()
            .filter(|target| !target.is_empty())
            .or_else(|| ticket_in_vault_path(&record.cwd));
    }
    record
        .origin
        .r#ref
        .as_deref()
        .filter(|reference| !reference.is_empty())
}

/// The ticket of a run folder `…/Pipelines/<repo>/<ticket>/…`, for run sessions started with no
/// `target` (the pipeline doctor works in `Pipelines/<repo>/<ticket>/doctor/<case>`).
pub fn ticket_in_vault_path(cwd: &str) -> Option<&str> {
    let mut parts = cwd.split(['\\', '/']).filter(|part| !part.is_empty());
    parts.find(|part| part.eq_ignore_ascii_case(PIPELINES_DIR))?;
    parts.next()?;
    parts.next()
}

/// The pipeline run a stage session belongs to (`origin.ref` of a `run` origin).
pub fn run_of(record: &SessionRecord) -> Option<&str> {
    record
        .origin
        .r#ref
        .as_deref()
        .filter(|reference| !reference.is_empty() && record.origin.is(OriginKind::Run))
}

/// The token counters of a session, zero while it has reported none.
pub fn session_tokens(record: &SessionRecord) -> TokenTotals {
    record
        .usage
        .as_ref()
        .map(TokenTotals::from)
        .unwrap_or_default()
}

fn sorted_lines(lines: BTreeMap<String, CostLine>) -> Vec<CostLine> {
    let mut lines: Vec<CostLine> = lines.into_values().collect();
    lines.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| a.key.cmp(&b.key))
    });
    lines
}

fn window_fraction(
    all: &[SessionRecord],
    key: &str,
    window: &RateLimitWindow,
    span_seconds: i64,
) -> f64 {
    let end_ms = window.resets_at * 1000;
    let start_ms = end_ms - span_seconds * 1000;
    let (mut ticket_tokens, mut all_tokens) = (0u64, 0u64);
    for record in all {
        let Some(started) = started_ms(record) else {
            continue;
        };
        if started < start_ms || started > end_ms {
            continue;
        }
        let tokens = session_tokens(record).total();
        all_tokens += tokens;
        if ticket_of(record) == Some(key) {
            ticket_tokens += tokens;
        }
    }
    if all_tokens == 0 {
        return 0.0;
    }
    window.utilization * ticket_tokens as f64 / all_tokens as f64
}

/// The fraction of each rate-limit window a ticket used: the utilization of the window times
/// the ticket's share of the tokens of the sessions started inside that window.
pub fn window_share(
    all: &[SessionRecord],
    key: &str,
    rate_limit: Option<&RateLimitInfo>,
) -> WindowShare {
    let Some(info) = rate_limit else {
        return WindowShare::default();
    };
    WindowShare {
        five_hour: info
            .five_hour
            .as_ref()
            .map(|window| window_fraction(all, key, window, FIVE_HOUR_SECONDS)),
        seven_day: info
            .seven_day
            .as_ref()
            .map(|window| window_fraction(all, key, window, SEVEN_DAY_SECONDS)),
    }
}

fn add_to(lines: &mut BTreeMap<String, CostLine>, key: &str, tokens: &TokenTotals) {
    lines
        .entry(key.to_string())
        .or_insert_with(|| CostLine::new(key))
        .add(tokens, 1);
}

/// Dollars of one session: its recorded `cost_usd`, else its tokens priced at its model's tier.
fn session_cost_usd(record: &SessionRecord, tokens: &TokenTotals, table: &PriceTable) -> Option<f64> {
    if let Some(cost) = record.cost_usd {
        return Some(cost);
    }
    let model = record.model.as_deref()?;
    let price = table.price(tier(model).as_str());
    Some(reprice_counters(
        tokens.input,
        tokens.output,
        tokens.cache_create,
        tokens.cache_read,
        &price,
    ))
}

/// The cost of ticket `key`: its sessions grouped by pipeline (flow), step, stage and model.
/// `stages` are the stage rows of the ticket's runs; `all` is every session, for the window share.
pub fn ticket_cost(
    key: &str,
    all: &[SessionRecord],
    stages: &[StageRow],
    route: Option<String>,
    rate_limit: Option<&RateLimitInfo>,
) -> TicketCost {
    let mut totals = CostLine::new(TOTAL_KEY);
    let mut by_flow = BTreeMap::new();
    let mut by_step = BTreeMap::new();
    let mut by_model = BTreeMap::new();
    let table = PriceTable::from_env_or_embedded();
    let mut cost_usd: Option<f64> = None;
    for record in all.iter().filter(|record| ticket_of(record) == Some(key)) {
        let tokens = session_tokens(record);
        totals.add(&tokens, 1);
        if let Some(dollars) = session_cost_usd(record, &tokens, &table) {
            cost_usd = Some(cost_usd.unwrap_or(0.0) + dollars);
        }
        add_to(
            &mut by_flow,
            record.template_id.as_deref().unwrap_or(UNKNOWN_KEY),
            &tokens,
        );
        add_to(
            &mut by_step,
            record.origin.label.as_deref().unwrap_or(UNKNOWN_KEY),
            &tokens,
        );
        if stages.is_empty() {
            add_to(
                &mut by_model,
                record.model.as_deref().unwrap_or(UNKNOWN_KEY),
                &tokens,
            );
        }
    }
    let mut by_stage = BTreeMap::new();
    for row in stages {
        add_to(&mut by_stage, &row.stage, &row.tokens);
        add_to(
            &mut by_model,
            row.model.as_deref().unwrap_or(UNKNOWN_KEY),
            &row.tokens,
        );
    }
    TicketCost {
        key: key.to_string(),
        route,
        totals,
        by_flow: sorted_lines(by_flow),
        by_step: sorted_lines(by_step),
        by_stage: sorted_lines(by_stage),
        by_model: sorted_lines(by_model),
        window_share: window_share(all, key, rate_limit),
        cost_usd,
        budget: None,
        pixellab: None,
    }
}

/// Stage rows of every run of ticket `key` under `<vault>/Pipelines/<repo>/<key>/<run>/`.
pub fn load_stage_rows(vault: &Path, key: &str) -> Vec<StageRow> {
    let Ok(repos) = std::fs::read_dir(vault.join(PIPELINES_DIR)) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for repo in repos.flatten() {
        let Ok(runs) = std::fs::read_dir(repo.path().join(key)) else {
            continue;
        };
        let mut run_dirs: Vec<_> = runs.flatten().map(|run| run.path()).collect();
        run_dirs.sort_by(|a, b| b.cmp(a));
        run_dirs.truncate(MAX_RUNS_PER_TICKET);
        for run_dir in run_dirs {
            if let Some(state) = crate::runs::read_run_state(&run_dir) {
                rows.extend(stage_rows_of(&state));
            }
        }
    }
    rows
}

/// The stages of one run state that carry token usage.
pub fn stage_rows_of(state: &aidp_core::models::run_state::RunState) -> Vec<StageRow> {
    state
        .stages
        .iter()
        .filter_map(|stage| {
            let usage = stage.usage.as_ref()?;
            Some(StageRow {
                run_id: state.run_id.clone(),
                stage: stage.name.clone(),
                model: usage.model.clone(),
                tokens: TokenTotals {
                    input: usage.input,
                    output: usage.output,
                    cache_read: usage.cache_read,
                    cache_create: usage.cache_creation,
                },
            })
        })
        .collect()
}

/// What `groupBy=stage|route` needs besides the sessions: stage rows and route per ticket.
#[derive(Default)]
pub struct CostContext {
    pub stages: HashMap<String, Vec<StageRow>>,
    pub routes: HashMap<String, String>,
    /// Route of each run id, read from its run state; wins over the ticket's route.
    pub run_routes: HashMap<String, String>,
}

/// Whether `groupBy` is served by [`aggregate_contextual`].
pub fn is_contextual_group(group_by: &str) -> bool {
    matches!(
        group_by,
        GROUP_STAGE | GROUP_ROUTE | GROUP_TICKET | GROUP_MODEL
    )
}

fn add_tokens(bucket: &mut UsageBucket, tokens: &TokenTotals) {
    bucket.input_tokens += tokens.input;
    bucket.output_tokens += tokens.output;
    bucket.cache_read_tokens += tokens.cache_read;
    bucket.cache_create_tokens += tokens.cache_create;
    bucket.total_tokens += tokens.total();
}

fn add_session(bucket: &mut UsageBucket, record: &SessionRecord, tokens: &TokenTotals) {
    add_tokens(bucket, tokens);
    bucket.sessions += 1;
    bucket.turns += u64::from(record.turns);
    bucket.estimated_cost_usd += record.cost_usd.unwrap_or(0.0);
}

fn bucket_for<'a>(buckets: &'a mut HashMap<String, UsageBucket>, key: &str) -> &'a mut UsageBucket {
    buckets
        .entry(key.to_string())
        .or_insert_with(|| UsageBucket {
            key: key.to_string(),
            ..UsageBucket::default()
        })
}

/// `GET /api/usage` for `groupBy=ticket|model|stage|route`.
/// Stage buckets count the stage rows of the tickets that have a session in range.
pub fn aggregate_contextual(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
    group_by: &str,
    context: &CostContext,
) -> UsageReport {
    let mut totals = UsageBucket {
        key: TOTAL_KEY.to_string(),
        ..UsageBucket::default()
    };
    let mut buckets: HashMap<String, UsageBucket> = HashMap::new();
    let mut seen_tickets: Vec<&str> = Vec::new();
    for record in records.iter().filter(|r| in_range(&r.started_at, from, to)) {
        let tokens = session_tokens(record);
        add_session(&mut totals, record, &tokens);
        let ticket = ticket_of(record);
        let key = match group_by {
            GROUP_TICKET => ticket.unwrap_or(UNKNOWN_KEY).to_string(),
            GROUP_MODEL => record
                .model
                .clone()
                .unwrap_or_else(|| UNKNOWN_KEY.to_string()),
            GROUP_ROUTE => run_of(record)
                .and_then(|run| context.run_routes.get(run).cloned())
                .or_else(|| ticket.and_then(|key| context.routes.get(key).cloned()))
                .or_else(|| {
                    record
                        .template_id
                        .as_deref()
                        .and_then(aidp_tickets::ticket_route::route_of_template)
                        .map(|route| route.as_str().to_string())
                })
                .unwrap_or_else(|| UNKNOWN_KEY.to_string()),
            _ => {
                if let Some(ticket) = ticket.filter(|ticket| !seen_tickets.contains(ticket)) {
                    seen_tickets.push(ticket);
                }
                continue;
            }
        };
        let bucket = bucket_for(&mut buckets, &key);
        add_session(bucket, record, &tokens);
        if group_by == GROUP_TICKET {
            if let Some(run) = run_of(record).filter(|run| !bucket.runs.iter().any(|r| r == run)) {
                bucket.runs.push(run.to_string());
            }
        }
    }
    if group_by == GROUP_STAGE {
        for row in seen_tickets
            .iter()
            .filter_map(|ticket| context.stages.get(*ticket))
            .flatten()
        {
            let bucket = bucket_for(&mut buckets, &row.stage);
            add_tokens(bucket, &row.tokens);
            bucket.sessions += 1;
        }
    }
    let mut buckets: Vec<UsageBucket> = buckets.into_values().collect();
    buckets.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| a.key.cmp(&b.key))
    });
    UsageReport {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        group_by: group_by.to_string(),
        totals,
        buckets,
    }
}

/// One ticket's total in the anomaly report.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyPoint {
    pub key: String,
    pub total_tokens: u64,
    pub modified_z: f64,
    pub flagged: bool,
}

/// `GET /api/cost/anomalies`: tickets whose token total is an outlier among all tickets.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyReport {
    pub status: &'static str,
    pub n: usize,
    pub median: f64,
    pub mad: f64,
    pub points: Vec<AnomalyPoint>,
    pub any_flagged: bool,
}

/// Flags the tickets whose token total has a modified z-score above the shared threshold.
/// Fewer than three tickets, or a zero spread, flags nothing.
pub fn anomalies(records: &[SessionRecord], from: Option<&str>, to: Option<&str>) -> AnomalyReport {
    let mut per_ticket: BTreeMap<String, u64> = BTreeMap::new();
    for record in records.iter().filter(|r| in_range(&r.started_at, from, to)) {
        if let Some(ticket) = ticket_of(record) {
            *per_ticket.entry(ticket.to_string()).or_default() += session_tokens(record).total();
        }
    }
    let n = per_ticket.len();
    if n < ANOMALY_MIN_POINTS {
        return AnomalyReport {
            status: ANOMALY_INSUFFICIENT,
            n,
            median: 0.0,
            mad: 0.0,
            points: Vec::new(),
            any_flagged: false,
        };
    }
    let values: Vec<f64> = per_ticket.values().map(|total| *total as f64).collect();
    let (median, mad) = median_and_mad(&values);
    let scores = modified_z_scores(&values);
    let points: Vec<AnomalyPoint> = per_ticket
        .into_iter()
        .zip(scores)
        .map(|((key, total_tokens), modified_z)| AnomalyPoint {
            key,
            total_tokens,
            modified_z,
            flagged: mad != 0.0 && is_anomaly(modified_z),
        })
        .collect();
    AnomalyReport {
        status: ANOMALY_OK,
        n,
        median,
        mad,
        any_flagged: points.iter().any(|point| point.flagged),
        points,
    }
}

/// `1,2 M` above a million, `340 k` below; French decimal comma.
fn amount(tokens: u64, million: bool) -> String {
    if million {
        format!("{:.1}", tokens as f64 / MILLION as f64).replace('.', ",") + FOOTER_UNIT_MILLION
    } else {
        format!("{:.0}", tokens as f64 / THOUSAND as f64) + FOOTER_UNIT_THOUSAND
    }
}

/// The report footer of the cost contract:
/// `Coût : 1,2 M tokens (in 0,3 M / out 0,1 M / cache 0,8 M), 4 % de la fenêtre 5 h`.
/// The unit (`M` or `k`) follows the total; the window clause is left out when the share of the
/// 5-hour window is unknown.
pub fn footer(tokens: &TokenTotals, five_hour_share: Option<f64>) -> String {
    let total = tokens.total();
    let million = total >= MILLION;
    let mut text = format!(
        "{FOOTER_PREFIX}{} tokens (in {} / out {} / cache {})",
        amount(total, million),
        amount(tokens.input, million),
        amount(tokens.output, million),
        amount(tokens.cache(), million),
    );
    if let Some(share) = five_hour_share {
        let percent = share * 100.0;
        let shown = if percent > 0.0 && percent < 0.5 {
            "<1".to_string()
        } else {
            format!("{percent:.0}")
        };
        text.push_str(", ");
        text.push_str(&shown);
        text.push_str(FOOTER_WINDOW_SUFFIX);
    }
    text
}

/// Footer of a ticket's whole cost: the function the report writers call.
pub fn ticket_footer(cost: &TicketCost) -> String {
    footer(&cost.totals.tokens, cost.window_share.five_hour)
}

/// Footer of one pipeline run, from its stage rows.
pub fn run_footer(stages: &[StageRow], five_hour_share: Option<f64>) -> String {
    let mut tokens = TokenTotals::default();
    for row in stages {
        tokens.add(&row.tokens);
    }
    footer(&tokens, five_hour_share)
}

#[cfg(test)]
#[path = "../tests/unit/ticket_cost_tests.rs"]
mod tests;
