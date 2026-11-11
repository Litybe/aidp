//! Art gallery: art runs only, newest first, cover choice, score, filters, cursor.

use super::{gallery, ArtQuery};
use crate::models::native_run_view::NativeRunView;
use aidp_testkit::TempWorkspace;
use serde_json::json;
use std::fs;

fn view(ws: &TempWorkspace, id: &str, template: &str, status: &str, started: &str, ticket: &str) -> NativeRunView {
    let dir = ws.child(id);
    serde_json::from_value(json!({
        "runId": id,
        "templateId": template,
        "status": status,
        "workspace": ws.child("w"),
        "runDir": dir,
        "request": { "projectId": "p", "templateId": template, "ticket": ticket },
        "startedAt": started,
        "stages": [],
    }))
    .unwrap()
}

fn put(ws: &TempWorkspace, rel: &str, bytes: &[u8]) {
    let path = ws.child(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn query() -> ArtQuery {
    ArtQuery { limit: 10, ..ArtQuery::default() }
}

fn fixture() -> (TempWorkspace, Vec<NativeRunView>) {
    let ws = TempWorkspace::new("art-assets");
    put(&ws, "r1/generate/a.png", b"png");
    put(&ws, "r1/fix/b.png", b"png");
    put(&ws, "r1/export/final.png", b"png");
    put(&ws, "r1/verify/preview_1x.png", b"png");
    put(&ws, "r1/verify/findings.json", br#"{"verdict":{"score_out_of_10":7.5}}"#);
    put(&ws, "r2/generate/c.png", b"png");
    put(&ws, "r3/generate/d.png", b"png");
    put(&ws, "r4/report/notes.md", b"x");
    let runs = vec![
        view(&ws, "r1", "art-produce", "done", "2026-11-01T10:00:00Z", "GH-1"),
        view(&ws, "r2", "art-fix", "failed", "2026-11-02T10:00:00Z", "GH-2"),
        view(&ws, "r3", "dev-unity", "done", "2026-11-03T10:00:00Z", "GH-3"),
        view(&ws, "r4", "art-review", "done", "2026-11-04T10:00:00Z", "GH-4"),
    ];
    (ws, runs)
}

#[test]
fn lists_art_runs_with_images_newest_first() {
    let (_ws, runs) = fixture();
    let (found, next) = gallery(&runs, &query());
    let ids: Vec<&str> = found.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["r2", "r1"], "non-art and imageless runs are dropped");
    assert!(next.is_none());
    assert_eq!(found[0].status, "failed");
}

#[test]
fn cover_prefers_export_over_preview_and_score_is_read() {
    let (_ws, runs) = fixture();
    let (found, _) = gallery(&runs, &query());
    let r1 = found.iter().find(|r| r.run_id == "r1").unwrap();
    assert_eq!(r1.cover.path, "export/final.png");
    assert_eq!(r1.images.len(), 4);
    assert_eq!(r1.score, Some(7.5));
    assert_eq!(r1.ticket.as_deref(), Some("GH-1"));
}

#[test]
fn filters_by_status_issue_and_text() {
    let (_ws, runs) = fixture();
    let by = |q: ArtQuery| gallery(&runs, &q).0.len();
    assert_eq!(by(ArtQuery { status: Some("done".into()), ..query() }), 1);
    assert_eq!(by(ArtQuery { issue: Some("2".into()), ..query() }), 1);
    assert_eq!(by(ArtQuery { issue: Some("GH-1".into()), ..query() }), 1);
    assert_eq!(by(ArtQuery { q: Some("FINAL".into()), ..query() }), 1);
    assert_eq!(by(ArtQuery { q: Some("nothing".into()), ..query() }), 0);
}

#[test]
fn paginates_with_a_cursor() {
    let (_ws, runs) = fixture();
    let (first, next) = gallery(&runs, &ArtQuery { limit: 1, ..query() });
    assert_eq!(first[0].run_id, "r2");
    let cursor = next.expect("more pages");
    let (second, next) = gallery(&runs, &ArtQuery { limit: 1, before: Some(cursor), ..query() });
    assert_eq!(second[0].run_id, "r1");
    assert!(next.is_none());
}
