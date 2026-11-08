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

#[test]
fn bom_is_stripped_only_at_the_start() {
    assert_eq!(strip_bom("\u{FEFF}{}"), "{}");
    assert_eq!(strip_bom("{}"), "{}");
    assert_eq!(strip_bom("a\u{FEFF}"), "a\u{FEFF}");
}

#[test]
fn lenient_read_drops_the_bom_of_a_powershell_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.json");
    std::fs::write(&file, b"\xEF\xBB\xBF{\"a\":1}").unwrap();
    assert_eq!(read_text_lenient(&file).unwrap(), "{\"a\":1}");
}
