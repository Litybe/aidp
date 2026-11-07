use super::*;
use crate::REDACTED;

#[test]
fn parses_verb_url_headers_body() {
    let mut vars = BTreeMap::new();
    vars.insert("base_url".to_string(), "https://dev.example".to_string());
    vars.insert("id".to_string(), "42".to_string());
    let text = "\
### create
# comment
POST {{base_url}}/users/{{id}}
Content-Type: application/json
X-Env: dev

{ \"name\": \"a\" }
";
    let specs = parse_http_file(text, &mut vars);
    assert_eq!(specs.len(), 1);
    let s = &specs[0];
    assert_eq!(s.method, "POST");
    assert_eq!(s.url, "https://dev.example/users/42");
    assert_eq!(s.headers.len(), 2);
    assert_eq!(s.body.as_deref(), Some("{ \"name\": \"a\" }"));
}

#[test]
fn defaults_method_to_get_and_splits_multiple() {
    let mut vars = BTreeMap::new();
    let text = "https://a.example/one\n###\nGET https://a.example/two\n";
    let specs = parse_http_file(text, &mut vars);
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].method, "GET");
    assert_eq!(specs[0].url, "https://a.example/one");
    assert_eq!(specs[1].url, "https://a.example/two");
}

#[test]
fn sensitive_headers_are_masked_others_kept() {
    assert_eq!(mask_header("Set-Cookie", "sid=abc"), REDACTED);
    assert_eq!(mask_header("x-request-id", "req-1"), "req-1");
}
