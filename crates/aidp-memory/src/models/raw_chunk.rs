/// A passage cut from one file, before it gets an id and a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    pub title: String,
    pub text: String,
    pub line_start: usize,
    pub line_end: usize,
    /// `section`, `type`, `members`, `method` or `text`.
    pub kind: &'static str,
    /// Qualified symbol name for code chunks.
    pub symbol: Option<String>,
}
