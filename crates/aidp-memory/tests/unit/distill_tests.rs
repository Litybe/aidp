use super::*;

const SESSION: &str = "abcdef12";
const REPLY: &str = "Voici :\n```json\n{\"proposals\":[{\"kind\":\"decision\",\"title\":\"T\",\"text\":\"X\"}]}\n```";

#[test]
fn a_turn_needs_several_new_prompts() {
    assert!(!should_distill(DistillTrigger::Turn, 4, 0));
    assert!(should_distill(DistillTrigger::Turn, 5, 0));
    assert!(!should_distill(DistillTrigger::Turn, 9, 5));
    assert!(should_distill(DistillTrigger::Turn, 10, 5));
}

#[test]
fn a_boundary_needs_one_new_prompt_and_a_real_session() {
    assert!(should_distill(DistillTrigger::Boundary, 2, 0));
    assert!(!should_distill(DistillTrigger::Boundary, 1, 0));
    assert!(!should_distill(DistillTrigger::Boundary, 6, 6));
    assert!(should_distill(DistillTrigger::Boundary, 7, 6));
}

#[test]
fn the_reply_is_extracted_and_tied_to_the_session() {
    let document: Value =
        serde_json::from_str(&proposals_from_reply(REPLY, SESSION).unwrap()).unwrap();
    assert_eq!(document["proposals"][0]["refs"][0]["type"], "session");
    assert_eq!(document["proposals"][0]["refs"][0]["id"], SESSION);
}

#[test]
fn the_reply_is_capped() {
    let items = vec![json!({"kind":"fact","title":"T","text":"X"}); 8];
    let reply = json!({ "proposals": items }).to_string();
    let document: Value =
        serde_json::from_str(&proposals_from_reply(&reply, SESSION).unwrap()).unwrap();
    assert_eq!(
        document["proposals"].as_array().unwrap().len(),
        MAX_PROPOSALS_PER_FILE
    );
}

#[test]
fn a_reply_without_json_is_invalid() {
    for reply in ["aucune", "} puis {", "{\"autre\":1}"] {
        assert!(matches!(
            proposals_from_reply(reply, SESSION),
            Err(MemoryError::Invalid(_))
        ));
    }
}
