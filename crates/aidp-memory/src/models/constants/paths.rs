/// Folder of the BIBLE under a project.
pub const BIBLE_DIR: &str = ".claude/BIBLE";
/// Folder of the design documents under a project.
pub const DOC_DIR: &str = "Docs";
/// Readme indexed with the documents.
pub const README_FILE: &str = "README.md";
/// Folder of the C# code under a project.
pub const CODE_DIR: &str = "Assets";
/// Graphify graph relative to a project.
pub const GRAPH_FILE: &str = "graphify-out/graph.json";
/// Registry of the daemon projects, under the gateway directory.
pub const PROJECTS_FILE: &str = "projects.json";
/// Folder of the memory database under the gateway directory.
pub const MEMORY_DIR: &str = "memory";
/// File name of the memory database.
pub const DB_FILE: &str = "memory.db";
/// File a pipeline stage writes its memory proposals into.
pub const PROPOSALS_FILE: &str = "memory-proposals.json";
/// Directories never entered while indexing.
pub const EXCLUDED_DIRS: &[&str] = &[
    "Library",
    "Temp",
    "Logs",
    "obj",
    "bin",
    ".git",
    "node_modules",
    "graphify-out",
    "UserSettings",
    ".state",
];
/// Extension of C# files.
pub const CSHARP_EXT: &str = "cs";
/// Extension of Markdown files.
pub const MARKDOWN_EXT: &str = "md";
/// Extensions read as plain text besides Markdown, for run artefacts.
pub const TEXT_EXTS: &[&str] = &["json", "txt", "yaml"];
/// Path separator stored in the index.
pub const SEPARATOR: char = '/';
/// Windows path separator folded into `SEPARATOR`.
pub const BACKSLASH: char = '\\';
