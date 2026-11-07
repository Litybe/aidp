use super::*;
use crate::models::enums::squad_degraded_kind::SquadDegradedKind;
use crate::models::squad_config::SquadConfig;
use aidp_core::clock::FixedClock;

use crate::squad::test_support::*;

const TEAM: &str = "acme/squad";

fn team_config() -> SquadConfig {
    SquadConfig {
        github_team: TEAM.to_string(),
        ..SquadConfig::default()
    }
}

#[test]
fn team_failure_is_degraded_with_a_read_org_hint_and_fallback_repos() {
    let dir = tempfile::tempdir().unwrap();
    let runner = FakeRunner::new(Box::new(|_| Err(FORBIDDEN.to_string())));
    let clock = FixedClock::from_rfc3339(NOW);
    let fallback = vec!["acme/fallback".to_string()];
    let cfg = team_config();
    let (repos, degraded) = squad_repos(&cfg, &fallback, dir.path(), &clock, &runner);
    assert_eq!(repos, fallback);
    assert_eq!(degraded.len(), 1);
    assert_eq!(degraded[0].kind, SquadDegradedKind::Team);
    assert!(degraded[0].message.contains("read:org"));
    assert!(degraded[0].message.contains("403"));
}

#[test]
fn team_failure_is_never_cached() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = team_config();
    let clock = FixedClock::from_rfc3339(NOW);
    let failing = FakeRunner::new(Box::new(|_| Err(FORBIDDEN.to_string())));
    squad_repos(&cfg, &[], dir.path(), &clock, &failing);
    let healthy = team_runner();
    let (repos, degraded) = squad_repos(&cfg, &[], dir.path(), &clock, &healthy);
    assert_eq!(healthy.call_count(), 1);
    assert!(degraded.is_empty());
    assert_eq!(repos.len(), 3);
}

#[test]
fn repos_cache_hit_within_a_day_does_not_call_the_runner() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = team_config();
    let clock = FixedClock::from_rfc3339(NOW);
    let first = team_runner();
    let (expected, _) = squad_repos(&cfg, &[], dir.path(), &clock, &first);
    assert_eq!(first.call_count(), 1);
    let second = team_runner();
    let (cached, degraded) = squad_repos(&cfg, &[], dir.path(), &clock, &second);
    assert_eq!(second.call_count(), 0);
    assert_eq!(cached, expected);
    assert!(degraded.is_empty());
}

#[test]
fn repos_cache_older_than_a_day_is_refetched() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = team_config();
    let first_clock = FixedClock::from_rfc3339(NOW);
    squad_repos(&cfg, &[], dir.path(), &first_clock, &team_runner());
    let later_clock = FixedClock::from_rfc3339(TWO_DAYS_LATER);
    let second = team_runner();
    squad_repos(&cfg, &[], dir.path(), &later_clock, &second);
    assert_eq!(second.call_count(), 1);
}

#[test]
fn extra_repos_are_added_and_excluded_repos_removed() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = SquadConfig {
        extra_repos: vec!["acme/extra".to_string()],
        exclude_repos: vec!["acme/team-repo-b".to_string()],
        ..team_config()
    };
    let clock = FixedClock::from_rfc3339(NOW);
    let (repos, _) = squad_repos(&cfg, &[], dir.path(), &clock, &team_runner());
    assert_eq!(
        repos,
        vec![
            "acme/extra".to_string(),
            "acme/team-repo-a".to_string(),
            "acme/team-repo-c".to_string()
        ]
    );
}

#[test]
fn without_a_team_the_fallback_repos_are_used_without_a_gh_call() {
    let dir = tempfile::tempdir().unwrap();
    let runner = team_runner();
    let clock = FixedClock::from_rfc3339(NOW);
    let fallback = vec!["acme/fallback".to_string()];
    let (repos, degraded) = squad_repos(
        &SquadConfig::default(),
        &fallback,
        dir.path(),
        &clock,
        &runner,
    );
    assert_eq!(repos, fallback);
    assert!(degraded.is_empty());
    assert_eq!(runner.call_count(), 0);
}
