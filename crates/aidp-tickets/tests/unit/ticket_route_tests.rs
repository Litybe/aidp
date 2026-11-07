use super::*;
use serde_json::json;

#[test]
fn the_route_field_wins() {
    let ticket = json!({ "route": "Art", "labels": ["route:map"], "template": "dev-unity" });
    assert_eq!(route_of_value(&ticket), Some(BudgetRoute::Art));
}

#[test]
fn a_route_label_is_read_when_there_is_no_field() {
    let ticket = json!({ "labels": ["bug", "aidp:route:doc"], "template": "dev-unity" });
    assert_eq!(route_of_value(&ticket), Some(BudgetRoute::Doc));
}

#[test]
fn the_template_is_the_last_resort() {
    assert_eq!(
        route_of_value(&json!({ "template": "pixelart" })),
        Some(BudgetRoute::Art)
    );
    assert_eq!(
        route_of_value(&json!({ "template": "dev-unity" })),
        Some(BudgetRoute::Code)
    );
    assert_eq!(
        route_of_value(&json!({ "template": "map-room" })),
        Some(BudgetRoute::Map)
    );
    assert_eq!(route_of_value(&json!({ "template": "study" })), None);
}

#[test]
fn an_unknown_route_value_falls_through() {
    let ticket = json!({ "route": "audio", "template": "dev-unity" });
    assert_eq!(route_of_value(&ticket), Some(BudgetRoute::Code));
}
