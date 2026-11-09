use super::*;

#[test]
fn other_paths_and_methods_are_not_answered() {
    assert!(handle(None, None, &Method::Post, "/api/doctor").is_none());
    assert!(handle(None, None, &Method::Get, "/api/flows/GH-1").is_none());
    assert!(handle(None, None, &Method::Get, "/api/flows/GH-1/journal").is_none());
}

#[test]
fn without_a_doctor_the_routes_answer_unavailable() {
    let (status, body) = handle(None, None, &Method::Get, "/api/flows/GH-1/doctor").unwrap();
    assert_eq!(status, STATUS_UNAVAILABLE);
    assert_eq!(body["error"], UNAVAILABLE);
    let (status, _) = handle(None, None, &Method::Get, "/api/doctor").unwrap();
    assert_eq!(status, STATUS_UNAVAILABLE);
}
