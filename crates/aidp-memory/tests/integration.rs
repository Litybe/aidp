//! Integration tests, linked into one binary: one test binary per file cost
//! a full link of the crate's dependency graph each.
mod hybrid_flow;
mod knowledge_flow;
mod rag_flow;
mod stage_context;
