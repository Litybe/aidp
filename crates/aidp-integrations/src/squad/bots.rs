//! Recognising bot authors and filtering a listing on them.

use crate::models::constants::squad::BOT_LOGIN_SUFFIX;
use crate::models::enums::bots_filter::BotsFilter;
use crate::models::squad_config::SquadConfig;
use crate::models::squad_pr::SquadPr;

/// True for a configured bot author or any login ending in `[bot]`.
pub fn is_bot(author: &str, cfg: &SquadConfig) -> bool {
    let login = author.to_lowercase();
    login.ends_with(BOT_LOGIN_SUFFIX)
        || cfg
            .bot_authors
            .iter()
            .any(|bot| bot.to_lowercase() == login)
}

/// Whether `filter` keeps `pr` given who authored it.
pub fn accepts(filter: BotsFilter, pr: &SquadPr, cfg: &SquadConfig) -> bool {
    match filter {
        BotsFilter::Show => true,
        BotsFilter::Hide => !is_bot(&pr.author, cfg),
        BotsFilter::Only => is_bot(&pr.author, cfg),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/bots_tests.rs"]
mod tests;
