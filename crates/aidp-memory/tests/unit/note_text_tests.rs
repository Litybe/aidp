use super::*;
use crate::models::enums::{Domain, NoteStatus};

const NOTE: &str = "---\ntype: note\ndomain: unity\nstatus: active\nupdated: 2026-01-01\n---\n# Titre\n\nCorps.\n\n## Evidence\n- 2026-01-01 run:a\n\n## Autre\nSuite.\n";

fn draft() -> NoteDraft {
    NoteDraft {
        domain: Domain::Unity,
        tags: vec!["movement".into(), "jump".into()],
        agents: vec!["executor".into()],
        status: NoteStatus::Active,
        importance: 0.8,
        updated: "2026-11-11".into(),
        source: "run:abc".into(),
        title: "Coyote time".into(),
        body: "Texte de la note.".into(),
        extra: vec![("target".into(), "\"[[Knowledge/a]]\"".into())],
        evidence: vec!["2026-11-11 run:abc: why".into()],
    }
}

#[test]
fn a_note_is_rendered_in_the_frontmatter_order_of_existing_notes() {
    let expected = "---\ntype: note\ndomain: unity\ntags: [movement, jump]\nagents: [executor]\nstatus: active\nimportance: 0.8\nupdated: 2026-11-11\nsource: run:abc\ntarget: \"[[Knowledge/a]]\"\n---\n# Coyote time\n\nTexte de la note.\n\n## Evidence\n- 2026-11-11 run:abc: why\n";
    assert_eq!(render_note(&draft()), expected);
}

#[test]
fn a_rendered_note_parses_back() {
    let note = crate::knowledge_note::parse_knowledge(&render_note(&draft()), "x");
    assert_eq!(note.title, "Coyote time");
    assert_eq!(note.front.domain.as_deref(), Some("unity"));
    assert_eq!(note.front.tags, ["movement", "jump"]);
    assert_eq!(note.front.agents, ["executor"]);
    assert_eq!(note.front.importance, Some(0.8));
    assert_eq!(note.front.updated.as_deref(), Some("2026-11-11"));
    assert!(note.is_indexable());
}

#[test]
fn a_leading_title_line_of_the_body_is_not_repeated() {
    let mut with_title = draft();
    with_title.body = "# Coyote time\n\nTexte.".into();
    with_title.evidence.clear();
    assert!(render_note(&with_title).ends_with("# Coyote time\n\nTexte.\n"));
}

#[test]
fn set_field_replaces_or_adds() {
    let changed = set_field(NOTE, "status", "superseded");
    assert!(changed.contains("status: superseded\n"));
    assert!(!changed.contains("status: active"));
    let added = set_field(NOTE, "superseded_by", "\"[[Knowledge/x]]\"");
    assert!(added.contains("updated: 2026-01-01\nsuperseded_by: \"[[Knowledge/x]]\"\n---\n# Titre"));
    assert_eq!(set_field("sans frontmatter", "a", "b"), "sans frontmatter");
}

#[test]
fn remove_fields_drops_the_named_lines() {
    let stripped = remove_fields(NOTE, &["domain", "updated"]);
    assert!(!stripped.contains("domain:"));
    assert!(!stripped.contains("updated:"));
    assert!(stripped.contains("type: note"));
    assert!(stripped.ends_with("Suite.\n"));
}

#[test]
fn evidence_is_read_and_removed() {
    assert_eq!(evidence_of(NOTE), ["- 2026-01-01 run:a"]);
    let body = without_evidence(&body_of(NOTE));
    assert!(!body.contains("Evidence"));
    assert!(body.contains("## Autre\nSuite."));
}

#[test]
fn append_evidence_extends_the_section_in_place() {
    let changed = append_evidence(NOTE, "2026-02-02 run:b");
    assert!(changed.contains("- 2026-01-01 run:a\n- 2026-02-02 run:b\n\n## Autre"));
}

#[test]
fn append_evidence_creates_the_section_when_missing() {
    let changed = append_evidence("---\ntype: note\n---\n# T\n\nTexte.\n", "x");
    assert!(changed.ends_with("Texte.\n\n## Evidence\n- x\n"));
}

#[test]
fn replace_body_keeps_the_frontmatter() {
    let changed = replace_body(NOTE, "# Neuf\n");
    assert!(changed.starts_with("---\ntype: note\n"));
    assert!(changed.ends_with("---\n# Neuf\n"));
}
