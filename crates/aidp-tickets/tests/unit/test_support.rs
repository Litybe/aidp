//! Ticket and blocker link fixtures shared by the unit test files.

#![allow(dead_code)]

use crate::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use crate::models::blocker_link::BlockerLink;
use crate::models::enums::issue_state::IssueState;
use crate::models::github_meta::GithubMeta;
use chrono::Utc;

/// Service every shared ticket belongs to.
pub const SERVICE: &str = "svc";

/// Repository every shared GitHub mirror belongs to.
pub const REPO: &str = "Litybe/PixelHood";

/// A to-do ticket of the given source, with no template, epic or GitHub metadata.
pub fn ticket(key: &str, source: Source) -> Ticket {
    let now = Utc::now();
    Ticket {
        key: key.to_string(),
        source,
        r#type: IssueType::Task,
        summary: key.to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: Priority::Medium.rank(),
        service: SERVICE.to_string(),
        template: None,
        track: None,
        route: None,
        flow: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        milestone: None,
        created: now,
        updated: now,
        github: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    }
}

/// The GitHub metadata of issue `number`, blocked by `blocked_by`.
pub fn github_meta(number: u64, blocked_by: Vec<BlockerLink>) -> GithubMeta {
    GithubMeta {
        repo: REPO.to_string(),
        number,
        url: String::new(),
        state: IssueState::Open,
        synced_at: None,
        blocked_by,
        base_branch: None,
        parent: None,
    }
}

/// A blocker link with the given key and state.
pub fn link(key: &str, state: IssueState) -> BlockerLink {
    BlockerLink {
        key: key.to_string(),
        state,
    }
}
