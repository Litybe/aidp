//! Names of the pipeline inputs a flow step fills from the ticket it delivers.

/// Input naming the run's feature folder and branch.
pub const INPUT_FEATURE: &str = "feature";
/// Input carrying the GitHub issue number of an issue-mode run (the run reads the issue itself).
pub const INPUT_ISSUE: &str = "issue";
/// Input carrying the free-text demand of a prompt-mode run.
pub const INPUT_PROMPT: &str = "prompt";
/// Input carrying the run depth.
pub const INPUT_TRACK: &str = "track";
/// Branch the ticket starts from and its PR targets, when it is not the default one (a bug ticket).
pub const INPUT_BASE_BRANCH: &str = "base_branch";
pub const GITHUB_BASE_BRANCH: &str = "base_branch";
/// Run input naming the ticket route, read by the model policy.
pub const INPUT_ROUTE: &str = "route";
/// Input telling a run who publishes its branch.
pub const INPUT_PUBLISH: &str = "publish";
/// `publish` value of a run whose flow publishes the branch: the run commits and never pushes.
pub const PUBLISH_BY_HOST: &str = "host";
/// `with` key of a pipeline step naming the human step whose stored report heads its `prompt`.
pub const WITH_REPORT_FROM: &str = "report_from";
/// Separator between the report and the ticket body in a prompt.
pub const PROMPT_SEPARATOR: &str = "\n\n";
/// Ticket field holding its one-line summary, the demand of a ticket whose note has no body.
pub const TICKET_SUMMARY: &str = "summary";
/// Ticket field telling a GitHub mirror from a local ticket.
pub const TICKET_SOURCE: &str = "source";
/// Ticket field carrying the run depth.
pub const TICKET_TRACK: &str = "track";
/// Ticket field holding the GitHub metadata of a mirror.
pub const TICKET_GITHUB: &str = "github";
/// Field of the GitHub metadata carrying the issue number.
pub const GITHUB_NUMBER: &str = "number";
