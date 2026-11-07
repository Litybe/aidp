use super::*;
use aidp_testkit::TempWorkspace;
use aidp_tickets::budget::Window;
use aidp_tickets::config::{BudgetConfig, DispatcherConfig, NightConfig, PixellabBudget};
use serde_json::json;

const HEAVY_TEMPLATE: &str =
    "version: 5\nid: heavy\nstages:\n  - name: plan\n    agent: planner\n  - name: report\n";
const LIGHT_TEMPLATE: &str = "version: 5\nid: light\nstages:\n  - name: implement\n";
const NOON: u32 = 12 * 60;
const MIDNIGHT: u32 = 0;
const HOUR_MS: i64 = 3_600_000;

fn ticket(key: &str, template: &str, route: &str, manual: bool) -> Ticket {
    serde_json::from_value(json!({
        "key": key, "source": "local", "type": "Task", "summary": key, "status": "todo",
        "priority": "Medium", "service": "svc", "template": template,
        "labels": [format!("route:{route}")],
        "created": "2026-11-06T10:00:00Z", "updated": "2026-11-06T10:00:00Z",
        "autopilot": { "manual_priority": manual },
    }))
    .unwrap()
}

fn skill_with_templates() -> TempWorkspace {
    let skill = TempWorkspace::new("gates-skill");
    let templates = skill.path().join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(templates.join("heavy.yaml"), HEAVY_TEMPLATE).unwrap();
    std::fs::write(templates.join("light.yaml"), LIGHT_TEMPLATE).unwrap();
    skill
}

fn write_balance(vault: &Path, credits: f64) {
    let dir = vault
        .join("Pipelines")
        .join("game")
        .join("GH-1")
        .join("run-a")
        .join("generate");
    std::fs::create_dir_all(&dir).unwrap();
    let body = json!({
        "balanceAfter": { "credits": credits }, "generations": 1, "recordedAt": "2026-11-06T10:00:00Z"
    });
    std::fs::write(dir.join("pixellab-usage.json"), body.to_string()).unwrap();
    crate::pixellab_usage::forget_balance(vault);
}

struct Setup {
    dispatcher: DispatcherConfig,
    budget: BudgetConfig,
}

fn setup(night: bool, reserve: f64) -> Setup {
    let dispatcher = DispatcherConfig {
        night: NightConfig {
            enabled: night,
            ..NightConfig::default()
        },
        ..DispatcherConfig::default()
    };
    let budget = BudgetConfig {
        pixellab: PixellabBudget {
            min_reserve: reserve,
            per_ticket: 10,
        },
        ..BudgetConfig::default()
    };
    Setup { dispatcher, budget }
}

fn held(
    setup: &Setup,
    vault: &Path,
    skill: &Path,
    candidates: &[Ticket],
    local_minutes: u32,
    snapshot: Option<&RateSnapshot>,
) -> Vec<(String, String)> {
    let inputs = GateInputs {
        vault,
        skill_root: Some(skill),
        dispatcher: &setup.dispatcher,
        budget: &setup.budget,
        snapshot,
        now_ms: 100 * HOUR_MS,
        local_minutes,
    };
    held_candidates(candidates, &inputs)
}

#[test]
fn nothing_is_held_when_no_gate_is_on() {
    let vault = TempWorkspace::new("gates-off-vault");
    let skill = skill_with_templates();
    let candidates = [ticket("GH-1", "heavy", "code", false)];
    assert!(held(
        &setup(false, 0.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None
    )
    .is_empty());
}

#[test]
fn an_art_ticket_is_held_below_the_pixellab_reserve() {
    let vault = TempWorkspace::new("gates-pixellab-vault");
    let skill = skill_with_templates();
    write_balance(vault.path(), 3.0);
    let candidates = [
        ticket("GH-1", "light", "art", false),
        ticket("GH-2", "light", "code", false),
        ticket("GH-3", "light", "art", true),
    ];
    let result = held(
        &setup(false, 5.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None,
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "GH-1");
    assert!(result[0].1.starts_with("pixellab:"));
}

#[test]
fn an_art_ticket_starts_with_enough_credits_or_an_unknown_balance() {
    let vault = TempWorkspace::new("gates-pixellab-ok-vault");
    let skill = skill_with_templates();
    let candidates = [ticket("GH-1", "light", "art", false)];
    assert!(held(
        &setup(false, 5.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None
    )
    .is_empty());
    write_balance(vault.path(), 9.0);
    assert!(held(
        &setup(false, 5.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None
    )
    .is_empty());
}

#[test]
fn a_heavy_ticket_waits_for_the_night_and_a_light_one_does_not() {
    let vault = TempWorkspace::new("gates-night-vault");
    let skill = skill_with_templates();
    let candidates = [
        ticket("GH-1", "heavy", "code", false),
        ticket("GH-2", "light", "code", false),
    ];
    let result = held(
        &setup(true, 0.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None,
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "GH-1");
    assert!(result[0].1.starts_with("night:"));
}

#[test]
fn at_night_a_heavy_ticket_starts_just_after_a_reset() {
    let vault = TempWorkspace::new("gates-night-reset-vault");
    let skill = skill_with_templates();
    let candidates = [ticket("GH-1", "heavy", "code", false)];
    let now_ms = 100 * HOUR_MS;
    let fresh = RateSnapshot {
        status: "allowed".into(),
        is_using_overage: false,
        five_hour: Some(Window {
            utilization: 0.0,
            resets_at_s: (now_ms - 5 * 60_000 + 5 * HOUR_MS) / 1000,
        }),
        seven_day: None,
        observed_at_ms: now_ms,
    };
    let stale = RateSnapshot {
        five_hour: Some(Window {
            utilization: 0.5,
            resets_at_s: (now_ms + 2 * HOUR_MS) / 1000,
        }),
        ..fresh.clone()
    };
    let setup = setup(true, 0.0);
    assert!(held(
        &setup,
        vault.path(),
        skill.path(),
        &candidates,
        MIDNIGHT,
        Some(&fresh)
    )
    .is_empty());
    assert_eq!(
        held(
            &setup,
            vault.path(),
            skill.path(),
            &candidates,
            MIDNIGHT,
            Some(&stale)
        )
        .len(),
        1
    );
}

#[test]
fn the_model_policy_can_make_a_heavy_template_light() {
    let vault = TempWorkspace::new("gates-policy-vault");
    let skill = skill_with_templates();
    std::fs::create_dir_all(vault.path().join("TODO")).unwrap();
    std::fs::write(
        vault.path().join("TODO").join("config.yaml"),
        "models:\n  code:\n    plan: sonnet\n",
    )
    .unwrap();
    let candidates = [ticket("GH-1", "heavy", "code", false)];
    assert!(held(
        &setup(true, 0.0),
        vault.path(),
        skill.path(),
        &candidates,
        NOON,
        None
    )
    .is_empty());
}

#[test]
fn the_pixellab_hold_needs_a_known_balance_and_a_reserve() {
    assert!(pixellab_hold(5.0, Some(4.0)).is_some());
    assert!(pixellab_hold(5.0, Some(5.0)).is_none());
    assert!(pixellab_hold(5.0, None).is_none());
    assert!(pixellab_hold(0.0, Some(0.0)).is_none());
}
