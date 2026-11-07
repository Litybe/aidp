use super::*;

#[test]
fn splits_by_heading_with_path_titles_and_lines() {
    let text =
        "---\ntitle: x\n---\n# Heroine\nIntro\n## Saut\nLe saut double.\n## Dash\nLe dash.\n";
    let chunks = chunk_markdown(text, "fallback");
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].title, "Heroine");
    assert_eq!(chunks[1].title, "Heroine > Saut");
    assert_eq!(chunks[1].line_start, 6);
    assert_eq!(chunks[1].line_end, 7);
    assert_eq!(chunks[2].title, "Heroine > Dash");
}

#[test]
fn heading_inside_code_fence_is_not_a_heading() {
    let text = "# A\n```\n# not a heading\n```\ntext\n";
    let chunks = chunk_markdown(text, "f");
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].text.contains("# not a heading"));
}

#[test]
fn text_before_first_heading_uses_fallback_title() {
    let chunks = chunk_markdown("preamble\n# H\nbody\n", "note");
    assert_eq!(chunks[0].title, "note");
}

#[test]
fn heading_only_sections_are_skipped() {
    let chunks = chunk_markdown("# Parent\n## Child\nbody\n", "f");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].title, "Parent > Child");
}

#[test]
fn long_section_respects_max_size_with_overlap() {
    let body = (0..120)
        .map(|n| format!("phrase numero {n} avec du texte"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = chunk_markdown(&format!("# Long\n{body}\n"), "f");
    assert!(chunks.len() > 2);
    assert!(chunks
        .iter()
        .all(|c| c.text.chars().count() <= MARKDOWN_MAX_CHARS));
    assert!(chunks[1].line_start <= chunks[0].line_end);
}
