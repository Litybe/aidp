//! Gateway HTTP route paths shared by the client verbs.

/// Gateway status route.
pub const GATEWAY_STATUS: &str = "/api/gateway/status";
/// Sessions collection route.
pub const SESSIONS: &str = "/api/sessions";
/// Automations collection route.
pub const AUTOMATIONS: &str = "/api/automations";
/// Automation runs collection route.
pub const AUTOMATION_RUNS: &str = "/api/automation-runs";
/// Projects collection route.
pub const PROJECTS: &str = "/api/projects";
/// Projects import route.
pub const PROJECTS_IMPORT: &str = "/api/projects/import";
/// Presets collection route.
pub const PRESETS: &str = "/api/presets";
/// Queue collection route.
pub const QUEUE: &str = "/api/queue";
/// Dispatcher status route.
pub const DISPATCHER: &str = "/api/dispatcher";
/// Dispatcher pause route.
pub const DISPATCHER_PAUSE: &str = "/api/dispatcher/pause";
/// Dispatcher resume route.
pub const DISPATCHER_RESUME: &str = "/api/dispatcher/resume";
/// Dispatcher plan route.
pub const DISPATCHER_PLAN: &str = "/api/dispatcher/plan";
/// Dispatcher tick route.
pub const DISPATCHER_TICK: &str = "/api/dispatcher/tick";
/// Dispatcher budget route.
pub const DISPATCHER_BUDGET: &str = "/api/dispatcher/budget";
/// Tickets collection route.
pub const TICKETS: &str = "/api/tickets";
/// Embedding model service of the daemon.
pub const MEMORY_MODEL: &str = "/api/memory/model";
/// Recent events route.
pub const EVENTS_RECENT: &str = "/api/events/recent";
/// Path segment of a project's Claude Code transcripts, under `/api/projects/:id/`.
pub const CLAUDE_SESSIONS_SEGMENT: &str = "claude-sessions";
/// Segment of a ticket's flow route, under `/api/tickets/:key/`.
pub const TICKET_FLOW_SEGMENT: &str = "flow";
/// Flow runs collection route.
pub const FLOWS: &str = "/api/flows";
/// Segment of a ticket's cost route, under `/api/tickets/:key/`.
pub const TICKET_COST_SEGMENT: &str = "cost";
/// Segment of the all-tickets cost route, `/api/tickets/costs`.
pub const TICKET_COSTS_SEGMENT: &str = "costs";
/// Cost anomalies route.
pub const COST_ANOMALIES: &str = "/api/cost/anomalies";
