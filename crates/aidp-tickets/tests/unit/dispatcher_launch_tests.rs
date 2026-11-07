use super::DispatcherLaunch;

const SESSION_SLUG: &str = "session";
const FLOW_SLUG: &str = "flow";

/// The default launch mode is a pipeline session.
#[test]
fn default_is_session() {
    assert_eq!(DispatcherLaunch::default(), DispatcherLaunch::Session);
}

/// Each variant serializes to its lowercase slug and parses back.
#[test]
fn variants_round_trip_through_their_slug() {
    for (variant, slug) in [
        (DispatcherLaunch::Session, SESSION_SLUG),
        (DispatcherLaunch::Flow, FLOW_SLUG),
    ] {
        assert_eq!(serde_yaml::to_string(&variant).unwrap().trim(), slug);
        assert_eq!(
            serde_yaml::from_str::<DispatcherLaunch>(slug).unwrap(),
            variant
        );
    }
}
