use super::*;

fn numbered(text: &str) -> Vec<(usize, &str)> {
    text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect()
}

#[test]
fn small_input_is_one_piece_with_trimmed_blank_edges() {
    let pieces = pack_lines(&numbered("\nalpha\nbeta\n\n"), 100, 0);
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].line_start, 2);
    assert_eq!(pieces[0].line_end, 3);
    assert_eq!(pieces[0].text, "alpha\nbeta");
}

#[test]
fn long_input_splits_with_overlap() {
    let text = (1..=10)
        .map(|n| format!("line number {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let pieces = pack_lines(&numbered(&text), 50, 20);
    assert!(pieces.len() > 2);
    for pair in pieces.windows(2) {
        assert!(pair[1].line_start <= pair[0].line_end);
        assert!(pair[1].line_end > pair[0].line_end);
    }
    assert!(pieces.iter().all(|p| p.text.chars().count() <= 50));
}

#[test]
fn oversized_line_is_cut() {
    let line = "x".repeat(250);
    let pieces = pack_lines(&[(7, line.as_str())], 100, 0);
    assert_eq!(pieces.len(), 3);
    assert!(pieces.iter().all(|p| p.line_start == 7));
}
