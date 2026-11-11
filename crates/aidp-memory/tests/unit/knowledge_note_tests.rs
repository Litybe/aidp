use super::*;
use crate::chunking::markdown::chunk_markdown;

const NOTE: &str = include_str!("../fixtures/knowledge/Knowledge/_global/unity/coyote-time.md");
const PROPOSED: &str = include_str!("../fixtures/knowledge/Knowledge/_global/csharp/draft-rule.md");
const SUPERSEDED: &str =
    include_str!("../fixtures/knowledge/Knowledge/_global/mechanics/old-rule.md");
const GLOSSARY: &str = include_str!("../fixtures/knowledge/Knowledge/_glossary.md");
const STEM: &str = "stem";

#[test]
fn frontmatter_fields_are_read() {
    let note = parse_knowledge(NOTE, "coyote-time");
    assert_eq!(note.front.note_type.as_deref(), Some("note"));
    assert_eq!(note.front.domain.as_deref(), Some("unity"));
    assert_eq!(note.front.tags, ["movement", "jump"]);
    assert_eq!(note.front.agents, ["unity-issue-implementer", "executor"]);
    assert_eq!(note.front.status.as_deref(), Some("active"));
    assert_eq!(note.front.importance, Some(0.9));
    assert_eq!(note.title, "Temps de grace au saut");
    assert!(note.is_indexable());
}

#[test]
fn crlf_files_parse_like_lf_ones() {
    let note = parse_knowledge(&NOTE.replace('\n', "\r\n"), STEM);
    assert_eq!(note.front.domain.as_deref(), Some("unity"));
    assert_eq!(note.title, "Temps de grace au saut");
}

#[test]
fn proposed_superseded_and_glossary_files_are_not_indexable() {
    assert!(!parse_knowledge(PROPOSED, STEM).is_indexable());
    assert!(!parse_knowledge(SUPERSEDED, STEM).is_indexable());
    assert!(!parse_knowledge(GLOSSARY, STEM).is_indexable());
}

#[test]
fn a_file_without_valid_frontmatter_is_indexable_with_its_heading_as_title() {
    let plain = parse_knowledge("# Journal\n\ntexte", STEM);
    assert!(plain.is_indexable());
    assert_eq!(plain.title, "Journal");
    assert_eq!(plain.front, NoteFrontmatter::default());
    let broken = parse_knowledge("---\nimportance: [oops\n---\n\n# Titre\n", STEM);
    assert!(broken.is_indexable());
    assert_eq!(broken.front, NoteFrontmatter::default());
    assert_eq!(parse_knowledge("sans titre", STEM).title, STEM);
}

#[test]
fn the_frontmatter_title_beats_the_heading() {
    let note = parse_knowledge("---\ntitle: Nom\n---\n# Autre\n", STEM);
    assert_eq!(note.title, "Nom");
}

#[test]
fn chunk_titles_start_with_the_note_title() {
    let note = parse_knowledge("---\ntitle: Nom\n---\nintro\n\n## Partie\ncorps\n", STEM);
    let mut chunks = chunk_markdown("intro\n\n## Partie\ncorps\n", STEM);
    note.title_chunks(&mut chunks, STEM);
    let titles: Vec<&str> = chunks.iter().map(|chunk| chunk.title.as_str()).collect();
    assert_eq!(titles, ["Nom", "Nom > Partie"]);
    let mut again = chunks.clone();
    note.title_chunks(&mut again, STEM);
    assert_eq!(again, chunks);
}
