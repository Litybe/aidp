use crate::models::constants::review_post::{
    MARKER_ASSIGN, MARKER_CLOSE, MARKER_ID_KEY, MARKER_OPEN, MARKER_PASS_KEY, MARKER_RUN_KEY,
};

/// Hidden HTML comment tying a posted comment to a finding, a run and a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingMarker {
    pub id: String,
    pub run: String,
    pub pass: u32,
}

impl FindingMarker {
    /// The marker as it is written at the head of a comment body.
    pub fn render(&self) -> String {
        format!(
            "{MARKER_OPEN} {MARKER_ID_KEY}{MARKER_ASSIGN}{} {MARKER_RUN_KEY}{MARKER_ASSIGN}{} {MARKER_PASS_KEY}{MARKER_ASSIGN}{} {MARKER_CLOSE}",
            self.id, self.run, self.pass
        )
    }

    /// Every well-formed marker found in `text`, in order.
    pub fn parse_all(text: &str) -> Vec<Self> {
        text.split(MARKER_OPEN)
            .skip(1)
            .filter_map(|tail| tail.split_once(MARKER_CLOSE))
            .filter_map(|(inner, _)| Self::from_fields(inner))
            .collect()
    }

    /// The first well-formed marker of `text`.
    pub fn parse(text: &str) -> Option<Self> {
        Self::parse_all(text).into_iter().next()
    }

    fn from_fields(inner: &str) -> Option<Self> {
        let field = |key: &str| {
            inner.split_whitespace().find_map(|token| {
                let (name, value) = token.split_once(MARKER_ASSIGN)?;
                (name == key).then(|| value.to_string())
            })
        };
        Some(Self {
            id: field(MARKER_ID_KEY)?,
            run: field(MARKER_RUN_KEY)?,
            pass: field(MARKER_PASS_KEY)?.parse().ok()?,
        })
    }
}
