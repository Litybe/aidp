use crate::models::constants::exit_status as slug;
use crate::models::enums::ErrorKind;

/// Process exit status this binary can terminate with, one variant per
/// distinct outcome family a caller (a script, a CI job, a hook) branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitStatus {
    /// The command completed with no error.
    Success = 0,
    /// An unclassified internal failure.
    Internal = 1,
    /// A hook asked the session to wake up and act.
    HookWake = 2,
    /// The requested resource does not exist.
    NotFound = 3,
    /// The action is refused by policy, scope, or authorization.
    PolicyRefusal = 4,
    /// A call to an upstream system failed.
    UpstreamFailure = 5,
    /// A check or gate produced a negative verdict.
    NegativeVerdict = 6,
    /// The command was invoked or configured incorrectly.
    Usage = 7,
}

/// Every [`ExitStatus`] variant, in ascending code order.
pub const ALL: [ExitStatus; 8] = [
    ExitStatus::Success,
    ExitStatus::Internal,
    ExitStatus::HookWake,
    ExitStatus::NotFound,
    ExitStatus::PolicyRefusal,
    ExitStatus::UpstreamFailure,
    ExitStatus::NegativeVerdict,
    ExitStatus::Usage,
];

impl ExitStatus {
    /// The numeric process exit code for this status.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The stable slug identifying this status's family.
    pub const fn family(self) -> &'static str {
        match self {
            ExitStatus::Success => slug::SUCCESS,
            ExitStatus::Internal => slug::INTERNAL,
            ExitStatus::HookWake => slug::HOOK_WAKE,
            ExitStatus::NotFound => slug::NOT_FOUND,
            ExitStatus::PolicyRefusal => slug::POLICY_REFUSAL,
            ExitStatus::UpstreamFailure => slug::UPSTREAM_FAILURE,
            ExitStatus::NegativeVerdict => slug::NEGATIVE_VERDICT,
            ExitStatus::Usage => slug::USAGE,
        }
    }

    /// Classifies an [`ErrorKind`] into the [`ExitStatus`] a CLI should exit with.
    pub fn from_kind(kind: ErrorKind) -> ExitStatus {
        match kind {
            ErrorKind::NotFound => ExitStatus::NotFound,
            ErrorKind::Conflict => ExitStatus::PolicyRefusal,
            ErrorKind::Unauthorized => ExitStatus::PolicyRefusal,
            ErrorKind::Config => ExitStatus::Usage,
            ErrorKind::Upstream => ExitStatus::UpstreamFailure,
            ErrorKind::ShuttingDown => ExitStatus::UpstreamFailure,
            ErrorKind::Io => ExitStatus::Internal,
        }
    }
}

impl From<ExitStatus> for std::process::ExitCode {
    fn from(status: ExitStatus) -> Self {
        std::process::ExitCode::from(status.code())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/exit_status_tests.rs"]
mod exit_status_tests;
