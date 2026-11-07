use crate::commands::CommandOutput;
use aidp_core::exit_status::classify_core;
use aidp_core::models::enums::HasErrorKind;
use aidp_core::ExitStatus;
use aidp_gateway::session_error::SessionError;
use aidp_integrations::IntegrationError;
use aidp_memory::MemoryError;
use aidp_tickets::TicketError;

/// Classifies an `anyhow::Error` returned from `real_main` into the
/// [`ExitStatus`] the process should terminate with.
///
/// Tries `aidp-core`'s own classification first, then walks the chain for
/// each crate's typed error, then falls back to [`ExitStatus::Internal`] —
/// never [`ExitStatus::NotFound`], which would misreport an unclassified
/// failure as a missing resource.
pub fn classify(e: &anyhow::Error) -> ExitStatus {
    if let Some(status) = classify_core(e) {
        return status;
    }
    for cause in e.chain() {
        if let Some(err) = cause.downcast_ref::<TicketError>() {
            return ExitStatus::from_kind(err.kind());
        }
        if let Some(err) = cause.downcast_ref::<MemoryError>() {
            return ExitStatus::from_kind(err.kind());
        }
        if let Some(err) = cause.downcast_ref::<IntegrationError>() {
            return ExitStatus::from_kind(err.kind());
        }
        if let Some(err) = cause.downcast_ref::<SessionError>() {
            return ExitStatus::from_kind(err.kind());
        }
    }
    ExitStatus::Internal
}

/// The exit status of a finished command: its failure classified, else its own status, else success.
pub fn exit_status(output: &CommandOutput) -> ExitStatus {
    match &output.failure {
        Some(failure) => classify(failure),
        None => output.exit.unwrap_or(ExitStatus::Success),
    }
}

#[cfg(test)]
#[path = "../tests/unit/exit_tests.rs"]
mod exit_tests;
