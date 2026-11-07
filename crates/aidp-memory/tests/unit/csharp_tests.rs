use super::*;

const SOURCE: &str = r#"using UnityEngine;

namespace Game.Player
{
    /// <summary>Moves the hero.</summary>
    public class PlayerMovement : MonoBehaviour
    {
        [SerializeField] private float speed = 5f;
        public int Health { get; set; }

        private void Update()
        {
            if (Input.anyKey)
            {
                Move("}");
            }
        }

        public void Move(string label) => Debug.Log(label);

        private static readonly int[] Table = { 1, 2, 3 };

        public class Inner
        {
            public void Ping() { }
        }
    }
}
"#;

fn find<'a>(chunks: &'a [RawChunk], title: &str) -> &'a RawChunk {
    chunks.iter().find(|c| c.title == title).unwrap_or_else(|| {
        panic!(
            "missing {title}: {:?}",
            chunks.iter().map(|c| &c.title).collect::<Vec<_>>()
        )
    })
}

#[test]
fn methods_are_chunks_with_exact_lines() {
    let chunks = chunk_csharp(SOURCE, "PlayerMovement");
    let update = find(&chunks, "PlayerMovement.Update");
    assert_eq!((update.line_start, update.line_end), (11, 17));
    assert!(update.text.contains("Move(\"}\")"));
    let arrow = find(&chunks, "PlayerMovement.Move");
    assert_eq!((arrow.line_start, arrow.line_end), (19, 19));
}

#[test]
fn nested_types_get_qualified_names() {
    let chunks = chunk_csharp(SOURCE, "PlayerMovement");
    let ping = find(&chunks, "PlayerMovement.Inner.Ping");
    assert_eq!(ping.kind, "method");
}

#[test]
fn type_chunk_keeps_doc_comment_and_fields_without_methods() {
    let chunks = chunk_csharp(SOURCE, "PlayerMovement");
    let head = find(&chunks, "PlayerMovement");
    assert_eq!(head.line_start, 5);
    assert!(head.text.contains("<summary>"));
    assert!(head.text.contains("speed"));
    assert!(!head.text.contains("Debug.Log"));
}

#[test]
fn members_after_methods_form_their_own_chunk() {
    let chunks = chunk_csharp(SOURCE, "PlayerMovement");
    let members = find(&chunks, "PlayerMovement (members)");
    assert!(members.text.contains("Table"));
}

#[test]
fn file_without_type_falls_back_to_text_chunks() {
    let chunks = chunk_csharp("// nothing here\nint x = 1;\n", "Loose");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].kind, "text");
    assert_eq!(chunks[0].title, "Loose");
}

#[test]
fn enum_is_one_type_chunk() {
    let chunks = chunk_csharp("public enum Mode\n{\n    A,\n    B\n}\n", "Mode");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].title, "Mode");
    assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 5));
}
