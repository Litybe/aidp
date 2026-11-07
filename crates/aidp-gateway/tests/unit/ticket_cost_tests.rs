use super::*;
use crate::sessions::RateLimitWindow;
use serde_json::json;

const TICKET: &str = "GH-12";
const OTHER: &str = "GH-13";

fn session(
    id: &str,
    ticket: Option<&str>,
    step: &str,
    started_at: &str,
    usage: (u64, u64, u64, u64),
) -> SessionRecord {
    let mut origin = json!({ "kind": "flow", "label": step });
    if let Some(ticket) = ticket {
        origin["ref"] = json!(ticket);
    }
    serde_json::from_value(json!({
        "id": id,
        "projectId": "p",
        "projectName": "P",
        "cwd": "/tmp",
        "engine": "v7",
        "templateId": "dev-unity",
        "prompt": "x",
        "permissionMode": "open",
        "status": "done",
        "startedAt": started_at,
        "model": "claude-sonnet-5",
        "origin": origin,
        "usage": {
            "input": usage.0, "output": usage.1, "cacheRead": usage.2, "cacheCreate": usage.3
        },
    }))
    .unwrap()
}

fn rate_limit(five_hour: f64, resets_at: i64) -> RateLimitInfo {
    RateLimitInfo {
        status: "allowed".to_string(),
        is_using_overage: false,
        five_hour: Some(RateLimitWindow {
            utilization: five_hour,
            resets_at,
        }),
        seven_day: None,
        observed_at: 0,
    }
}

#[test]
fn ticket_cost_sums_only_the_sessions_of_the_ticket() {
    let all = vec![
        session(
            "a",
            Some(TICKET),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (10, 5, 100, 20),
        ),
        session(
            "b",
            Some(TICKET),
            "review",
            "2026-11-06T11:00:00.000Z",
            (1, 1, 1, 1),
        ),
        session(
            "c",
            Some(OTHER),
            "implement",
            "2026-11-06T11:00:00.000Z",
            (999, 0, 0, 0),
        ),
        session(
            "d",
            None,
            "implement",
            "2026-11-06T11:00:00.000Z",
            (777, 0, 0, 0),
        ),
    ];
    let cost = ticket_cost(TICKET, &all, &[], Some("code".into()), None);
    assert_eq!(cost.totals.total_tokens, 139);
    assert_eq!(cost.totals.sessions, 2);
    assert_eq!(cost.totals.tokens.cache(), 122);
    assert_eq!(cost.route.as_deref(), Some("code"));
    let steps: Vec<&str> = cost.by_step.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(steps, vec!["implement", "review"]);
    assert_eq!(cost.by_flow.len(), 1);
    assert_eq!(cost.by_flow[0].key, "dev-unity");
    assert_eq!(cost.by_model[0].key, "claude-sonnet-5");
}

#[test]
fn stage_rows_feed_the_stage_and_model_breakdowns() {
    let all = vec![session(
        "a",
        Some(TICKET),
        "implement",
        "2026-11-06T10:00:00.000Z",
        (10, 5, 0, 0),
    )];
    let stages = vec![
        StageRow {
            run_id: "r".into(),
            stage: "plan".into(),
            model: Some("opus".into()),
            tokens: TokenTotals {
                input: 4,
                output: 1,
                cache_read: 0,
                cache_create: 0,
            },
        },
        StageRow {
            run_id: "r".into(),
            stage: "report".into(),
            model: Some("haiku".into()),
            tokens: TokenTotals {
                input: 2,
                output: 2,
                cache_read: 0,
                cache_create: 0,
            },
        },
    ];
    let cost = ticket_cost(TICKET, &all, &stages, None, None);
    let stage_keys: Vec<&str> = cost.by_stage.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(stage_keys, vec!["plan", "report"]);
    let model_keys: Vec<&str> = cost.by_model.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(model_keys, vec!["opus", "haiku"]);
}

#[test]
fn window_share_is_the_utilization_times_the_ticket_share_of_the_window() {
    let all = vec![
        session(
            "a",
            Some(TICKET),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (100, 0, 0, 0),
        ),
        session(
            "b",
            Some(OTHER),
            "implement",
            "2026-11-06T10:30:00.000Z",
            (300, 0, 0, 0),
        ),
        session(
            "old",
            Some(TICKET),
            "implement",
            "2026-11-01T10:00:00.000Z",
            (5000, 0, 0, 0),
        ),
    ];
    let resets_at = chrono::DateTime::parse_from_rfc3339("2026-11-06T13:00:00Z")
        .unwrap()
        .timestamp();
    let share = window_share(&all, TICKET, Some(&rate_limit(0.4, resets_at)));
    assert!((share.five_hour.unwrap() - 0.1).abs() < 1e-9);
    assert_eq!(share.seven_day, None);
    assert_eq!(window_share(&all, TICKET, None), WindowShare::default());
}

