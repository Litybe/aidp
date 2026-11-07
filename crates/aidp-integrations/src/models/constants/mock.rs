//! Status names, ids, messages and encoding tables of the local mock server.

pub const INTERNAL_ROUTE_PREFIX: &str = "__";
pub const DEFAULT_FAILURE_STATUS: u16 = 503;
pub const FILE_TYPE: &str = "file";
pub const BASE64_ENCODING: &str = "base64";
pub const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub const KEY_PLACEHOLDER: &str = "{key}";
pub const UNAUTHORIZED_MESSAGE: &str = "Unauthorized: missing Authorization header";
pub const INJECTED_FAILURE_MESSAGE: &str = "injected failure";
pub const NOT_FOUND_MESSAGE: &str = "Not Found";
