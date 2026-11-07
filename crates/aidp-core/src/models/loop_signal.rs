use crate::models::enums::loop_producer::LoopProducer;

/// A parsed loop condition: the expression text, its signal property and its producer state.
pub(crate) struct LoopSignal {
    pub(crate) condition: Option<String>,
    pub(crate) property: Option<String>,
    pub(crate) producer: LoopProducer,
}
