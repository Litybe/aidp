use super::*;

#[test]
fn identical_strings_have_zero_distance() {
    assert_eq!(levenshtein("dev", "dev"), 0);
}

#[test]
fn empty_side_costs_the_other_length() {
    assert_eq!(levenshtein("", "audit"), 5);
    assert_eq!(levenshtein("audit", ""), 5);
}

#[test]
fn classic_pair_is_three() {
    assert_eq!(levenshtein("kitten", "sitting"), 3);
}

#[test]
fn counts_scalar_values_not_bytes() {
    assert_eq!(levenshtein("é", "e"), 1);
}
