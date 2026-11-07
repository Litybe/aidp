use super::Format;

/// Every format survives a Display then FromStr round trip.
#[test]
fn every_format_round_trips_through_display_and_from_str() {
    for format in [
        Format::JsonCompact,
        Format::JsonPretty,
        Format::Ndjson,
        Format::Text,
    ] {
        assert_eq!(format.to_string().parse::<Format>().unwrap(), format);
    }
}

/// The parse error names the four accepted values.
#[test]
fn unknown_format_error_lists_the_four_values() {
    let message = "yaml".parse::<Format>().unwrap_err();
    for expected in ["json-compact", "json-pretty", "ndjson", "text"] {
        assert!(message.contains(expected), "{message}");
    }
}
