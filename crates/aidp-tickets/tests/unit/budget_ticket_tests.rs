use super::*;
use aidp_testkit::TempWorkspace;

const KEY: &str = "GH-7";

#[test]
fn check_compares_with_the_ceiling() {
    assert_eq!(check(10, 0), TicketBudgetCheck::Unlimited);
    assert_eq!(
        check(9, 10),
        TicketBudgetCheck::Within { used: 9, limit: 10 }
    );
    assert!(check(10, 10).is_exceeded());
    assert!(check(11, 10).is_exceeded());
}

#[test]
fn the_ceiling_follows_the_route_and_falls_back_to_default() {
    let cfg = PerTicketBudget::default();
    let ceilings = TicketCeilings::default();
    assert_eq!(
        ceilings.token_limit(&cfg, Some(BudgetRoute::Code), KEY),
        3_000_000
    );
    assert_eq!(
        ceilings.token_limit(&cfg, Some(BudgetRoute::Doc), KEY),
        500_000
    );
    assert_eq!(ceilings.token_limit(&cfg, None, KEY), 2_000_000);
}

#[test]
fn a_zero_ceiling_stays_unlimited_even_after_a_raise() {
    let cfg = PerTicketBudget {
        doc: crate::models::token_amount::TokenAmount(0),
        ..PerTicketBudget::default()
    };
    let mut ceilings = TicketCeilings::default();
    ceilings.raise(KEY, 100, 0, 0, 0);
    assert_eq!(ceilings.token_limit(&cfg, Some(BudgetRoute::Doc), KEY), 0);
}

#[test]
fn a_raise_grants_one_more_allowance_to_that_ticket_only() {
    let cfg = PerTicketBudget::default();
    let mut ceilings = TicketCeilings::default();
    ceilings.raise(KEY, 3_100_000, 3_000_000, 10, 10);
    assert_eq!(
        ceilings.token_limit(&cfg, Some(BudgetRoute::Code), KEY),
        6_100_000
    );
    assert_eq!(
        ceilings.token_limit(&cfg, Some(BudgetRoute::Code), "GH-8"),
        3_000_000
    );
    assert_eq!(ceilings.generation_limit(10, KEY), 20);
    assert_eq!(ceilings.generation_limit(10, "GH-8"), 10);
    assert_eq!(ceilings.generation_limit(0, KEY), 0);
}

#[test]
fn raised_ceilings_survive_a_reload() {
    let vault = TempWorkspace::new("ticket-ceilings");
    let mut ceilings = TicketCeilings::default();
    ceilings.raise(KEY, 5, 10, 1, 10);
    ceilings.save(vault.path()).unwrap();
    assert_eq!(TicketCeilings::load(vault.path()), ceilings);
    assert_eq!(
        TicketCeilings::load(TempWorkspace::new("ticket-ceilings-empty").path()),
        TicketCeilings::default()
    );
}

#[test]
fn the_prompt_names_the_ticket_and_both_numbers() {
    let prompt = exceeded_prompt(KEY, "tokens", 3_100_000, 3_000_000);
    assert!(prompt.contains(KEY));
    assert!(prompt.contains("3100000 / 3000000"));
}
