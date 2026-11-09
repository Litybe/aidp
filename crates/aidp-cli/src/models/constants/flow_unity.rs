//! Names of the `flow unity-refresh` verb: the file bridge of the playtest package (contract C1), its
//! waits, and the outputs the flow reads.

pub const COMPILE_REQUESTS_DIR: [&str; 3] = [".aidp", "compile", "requests"];
pub const COMPILE_RESULTS_DIR: [&str; 3] = [".aidp", "compile", "results"];
pub const HEARTBEAT_FILE: [&str; 3] = [".aidp", "editor", "heartbeat.json"];
pub const JSON_EXTENSION: &str = "json";
pub const TEMP_EXTENSION: &str = "tmp";
pub const REQUEST_ID_PREFIX: &str = "refresh-";
pub const EMPTY_REQUEST_BODY: &str = "{}";

pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
pub const POLL_INTERVAL_MILLIS: u64 = 2000;
/// A heartbeat older than this means the editor's main thread is stuck. The reimport of a branch
/// silences the heartbeat too: 135 s observed on PixelHood, so this leaves it a wide margin.
pub const HEARTBEAT_STALE_SECONDS: i64 = 300;
/// A domain reload can keep the heartbeat silent this long before the editor counts as stuck.
pub const HEARTBEAT_RELOAD_STALE_SECONDS: i64 = 300;

pub const RESULT_FIELD_OK: &str = "ok";
pub const RESULT_FIELD_ERRORS: &str = "errors";
pub const RESULT_FIELD_WARNINGS: &str = "warnings";
pub const RESULT_FIELD_DURATION_MS: &str = "durationMs";
pub const HEARTBEAT_FIELD_AT: &str = "at";
pub const HEARTBEAT_FIELD_STATE: &str = "state";
pub const HEARTBEAT_STATE_RELOADING: &str = "reloading";

pub const REASON_NOT_UNITY_PROJECT: &str = "not-a-unity-project";
pub const REASON_EDITOR_CLOSED: &str = "unity-editor-closed";
pub const BLOCKED_UNITY_EDITOR: &str = "blocked: unity-editor";
