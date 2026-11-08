/// UTF-8 byte order mark that Windows PowerShell 5.1 prefixes to the files it writes.
const UTF8_BOM: char = '\u{FEFF}';

/// `text` without a leading UTF-8 BOM; unchanged when there is none.
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix(UTF8_BOM).unwrap_or(text)
}

/// Reads a text file written by an agent, dropping a leading UTF-8 BOM.
pub fn read_text_lenient(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let mut text = std::fs::read_to_string(path)?;
    if text.starts_with(UTF8_BOM) {
        text.drain(..UTF8_BOM.len_utf8());
    }
    Ok(text)
}

/// Levenshtein edit distance over Unicode scalar values.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

#[cfg(test)]
#[path = "../tests/unit/text_tests.rs"]
mod tests;
