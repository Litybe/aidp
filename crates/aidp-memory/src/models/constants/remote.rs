//! How a process other than the daemon reaches the single embedding model.

/// Environment variable holding the daemon base URL.
pub const ENV_GATEWAY_URL: &str = "AIDP_GATEWAY_URL";
/// Environment variable holding the daemon UI token.
pub const ENV_UI_TOKEN: &str = "AIDP_UI_TOKEN";
/// Daemon address when the environment names none.
pub const DEFAULT_GATEWAY_URL: &str = "http://127.0.0.1:3001";
/// Route of the model status.
pub const MODEL_PATH: &str = "/api/memory/model";
/// Route that embeds texts with the running model.
pub const EMBED_PATH: &str = "/api/embed";
/// Longest wait to open the connection to the daemon, in milliseconds.
pub const CONNECT_TIMEOUT_MS: u64 = 300;
/// Longest wait for the status probe, in milliseconds.
pub const PROBE_TIMEOUT_MS: u64 = 500;
/// Longest wait for one embedding batch, in seconds.
pub const EMBED_READ_TIMEOUT_SECS: u64 = 10;
/// Longest wait for one query embedding in a hook, in milliseconds.
pub const HOOK_EMBED_TIMEOUT_MS: u64 = 500;
/// Texts one call of `POST /api/embed` accepts.
pub const EMBED_MAX_TEXTS: usize = 256;
/// Authorization scheme of the UI token.
pub const BEARER_PREFIX: &str = "Bearer ";
