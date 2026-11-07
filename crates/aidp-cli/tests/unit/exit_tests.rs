use aidp_core::ExitStatus;
use aidp_gateway::session_error::SessionError;
use aidp_integrations::IntegrationError;
use aidp_tickets::TicketError;

#[test]
fn ticket_error_not_found_classifies_not_found() {
    let e = anyhow::Error::new(TicketError::NotFound("PIXEL-1".into()));
    assert_eq!(super::classify(&e), ExitStatus::NotFound);
}

#[test]
fn integration_error_config_classifies_usage() {
    let e = anyhow::Error::new(IntegrationError::Config {
        var: "SONAR_TOKEN",
        remedy: "set SONAR_TOKEN",
    });
    assert_eq!(super::classify(&e), ExitStatus::Usage);
}

#[test]
fn session_error_shutting_down_classifies_upstream_failure() {
    let e = anyhow::Error::new(SessionError::ShuttingDown);
    assert_eq!(super::classify(&e), ExitStatus::UpstreamFailure);
}

#[test]
fn unclassifiable_error_falls_back_to_internal() {
    let e = anyhow::anyhow!("something unexpected happened");
    assert_eq!(super::classify(&e), ExitStatus::Internal);
}
