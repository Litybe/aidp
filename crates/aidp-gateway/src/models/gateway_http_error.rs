use std::fmt;

/// A daemon reply with a failure status; `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayHttpError {
    pub status: u16,
    pub message: String,
}

impl fmt::Display for GatewayHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GatewayHttpError {}
