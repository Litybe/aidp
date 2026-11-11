use super::*;
use crate::models::enums::Route;

#[test]
fn pixel_agents_read_pixel_domains() {
    for agent in [
        "pixel-art-director",
        "pixel-artist",
        "pixel-fixer",
        "pixel-reviewer",
    ] {
        assert_eq!(
            domains_for(agent),
            [Domain::PixelArt, Domain::PixelAnimation, Domain::Tooling],
            "{agent}"
        );
    }
    assert_eq!(
        domains_for("pixel-animator"),
        [Domain::PixelAnimation, Domain::PixelArt, Domain::Tooling]
    );
}

#[test]
fn unity_agents_read_code_domains() {
    for agent in [
        "unity-issue-implementer",
        "unity-review-fixer",
        "unity-pr-reviewer",
    ] {
        assert_eq!(
            domains_for(agent),
            [Domain::Unity, Domain::Csharp, Domain::Mechanics]
        );
    }
    assert_eq!(
        domains_for("unity-playtester"),
        [Domain::Mechanics, Domain::Unity, Domain::LevelDesign]
    );
}

#[test]
fn map_agents_read_level_domains() {
    assert_eq!(
        domains_for("map-designer"),
        [
            Domain::LevelDesign,
            Domain::Rooms,
            Domain::Mechanics,
            Domain::Tooling
        ]
    );
    assert_eq!(role_of("map-designer").unwrap().route, Some(Route::Map));
}

#[test]
fn generic_and_unknown_agents_read_everything() {
    for agent in [
        "analyzer",
        "planner",
        "reviewer",
        "executor",
        "who-knows",
        "",
    ] {
        assert!(domains_for(agent).is_empty(), "{agent}");
    }
}

#[test]
fn plugin_prefix_and_case_are_ignored() {
    assert_eq!(agent_name("my-plugin:Pixel-Artist"), "pixel-artist");
    assert_eq!(
        domains_for("caveman:pixel-artist"),
        domains_for("pixel-artist")
    );
    assert_eq!(role_of("a:b:unity-playtester"), role_of("unity-playtester"));
}

#[test]
fn explicit_domains_win_over_the_role() {
    assert_eq!(
        effective_domains(&[Domain::Lore], Some("pixel-artist")),
        [Domain::Lore]
    );
    assert_eq!(
        effective_domains(&[], Some("pixel-artist")),
        domains_for("pixel-artist")
    );
    assert!(effective_domains(&[], None).is_empty());
}

#[test]
fn domain_spellings_are_kebab_case() {
    assert_eq!(
        Domain::parse("Pixel-Animation").unwrap(),
        Domain::PixelAnimation
    );
    assert_eq!(
        serde_json::to_string(&Domain::LevelDesign).unwrap(),
        "\"level-design\""
    );
    assert_eq!(
        serde_json::from_str::<Domain>("\"game-design\"").unwrap(),
        Domain::GameDesign
    );
    assert_eq!(Domain::ALL.len(), 10);
    assert!(Domain::parse("cooking").is_err());
}
