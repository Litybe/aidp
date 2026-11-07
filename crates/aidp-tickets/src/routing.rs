//! Routing a GitHub issue (or an existing local `Ticket`) to a route, per
//! `TODO/routing.yaml`'s `rules:` table (lot 8a, vision §5.3). See
//! [`crate::config::Routing`] for the file format and the load-time
//! tolerance contract; this module only defines the facts a rule matches
//! against, the rule shape, and [`route`] itself.
//!
//! Deliberately vault- and tracker-agnostic: [`RouteFacts`] is a plain data
//! struct built either from a `GithubIssue` (by the GitHub sync) or from an
//! existing `Ticket`'s frontmatter (by `aidp ticket route --from-unrouted`).
//! This module must not depend on the adapter crate.
//!
//! For a local ticket the winning rule's `service` is the service itself.
//! For a GitHub mirror it names the route: the mirror stays filed under its
//! repository's service, and the route's `repos:` entry gives the
//! template, the flow and the track.

use crate::config::Routing;
use crate::models::track::Track;
use serde::{Deserialize, Serialize};

/// The facts a routing rule matches against. Built by the caller (the GitHub
/// sync, or `aidp ticket route --from-unrouted` reading a `Ticket`'s
/// frontmatter) — this module never constructs one itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteFacts {
    pub key: String,
    pub summary: String,
    pub issuetype: Option<String>,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub epic: Option<String>,
    /// Issue body, searched by the `keywords` criterion with the summary.
    pub body: String,
}

/// The criteria of one rule's `match:`. Every field is optional; a criterion
/// that is `None` is not checked (vacuously true). All present criteria are
/// ANDed together by [`route`]. `summary_regex` is kept as a plain `String`
/// (not a compiled `Regex`) so `Rule`/`Routing` can keep deriving
/// `PartialEq`/`Serialize`/`Deserialize`; it is compiled on demand by
/// [`route`] and at validation time by [`compile_rules`].
///
/// `label` and every entry of `labels` compare case-insensitively and accept
/// a trailing `*` as a prefix glob (`pixel:*`); `labels` holds when any entry
/// matches. `keywords` holds when any keyword appears as a whole word,
/// case-insensitively, in the summary or the body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuleCriteria {
    pub component: Option<String>,
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub epic: Option<String>,
    pub key_prefix: Option<String>,
    pub summary_regex: Option<String>,
    pub issuetype: Option<String>,
}

impl RuleCriteria {
    /// True when no criterion is set at all — such a `match: {}` must never
    /// win a route (see [`route`]).
    fn is_empty(&self) -> bool {
        self.component.is_none()
            && self.label.is_none()
            && self.labels.is_empty()
            && self.keywords.is_empty()
            && self.epic.is_none()
            && self.key_prefix.is_none()
            && self.summary_regex.is_none()
            && self.issuetype.is_none()
    }
}

/// One entry of `routing.yaml`'s `rules:` list. The YAML key is `match`, a
/// Rust keyword, hence the raw identifier `r#match`. `route:` is accepted as
/// a synonym of `service:`, the clearer word for a GitHub mirror.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub r#match: RuleCriteria,
    #[serde(alias = "route")]
    pub service: String,
}

/// The outcome of a successful [`route`]: the matched rule's `service`, plus
/// `template`/`track` resolved from `routing.repos[service]` (falling back to
/// `routing.defaults.track` for `track`). `template` is `None` when the
/// service has no `repos:` entry, or the entry sets no `template:` — the
/// caller then creates the ticket with `template: null`, same as
/// `Routing::template_for` today.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub service: String,
    pub template: Option<String>,
    pub track: Option<Track>,
    /// Delivery flow of the route's `repos:` entry.
    pub flow: Option<String>,
    /// Route family of the `repos:` entry (`code`, `art`, `map`, `doc`), else the rule's `service`.
    pub route: String,
}

/// Does `pattern` (exact, or a prefix ending in `*`) match `label`, ignoring case?
pub fn label_matches(pattern: &str, label: &str) -> bool {
    let pattern = pattern.trim();
    match pattern.strip_suffix('*') {
        Some(prefix) => label
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase()),
        None => label.eq_ignore_ascii_case(pattern),
    }
}

/// Does `keyword` appear as a whole word in `text`, ignoring case?
fn has_word(text: &str, keyword: &str) -> bool {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return false;
    }
    let pattern = format!(r"(?i)\b{}\b", regex::escape(keyword));
    regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(text))
}

