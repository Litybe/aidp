//! Usage: two pure things behind `GET /api/usage` and `GET /api/usage/rate-limit`.
//!
//! (1) A pure aggregation over the session records the daemon already keeps,
//! grouped by day / project / template / origin — no I/O, no disk fixtures,
//! same pure/impure split as `inbox.rs` (see that module's header).
//!
//! (2) The append-only history of the `rate_limit` snapshots the claude CLI
//! emits, which today only ever lives in memory and is lost on restart. This
//! module owns the on-disk file format (`store::append_jsonl`/`read_jsonl`)
//! and the "what can we trust right after a restart" query.
//!
//! No price table, no dependency on `aidp-core`: `SessionRecord.cost_usd` is
//! already filled in from the CLI's own `total_cost_usd`, so the aggregate
//! only ever sums it.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::Serialize;

use crate::sessions::{RateLimitInfo, SessionRecord};
use crate::store::{append_jsonl, read_jsonl};

// ---------------------------------------------------------------------
// Rate-limit snapshot history
// ---------------------------------------------------------------------

const DEFAULT_HISTORY_LIMIT: usize = 200;

pub fn rate_limit_file(state_dir: &Path) -> PathBuf {
    state_dir.join("rate-limit.jsonl")
}

pub fn append_rate_limit(state_dir: &Path, info: &RateLimitInfo) {
    append_jsonl(&rate_limit_file(state_dir), info);
}

/// Newest first, at most `limit` entries (0 = a sane default of 200).
pub fn read_rate_limit_history(state_dir: &Path, limit: usize) -> Vec<RateLimitInfo> {
    let limit = if limit == 0 {
        DEFAULT_HISTORY_LIMIT
    } else {
        limit
    };
    let mut entries: Vec<RateLimitInfo> = read_jsonl(&rate_limit_file(state_dir));
    entries.reverse();
    entries.truncate(limit);
    entries
}

/// The newest snapshot whose 5h window has not yet reset — what the daemon
/// can legitimately trust as the current window right after a restart.
/// `None` when there is no history, or the newest entry's window already
/// reset (or has no `five_hour` window at all).
pub fn latest_live_snapshot(state_dir: &Path, now_ms: i64) -> Option<RateLimitInfo> {
    let entries: Vec<RateLimitInfo> = read_jsonl(&rate_limit_file(state_dir));
    let latest = entries.last()?;
    let five_hour = latest.five_hour.as_ref()?;
    if five_hour.resets_at * 1000 > now_ms {
        Some(latest.clone())
    } else {
        None
    }
}

// ---------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_create_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub sessions: u64,
    pub turns: u64,
    /// Run ids behind a `ticket` bucket (tooltip of the page), empty for the other groupings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub group_by: String,
    pub totals: UsageBucket,
    pub buckets: Vec<UsageBucket>,
}

/// `YYYY-MM-DD` becomes `YYYY-MM-DDT00:00:00.000Z`; anything else is
/// returned trimmed as-is.
pub fn normalize_bound(raw: &str) -> String {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    let is_bare_date = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => true,
            _ => b.is_ascii_digit(),
        });
    if is_bare_date {
        format!("{trimmed}T00:00:00.000Z")
    } else {
        trimmed.to_string()
    }
}

/// `from` inclusive, `to` exclusive, plain lexicographic comparison — valid
/// because `started_at` is always `chrono::Utc::now().to_rfc3339_opts(Millis, true)`.
pub fn in_range(started_at: &str, from: Option<&str>, to: Option<&str>) -> bool {
    if let Some(f) = from {
        if started_at < f {
            return false;
        }
    }
    if let Some(t) = to {
        if started_at >= t {
            return false;
        }
    }
    true
}

fn add_record(bucket: &mut UsageBucket, r: &SessionRecord) {
    if let Some(u) = &r.usage {
        bucket.input_tokens += u.input;
        bucket.output_tokens += u.output;
        bucket.cache_read_tokens += u.cache_read;
        bucket.cache_create_tokens += u.cache_create;
        bucket.total_tokens += u.input + u.output + u.cache_read + u.cache_create;
    }
    bucket.estimated_cost_usd += r.cost_usd.unwrap_or(0.0);
    bucket.sessions += 1;
    bucket.turns += r.turns as u64;
}

