use super::*;

const PATH_SENTINEL: &str = "T-SENTINEL/B-SENTINEL/token-SENTINEL";
const PASSWORD_SENTINEL: &str = "pw-SENTINEL";

#[test]
fn webhook_url_keeps_scheme_and_host_only() {
    let redacted = redact_url(&format!(
        "https://hooks.example.com/services/{PATH_SENTINEL}"
    ));
    assert!(redacted.starts_with("https://hooks.example.com"));
    assert!(!redacted.contains("SENTINEL"));
}

#[test]
fn userinfo_and_query_are_dropped() {
    let redacted = redact_url(&format!(
        "https://user:{PASSWORD_SENTINEL}@h.example.com:8443/p?k=v"
    ));
    assert_eq!(redacted, format!("https://h.example.com:8443/{REDACTED}"));
}

#[test]
fn invalid_url_becomes_marker() {
    assert_eq!(redact_url("not a url"), REDACTED);
}

#[test]
fn error_text_loses_url_and_password() {
    let raw = format!("https://u:{PASSWORD_SENTINEL}@h.example.com/x");
    let text = format!("failed {raw}; retry with {PASSWORD_SENTINEL}");
    assert!(!redact_url_in(&text, &raw).contains("SENTINEL"));
}
