use crate::models::constants::flow_execution::{TICKET_PROJECT, TICKET_SERVICE};
use crate::models::constants::flow_inputs::TICKET_SUMMARY;
use aidp_flows::FlowState;
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};

/// Project reference supplied by an external ticket or by a local ticket's service.
pub fn project_name(ticket: &serde_json::Value) -> Option<&str> {
    [TICKET_PROJECT, TICKET_SERVICE]
        .iter()
        .find_map(|field| ticket.get(*field)?.as_str())
}

/// A probe-compatible ticket snapshot when a flow was started without a local note.
pub fn ticket(state: &FlowState) -> Option<Ticket> {
    if let Ok(ticket) = serde_json::from_value(state.ticket.clone()) {
        return Some(ticket);
    }
    let now = chrono::Utc::now();
    Some(Ticket {
        key: super::pipeline_inputs::ticket_key(state).to_string(),
        source: Source::Github,
        r#type: state
            .ticket
            .get(crate::models::constants::flow_execution::TICKET_TYPE)
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or(IssueType::Task),
        summary: state
            .ticket
            .get(TICKET_SUMMARY)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: Status::InProgress,
        priority: Priority::Medium,
        priority_rank: 0,
        service: project_name(&state.ticket)?.to_string(),
        template: None,
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    })
}
