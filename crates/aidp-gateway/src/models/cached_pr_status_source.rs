use crate::models::{PrStatusSource, TicketLocator};
use crate::prs::{handle_ticket_pr, PrStatusCache};
use serde_json::Value;
use std::sync::Arc;

/// A source that reads through the daemon's PR status cache.
pub struct CachedPrStatusSource {
    cache: Arc<PrStatusCache>,
    locate: TicketLocator,
}

impl CachedPrStatusSource {
    pub fn new(cache: Arc<PrStatusCache>, locate: TicketLocator) -> Self {
        CachedPrStatusSource { cache, locate }
    }
}

impl PrStatusSource for CachedPrStatusSource {
    fn refresh(&self, key: &str) -> Option<Value> {
        let (ticket, project_dir) = (self.locate)(key)?;
        handle_ticket_pr(&self.cache, &ticket, &ticket.key, &project_dir, true).ok()
    }

    fn status(&self, key: &str) -> Option<Value> {
        let (ticket, project_dir) = (self.locate)(key)?;
        handle_ticket_pr(&self.cache, &ticket, &ticket.key, &project_dir, false).ok()
    }
}
