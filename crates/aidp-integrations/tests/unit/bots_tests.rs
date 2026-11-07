use super::*;
use crate::models::enums::bots_filter::BotsFilter;
use crate::models::squad_config::SquadConfig;

use crate::squad::test_support::*;

#[test]
fn bot_detection_table() {
    let cfg = SquadConfig::default();
    let cases = [
        ("dependabot[bot]", true),
        ("Renovate[bot]", true),
        ("some-new-app[bot]", true),
        ("bob", false),
        ("robot-lover", false),
    ];
    for (author, expected) in cases {
        assert_eq!(is_bot(author, &cfg), expected, "{author}");
    }
}

#[test]
fn configured_bot_author_without_suffix_is_a_bot() {
    let cfg = SquadConfig {
        bot_authors: vec!["ci-helper".to_string()],
        ..SquadConfig::default()
    };
    assert!(is_bot("CI-Helper", &cfg));
}

#[test]
fn bots_filter_table() {
    let cfg = SquadConfig::default();
    let bot = pr_by("dependabot[bot]");
    let human = pr_by("bob");
    let cases = [
        (BotsFilter::Hide, false, true),
        (BotsFilter::Show, true, true),
        (BotsFilter::Only, true, false),
    ];
    for (filter, keeps_bot, keeps_human) in cases {
        assert_eq!(accepts(filter, &bot, &cfg), keeps_bot, "{filter:?} bot");
        assert_eq!(
            accepts(filter, &human, &cfg),
            keeps_human,
            "{filter:?} human"
        );
    }
}

#[test]
fn bots_are_hidden_by_default() {
    assert_eq!(BotsFilter::default(), BotsFilter::Hide);
}
