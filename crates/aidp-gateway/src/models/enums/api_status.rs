use aidp_integrations::ErrorKind;

/// The status/code pair an HTTP reply carries for a failure. `Io` and
/// `Internal` share HTTP 500, and `Config` and `ShuttingDown` share 503 - the
/// wire `code` slug is what tells those apart, not the status alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStatus {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    PayloadTooLarge,
    Internal,
    Io,
    Upstream,
    Config,
    ShuttingDown,
    Error,
}

impl ApiStatus {
    /// The HTTP status code this variant answers with.
    pub const fn status(self) -> u16 {
        match self {
            ApiStatus::BadRequest => 400,
            ApiStatus::Unauthorized => 401,
            ApiStatus::Forbidden => 403,
            ApiStatus::NotFound => 404,
            ApiStatus::Conflict => 409,
            ApiStatus::PayloadTooLarge => 413,
            ApiStatus::Internal | ApiStatus::Io => 500,
            ApiStatus::Upstream => 502,
            ApiStatus::Config | ApiStatus::ShuttingDown => 503,
            ApiStatus::Error => 500,
        }
    }

    /// The stable wire slug for this variant, unchanged across the HTTP API.
    pub const fn code(self) -> &'static str {
        match self {
            ApiStatus::BadRequest => "bad_request",
            ApiStatus::Unauthorized => "unauthorized",
            ApiStatus::Forbidden => "forbidden",
            ApiStatus::NotFound => "not_found",
            ApiStatus::Conflict => "conflict",
            ApiStatus::PayloadTooLarge => "payload_too_large",
            ApiStatus::Internal => "internal",
            ApiStatus::Io => "io",
            ApiStatus::Upstream => "upstream",
            ApiStatus::Config => "config",
            ApiStatus::ShuttingDown => "shutting_down",
            ApiStatus::Error => "error",
        }
    }

    /// Recover the untyped status/code an [`ApiStatus`] this crate no longer
    /// emits directly (a raw HTTP status number) would have carried, for the
    /// handful of call sites that only have a bare status to hand.
    pub const fn from_status(status: u16) -> Self {
        match status {
            400 => ApiStatus::BadRequest,
            401 => ApiStatus::Unauthorized,
            403 => ApiStatus::Forbidden,
            404 => ApiStatus::NotFound,
            409 => ApiStatus::Conflict,
            413 => ApiStatus::PayloadTooLarge,
            500 => ApiStatus::Internal,
            502 => ApiStatus::Upstream,
            503 => ApiStatus::Config,
            _ => ApiStatus::Error,
        }
    }
}

impl From<ErrorKind> for ApiStatus {
    fn from(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::NotFound => ApiStatus::NotFound,
            ErrorKind::Conflict => ApiStatus::Conflict,
            ErrorKind::Unauthorized => ApiStatus::Unauthorized,
            ErrorKind::Config => ApiStatus::Config,
            ErrorKind::Upstream => ApiStatus::Upstream,
            ErrorKind::Io => ApiStatus::Io,
            ErrorKind::ShuttingDown => ApiStatus::ShuttingDown,
        }
    }
}
