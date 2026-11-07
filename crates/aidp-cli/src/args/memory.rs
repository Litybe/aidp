//! The `aidp rag` and `aidp memory` subcommand trees.

use clap::Subcommand;
use std::path::PathBuf;

/// Project selector shared by every verb: a registered key or a directory.
#[derive(clap::Args, Clone)]
pub struct ProjectArg {
    /// Project key (registered in the daemon) or path; the current directory when omitted.
    #[arg(long, value_name = "KEY_OR_PATH")]
    pub project: Option<String>,
}

#[derive(Subcommand)]
pub enum RagOp {
    /// Index the project corpus (BIBLE, docs, C# code, runs, tickets, knowledge); incremental.
    Index {
        /// Project to index.
        #[command(flatten)]
        project: ProjectArg,
        /// Sources to refresh, comma separated: bible,doc,code,run,ticket,knowledge.
        #[arg(long, value_name = "LIST")]
        source: Option<String>,
        /// Re-chunk every file even when unchanged.
        #[arg(long)]
        full: bool,
    },
    /// Search the index (BM25 plus code graph) and pack excerpts under a token budget.
    Search {
        /// The question or keywords.
        #[arg(value_name = "QUERY")]
        query: String,
        /// Project to search.
        #[command(flatten)]
        project: ProjectArg,
        /// Restrict to sources, comma separated.
        #[arg(long, value_name = "LIST")]
        source: Option<String>,
        /// Restrict to paths matching a glob.
        #[arg(long, value_name = "GLOB")]
        path: Option<String>,
        /// Number of results before packing.
        #[arg(long, default_value_t = 20)]
        k: usize,
        /// Token budget of the packed excerpts (tokens are characters / 4).
        #[arg(long, default_value_t = 1500)]
        budget: usize,
    },
}

#[derive(Subcommand)]
pub enum MemoryOp {
    /// Print the Markdown block of active memories and excerpts for a stage.
    Context {
        /// Project of the memories.
        #[command(flatten)]
        project: ProjectArg,
        /// Issue whose mirrored ticket text drives the search, such as GH-214.
        #[arg(long, value_name = "GH-N")]
        issue: Option<String>,
        /// Kind of work: code, art, map or doc.
        #[arg(long)]
        route: Option<String>,
        /// Stage id, added to the query.
        #[arg(long)]
        stage: Option<String>,
        /// Token budget of the block.
        #[arg(long, default_value_t = 2000)]
        budget: usize,
    },
    /// Add a memory (proposed unless `--status active`).
    Add {
        /// Project of the memory.
        #[command(flatten)]
        project: ProjectArg,
        /// decision, convention, lesson, gotcha, preference or fact.
        #[arg(long)]
        kind: String,
        /// Short title, 120 characters at most.
        #[arg(long)]
        title: String,
        /// Actionable text, 600 characters at most.
        #[arg(long)]
        text: String,
        /// Route the memory applies to: code, art, map or doc.
        #[arg(long)]
        route: Option<String>,
        /// Path glob the memory applies to; repeatable.
        #[arg(long = "path", value_name = "GLOB")]
        paths: Vec<String>,
        /// Tag; repeatable.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Reference such as issue:GH-12 or pr:231; repeatable.
        #[arg(long = "ref", value_name = "TYPE:ID")]
        refs: Vec<String>,
        /// proposed or active.
        #[arg(long, default_value = "proposed")]
        status: String,
        /// Confidence between 0 and 1.
        #[arg(long, default_value_t = 0.7)]
        confidence: f64,
        /// Id of the memory this one replaces.
        #[arg(long, value_name = "ID")]
        supersedes: Option<String>,
    },
    /// List memories.
    List {
        /// Project of the memories.
        #[command(flatten)]
        project: ProjectArg,
        /// Keep one status: proposed, active, stale, archived or superseded.
        #[arg(long)]
        status: Option<String>,
        /// Keep one kind.
        #[arg(long)]
        kind: Option<String>,
        /// Keep memories whose title or text contains these words.
        #[arg(long, value_name = "WORDS")]
        q: Option<String>,
    },
    /// Show one memory.
    Show {
        /// Memory id such as M-12.
        #[arg(value_name = "ID")]
        id: String,
        /// Project of the memory.
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Edit a memory.
    Edit {
        /// Memory id such as M-12.
        #[arg(value_name = "ID")]
        id: String,
        /// Project of the memory.
        #[command(flatten)]
        project: ProjectArg,
        /// New kind.
        #[arg(long)]
        kind: Option<String>,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New text.
        #[arg(long)]
        text: Option<String>,
        /// New route.
        #[arg(long)]
        route: Option<String>,
        /// Replace the path globs; repeatable.
        #[arg(long = "path", value_name = "GLOB")]
        paths: Vec<String>,
        /// Replace the tags; repeatable.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// New confidence between 0 and 1.
        #[arg(long)]
        confidence: Option<f64>,
    },
    /// Activate a proposed or stale memory.
    Approve {
        /// Memory id such as M-12.
        #[arg(value_name = "ID")]
        id: String,
        /// Project of the memory.
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Archive a memory.
    Archive {
        /// Memory id such as M-12.
        #[arg(value_name = "ID")]
        id: String,
        /// Project of the memory.
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Fold a memory into another: evidence adds up, the source is superseded.
    Merge {
        /// Memory to fold.
        #[arg(value_name = "ID")]
        id: String,
        /// Memory that keeps the result.
        #[arg(long, value_name = "ID")]
        into: String,
        /// Project of the memories.
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Mark stale the active memories unused for 60 days.
    Gc {
        /// Project of the memories.
        #[command(flatten)]
        project: ProjectArg,
    },
    /// Ingest memory-proposals.json files from a stage folder or a file.
    Ingest {
        /// A proposals file, or a folder scanned for memory-proposals.json.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Project of the memories.
        #[command(flatten)]
        project: ProjectArg,
    },
}
