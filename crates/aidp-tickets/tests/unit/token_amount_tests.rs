use super::*;

#[test]
fn suffixes_scale_the_amount() {
    assert_eq!(TokenAmount::parse("3M"), Some(TokenAmount(3_000_000)));
    assert_eq!(TokenAmount::parse("1.5m"), Some(TokenAmount(1_500_000)));
    assert_eq!(TokenAmount::parse("0,5M"), Some(TokenAmount(500_000)));
    assert_eq!(TokenAmount::parse("500k"), Some(TokenAmount(500_000)));
    assert_eq!(TokenAmount::parse("1_000"), Some(TokenAmount(1_000)));
}

#[test]
fn garbage_is_refused() {
    assert_eq!(TokenAmount::parse(""), None);
    assert_eq!(TokenAmount::parse("lots"), None);
    assert_eq!(TokenAmount::parse("-3M"), None);
}

#[test]
fn yaml_accepts_numbers_and_strings() {
    let numbers: Vec<TokenAmount> = serde_yaml::from_str("[3000000, 2.0e6, 1.5M, '500k']").unwrap();
    assert_eq!(
        numbers,
        vec![
            TokenAmount(3_000_000),
            TokenAmount(2_000_000),
            TokenAmount(1_500_000),
            TokenAmount(500_000)
        ]
    );
}

#[test]
fn amounts_serialize_as_plain_numbers() {
    assert_eq!(
        serde_json::to_string(&TokenAmount(1_500_000)).unwrap(),
        "1500000"
    );
}
