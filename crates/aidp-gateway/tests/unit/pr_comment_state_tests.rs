use super::*;
use aidp_testkit::TempWorkspace;

const REPO: &str = "acme/widgets";
const NUMBER: u64 = 7;
const POINT: &str = "p1";
const BOGUS_STATE: &str = "\"bogus\"";

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn path_is_one_file_per_pr() {
    let path = state_file(Path::new("/s"), REPO, NUMBER).unwrap();
    assert!(path.ends_with("prs/acme__widgets__7.json"));
}

#[test]
fn unsafe_repo_components_are_refused() {
    assert!(state_file(Path::new("/s"), "acme", NUMBER).is_err());
    assert!(state_file(Path::new("/s"), "../x/widgets", NUMBER).is_err());
    assert!(state_file(Path::new("/s"), "acme/.hidden", NUMBER).is_err());
}

#[test]
fn point_state_round_trips_and_none_removes() {
    let ws = TempWorkspace::new("pr-comment-state-points");
    let store = PrCommentState::new(ws.path());
    let set = store
        .set_point(REPO, NUMBER, POINT, PointState::Ignored)
        .unwrap();
    assert_eq!(set.points.get(POINT), Some(&PointState::Ignored));
    assert_eq!(load(ws.path(), REPO, NUMBER), set);
    let cleared = store
        .set_point(REPO, NUMBER, POINT, PointState::None)
        .unwrap();
    assert!(cleared.points.is_empty());
}

#[test]
fn unknown_point_state_is_refused() {
    assert!(serde_json::from_str::<PointState>(BOGUS_STATE).is_err());
}

#[test]
fn handed_ids_are_deduplicated() {
    let ws = TempWorkspace::new("pr-comment-state-handed");
    let store = PrCommentState::new(ws.path());
    store.add_handed(REPO, NUMBER, &ids(&["a", "b"])).unwrap();
    let file = store.add_handed(REPO, NUMBER, &ids(&["b", "c"])).unwrap();
    assert_eq!(file.handed, ids(&["a", "b", "c"]));
}

#[test]
fn missing_file_loads_empty() {
    let ws = TempWorkspace::new("pr-comment-state-missing");
    assert_eq!(load(ws.path(), REPO, NUMBER), PrCommentStateFile::default());
}
