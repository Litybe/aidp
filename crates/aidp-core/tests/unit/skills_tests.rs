use super::*;
use crate::models::constants::skill_rules::RUST_DEV_SKILL;

const UNRESOLVED_PIPE: &str = include_str!("../fixtures/skills/unresolved.yaml");
const GRAPHIFY_PIPE: &str = include_str!("../fixtures/skills/graphify.yaml");
const PRESENT_SKILL: &str = "present";
const BY_MAP_SKILL: &str = "by-map";
const A_SKILL: &str = "a";
const PLAIN_STAGE: &str = "plain";
const ROUTED_STAGE: &str = "routed";
const INNER_STAGE: &str = "inner";

/// A fresh, empty directory unique to this test process.
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aidp-skills-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every declared skill missing from the search root is named with its
/// stage: plain lists, both `skills_by` buckets and loop bodies alike.
#[test]
fn unresolved_names_every_missing_skill_with_its_stage() {
    let root = tmp_dir("unresolved");
    std::fs::create_dir_all(root.join(PRESENT_SKILL)).unwrap();
    std::fs::write(root.join(PRESENT_SKILL).join("SKILL.md"), "").unwrap();
    let pipe: Pipeline = serde_yaml::from_str(UNRESOLVED_PIPE).unwrap();

    let missing = unresolved(&pipe, &[root.as_path()]);
    let _ = std::fs::remove_dir_all(&root);

    let pairs: Vec<(&str, &str)> = missing
        .iter()
        .map(|u| (u.stage.as_str(), u.skill.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (PLAIN_STAGE, "ghost"),
            (ROUTED_STAGE, BY_MAP_SKILL),
            (ROUTED_STAGE, "by-default"),
            (INNER_STAGE, "deep-ghost"),
        ]
    );
}

fn graphify_pipe() -> Pipeline {
    serde_yaml::from_str(GRAPHIFY_PIPE).unwrap()
}

/// Without a graph the skill leaves every list at every depth, one warning per stage.
#[test]
fn without_graph_removes_the_skill_everywhere_and_warns_per_stage() {
    let pipe = graphify_pipe();
    let (filtered, warnings) = without_unavailable(&pipe, false);

    assert!(matches!(filtered, Cow::Owned(_)));
    assert_eq!(filtered.stages[0].skills, vec![RUST_DEV_SKILL.to_string()]);
    let by = filtered.stages[1].skills_by.as_ref().unwrap();
    assert_eq!(by.map[A_SKILL], vec![BY_MAP_SKILL.to_string()]);
    assert!(by.default.is_empty());
    assert!(filtered.stages[2].body[0].skills.is_empty());
    assert_eq!(warnings.len(), 3);
    for (warning, stage) in warnings
        .iter()
        .zip([PLAIN_STAGE, ROUTED_STAGE, INNER_STAGE])
    {
        assert!(
            warning.starts_with(SKILL_DROPPED_WARNING_PREFIX),
            "{warning}"
        );
        assert!(warning.ends_with(stage), "{warning}");
    }
}

/// With a graph the pipeline is borrowed untouched and nothing is reported.
#[test]
fn with_graph_borrows_the_pipeline_unchanged() {
    let pipe = graphify_pipe();
    let (kept, warnings) = without_unavailable(&pipe, true);

    assert!(matches!(kept, Cow::Borrowed(_)));
    assert_eq!(kept.stages[0].skills[0], GRAPHIFY_CONTEXT_SKILL);
    assert!(warnings.is_empty());
}
