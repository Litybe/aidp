//! The squad's repositories: the GitHub team's, plus config extras, minus exclusions.

use crate::models::constants::squad::{
    GH_API, GH_JQ_FLAG, GH_PAGINATE_FLAG, GH_TEAM_REPOS_JQ, MESSAGE_TEAM_FAILED,
    MESSAGE_TEAM_INVALID, REPOS_CACHE_FILE, REPOS_CACHE_TTL_MS, SLUG_SEPARATOR, TEAM_PATH_ORGS,
    TEAM_PATH_REPOS, TEAM_PATH_TEAMS,
};
use crate::models::enums::squad_degraded_kind::SquadDegradedKind;
use crate::models::squad_config::SquadConfig;
use crate::models::squad_degraded::SquadDegraded;
use crate::models::squad_repos_cache::SquadReposCache;
use crate::squad::fetch::GhRunner;
use aidp_core::clock::Clock;
use std::collections::BTreeSet;
use std::path::Path;

fn read_cache(cache_dir: &Path, cfg: &SquadConfig, now_ms: i64) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(cache_dir.join(REPOS_CACHE_FILE)).ok()?;
    let cache: SquadReposCache = serde_json::from_str(&text).ok()?;
    let fresh = now_ms - cache.fetched_at_ms < REPOS_CACHE_TTL_MS;
    (cache.github_team == cfg.github_team && fresh).then_some(cache.repos)
}

fn write_cache(cache_dir: &Path, cfg: &SquadConfig, now_ms: i64, repos: &[String]) {
    let cache = SquadReposCache {
        github_team: cfg.github_team.clone(),
        fetched_at_ms: now_ms,
        repos: repos.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = std::fs::create_dir_all(cache_dir);
        let _ = std::fs::write(cache_dir.join(REPOS_CACHE_FILE), text);
    }
}

fn fetch_team_repos(cfg: &SquadConfig, runner: &dyn GhRunner) -> Result<Vec<String>, String> {
    let (org, slug) = cfg
        .github_team
        .split_once(SLUG_SEPARATOR)
        .filter(|(org, slug)| !org.is_empty() && !slug.is_empty())
        .ok_or_else(|| MESSAGE_TEAM_INVALID.to_string())?;
    let path = format!("{TEAM_PATH_ORGS}/{org}/{TEAM_PATH_TEAMS}/{slug}/{TEAM_PATH_REPOS}");
    let args = [
        GH_API,
        &path,
        GH_PAGINATE_FLAG,
        GH_JQ_FLAG,
        GH_TEAM_REPOS_JQ,
    ]
    .map(str::to_string);
    let text = runner.run(&args)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Team repos from the 24h cache, else from GitHub; a failure is never cached.
fn team_repos(
    cfg: &SquadConfig,
    cache_dir: &Path,
    clock: &dyn Clock,
    runner: &dyn GhRunner,
) -> Result<Vec<String>, String> {
    let now_ms = clock.now_ms();
    if let Some(cached) = read_cache(cache_dir, cfg, now_ms) {
        return Ok(cached);
    }
    let fetched = fetch_team_repos(cfg, runner)?;
    write_cache(cache_dir, cfg, now_ms, &fetched);
    Ok(fetched)
}

/// Sorted, deduplicated repos of the squad. Without a team, or when it cannot be read, the
/// extras and `fallback_repos` stand in; a read failure also says in `degraded` how to fix it.
pub fn squad_repos(
    cfg: &SquadConfig,
    fallback_repos: &[String],
    cache_dir: &Path,
    clock: &dyn Clock,
    runner: &dyn GhRunner,
) -> (Vec<String>, Vec<SquadDegraded>) {
    let mut degraded = Vec::new();
    let mut repos: BTreeSet<String> = cfg.extra_repos.iter().cloned().collect();
    if cfg.github_team.is_empty() {
        repos.extend(fallback_repos.iter().cloned());
        for excluded in &cfg.exclude_repos {
            repos.remove(excluded);
        }
        return (repos.into_iter().collect(), degraded);
    }
    match team_repos(cfg, cache_dir, clock, runner) {
        Ok(team) => repos.extend(team),
        Err(error) => {
            repos.extend(fallback_repos.iter().cloned());
            degraded.push(SquadDegraded {
                kind: SquadDegradedKind::Team,
                repos: Vec::new(),
                message: format!("{MESSAGE_TEAM_FAILED}: {}", error.trim()),
            });
        }
    }
    for excluded in &cfg.exclude_repos {
        repos.remove(excluded);
    }
    (repos.into_iter().collect(), degraded)
}

#[cfg(test)]
#[path = "../../tests/unit/repos_tests.rs"]
mod tests;
