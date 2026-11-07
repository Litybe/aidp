/// How a fix session ended, with the tail of its transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOutcome {
    pub succeeded: bool,
    pub error: Option<String>,
    pub transcript_tail: String,
}
