use super::*;
use aidp_tickets::model::{IssueType, Priority, Source, Status};
use aidp_tickets::Track;
use clap::ValueEnum;

#[test]
fn status_arg_converts_every_variant_to_its_domain_name() {
    for arg in StatusArg::value_variants() {
        let name = arg.to_possible_value().unwrap().get_name().to_string();
        assert_eq!(Status::from(*arg).as_str(), name);
    }
    assert_eq!(StatusArg::value_variants().len(), 7);
}

#[test]
fn priority_arg_accepts_names_and_aliases() {
    for arg in PriorityArg::value_variants() {
        let name = arg.to_possible_value().unwrap().get_name().to_string();
        assert_eq!(Priority::from(*arg).as_str().to_lowercase(), name);
    }
    for spelling in ["highest", "p1", "1"] {
        let parsed = PriorityArg::from_str(spelling, true).unwrap();
        assert_eq!(Priority::from(parsed), Priority::Highest);
    }
    let lowest = PriorityArg::from_str("P5", true).unwrap();
    assert_eq!(Priority::from(lowest), Priority::Lowest);
}

#[test]
fn source_arg_converts_every_variant() {
    for arg in SourceArg::value_variants() {
        let name = arg.to_possible_value().unwrap().get_name().to_string();
        assert_eq!(Source::from(*arg).as_str(), name);
    }
}

#[test]
fn issue_type_arg_converts_every_variant() {
    for arg in IssueTypeArg::value_variants() {
        let name = arg.to_possible_value().unwrap().get_name().to_string();
        let domain = IssueType::from(*arg);
        assert_eq!(name.parse::<IssueType>().unwrap(), domain);
    }
}

#[test]
fn track_arg_matches_its_cli_spelling() {
    for arg in TrackArg::value_variants() {
        let name = arg.to_possible_value().unwrap().get_name().to_string();
        assert_eq!(Track::from(*arg).as_str(), name);
    }
}
