use crate::bus::Bus;
use crate::pr_drafts::PrDrafts;
use crate::pr_viewed::PrViewed;
use crate::prs::PrStatusCache;
use crate::squad_prs::SquadPrsCache;
use aidp_core::process_runner::ProcessRunner;
use std::path::Path;

/// What the PR page routes share: caches, bus, local stores and the `gh` process runner.
pub struct PrDetailContext<'a> {
    pub cache: &'a PrStatusCache,
    pub bus: &'a Bus,
    pub squad: &'a SquadPrsCache,
    pub drafts: &'a PrDrafts,
    pub viewed: &'a PrViewed,
    pub runner: &'a dyn ProcessRunner,
    pub workspace_root: Option<&'a Path>,
}
