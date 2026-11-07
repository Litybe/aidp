use aidp_gateway::crypto::{ct_eq, hmac_sha256_hex};
use aidp_gateway::template::render;
use serde_json::json;

#[test]
fn hmac_rfc4231_vectors() {
    // All seven test cases from RFC 4231 §4.2–§4.8
    // Key bytes, data bytes, expected lowercase hex digest
    #[allow(clippy::useless_vec)]
    let vectors = vec![
        (
            // Case 1: 20-byte key, "Hi There"
            vec![0x0b; 20],
            b"Hi There".to_vec(),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        ),
        (
            // Case 2: "Jefe", "what do ya want for nothing?"
            b"Jefe".to_vec(),
            b"what do ya want for nothing?".to_vec(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
        (
            // Case 3: 20-byte key of 0xaa, 50 bytes of 0xdd
            vec![0xaa; 20],
            vec![0xdd; 50],
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        ),
        (
            // Case 4: 25 bytes (0x0102...1819), 50 bytes of 0xcd
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19],
            vec![0xcd; 50],
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
        ),
        (
            // Case 5: 20-byte key of 0x0c, "Test With Truncation"
            // RFC 4231 truncates to 128 bits (first 32 hex chars); we assert the full digest
            vec![0x0c; 20],
            b"Test With Truncation".to_vec(),
            // Full digest (RFC 4231 only specifies the first 16 bytes truncated)
            "a3b6167473100ee06e0c796c2955552bfa6f7c0a6a8aef8b93f860aab0cd20c5",
        ),
        (
            // Case 6: 131-byte key of 0xaa, "Test Using Larger Than Block-Size Key - Hash Key First"
            // This exercises the Sha256::digest(key) reduction path (crypto.rs)
            vec![0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
        ),
        (
            // Case 7: 131-byte key of 0xaa, longer data
            // Also exercises key reduction (the whole point of this test)
            vec![0xaa; 131],
            b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.".to_vec(),
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
        ),
    ];

    for (i, (key, data, expected)) in vectors.iter().enumerate() {
        let result = hmac_sha256_hex(key, data);
        assert_eq!(
            result,
            *expected,
            "RFC 4231 case {} failed: expected {}, got {}",
            i + 1,
            expected,
            result
        );
    }
}

#[test]
fn ct_eq_boundaries() {
    // Test constant-time comparison across boundary conditions
    let cases = vec![
        ("abc", "abc", true),
        ("abc", "abcd", false),
        ("", "", true),
        ("", "x", false),
        ("secret", "secrez", false),
        ("clé", "clé", true),
    ];

    for (a, b, expected) in cases {
        let result = ct_eq(a, b);
        assert_eq!(
            result, expected,
            "ct_eq({:?}, {:?}) failed: expected {}, got {}",
            a, b, expected, result
        );
    }

    // Note: ("abc", "abcd") is decided by the length comparison at crypto.rs,
    // before any byte is read — it never enters the fold at line 217.
}

#[test]
fn render_frozen_behaviour() {
    // Pin render's current behaviour as reference tests.
    // These test frozen semantics; changes to crypto.rs should update these
    // assertions to match the new code, not vice versa.

    // Case 1: missing key renders as empty string
    // This is frozen-by-design and flagged (the webhook is silently posted to a truncated URL).
    let ctx = json!({"payload": {"other": 1}});
    let tpl = "https://hooks.example.com/{{payload.issue.key}}/notify";
    let result = render(tpl, &ctx);
    assert_eq!(
        result, "https://hooks.example.com//notify",
        "Missing key should render as empty string (frozen behaviour)"
    );

    // Case 2: unclosed {{ renders literally
    let ctx = json!({});
    let tpl = "a{{unclosed";
    let result = render(tpl, &ctx);
    assert_eq!(result, "a{{unclosed");

    // Case 3: null value renders as empty string
    let ctx = json!({"k": null});
    let tpl = "x{{k}}y";
    let result = render(tpl, &ctx);
    assert_eq!(result, "xy");

    // Case 4: nested key lookup (three levels)
    let ctx = json!({"a": {"b": {"c": "v"}}});
    let tpl = "{{a.b.c}}";
    let result = render(tpl, &ctx);
    assert_eq!(result, "v");
}
