use super::*;
use aidp_integrations::ErrorKind;

/// Every [`ApiStatus`] variant's `status()`/`code()` pair, checked against
/// the wire values this crate has always answered with.
const EXPECTED: &[(ApiStatus, u16, &str)] = &[
    (ApiStatus::BadRequest, 400, "bad_request"),
    (ApiStatus::Unauthorized, 401, "unauthorized"),
    (ApiStatus::Forbidden, 403, "forbidden"),
    (ApiStatus::NotFound, 404, "not_found"),
    (ApiStatus::Conflict, 409, "conflict"),
    (ApiStatus::PayloadTooLarge, 413, "payload_too_large"),
    (ApiStatus::Internal, 500, "internal"),
    (ApiStatus::Io, 500, "io"),
    (ApiStatus::Upstream, 502, "upstream"),
    (ApiStatus::Config, 503, "config"),
    (ApiStatus::ShuttingDown, 503, "shutting_down"),
    (ApiStatus::Error, 500, "error"),
];

#[test]
fn status_and_code_match_the_documented_wire_values() {
    for (variant, expected_status, expected_code) in EXPECTED {
        assert_eq!(variant.status(), *expected_status, "code {expected_code}");
        assert_eq!(variant.code(), *expected_code);
    }
}

#[test]
fn io_and_internal_share_five_hundred_but_answer_different_codes() {
    assert_eq!(ApiStatus::Io.status(), ApiStatus::Internal.status());
    assert_ne!(ApiStatus::Io.code(), ApiStatus::Internal.code());
}

#[test]
fn config_and_shutting_down_share_five_oh_three_but_answer_different_codes() {
    assert_eq!(ApiStatus::Config.status(), ApiStatus::ShuttingDown.status());
    assert_ne!(ApiStatus::Config.code(), ApiStatus::ShuttingDown.code());
}

#[test]
fn from_status_recovers_the_documented_variant_for_every_known_code() {
    for (variant, status, _code) in EXPECTED {
        if matches!(
            variant,
            ApiStatus::Io | ApiStatus::ShuttingDown | ApiStatus::Error
        ) {
            // Each shares its HTTP status with another variant (Internal,
            // Config, Internal again): `from_status` recovers the shared
            // default for that status, not every variant that maps to it.
            continue;
        }
        assert_eq!(ApiStatus::from_status(*status), *variant);
    }
}

#[test]
fn from_status_falls_back_to_error_on_an_unknown_code() {
    assert_eq!(ApiStatus::from_status(418), ApiStatus::Error);
}

#[test]
fn error_kind_converts_to_the_matching_api_status() {
    let cases = [
        (ErrorKind::NotFound, ApiStatus::NotFound),
        (ErrorKind::Conflict, ApiStatus::Conflict),
        (ErrorKind::Unauthorized, ApiStatus::Unauthorized),
        (ErrorKind::Config, ApiStatus::Config),
        (ErrorKind::Upstream, ApiStatus::Upstream),
        (ErrorKind::Io, ApiStatus::Io),
        (ErrorKind::ShuttingDown, ApiStatus::ShuttingDown),
    ];
    for (kind, expected) in cases {
        assert_eq!(ApiStatus::from(kind), expected);
    }
}
