//! Model pricing and cost computation.
//!
//! Prices come from the embedded `assets/model-prices.json` (USD per million
//! tokens). [`PriceTable`] owns one parsed grid, overridable through
//! `AIDP_MODEL_PRICES`; a bad override is reported, never a panic.
//!
//! An unknown model tier is priced at zero, never at a guessed number — a
//! cost derived from an unrecognized model id must be reported as
//! `source: "unavailable"` by the caller, never as a silent zero-cost row.

use crate::models::constants::cost::{
    BATCH_SERVICE_TIER, MODEL_PRICES_ENV, UNKNOWN_PRICING_VERSION, UNKNOWN_TIER,
};
use crate::pipeline::Stage;
use crate::run::ComplexityBucket;
use crate::tokens::TokenUsage;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub mod anomaly;
pub mod repricing;

const EMBEDDED_PRICES_JSON: &str = include_str!("../assets/model-prices.json");

#[derive(Debug, Clone, Copy, Default)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

#[derive(Deserialize)]
struct PriceRow {
    input: f64,
    output: f64,
    cache_write_5m: f64,
    cache_write_1h: f64,
    cache_read: f64,
}

#[derive(Deserialize)]
struct PricesFile {
    pricing_version: String,
    tiers: BTreeMap<String, PriceRow>,
}

/// Where the raw pricing grid text came from, used to word a load error.
enum PricesOrigin {
    Embedded,
    Override(PathBuf),
}

/// Parse a pricing grid, naming its origin in the error so a bad override is actionable.
fn parse_prices(raw: &str, origin: &PricesOrigin) -> Result<PricesFile, String> {
    serde_json::from_str(raw).map_err(|error| match origin {
        PricesOrigin::Embedded => {
            format!("embedded model-prices.json is not a valid pricing grid: {error}")
        }
        PricesOrigin::Override(path) => format!(
            "{MODEL_PRICES_ENV}={} is not a valid pricing grid: {error}",
            path.display()
        ),
    })
}

/// Word a read failure on an override path the same way [`parse_prices`]
/// words a parse failure, so both name the env var and the resolved path.
fn read_error(path: &std::path::Path, error: std::io::Error) -> String {
    format!(
        "{MODEL_PRICES_ENV}={} could not be read: {error}",
        path.display()
    )
}

/// Read the override grid when set to a non-empty value, else the embedded
/// one, then parse it. A set but unreadable override (missing file,
/// permissions, non-UTF-8 content) is a load error, not a silent fallback
/// to the embedded grid — an empty value still means no override.
fn load_prices() -> Result<PricesFile, String> {
    match std::env::var(MODEL_PRICES_ENV) {
        Ok(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            let absolute = std::path::absolute(&path).unwrap_or(path);
            let raw = std::fs::read_to_string(&absolute).map_err(|e| read_error(&absolute, e))?;
            parse_prices(&raw, &PricesOrigin::Override(absolute))
        }
        _ => parse_prices(EMBEDDED_PRICES_JSON, &PricesOrigin::Embedded),
    }
}

/// An owned, parsed pricing grid. Built fresh by its constructors — no
/// global cache, so an env-var override is re-read on every construction
/// rather than pinned to whatever it held on first use.
pub struct PriceTable {
    result: Result<PricesFile, String>,
}

impl PriceTable {
    /// The pricing grid embedded in the binary at build time.
    pub fn embedded() -> Self {
        Self {
            result: parse_prices(EMBEDDED_PRICES_JSON, &PricesOrigin::Embedded),
        }
    }

    /// Parse a pricing grid from raw JSON text, as if it were the embedded grid.
    pub fn from_json(raw: &str) -> Self {
        Self {
            result: parse_prices(raw, &PricesOrigin::Embedded),
        }
    }

    /// The override grid named by [`MODEL_PRICES_ENV`] when set to a
    /// non-empty value, else the embedded grid.
    pub fn from_env_or_embedded() -> Self {
        Self {
            result: load_prices(),
        }
    }

    /// Why the pricing grid could not be loaded, or `None` when it loaded.
    pub fn load_error(&self) -> Option<&str> {
        self.result.as_ref().err().map(String::as_str)
    }

    /// The pricing-grid version string (e.g. `"2026-09-01"`), or
    /// [`UNKNOWN_PRICING_VERSION`] when the grid failed to load — check
    /// [`Self::load_error`] first to tell the two apart.
    pub fn pricing_version(&self) -> &str {
        self.result
            .as_ref()
            .map(|file| file.pricing_version.as_str())
            .unwrap_or(UNKNOWN_PRICING_VERSION)
    }

    /// Per-million-token prices for a tier; an unknown tier or an unloaded
    /// grid yields all-zero, so check [`Self::load_error`] first.
    pub fn price(&self, tier: &str) -> Price {
        let Ok(file) = &self.result else {
            return Price::default();
        };
        let row = file
            .tiers
            .get(tier)
            .or_else(|| file.tiers.get(UNKNOWN_TIER));
        match row {
            Some(r) => Price {
                input: r.input,
                output: r.output,
                cache_write_5m: r.cache_write_5m,
                cache_write_1h: r.cache_write_1h,
                cache_read: r.cache_read,
            },
            None => Price::default(),
        }
    }

