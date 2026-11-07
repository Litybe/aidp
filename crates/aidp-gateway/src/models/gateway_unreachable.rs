use std::fmt;

/// The daemon could not be reached; `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayUnreachable {
    pub message: String,
}

impl fmt::Display for GatewayUnreachable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GatewayUnreachable {}
