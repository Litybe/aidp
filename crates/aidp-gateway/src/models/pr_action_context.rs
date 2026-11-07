use crate::bus::Bus;
use crate::prs::PrStatusCache;

/// What the daemon shares with every action: the status cache, the bus and the vault.
pub struct PrActionContext<'a> {
    pub cache: &'a PrStatusCache,
    pub bus: &'a Bus,
    pub vault_root: &'a std::path::Path,
}
