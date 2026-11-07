use super::SkipReason;

/// Every variant's `as_str` must equal the dispatcher's documented stable
/// slug, unchanged across the daemon's state file, its HTTP API and the
/// docs.
#[test]
fn every_variant_serializes_to_its_stable_slug() {
    let cases = [
        (SkipReason::ProjectBusy, "project-busy"),
        (SkipReason::Vanished, "vanished"),
        (SkipReason::ChangedSincePlan, "changed-since-plan"),
        (SkipReason::NeedsRouting, "needs-routing"),
        (SkipReason::IllegalTransition, "illegal-transition"),
        (SkipReason::SaveFailed, "save-failed"),
        (SkipReason::InputsFileFailed, "inputs-file-failed"),
        (SkipReason::StartFailed, "start-failed"),
        (SkipReason::QueueRace, "queue-race"),
        (SkipReason::WorkspaceBusy, "workspace-busy"),
    ];
    for (reason, expected) in cases {
        assert_eq!(reason.as_str(), expected);
    }
}

/// `Display` must format identically to `as_str`, since callers build
/// journalled event strings (e.g. `queue-race:{queue_id}`) by interpolating
/// it directly.
#[test]
fn display_matches_as_str() {
    for reason in [
        SkipReason::ProjectBusy,
        SkipReason::Vanished,
        SkipReason::ChangedSincePlan,
        SkipReason::NeedsRouting,
        SkipReason::IllegalTransition,
        SkipReason::SaveFailed,
        SkipReason::InputsFileFailed,
        SkipReason::StartFailed,
        SkipReason::QueueRace,
        SkipReason::WorkspaceBusy,
    ] {
        assert_eq!(reason.to_string(), reason.as_str());
    }
}