    /// USD cost of one usage record for a tier; a `"batch"` service tier is
    /// billed at half price. Zero when the grid failed to load.
    pub fn cost_usd(&self, tier: &str, u: &TokenUsage, service_tier: Option<&str>) -> f64 {
        let p = self.price(tier);
        let per_million = |tokens: u64, rate: f64| (tokens as f64) * rate / 1_000_000.0;

        let split_total = u.cache_creation_5m_tokens + u.cache_creation_1h_tokens;
        let unsplit = u.cache_creation_input_tokens.saturating_sub(split_total);

        let mut total = per_million(u.input_tokens, p.input)
            + per_million(u.output_tokens, p.output)
            + per_million(u.cache_creation_5m_tokens, p.cache_write_5m)
            + per_million(u.cache_creation_1h_tokens, p.cache_write_1h)
            + per_million(unsplit, p.cache_write_5m)
            + per_million(u.cache_read_input_tokens, p.cache_read);

        if service_tier == Some(BATCH_SERVICE_TIER) {
            total *= 0.5;
        }

        total
    }
}

/// Leak an owned copy of `s` into a `'static` borrow for the free-function
/// wrappers, whose [`PriceTable`] does not outlive the call.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// Why the pricing grid could not be loaded, or `None` when it loaded.
pub fn load_error() -> Option<&'static str> {
    PriceTable::from_env_or_embedded()
        .load_error()
        .map(leak_str)
}

/// Returns the pricing-grid version string (e.g. `"2026-09-01"`), or `None`
/// when the grid failed to load.
pub fn pricing_version() -> Option<&'static str> {
    let table = PriceTable::from_env_or_embedded();
    match table.load_error() {
        Some(_) => None,
        None => Some(leak_str(table.pricing_version())),
    }
}

/// Classify a model id into a pricing tier by SUBSTRING match — real model
/// ids observed in the wild (`claude-sonnet-5`, `claude-opus-4-8`,
/// `claude-opus-5`, ...) do not follow one stable naming scheme, so a
/// substring match against `haiku|sonnet|opus|fable` is the only robust approach.
/// `opus-5-5` is checked before `opus`: Opus 5.5 has its own, lower price row.
/// Anything else maps to [`ModelTier::Unknown`] (priced at zero — see module docs).
pub fn tier(model_id: &str) -> ModelTier {
    let lower = model_id.to_ascii_lowercase();
    if lower.contains("haiku") {
        ModelTier::Haiku
    } else if lower.contains("sonnet") {
        ModelTier::Sonnet
    } else if lower.contains("opus-5-5") {
        ModelTier::Opus55
    } else if lower.contains("opus") {
        ModelTier::Opus
    } else if lower.contains("fable") {
        ModelTier::Fable
    } else {
        ModelTier::Unknown
    }
}

/// Pricing tier of a model, keyed in the price grid by [`ModelTier::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Haiku,
    Sonnet,
    Opus55,
    Opus,
    Fable,
    Unknown,
}

impl ModelTier {
    /// Price-grid key of the tier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Haiku => "haiku",
            Self::Sonnet => "sonnet",
            Self::Opus55 => "opus-5-5",
            Self::Opus => "opus",
            Self::Fable => "fable",
            Self::Unknown => "unknown",
        }
    }
}

/// Look up the per-million-token prices for a tier, using the env override
/// or the embedded grid. See [`PriceTable::price`] for the fallback rules.
pub fn price(tier: &str) -> Price {
    PriceTable::from_env_or_embedded().price(tier)
}

/// [`PriceTable::cost_usd`] on the env override or the embedded grid.
/// `inference_geo` has no known pricing effect and is deliberately ignored.
pub fn cost_usd(tier: &str, u: &TokenUsage, service_tier: Option<&str>) -> f64 {
    PriceTable::from_env_or_embedded().cost_usd(tier, u, service_tier)
}

