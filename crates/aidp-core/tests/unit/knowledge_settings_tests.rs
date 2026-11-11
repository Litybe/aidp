use super::*;
use crate::pipeline::parse;

const SHORTHAND: &str = "version: 5\nid: p\nsettings:\n  knowledge: true\nstages:\n  - name: a\n";
const FULL: &str = "version: 5\nid: p\nsettings:\n  knowledge:\n    enabled: true\n    budget: 900\n    route: art\n    stages: [a]\nstages:\n  - name: a\n  - name: b\n";
const LEGACY: &str = "version: 5\nid: p\nsettings:\n  knowledge:\n    prompt_budget_chars: 2000\nstages:\n  - name: a\n";
const BAD_ROUTE: &str =
    "version: 5\nid: p\nsettings:\n  knowledge:\n    route: music\nstages:\n  - name: a\n";
const BAD_BUDGET: &str =
    "version: 5\nid: p\nsettings:\n  knowledge:\n    budget: 3\nstages:\n  - name: a\n";
const ABSENT: &str = "version: 5\nid: p\nstages:\n  - name: a\n";

#[test]
fn shorthand_enables_with_defaults() {
    let settings = KnowledgeSettings::from_settings(&parse(SHORTHAND).unwrap().settings);
    assert!(settings.enabled);
    assert_eq!(settings.budget, DEFAULT_KNOWLEDGE_BUDGET_TOKENS);
    assert!(settings.covers("anything"));
}

#[test]
fn full_mapping_is_read() {
    let settings = KnowledgeSettings::from_settings(&parse(FULL).unwrap().settings);
    assert_eq!(settings.budget, 900);
    assert_eq!(settings.route, Some(KnowledgeRoute::Art));
    assert!(settings.covers("a"));
    assert!(!settings.covers("b"));
}

#[test]
fn legacy_mapping_and_absent_block_stay_disabled() {
    for text in [LEGACY, ABSENT] {
        let settings = KnowledgeSettings::from_settings(&parse(text).unwrap().settings);
        assert!(!settings.enabled);
        assert!(!settings.covers("a"));
    }
}

#[test]
fn schema_refuses_an_unknown_route_and_a_tiny_budget() {
    assert!(parse(BAD_ROUTE).is_err());
    assert!(parse(BAD_BUDGET).is_err());
}
