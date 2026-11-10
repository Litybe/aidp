use super::*;

#[test]
fn an_mcp_connect_timeout_is_recognised() {
    for text in [
        "stage implement failed: CONNECT_TIMEOUT",
        "MCP server unity-mcp: connection timed out after 30s",
        "mcp server pixellab connection timeout",
    ] {
        assert_eq!(classify(text), Some(Transient::McpConnect), "{text}");
    }
}

#[test]
fn a_unity_compile_gate_timeout_is_recognised() {
    for text in [
        "Unity compile gate timed out after 300 s",
        "compile gate: timeout waiting for the editor",
    ] {
        assert_eq!(classify(text), Some(Transient::UnityCompile), "{text}");
    }
}

#[test]
fn a_lost_session_is_recognised() {
    for text in [
        "session lost while the stage ran",
        "No conversation found with session ID 1234",
        "Session expired",
    ] {
        assert_eq!(classify(text), Some(Transient::SessionLost), "{text}");
    }
}

#[test]
fn an_overloaded_or_rate_limited_api_is_recognised() {
    for text in [
        "API Error: 529 overloaded_error",
        "rate_limit_error: slow down",
        "You have hit your usage limit",
        "HTTP 429 Too Many Requests",
    ] {
        assert_eq!(classify(text), Some(Transient::ApiOverload), "{text}");
    }
}

#[test]
fn a_real_failure_is_not_a_passing_fault() {
    for text in [
        "implement: needs_review",
        "correction did not converge",
        "test passed=false: 3 assertions failed",
        "",
    ] {
        assert_eq!(classify(text), None, "{text}");
    }
}

#[test]
fn every_fault_maps_to_its_category() {
    assert_eq!(
        Transient::McpConnect.category(),
        DoctorCategory::McpConnectTimeout
    );
    assert_eq!(
        Transient::UnityCompile.category(),
        DoctorCategory::UnityCompileTimeout
    );
    assert_eq!(
        Transient::SessionLost.category(),
        DoctorCategory::SessionLost
    );
}

#[test]
fn the_art_templates_are_told_apart() {
    assert!(is_art_template("art-produce"));
    assert!(is_art_template("art-fix"));
    assert!(is_art_template("pixelart"));
    assert!(!is_art_template("unity-implement"));
    assert!(!is_art_template("flow-doctor"));
}
