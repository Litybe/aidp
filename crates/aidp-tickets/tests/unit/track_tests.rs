use super::*;

const VARIANTS: [(Track, &str); 4] = [
    (Track::Quick, "quick"),
    (Track::Standard, "standard"),
    (Track::Deep, "deep"),
    (Track::Bug, "bug"),
];
const MIXED_CASE: &str = "  Deep ";
const UNKNOWN: &str = "turbo";

/// Every variant parses from and renders to its lowercase wire form.
#[test]
fn every_variant_round_trips_through_its_wire_form() {
    for (track, wire) in VARIANTS {
        assert_eq!(track.as_str(), wire);
        assert_eq!(track.to_string(), wire);
        assert_eq!(wire.parse::<Track>().unwrap(), track);
    }
}

/// Parsing ignores case and surrounding whitespace.
#[test]
fn parsing_is_case_insensitive_and_trims() {
    assert_eq!(MIXED_CASE.parse::<Track>().unwrap(), Track::Deep);
}

/// An unknown value is refused with the expected list in the message.
#[test]
fn unknown_track_is_refused() {
    let err = UNKNOWN.parse::<Track>().unwrap_err().to_string();
    assert!(err.contains(UNKNOWN));
    assert!(err.contains(EXPECTED));
}

/// Serde uses the lowercase spelling already written in notes.
#[test]
fn serde_spelling_is_lowercase() {
    for (track, wire) in VARIANTS {
        let yaml = serde_yaml::to_string(&track).unwrap();
        assert_eq!(yaml.trim(), wire);
        assert_eq!(serde_yaml::from_str::<Track>(wire).unwrap(), track);
    }
}
