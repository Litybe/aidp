/// The producer state of a loop's `when:` condition.
pub(crate) enum LoopProducer {
    None,
    Source(String),
    Found { stage_ref: String, producer: String },
}
