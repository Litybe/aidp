use super::with_db;
use crate::redact_url_in;

#[test]
fn adds_db_when_missing() {
    assert_eq!(
        with_db("mongodb://h:27017", "cheat"),
        "mongodb://h:27017/cheat"
    );
    assert_eq!(
        with_db("mongodb+srv://u:p@c.net?retryWrites=true", "cheat"),
        "mongodb+srv://u:p@c.net/cheat?retryWrites=true"
    );
}

#[test]
fn keeps_existing_db() {
    assert_eq!(
        with_db("mongodb://h:27017/other", "cheat"),
        "mongodb://h:27017/other"
    );
}

const PASSWORD_SENTINEL: &str = "pw-SENTINEL";

#[test]
fn mongo_output_hides_password_but_keeps_rest() {
    let uri = format!("mongodb://admin:{PASSWORD_SENTINEL}@db.local:27017/cheat");
    let stdout = format!("connecting admin:{PASSWORD_SENTINEL}@db.local ok {{\"n\":1}}");
    let redacted = redact_url_in(&stdout, &uri);
    assert!(!redacted.contains(PASSWORD_SENTINEL));
    assert!(redacted.contains("{\"n\":1}"));
}
