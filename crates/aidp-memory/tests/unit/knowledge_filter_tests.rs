use super::*;

fn meta(domain: Option<&str>, agents: &[&str]) -> ChunkMeta {
    ChunkMeta {
        domain: domain.map(str::to_string),
        agents: agents.iter().map(|agent| agent.to_string()).collect(),
        ..ChunkMeta::default()
    }
}

#[test]
fn no_filter_lets_everything_pass() {
    assert!(passes(&meta(None, &["x"]), &[], None));
}

#[test]
fn the_domain_must_be_in_the_list() {
    let note = meta(Some("unity"), &[]);
    assert!(passes(&note, &[Domain::Unity, Domain::Csharp], None));
    assert!(!passes(&note, &[Domain::Lore], None));
    assert!(!passes(&meta(None, &[]), &[Domain::Lore], None));
}

#[test]
fn an_empty_agent_list_is_for_everyone() {
    assert!(passes(&meta(Some("unity"), &[]), &[], Some("pixel-artist")));
}

#[test]
fn a_named_agent_list_keeps_only_those_agents() {
    let note = meta(Some("unity"), &["unity-pr-reviewer", "executor"]);
    assert!(passes(&note, &[], Some("unity-pr-reviewer")));
    assert!(passes(&note, &[], Some("plugin:Executor")));
    assert!(!passes(&note, &[], Some("pixel-artist")));
}

#[test]
fn importance_scales_the_score_from_one_to_one_and_a_half() {
    assert_eq!(importance_factor(None), 1.0);
    assert_eq!(importance_factor(Some(0.0)), 1.0);
    assert_eq!(importance_factor(Some(1.0)), 1.5);
    assert!((importance_factor(Some(0.4)) - 1.2).abs() < 1e-9);
    assert_eq!(importance_factor(Some(7.0)), 1.5);
}