fn bucket_key(record: &SessionRecord, group_by: &str) -> (String, Option<String>) {
    match group_by {
        "day" => {
            let day = record
                .started_at
                .get(..10)
                .unwrap_or(record.started_at.as_str());
            (day.to_string(), None)
        }
        "project" => (record.project_id.clone(), Some(record.project_name.clone())),
        "template" => (
            record
                .template_id
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            None,
        ),
        "origin" => (record.origin.kind.clone(), None),
        _ => unreachable!("group_by validated by caller"),
    }
}

pub fn aggregate(
    records: &[SessionRecord],
    from: Option<&str>,
    to: Option<&str>,
    group_by: &str,
) -> anyhow::Result<UsageReport> {
    let group_by = if group_by.is_empty() { "day" } else { group_by };
    if !matches!(group_by, "day" | "project" | "template" | "origin") {
        return Err(anyhow!(
            "groupBy must be one of day|project|template|origin|ticket|model|stage|route"
        ));
    }

    let mut totals = UsageBucket {
        key: "total".to_string(),
        ..Default::default()
    };
    let mut buckets: std::collections::HashMap<String, UsageBucket> =
        std::collections::HashMap::new();

    for r in records {
        if !in_range(&r.started_at, from, to) {
            continue;
        }
        add_record(&mut totals, r);

        let (key, label) = bucket_key(r, group_by);
        let bucket = buckets.entry(key.clone()).or_insert_with(|| UsageBucket {
            key: key.clone(),
            label: label.clone(),
            ..Default::default()
        });
        if bucket.label.is_none() {
            bucket.label = label;
        }
        add_record(bucket, r);
    }

    let mut buckets: Vec<UsageBucket> = buckets.into_values().collect();
    if group_by == "day" {
        buckets.sort_by(|a, b| a.key.cmp(&b.key));
    } else {
        buckets.sort_by(|a, b| {
            b.total_tokens
                .cmp(&a.total_tokens)
                .then_with(|| a.key.cmp(&b.key))
        });
    }

    Ok(UsageReport {
        from: from.map(|s| s.to_string()),
        to: to.map(|s| s.to_string()),
        group_by: group_by.to_string(),
        totals,
        buckets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::{Origin, TokenUsage};
    use aidp_testkit::TempWorkspace;

    fn temp_dir(tag: &str) -> TempWorkspace {
        TempWorkspace::new(&format!("usage-test-{tag}"))
    }

    struct MkSession<'a> {
        id: &'a str,
        project_id: &'a str,
        project_name: &'a str,
        template_id: Option<&'a str>,
        origin_kind: &'a str,
        started_at: &'a str,
        turns: u32,
        cost_usd: Option<f64>,
        usage: Option<TokenUsage>,
    }

    impl<'a> Default for MkSession<'a> {
        fn default() -> Self {
            Self {
                id: "",
                project_id: "p1",
                project_name: "Project One",
                template_id: None,
                origin_kind: "cli",
                started_at: "",
                turns: 0,
                cost_usd: None,
                usage: None,
            }
        }
    }

    fn mk_session(args: MkSession) -> SessionRecord {
        SessionRecord {
            mcp_config_id: None,
            provider: None,
            provider_session_id: None,
            id: args.id.to_string(),
            project_id: args.project_id.to_string(),
            project_name: args.project_name.to_string(),
            cwd: "/repo".to_string(),
            engine: "v7".to_string(),
            template_id: args.template_id.map(|s| s.to_string()),
            target: None,
            inputs: None,
            prompt: "do stuff".to_string(),
            permission_mode: "default".to_string(),
            effective_permission_mode: None,
            model: None,
            effort: None,
            max_budget_usd: None,
            status: crate::models::enums::SessionStatus::Done,
            started_at: args.started_at.to_string(),
            ended_at: None,
            start_sha: None,
            exit_code: None,
            claude_session_id: None,
            resumed_from: None,
            cost_usd: args.cost_usd,
            turns: args.turns,
            last_text: None,
            pending_question: None,
            origin: Origin::new(args.origin_kind),
            automation_id: None,
            automation_run_id: None,
            workflow_run_id: None,
            event_count: 0,
            auto_close: false,
            usage: args.usage,
            title: None,
            pinned: false,
            archived: false,
            unread: false,
            context_from: None,
            chain_depth: None,
            workspace: None,
            warnings: Vec::new(),
            headless: false,
        }
    }

    fn mk_usage(input: u64, output: u64, cache_read: u64, cache_create: u64) -> TokenUsage {
        TokenUsage {
            input,
            output,
            cache_read,
            cache_create,
        }
    }

    #[test]
    fn normalize_bound_expands_a_bare_date() {
        assert_eq!(normalize_bound("2026-09-10"), "2026-09-10T00:00:00.000Z");
        assert_eq!(
            normalize_bound("2026-09-10T12:00:00.000Z"),
            "2026-09-10T12:00:00.000Z"
        );
        assert_eq!(
            normalize_bound("  2026-09-10  "),
            "2026-09-10T00:00:00.000Z"
        );
    }

    #[test]
    fn in_range_is_from_inclusive_and_to_exclusive() {
        assert!(in_range(
            "2026-09-10T00:00:00.000Z",
            Some("2026-09-10T00:00:00.000Z"),
            Some("2026-09-11T00:00:00.000Z")
        ));
        assert!(!in_range(
            "2026-09-11T00:00:00.000Z",
            Some("2026-09-10T00:00:00.000Z"),
            Some("2026-09-11T00:00:00.000Z")
        ));
        assert!(!in_range(
            "2026-09-09T23:59:59.000Z",
            Some("2026-09-10T00:00:00.000Z"),
            None
        ));
        assert!(in_range("2026-09-10T00:00:00.000Z", None, None));
    }

    #[test]
    fn aggregate_by_day_sums_tokens_cost_sessions_and_turns() {
        let records = vec![
            mk_session(MkSession {
                id: "s1",
                template_id: Some("dev"),
                started_at: "2026-09-10T08:00:00.000Z",
                turns: 3,
                cost_usd: Some(1.5),
                usage: Some(mk_usage(100, 50, 10, 5)),
                ..Default::default()
            }),
            mk_session(MkSession {
                id: "s2",
                template_id: Some("dev"),
                started_at: "2026-09-10T09:00:00.000Z",
                turns: 2,
                cost_usd: Some(0.5),
                usage: Some(mk_usage(20, 10, 0, 0)),
                ..Default::default()
            }),
            mk_session(MkSession {
                id: "s3",
                template_id: Some("dev"),
                started_at: "2026-09-11T09:00:00.000Z",
                turns: 1,
                cost_usd: Some(2.0),
                usage: Some(mk_usage(5, 5, 0, 0)),
                ..Default::default()
            }),
        ];
        let report = aggregate(&records, None, None, "day").unwrap();
        assert_eq!(report.buckets.len(), 2);
        let day1 = &report.buckets[0];
        assert_eq!(day1.key, "2026-09-10");
        assert_eq!(day1.input_tokens, 120);
        assert_eq!(day1.output_tokens, 60);
        assert_eq!(day1.cache_read_tokens, 10);
        assert_eq!(day1.cache_create_tokens, 5);
        assert_eq!(day1.total_tokens, 195);
        assert!((day1.estimated_cost_usd - 2.0).abs() < 1e-9);
        assert_eq!(day1.sessions, 2);
        assert_eq!(day1.turns, 5);

        let day2 = &report.buckets[1];
        assert_eq!(day2.key, "2026-09-11");
        assert_eq!(day2.sessions, 1);

        assert_eq!(report.totals.key, "total");
        assert_eq!(report.totals.sessions, 3);
        assert_eq!(report.totals.turns, 6);
        assert!((report.totals.estimated_cost_usd - 4.0).abs() < 1e-9);
    }

    #[test]
    fn aggregate_by_project_carries_the_project_name_as_label() {
        let records = vec![mk_session(MkSession {
            id: "s1",
            template_id: Some("dev"),
            started_at: "2026-09-10T08:00:00.000Z",
            turns: 1,
            ..Default::default()
        })];
        let report = aggregate(&records, None, None, "project").unwrap();
        assert_eq!(report.buckets.len(), 1);
        assert_eq!(report.buckets[0].key, "p1");
        assert_eq!(report.buckets[0].label.as_deref(), Some("Project One"));
    }

    #[test]
    fn aggregate_by_template_groups_a_missing_template_under_none() {
        let records = vec![
            mk_session(MkSession {
                id: "s1",
                started_at: "2026-09-10T08:00:00.000Z",
                turns: 1,
                ..Default::default()
            }),
            mk_session(MkSession {
                id: "s2",
                template_id: Some("dev"),
                started_at: "2026-09-10T08:00:00.000Z",
                turns: 1,
                ..Default::default()
            }),
        ];
        let report = aggregate(&records, None, None, "template").unwrap();
        let keys: Vec<&str> = report.buckets.iter().map(|b| b.key.as_str()).collect();
        assert!(keys.contains(&"none"));
        assert!(keys.contains(&"dev"));
    }

    #[test]
    fn aggregate_by_origin_groups_on_origin_kind() {
        let records = vec![
            mk_session(MkSession {
                id: "s1",
                started_at: "2026-09-10T08:00:00.000Z",
                turns: 1,
                ..Default::default()
            }),
            mk_session(MkSession {
                id: "s2",
                origin_kind: "automation",
                started_at: "2026-09-10T08:00:00.000Z",
                turns: 1,
                ..Default::default()
            }),
        ];
        let report = aggregate(&records, None, None, "origin").unwrap();
        let keys: Vec<&str> = report.buckets.iter().map(|b| b.key.as_str()).collect();
        assert!(keys.contains(&"cli"));
        assert!(keys.contains(&"automation"));
    }

    #[test]
    fn aggregate_rejects_an_unknown_group_by() {
        let err = aggregate(&[], None, None, "week").unwrap_err().to_string();
        assert_eq!(
            err,
            "groupBy must be one of day|project|template|origin|ticket|model|stage|route"
        );
    }

    #[test]
    fn aggregate_counts_a_record_without_usage_as_one_session_with_zero_tokens() {
        let records = vec![mk_session(MkSession {
            id: "s1",
            started_at: "2026-09-10T08:00:00.000Z",
            turns: 4,
            ..Default::default()
        })];
        let report = aggregate(&records, None, None, "day").unwrap();
        assert_eq!(report.totals.sessions, 1);
        assert_eq!(report.totals.turns, 4);
        assert_eq!(report.totals.total_tokens, 0);
        assert_eq!(report.totals.estimated_cost_usd, 0.0);
    }

    fn mk_rate_limit(
        status: &str,
        five_hour_resets_at: Option<i64>,
        observed_at: i64,
    ) -> RateLimitInfo {
        RateLimitInfo {
            status: status.to_string(),
            is_using_overage: false,
            five_hour: five_hour_resets_at.map(|resets_at| crate::sessions::RateLimitWindow {
                utilization: 0.5,
                resets_at,
            }),
            seven_day: None,
            observed_at,
        }
    }

    #[test]
    fn rate_limit_history_round_trips_newest_first() {
        let _ws = temp_dir("history");
        let dir = _ws.path();
        append_rate_limit(dir, &mk_rate_limit("ok", Some(1000), 1));
        append_rate_limit(dir, &mk_rate_limit("ok", Some(2000), 2));
        append_rate_limit(dir, &mk_rate_limit("ok", Some(3000), 3));

        let history = read_rate_limit_history(dir, 0);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].observed_at, 3);
        assert_eq!(history[1].observed_at, 2);
        assert_eq!(history[2].observed_at, 1);

        let limited = read_rate_limit_history(dir, 2);
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].observed_at, 3);
        assert_eq!(limited[1].observed_at, 2);
    }

    #[test]
    fn latest_live_snapshot_ignores_a_window_that_already_reset() {
        let _ws = temp_dir("live");
        let dir = _ws.path();
        // resets_at is in seconds; now_ms in milliseconds.
        append_rate_limit(dir, &mk_rate_limit("ok", Some(1_000), 1)); // resets at 1_000_000 ms
        let now_ms = 2_000_000; // after the reset
        assert!(latest_live_snapshot(dir, now_ms).is_none());

        let _ws2 = temp_dir("live2");
        let dir2 = _ws2.path();
        append_rate_limit(dir2, &mk_rate_limit("ok", Some(3_000), 1)); // resets at 3_000_000 ms
        let now_ms2 = 2_000_000; // before the reset
        let live = latest_live_snapshot(dir2, now_ms2);
        assert!(live.is_some());
        assert_eq!(live.unwrap().observed_at, 1);

        let _ws3 = temp_dir("live3");
        let dir3 = _ws3.path();
        assert!(latest_live_snapshot(dir3, 0).is_none());
    }
}

#[cfg(test)]
#[path = "../tests/unit/usage_tests.rs"]
mod usage_tests;