#[test]
fn footer_follows_the_contract_in_millions() {
    let tokens = TokenTotals {
        input: 300_000,
        output: 100_000,
        cache_read: 700_000,
        cache_create: 100_000,
    };
    assert_eq!(
        footer(&tokens, Some(0.04)),
        "Coût : 1,2 M tokens (in 0,3 M / out 0,1 M / cache 0,8 M), 4 % de la fenêtre 5 h"
    );
}

#[test]
fn footer_uses_thousands_below_a_million_and_drops_an_unknown_window() {
    let tokens = TokenTotals {
        input: 12_000,
        output: 3_000,
        cache_read: 25_000,
        cache_create: 0,
    };
    assert_eq!(
        footer(&tokens, None),
        "Coût : 40 k tokens (in 12 k / out 3 k / cache 25 k)"
    );
}

#[test]
fn a_tiny_window_share_reads_less_than_one_percent() {
    let tokens = TokenTotals {
        input: 10,
        ..TokenTotals::default()
    };
    assert!(footer(&tokens, Some(0.002)).ends_with(", <1 % de la fenêtre 5 h"));
}

#[test]
fn run_footer_sums_the_stage_rows() {
    let rows = vec![
        StageRow {
            stage: "a".into(),
            tokens: TokenTotals {
                input: 500_000,
                ..Default::default()
            },
            ..Default::default()
        },
        StageRow {
            stage: "b".into(),
            tokens: TokenTotals {
                output: 700_000,
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    assert!(run_footer(&rows, None)
        .starts_with("Coût : 1,2 M tokens (in 0,5 M / out 0,7 M / cache 0,0 M)"));
}

#[test]
fn contextual_usage_groups_by_ticket_and_route() {
    let all = vec![
        session(
            "a",
            Some(TICKET),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (10, 0, 0, 0),
        ),
        session(
            "b",
            Some(TICKET),
            "review",
            "2026-11-06T11:00:00.000Z",
            (5, 0, 0, 0),
        ),
        session(
            "c",
            Some(OTHER),
            "implement",
            "2026-11-06T11:00:00.000Z",
            (1, 0, 0, 0),
        ),
    ];
    let mut context = CostContext::default();
    context.routes.insert(TICKET.into(), "code".into());
    let by_ticket = aggregate_contextual(&all, None, None, "ticket", &context);
    assert_eq!(by_ticket.buckets[0].key, TICKET);
    assert_eq!(by_ticket.buckets[0].total_tokens, 15);
    assert_eq!(by_ticket.totals.total_tokens, 16);
    let by_route = aggregate_contextual(&all, None, None, "route", &context);
    let keys: Vec<&str> = by_route.buckets.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(keys, vec!["code", "unknown"]);
}

#[test]
fn contextual_usage_by_stage_reads_the_stage_rows_of_tickets_in_range() {
    let all = vec![
        session(
            "a",
            Some(TICKET),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (10, 0, 0, 0),
        ),
        session(
            "b",
            Some(OTHER),
            "implement",
            "2026-10-01T10:00:00.000Z",
            (10, 0, 0, 0),
        ),
    ];
    let row = |stage: &str, input: u64| StageRow {
        stage: stage.into(),
        tokens: TokenTotals {
            input,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut context = CostContext::default();
    context
        .stages
        .insert(TICKET.into(), vec![row("plan", 7), row("review", 3)]);
    context.stages.insert(OTHER.into(), vec![row("plan", 100)]);
    let report = aggregate_contextual(
        &all,
        Some("2026-11-01T00:00:00.000Z"),
        None,
        "stage",
        &context,
    );
    let plan = report.buckets.iter().find(|b| b.key == "plan").unwrap();
    assert_eq!(plan.total_tokens, 7);
    assert_eq!(report.buckets.len(), 2);
}

#[test]
fn anomalies_flag_the_ticket_far_above_the_others() {
    let mut all = Vec::new();
    for (index, tokens) in [100u64, 110, 105, 95, 9_000].iter().enumerate() {
        let ticket = format!("GH-{index}");
        all.push(session(
            &format!("s{index}"),
            Some(&ticket),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (*tokens, 0, 0, 0),
        ));
    }
    let report = anomalies(&all, None, None);
    assert_eq!(report.status, "ok");
    assert!(report.any_flagged);
    let flagged: Vec<&str> = report
        .points
        .iter()
        .filter(|p| p.flagged)
        .map(|p| p.key.as_str())
        .collect();
    assert_eq!(flagged, vec!["GH-4"]);
}

#[test]
fn anomalies_need_three_tickets() {
    let all = vec![
        session(
            "a",
            Some(TICKET),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (10, 0, 0, 0),
        ),
        session(
            "b",
            Some(OTHER),
            "implement",
            "2026-11-06T10:00:00.000Z",
            (10, 0, 0, 0),
        ),
    ];
    let report = anomalies(&all, None, None);
    assert_eq!(report.status, "insufficient_data");
    assert_eq!(report.n, 2);
}
