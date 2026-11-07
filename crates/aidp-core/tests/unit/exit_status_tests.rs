use super::{ExitStatus, ALL};
use crate::models::enums::ErrorKind;
use std::collections::HashSet;

#[test]
fn every_variant_has_a_distinct_code_and_family() {
    let codes: HashSet<u8> = ALL.iter().map(|status| status.code()).collect();
    let families: HashSet<&str> = ALL.iter().map(|status| status.family()).collect();
    assert_eq!(codes.len(), ALL.len());
    assert_eq!(families.len(), ALL.len());
}

#[test]
fn codes_span_zero_to_seven() {
    let mut codes: Vec<u8> = ALL.iter().map(|status| status.code()).collect();
    codes.sort_unstable();
    assert_eq!(codes, (0..=7).collect::<Vec<u8>>());
}

#[test]
fn from_kind_covers_every_error_kind() {
    assert_eq!(
        ExitStatus::from_kind(ErrorKind::NotFound),
        ExitStatus::NotFound
    );
    assert_eq!(
        ExitStatus::from_kind(ErrorKind::Conflict),
        ExitStatus::PolicyRefusal
    );
    assert_eq!(
        ExitStatus::from_kind(ErrorKind::Unauthorized),
        ExitStatus::PolicyRefusal
    );
    assert_eq!(ExitStatus::from_kind(ErrorKind::Config), ExitStatus::Usage);
    assert_eq!(
        ExitStatus::from_kind(ErrorKind::Upstream),
        ExitStatus::UpstreamFailure
    );
    assert_eq!(
        ExitStatus::from_kind(ErrorKind::ShuttingDown),
        ExitStatus::UpstreamFailure
    );
    assert_eq!(ExitStatus::from_kind(ErrorKind::Io), ExitStatus::Internal);
}
