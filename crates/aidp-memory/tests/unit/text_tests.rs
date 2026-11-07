use super::*;

#[test]
fn splits_camel_snake_and_acronyms() {
    assert_eq!(split_identifier("PlayerMovement"), ["player", "movement"]);
    assert_eq!(split_identifier("max_jump_speed"), ["max", "jump", "speed"]);
    assert_eq!(split_identifier("HTTPServer2"), ["http", "server", "2"]);
}

#[test]
fn identifier_tokens_add_parts_and_joined_form() {
    let tokens = identifier_tokens("void ApplyDamage(int hit_points)");
    assert!(tokens.contains("apply"));
    assert!(tokens.contains("damage"));
    assert!(tokens.contains("applydamage"));
    assert!(tokens.contains("hit"));
    assert!(!tokens.contains("void"));
}

#[test]
fn match_expression_folds_accents_drops_stopwords_and_plurals() {
    let expression = match_expression("Où sont les ennemis de la forêt ?").unwrap();
    assert!(expression.contains("\"ennemi\"*"));
    assert!(expression.contains("\"foret\"*"));
    assert!(!expression.contains("\"les\""));
}

#[test]
fn match_expression_splits_camel_case() {
    let expression = match_expression("PlayerMovement").unwrap();
    assert!(expression.contains("\"player\"*"));
    assert!(expression.contains("\"movement\"*"));
}

#[test]
fn match_expression_is_none_for_stopwords_only() {
    assert!(match_expression("le la de").is_none());
}

#[test]
fn token_estimate_rounds_up() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abcde"), 2);
}

#[test]
fn jaccard_measures_overlap() {
    let left = word_set("saut double heroine");
    let right = word_set("saut double ennemi");
    assert!((jaccard(&left, &right) - 0.5).abs() < 1e-9);
    assert_eq!(jaccard(&left, &word_set("")), 0.0);
}

#[test]
fn truncate_respects_char_boundaries() {
    assert_eq!(truncate_chars("éàü", 2), "éà");
    assert_eq!(truncate_chars("ab", 5), "ab");
}
