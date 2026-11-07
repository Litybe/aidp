use aidp_tickets::model::Ticket;
use std::path::PathBuf;
use std::sync::Arc;

/// Resolves a key to its ticket and project directory.
pub type TicketLocator = Arc<dyn Fn(&str) -> Option<(Ticket, PathBuf)> + Send + Sync>;
