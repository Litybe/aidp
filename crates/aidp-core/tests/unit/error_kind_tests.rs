use super::{ErrorKind, HasErrorKind};

/// Every variant's `as_str` must equal its documented stable slug.
#[test]
fn every_variant_serializes_to_its_stable_slug() {
    let cases = [
        (ErrorKind::NotFound, "not_found"),
        (ErrorKind::Conflict, "conflict"),
        (ErrorKind::Unauthorized, "unauthorized"),
        (ErrorKind::Config, "config"),
        (ErrorKind::Upstream, "upstream"),
        (ErrorKind::Io, "io"),
        (ErrorKind::ShuttingDown, "shutting_down"),
    ];
    for (kind, expected) in cases {
        assert_eq!(kind.as_str(), expected);
    }
}

/// A type implementing `HasErrorKind` exposes its kind through the trait.
#[test]
fn has_error_kind_trait_exposes_the_kind() {
    struct SampleError;
    impl HasErrorKind for SampleError {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Upstream
        }
    }
    assert_eq!(SampleError.kind().as_str(), "upstream");
}
