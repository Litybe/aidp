use super::*;

fn hit(id: &str, text: &str) -> Hit {
    Hit {
        id: id.into(),
        source: Source::Doc,
        path: format!("Docs/{id}.md"),
        line_start: 3,
        line_end: 9,
        title: "Titre".into(),
        score: 1.0,
        text: text.into(),
        via: "bm25".into(),
    }
}

#[test]
fn citation_has_line_range() {
    assert_eq!(hit("a", "x").citation(), "Docs/a.md:L3-L9");
}

#[test]
fn packing_stays_under_budget_and_skips_what_does_not_fit() {
    let hits = vec![
        hit("a", &"x".repeat(200)),
        hit("b", &"y".repeat(2000)),
        hit("c", "court"),
    ];
    let packed = pack(hits, 100);
    let ids: Vec<&str> = packed.hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["a", "c"]);
    assert_eq!(packed.dropped, 1);
    assert!(packed.used_tokens <= 100);
}

#[test]
fn first_oversized_hit_is_cut_to_the_budget() {
    let packed = pack(vec![hit("a", &"x".repeat(5000))], 50);
    assert_eq!(packed.hits.len(), 1);
    assert!(packed.used_tokens <= 50);
    assert!(packed.hits[0].text.ends_with("..."));
}

#[test]
fn markdown_carries_citations_and_code_language() {
    let mut code = hit("a", "void Run() {}");
    code.source = Source::Code;
    let markdown = render_markdown(&pack(vec![code], 500));
    assert!(markdown.contains("### Docs/a.md:L3-L9 - Titre"));
    assert!(markdown.contains("```cs"));
}
