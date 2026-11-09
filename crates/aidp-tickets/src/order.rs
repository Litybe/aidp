//! Dequeue sort key: manual priority, milestone, priority, source, issue
//! number, created, key (plan §3.2, G5).
//!
//! The order is total and stable: every component of the tuple is itself
//! totally ordered, and the final component (`key`) is unique per ticket, so
//! no two distinct tickets ever compare equal.

use crate::model::Ticket;
use chrono::{DateTime, Utc};

/// The dequeue sort key of a ticket, all components ascending.
pub type SortKey = (
    u8,
    u8,
    u8,
    DateTime<Utc>,
    String,
    u8,
    u8,
    u64,
    DateTime<Utc>,
    String,
);

/// `(manual, milestone missing, due date missing, due date, milestone title, priority_rank,
/// source rank, issue number, created, key)`, all ascending:
///
/// 1. a ticket with `autopilot.manual_priority == Some(true)` (promoted from
///    the front or `aidp ticket start`) sorts before every other;
/// 2. then the nearest milestone: a ticket in a milestone before one without,
///    an earlier due date first (a milestone without a due date after the
///    dated ones), then the milestone title;
/// 3. then the priority — the `prio:high` / `prio:low` labels of a GitHub
///    mirror land in `priority_rank`;
/// 4. then a GitHub mirror before a local ticket, then the issue number
///    (local tickets have none), then FIFO on `created`, then the key.
pub fn sort_key(t: &Ticket) -> SortKey {
    let manual_priority_rank = if t.autopilot.manual_priority == Some(true) {
        0
    } else {
        1
    };
    let milestone = t.milestone.as_ref();
    let due = milestone.and_then(|m| m.due_on);
    (
        manual_priority_rank,
        u8::from(milestone.is_none()),
        u8::from(due.is_none()),
        due.unwrap_or(DateTime::<Utc>::MIN_UTC),
        milestone.map(|m| m.title.clone()).unwrap_or_default(),
        t.priority_rank,
        t.source.rank(),
        t.github.as_ref().map_or(u64::MAX, |g| g.number),
        t.created,
        t.key.clone(),
    )
}

