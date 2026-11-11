use super::*;
use crate::models::constants::limits::MAX_QUERY_TERMS;
use crate::text::match_expression_with;

const GLOSSARY: &str = include_str!("../fixtures/knowledge/Knowledge/_glossary.md");

fn glossary() -> Glossary {
    Glossary::parse(GLOSSARY).unwrap()
}

fn words(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

#[test]
fn the_yaml_block_is_parsed_into_groups() {
    let parsed = glossary();
    assert_eq!(parsed.groups.len(), 3);
    assert_eq!(parsed.groups[0][0], "coyote time");
    assert_eq!(parsed.groups[2], ["knockback", "recul"]);
}

#[test]
fn a_glossary_without_block_is_an_error() {
    assert!(Glossary::parse("---\ntype: glossary\n---\n\n# Titre\n").is_err());
}

#[test]
fn a_missing_file_gives_no_glossary() {
    assert!(Glossary::load(Path::new("nowhere/_glossary.md")).is_none());
}

#[test]
fn a_single_word_pulls_the_rest_of_its_group() {
    let added = glossary().expansions("le knockback est trop fort");
    assert_eq!(added, [words(&["recul"])]);
}

#[test]
fn a_phrase_member_pulls_the_other_members() {
    let added = glossary().expansions("ajouter un coyote time au saut");
    assert_eq!(
        added,
        [words(&["temps", "grace"]), words(&["grace", "time"])]
    );
}

#[test]
fn half_a_phrase_does_not_match() {
    assert!(glossary().expansions("coyote").is_empty());
    assert!(glossary().expansions("une autre question").is_empty());
}

#[test]
fn accents_case_and_plurals_do_not_block_a_match() {
    let added = glossary().expansions("TEMPS DE GRÂCE");
    assert!(added.contains(&words(&["coyote", "time"])));
    assert!(!glossary().expansions("des Knockbacks").is_empty());
}

#[test]
fn the_expression_gains_one_clause_per_synonym() {
    let glossary = glossary();
    let plain = match_expression_with("knockback", None).unwrap();
    assert_eq!(plain, "\"knockback\"*");
    let widened = match_expression_with("knockback", Some(&glossary)).unwrap();
    assert_eq!(widened, "\"knockback\"* OR \"recul\"*");
    let phrase = match_expression_with("coyote time", Some(&glossary)).unwrap();
    assert!(phrase.contains("(\"temp\"* AND \"grace\"*)"), "{phrase}");
}

#[test]
fn originals_come_first_and_the_cap_holds() {
    let glossary = glossary();
    let long: String = (0..30).map(|index| format!("mot{index} ")).collect();
    let expression = match_expression_with(&format!("knockback {long}"), Some(&glossary)).unwrap();
    assert_eq!(expression.split(" OR ").count(), MAX_QUERY_TERMS);
    assert!(!expression.contains("recul"));
    let room = match_expression_with("knockback un deux", Some(&glossary)).unwrap();
    assert!(room.ends_with("\"recul\"*"), "{room}");
}

#[test]
fn a_synonym_already_in_the_query_is_not_repeated() {
    let expression = match_expression_with("knockback recul", Some(&glossary())).unwrap();
    assert_eq!(expression.matches("\"recul\"*").count(), 1);
}