/// Does `criteria` match `facts`? Every present criterion must hold (AND); a
/// criterion absent from `criteria` never blocks a match. An entirely empty
/// `criteria` (no criterion set) never matches — callers must reject that
/// case themselves via [`RuleCriteria::is_empty`] before calling this, which
/// `route` and `compile_rules`'s tests both do.
fn criteria_match(criteria: &RuleCriteria, facts: &RouteFacts) -> bool {
    if let Some(component) = &criteria.component {
        if !facts.components.iter().any(|c| c == component) {
            return false;
        }
    }
    if let Some(label) = &criteria.label {
        if !facts.labels.iter().any(|l| label_matches(label, l)) {
            return false;
        }
    }
    if !criteria.labels.is_empty()
        && !criteria
            .labels
            .iter()
            .any(|pattern| facts.labels.iter().any(|l| label_matches(pattern, l)))
    {
        return false;
    }
    if !criteria.keywords.is_empty()
        && !criteria
            .keywords
            .iter()
            .any(|keyword| has_word(&facts.summary, keyword) || has_word(&facts.body, keyword))
    {
        return false;
    }
    if let Some(epic) = &criteria.epic {
        if facts.epic.as_deref() != Some(epic.as_str()) {
            return false;
        }
    }
    if let Some(prefix) = &criteria.key_prefix {
        let wanted = format!("{prefix}-");
        if !facts.key.starts_with(&wanted) {
            return false;
        }
    }
    if let Some(pattern) = &criteria.summary_regex {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if !re.is_match(&facts.summary) {
                    return false;
                }
            }
            // A rule surviving compile_rules always has a valid regex; this
            // arm only guards against a future caller building a Rule by
            // hand. Never panic: treat an uncompilable pattern as "does not
            // match" rather than crashing the sync.
            Err(_) => return false,
        }
    }
    if let Some(issuetype) = &criteria.issuetype {
        match &facts.issuetype {
            Some(actual) => {
                if !actual.eq_ignore_ascii_case(issuetype) {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

/// Walk `routing.rules` in order and return the first rule whose `match`
/// criteria all hold against `facts` (AND between criteria; first rule wins
/// between rules). A rule with an empty `match: {}` is skipped — it would
/// otherwise swallow every issue.
///
/// On a match, `template`/`track` are resolved from `routing.repos[service]`
/// (`track` falling back to `routing.defaults.track` when the repo entry sets
/// none). When the rule's `service` has no `repos:` entry at all, a `Route`
/// is still returned (`template: None`) and one warning is printed on
/// stderr naming the rule's service — this is the documented behaviour, not
/// an error condition.
pub fn route(facts: &RouteFacts, routing: &Routing) -> Option<Route> {
    for rule in &routing.rules {
        if rule.r#match.is_empty() {
            continue;
        }
        if !criteria_match(&rule.r#match, facts) {
            continue;
        }
        let repo = routing.repos.get(&rule.service);
        if repo.is_none() {
            tracing::warn!(
                "rule matching service '{}' has no repos: entry; \
                 routing with template: null",
                rule.service
            );
        }
        let template = repo.and_then(|r| r.template.clone());
        let track = repo.and_then(|r| r.track).or(routing.defaults.track);
        let flow = repo.and_then(|r| r.flow.clone());
        let family = repo
            .and_then(|r| r.route.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| rule.service.clone());
        return Some(Route {
            service: rule.service.clone(),
            template,
            track,
            flow,
            route: family,
        });
    }
    None
}

/// Why `pattern` is not a valid `summary_regex`, `None` when it compiles.
pub fn summary_regex_error(pattern: &str) -> Option<String> {
    regex::Regex::new(pattern)
        .err()
        .map(|error| error.to_string())
}

/// Tolerantly compile the raw `rules:` YAML value into a list of [`Rule`]s.
/// Never fails: an absent or non-sequence `rules:` yields an empty list (with
/// a warning for the latter); each sequence entry is validated on its own —
/// a malformed entry, an unknown criterion key, or an uncompilable
/// `summary_regex` drops *that entry only*, with a warning naming its index
/// and the reason. This is what keeps a bad `rules:` section from ever
/// costing the file its `repos:` table (see [`crate::config::load_routing`]).
pub(crate) fn compile_rules(raw: Option<&serde_yaml::Value>) -> Vec<Rule> {
    let Some(value) = raw else {
        return Vec::new();
    };
    let Some(seq) = value.as_sequence() else {
        tracing::warn!("'rules' is not a list; ignoring it (routing.repos is kept)");
        return Vec::new();
    };

    let mut rules = Vec::with_capacity(seq.len());
    for (idx, entry) in seq.iter().enumerate() {
        match serde_yaml::from_value::<Rule>(entry.clone()) {
            Ok(rule) => {
                if let Some(pattern) = &rule.r#match.summary_regex {
                    if let Err(e) = regex::Regex::new(pattern) {
                        tracing::warn!(
                            "rules[{idx}]: invalid summary_regex \
                             '{pattern}': {e}; dropping this rule"
                        );
                        continue;
                    }
                }
                rules.push(rule);
            }
            Err(e) => {
                tracing::warn!("rules[{idx}]: malformed rule: {e}; dropping it");
            }
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Routing, RoutingDefaults};
    use std::collections::BTreeMap;

    fn facts() -> RouteFacts {
        RouteFacts {
            key: "PIXEL-1".to_string(),
            summary: "Cheat alert on lambda pipeline".to_string(),
            issuetype: Some("Bug".to_string()),
            labels: vec!["aidp-viewer".to_string()],
            components: vec!["cheat-alerts-service".to_string()],
            epic: Some("PIXEL-5000".to_string()),
            body: String::new(),
        }
    }

    fn routing_with_rules(rules: Vec<Rule>) -> Routing {
        Routing {
            defaults: RoutingDefaults::default(),
            rules,
            repos: BTreeMap::new(),
        }
    }

    #[test]
    fn first_matching_rule_wins() {
        let rules = vec![
            Rule {
                r#match: RuleCriteria {
                    component: Some("cheat-alerts-service".to_string()),
                    ..Default::default()
                },
                service: "cheat-alerts-service".to_string(),
            },
            Rule {
                r#match: RuleCriteria {
                    label: Some("aidp-viewer".to_string()),
                    ..Default::default()
                },
                service: "aidp-viewer".to_string(),
            },
        ];
        let routing = routing_with_rules(rules);

        let route = route(&facts(), &routing).expect("a rule matches");

        assert_eq!(route.service, "cheat-alerts-service");
    }

    #[test]
    fn and_semantics_require_every_criterion() {
        let rules = vec![Rule {
            r#match: RuleCriteria {
                component: Some("cheat-alerts-service".to_string()),
                label: Some("does-not-exist".to_string()),
                ..Default::default()
            },
            service: "cheat-alerts-service".to_string(),
        }];
        let routing = routing_with_rules(rules);

        assert_eq!(route(&facts(), &routing), None);
    }

    #[test]
    fn key_prefix_matches_only_with_the_dash_boundary() {
        let rule = |prefix: &str| Rule {
            r#match: RuleCriteria {
                key_prefix: Some(prefix.to_string()),
                ..Default::default()
            },
            service: "svc".to_string(),
        };

        let matching = routing_with_rules(vec![rule("PIXEL")]);
        assert!(route(&facts(), &matching).is_some());

        let mut other_facts = facts();
        other_facts.key = "PIXELX-1".to_string();
        let non_matching = routing_with_rules(vec![rule("PIXEL")]);
        assert_eq!(route(&other_facts, &non_matching), None);
    }

    #[test]
    fn service_absent_from_repos_still_routes_with_no_template() {
        let rules = vec![Rule {
            r#match: RuleCriteria {
                component: Some("cheat-alerts-service".to_string()),
                ..Default::default()
            },
            service: "unlisted-service".to_string(),
        }];
        let routing = routing_with_rules(rules);

        let route = route(&facts(), &routing).expect("rule matches");

        assert_eq!(
            route,
            Route {
                service: "unlisted-service".to_string(),
                template: None,
                track: None,
                flow: None,
                route: "unlisted-service".to_string(),
            }
        );
    }

    #[test]
    fn invalid_summary_regex_is_dropped_others_survive() {
        let raw: serde_yaml::Value = serde_yaml::from_str(
            r#"
- match: { summary_regex: "(" }
  service: broken
- match: { component: cheat-alerts-service }
  service: cheat-alerts-service
"#,
        )
        .unwrap();

        let rules = compile_rules(Some(&raw));

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].service, "cheat-alerts-service");
    }

    #[test]
    fn defaults_track_fills_in_when_repo_entry_has_none() {
        use crate::config::RepoRoute;

        let mut repos = BTreeMap::new();
        repos.insert(
            "cheat-alerts-service".to_string(),
            RepoRoute {
                template: Some("dev-csharp".to_string()),
                track: None,
                flow: None,
                route: None,
            },
        );
        let routing = Routing {
            defaults: RoutingDefaults {
                track: Some(crate::models::track::Track::Standard),
            },
            rules: vec![Rule {
                r#match: RuleCriteria {
                    component: Some("cheat-alerts-service".to_string()),
                    ..Default::default()
                },
                service: "cheat-alerts-service".to_string(),
            }],
            repos,
        };

        let route = route(&facts(), &routing).expect("rule matches");

        assert_eq!(route.template, Some("dev-csharp".to_string()));
        assert_eq!(route.track, Some(crate::models::track::Track::Standard));
    }

    #[test]
    fn empty_match_never_matches() {
        let rules = vec![Rule {
            r#match: RuleCriteria::default(),
            service: "should-not-win".to_string(),
        }];
        let routing = routing_with_rules(rules);

        assert_eq!(route(&facts(), &routing), None);
    }
}
