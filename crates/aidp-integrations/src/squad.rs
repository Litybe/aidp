//! Squad pull request inbox: repositories, fetch, review state and lanes.
//! Pure classification lives here; caching and HTTP belong to the daemon.

pub mod bots;
pub mod config;
pub mod fetch;
pub mod lane;
pub mod repos;
pub mod review_state;

pub use bots::{accepts, is_bot};
pub use config::{load_config, load_config_from};
pub use fetch::{me_login, squad_prs, GhRunner, OsGhRunner};
pub use lane::{classify, mark_key};
pub use repos::squad_repos;
pub use review_state::my_review_state;

#[cfg(test)]
#[path = "../tests/unit/squad_test_support.rs"]
pub(crate) mod test_support;
