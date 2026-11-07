use super::*;
use crate::models::enums::Source;

fn hit(id: &str, score: f64, text: &str) -> Hit {
    Hit {
        id: id.into(),
        source: Source::Doc,
        path: format!("{id}.md"),
        line_start: 1,
        line_end: 2,
        title: String::new(),
        score,
        text: text.into(),
        via: "bm25".into(),
    }
}

#[test]
fn near_duplicate_is_pushed_below_a_distinct_hit() {
    let candidates = vec![
        hit("a", 10.0, "double saut heroine hauteur reglage"),
        hit("b", 9.5, "double saut heroine hauteur reglage"),
        hit("c", 6.0, "boss phase arene lumiere"),
    ];
    let ids: Vec<String> = select(candidates, 2, 0.7)
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(ids, ["a", "c"]);
}

#[test]
fn lambda_one_keeps_score_order() {
    let candidates = vec![
        hit("a", 3.0, "x y"),
        hit("b", 2.0, "x y"),
        hit("c", 1.0, "z"),
    ];
    let ids: Vec<String> = select(candidates, 3, 1.0)
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn k_caps_the_selection() {
    let candidates = vec![hit("a", 3.0, "aa"), hit("b", 2.0, "bb")];
    assert_eq!(select(candidates, 1, 0.7).len(), 1);
}
