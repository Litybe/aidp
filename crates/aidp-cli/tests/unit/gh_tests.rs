use super::*;

const LOGIN: &str = "Alice-Dev";
const LOGIN_LOWER: &str = "alice-dev";
const OTHER_LOGIN: &str = "bob";

/// An absent filter accepts every author.
#[test]
fn author_filter_absent_accepts_everyone() {
    assert!(author_matches(None, LOGIN));
}

/// The author filter ignores ASCII case.
#[test]
fn author_filter_ignores_case() {
    assert!(author_matches(Some(LOGIN_LOWER), LOGIN));
}

/// A different login is rejected.
#[test]
fn author_filter_rejects_other_login() {
    assert!(!author_matches(Some(OTHER_LOGIN), LOGIN));
}

const FORBIDDEN_STATUS: u16 = 403;
const CONFLICT_STATUS: u16 = 409;
const NOT_FOUND_STATUS: u16 = 404;
const BAD_REQUEST_STATUS: u16 = 400;
const SERVER_ERROR_STATUS: u16 = 500;
const FAILURE_MESSAGE: &str = "daemon said no";
const UNREACHABLE_MESSAGE: &str = "gateway unreachable (refused)";

/// Builds a daemon failure carrying the given HTTP status.
fn http_failure(status: u16) -> anyhow::Error {
    GatewayHttpError {
        status,
        message: FAILURE_MESSAGE.to_string(),
    }
    .into()
}

/// Extracts the typed exit status a mapped failure carries.
fn mapped_status(error: anyhow::Error) -> ExitStatus {
    daemon_failure(error)
        .downcast_ref::<TypedExit>()
        .map(|typed| typed.status)
        .unwrap()
}

/// A 403 reply is a policy refusal.
#[test]
fn daemon_forbidden_maps_to_policy_refusal() {
    assert_eq!(
        mapped_status(http_failure(FORBIDDEN_STATUS)),
        ExitStatus::PolicyRefusal
    );
}

/// A 409 reply is a policy refusal.
#[test]
fn daemon_conflict_maps_to_policy_refusal() {
    assert_eq!(
        mapped_status(http_failure(CONFLICT_STATUS)),
        ExitStatus::PolicyRefusal
    );
}

/// A 404 reply is not found.
#[test]
fn daemon_not_found_maps_to_not_found() {
    assert_eq!(
        mapped_status(http_failure(NOT_FOUND_STATUS)),
        ExitStatus::NotFound
    );
}

/// A 400 reply is a usage error.
#[test]
fn daemon_bad_request_maps_to_usage() {
    assert_eq!(
        mapped_status(http_failure(BAD_REQUEST_STATUS)),
        ExitStatus::Usage
    );
}

/// Any other status is internal, whatever the message says.
#[test]
fn daemon_other_status_maps_to_internal() {
    assert_eq!(
        mapped_status(http_failure(SERVER_ERROR_STATUS)),
        ExitStatus::Internal
    );
}

/// An unreachable daemon is an upstream failure.
#[test]
fn daemon_unreachable_maps_to_upstream_failure() {
    let error: anyhow::Error = GatewayUnreachable {
        message: UNREACHABLE_MESSAGE.to_string(),
    }
    .into();
    assert_eq!(mapped_status(error), ExitStatus::UpstreamFailure);
}

/// The original message survives the mapping unchanged.
#[test]
fn daemon_failure_keeps_message() {
    assert_eq!(
        daemon_failure(http_failure(FORBIDDEN_STATUS)).to_string(),
        FAILURE_MESSAGE
    );
}
