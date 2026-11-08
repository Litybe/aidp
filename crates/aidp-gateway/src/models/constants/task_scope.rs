//! Git arguments and status markers used by the per-task write attribution.

pub const STATUS_ARG: &str = "status";
pub const PORCELAIN_FLAG: &str = "--porcelain";
pub const NUL_SEPARATED_FLAG: &str = "-z";
pub const UNTRACKED_ALL_FLAG: &str = "-uall";
pub const REV_PARSE_ARG: &str = "rev-parse";
pub const HEAD_REF: &str = "HEAD";
pub const STATUS_CODE_WIDTH: usize = 2;
pub const STATUS_PREFIX_WIDTH: usize = 3;
pub const RENAME_MARKER: char = 'R';
pub const COPY_MARKER: char = 'C';
pub const NUL_SEPARATOR: char = '\u{0}';

/// Marker file whose presence makes a workspace a Unity project.
pub const UNITY_PROJECT_MARKER: &str = "ProjectSettings/ProjectVersion.txt";

/// Files the Unity editor rewrites on its own (package resolution, imports, compilation).
/// In a Unity project the scope guards ignore them: never reported, never restored, because an
/// open editor would rewrite them at once. An entry ending in `/` is a folder, one starting with
/// `*` is a file suffix at any depth, any other entry is an exact path.
pub const UNITY_EDITOR_OWNED: &[&str] = &[
    "Packages/packages-lock.json",
    "ProjectSettings/",
    "UserSettings/",
    "Library/",
    "Temp/",
    "Logs/",
    "obj/",
    "*.csproj",
    "*.sln",
];