/// Deterministic, auditable complexity bucket for a stage — NOT a lexical
/// score. Rules, in order:
/// * `large` — `plan_tasks` present, OR `scope.mode == "repo_wide"`, OR more
///   than 5 files in the plan's write-set.
/// * `small` — no `plan_tasks`/`fan_out`, prompt text < 8000 chars, and the
///   scope is a single file.
/// * `medium` — default.
pub fn bucket(stage: &Stage, prompt_chars: usize, plan_write_set_files: usize) -> ComplexityBucket {
    let scope_mode = stage.scope.as_ref().and_then(|s| s.mode.as_deref());

    if stage.plan_tasks.is_some() || scope_mode == Some("repo_wide") || plan_write_set_files > 5 {
        return ComplexityBucket::Large;
    }

    let scope_single_file = stage
        .scope
        .as_ref()
        .map(|s| s.allowed.len() <= 1 && s.writes.len() <= 1)
        .unwrap_or(false);

    if stage.plan_tasks.is_none()
        && stage.fan_out.is_none()
        && prompt_chars < 8000
        && scope_single_file
    {
        return ComplexityBucket::Small;
    }

    ComplexityBucket::Medium
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PlanTasksSpec, ScopeSpec, Stage};
    use crate::tokens::TokenUsage;

    fn base_stage(name: &str) -> Stage {
        Stage {
            name: name.to_string(),
            description: String::new(),
            policy: vec![],
            r#type: None,
            uses: None,
            composite_entry: None,
            composite_when: vec![],
            agent: None,
            model: None,
            effort: None,
            provider: None,
            skills: vec![],
            skills_by: None,
            needs: vec![],
            inputs: Default::default(),
            outputs: vec![],
            with: Default::default(),
            when: None,
            gate: None,
            scope: None,
            isolation: None,
            tools: None,
            fan_out: None,
            merge_findings: None,
            on_harness_gap: None,
            plan_tasks: None,
            max_iterations: None,
            on_exhausted: None,
            knowledge: None,
            body: vec![],
        }
    }

    #[test]
    fn four_components_and_unknown_tier() {
        assert_eq!(tier("claude-sonnet-5"), ModelTier::Sonnet);
        assert_eq!(tier("claude-opus-4-8"), ModelTier::Opus);
        assert_eq!(tier("claude-opus-5"), ModelTier::Opus);
        assert_eq!(tier("claude-opus-5-5"), ModelTier::Opus55);
        assert_eq!(tier("claude-fable-5-1"), ModelTier::Fable);
        assert_eq!(price("fable").output, 50.0);
        assert_eq!(price("opus-5-5").input, 4.0);
        assert_eq!(tier("gpt-4"), ModelTier::Unknown);

        let unknown_price = price("unknown");
        assert_eq!(unknown_price.input, 0.0);
        assert_eq!(unknown_price.output, 0.0);
        assert_eq!(unknown_price.cache_read, 0.0);

        let unknown_usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 1000,
            cache_creation_input_tokens: 1000,
            cache_read_input_tokens: 1000,
            total_tokens: 2000,
            billable_total: 4000,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
        };
        assert_eq!(cost_usd("unknown", &unknown_usage, None), 0.0);

        // Recompute fx01's volumes by hand against the sonnet tier.
        let fx01 = TokenUsage {
            input_tokens: 3,
            output_tokens: 728,
            cache_creation_input_tokens: 60_317,
            cache_read_input_tokens: 19_236,
            total_tokens: 731,
            billable_total: 80_284,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
        };
        let p = price("sonnet");
        let expected = (3.0 * p.input
            + 728.0 * p.output
            + 60_317.0 * p.cache_write_5m
            + 19_236.0 * p.cache_read)
            / 1_000_000.0;
        let got = cost_usd("sonnet", &fx01, None);
        assert!(
            (got - expected).abs() < 1e-9,
            "got={got} expected={expected}"
        );

        // Batch service tier halves the cost.
        let batch = cost_usd("sonnet", &fx01, Some("batch"));
        assert!((batch - expected * 0.5).abs() < 1e-9);
    }

    #[test]
    fn bucket_is_deterministic() {
        let mut large_stage = base_stage("implement");
        large_stage.plan_tasks = Some(PlanTasksSpec {
            from: "{{ stages.plan.outputs.plan }}".to_string(),
            max: 3,
            kinds: vec![],
            merge_hint: None,
            skills_by_kind: Default::default(),
        });
        assert_eq!(bucket(&large_stage, 100, 0), ComplexityBucket::Large);
        assert_eq!(bucket(&large_stage, 100, 0), ComplexityBucket::Large);

        let mut repo_wide_stage = base_stage("implement2");
        repo_wide_stage.scope = Some(ScopeSpec {
            mode: Some("repo_wide".to_string()),
            allowed: vec![],
            denied: vec![],
            writes: vec![],
            reads: vec![],
        });
        assert_eq!(bucket(&repo_wide_stage, 100, 0), ComplexityBucket::Large);

        let many_files_stage = base_stage("implement3");
        assert_eq!(bucket(&many_files_stage, 100, 6), ComplexityBucket::Large);

        let mut small_stage = base_stage("tiny");
        small_stage.scope = Some(ScopeSpec {
            mode: Some("custom".to_string()),
            allowed: vec!["src/one.rs".to_string()],
            denied: vec![],
            writes: vec!["src/one.rs".to_string()],
            reads: vec![],
        });
        let r1 = bucket(&small_stage, 500, 0);
        let r2 = bucket(&small_stage, 500, 0);
        assert_eq!(r1, ComplexityBucket::Small);
        assert_eq!(r1, r2, "bucket must be idempotent across repeated calls");

        let default_stage = base_stage("plain");
        assert_eq!(bucket(&default_stage, 100, 0), ComplexityBucket::Medium);
    }
}

#[cfg(test)]
#[path = "../tests/unit/cost_tests.rs"]
mod cost_tests;
