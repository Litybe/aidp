use crate::embedder::Embedder;
use std::sync::Arc;

/// An embedder shared between threads and retrievers.
#[derive(Clone)]
pub struct SharedEmbedder(pub Arc<dyn Embedder>);
