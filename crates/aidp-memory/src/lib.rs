//! `aidp-memory`: the RAG index and the project memory manager.
//!
//! One SQLite database (`<gateway>/memory/memory.db`, FTS5 with BM25) holds
//! the chunks of the project corpus and the memories. Public entry points:
//! [`MemoryStore`], [`Indexer`], [`Retriever`] ([`Bm25Retriever`]) and
//! [`MemoryManager`]. This crate reads no environment variable and parses no
//! arguments; the caller passes the gateway directory and the project.

pub mod chunking;
pub mod context;
pub mod distill;
pub mod error;
pub mod eval;
pub mod graph;
pub mod hook_context;
pub mod indexer;
pub mod manager;
pub mod mmr;
pub mod models;
pub mod pack;
pub mod project;
mod proposal_shape;
pub mod proposals;
pub mod retriever;
pub mod session_journal;
pub mod store;
pub mod text;
pub mod vault_notes;

pub use error::{MemoryError, Result};
pub use indexer::Indexer;
pub use manager::MemoryManager;
pub use models::add_outcome::AddOutcome;
pub use models::context_block::ContextBlock;
pub use models::context_request::ContextRequest;
pub use models::enums::{MemoryKind, MemoryStatus, RefType, Route, Source};
pub use models::hit::Hit;
pub use models::index_options::IndexOptions;
pub use models::index_report::IndexReport;
pub use models::index_root::IndexRoot;
pub use models::memory::Memory;
pub use models::new_memory::NewMemory;
pub use models::packed::Packed;
pub use models::project_ref::ProjectRef;
pub use models::search_query::SearchQuery;
pub use pack::{pack, render_markdown};
pub use project::{is_registered, resolve_project};
pub use retriever::{Bm25Retriever, Retriever};
pub use store::{db_path, MemoryStore};

#[cfg(test)]
#[path = "../tests/unit/support.rs"]
mod test_support;