/// Sort `tickets` into dequeue order. Ties are impossible in practice (the
/// key is the final tie-break and keys are unique), but the sort is stable
/// regardless: `Vec::sort_by_key` never reorders elements whose keys compare
/// equal.
pub fn sort(tickets: &mut [Ticket]) {
    tickets.sort_by_key(sort_key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Autopilot, IssueType, Priority, Source, Status};
    use chrono::Duration;

    fn ticket(key: &str, source: Source, priority: Priority, created: DateTime<Utc>) -> Ticket {
        Ticket {
            key: key.to_string(),
            source,
            r#type: IssueType::Task,
            summary: key.to_string(),
            status: Status::Todo,
            priority,
            priority_rank: priority.rank(),
            service: "svc".to_string(),
            template: None,
            track: None,
            route: None,
            flow: None,
            labels: Vec::new(),
            components: Vec::new(),
            epic: None,
            assignee: None,
            milestone: None,
            created,
            updated: created,
            github: None,
            autopilot: Autopilot::default(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn github_before_local_at_equal_priority() {
        let base = Utc::now();
        let mut tickets = vec![
            ticket("LOC-1", Source::Local, Priority::Medium, base),
            ticket("PIXEL-1", Source::Github, Priority::Medium, base),
        ];

        sort(&mut tickets);

        assert_eq!(tickets[0].key, "PIXEL-1");
        assert_eq!(tickets[1].key, "LOC-1");
    }

    #[test]
    fn priority_then_fifo() {
        let base = Utc::now();
        let mut tickets = vec![
            ticket(
                "A",
                Source::Github,
                Priority::Low,
                base + Duration::seconds(10),
            ),
            ticket(
                "B",
                Source::Github,
                Priority::Highest,
                base + Duration::seconds(20),
            ),
            ticket(
                "C",
                Source::Github,
                Priority::Highest,
                base + Duration::seconds(5),
            ),
        ];

        sort(&mut tickets);

        // Highest priority first regardless of age; among equal priority,
        // the earlier `created` (FIFO) wins.
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, vec!["C", "B", "A"]);
    }

    #[test]
    fn key_breaks_the_last_tie() {
        let base = Utc::now();
        let mut tickets = vec![
            ticket("B-1", Source::Github, Priority::Medium, base),
            ticket("A-1", Source::Github, Priority::Medium, base),
        ];

        sort(&mut tickets);

        assert_eq!(tickets[0].key, "A-1");
        assert_eq!(tickets[1].key, "B-1");
    }

    #[test]
    fn sort_is_stable_on_identical_keys() {
        let base = Utc::now();
        let mut tickets = vec![
            ticket("LOC-1", Source::Local, Priority::Medium, base),
            ticket("LOC-1", Source::Local, Priority::Medium, base),
        ];
        // Two tickets that compare equal under `sort_key` (same source,
        // priority, created and key) but distinguishable by another field, so
        // we can check the sort left their relative order untouched.
        tickets[0].summary = "first".to_string();
        tickets[1].summary = "second".to_string();

        sort(&mut tickets);

        assert_eq!(tickets[0].summary, "first");
        assert_eq!(tickets[1].summary, "second");
    }

    #[test]
    fn manual_priority_jumps_the_queue() {
        let base = Utc::now();
        let mut manual = ticket("LOC-1", Source::Local, Priority::Medium, base);
        manual.autopilot.manual_priority = Some(true);
        let mut tickets = vec![
            ticket(
                "PIXEL-1",
                Source::Github,
                Priority::Highest,
                base - Duration::seconds(10),
            ),
            manual,
        ];

        sort(&mut tickets);

        assert_eq!(tickets[0].key, "LOC-1");
        assert_eq!(tickets[1].key, "PIXEL-1");
    }

    #[test]
    fn no_manual_priority_keeps_the_existing_order() {
        let base = Utc::now();
        let mut tickets = vec![
            ticket("LOC-1", Source::Local, Priority::Medium, base),
            ticket("PIXEL-1", Source::Github, Priority::Medium, base),
        ];
        sort(&mut tickets);
        assert_eq!(tickets[0].key, "PIXEL-1");
        assert_eq!(tickets[1].key, "LOC-1");

        let mut tickets = vec![
            ticket(
                "A",
                Source::Github,
                Priority::Low,
                base + Duration::seconds(10),
            ),
            ticket(
                "B",
                Source::Github,
                Priority::Highest,
                base + Duration::seconds(20),
            ),
            ticket(
                "C",
                Source::Github,
                Priority::Highest,
                base + Duration::seconds(5),
            ),
        ];
        sort(&mut tickets);
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, vec!["C", "B", "A"]);
    }

    fn mirror(number: u64, milestone: Option<(&str, Option<i64>)>, priority: Priority) -> Ticket {
        use chrono::TimeZone;
        let base = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let mut t = ticket(&format!("GH-{number}"), Source::Github, priority, base);
        t.github = Some(crate::models::github_meta::GithubMeta {
            repo: "o/r".to_string(),
            number,
            url: String::new(),
            state: crate::models::enums::issue_state::IssueState::Open,
            synced_at: None,
            blocked_by: Vec::new(),
            base_branch: None,
            parent: None,
        });
        t.milestone = milestone.map(|(title, days)| crate::models::milestone::Milestone {
            title: title.to_string(),
            number: None,
            due_on: days.map(|d| base + Duration::days(d)),
        });
        t
    }

    #[test]
    fn nearest_milestone_then_priority_then_number() {
        let mut tickets = vec![
            mirror(5, None, Priority::High),
            mirror(9, Some(("RM-2", Some(30))), Priority::Medium),
            mirror(8, Some(("RM-1", Some(7))), Priority::Low),
            mirror(7, Some(("RM-1", Some(7))), Priority::High),
            mirror(3, Some(("Backlog", None)), Priority::High),
            mirror(12, Some(("RM-1", Some(7))), Priority::High),
        ];
        sort(&mut tickets);
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, vec!["GH-7", "GH-12", "GH-8", "GH-9", "GH-3", "GH-5"]);
    }

    #[test]
    fn a_milestone_without_due_date_sorts_by_title() {
        let mut tickets = vec![
            mirror(1, Some(("Zeta", None)), Priority::Medium),
            mirror(2, Some(("Alpha", None)), Priority::Medium),
        ];
        sort(&mut tickets);
        assert_eq!(tickets[0].key, "GH-2");
    }

    #[test]
    fn promoted_ticket_passes_the_nearest_milestone() {
        let mut promoted = mirror(40, None, Priority::Low);
        promoted.autopilot.manual_priority = Some(true);
        let mut tickets = vec![
            mirror(2, Some(("RM-1", Some(1))), Priority::Highest),
            promoted,
        ];
        sort(&mut tickets);
        assert_eq!(tickets[0].key, "GH-40");
    }
}
