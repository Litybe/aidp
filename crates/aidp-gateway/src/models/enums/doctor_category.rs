use serde::{Deserialize, Serialize};

/// The known pipeline failures a diagnosis classifies an incident into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCategory {
    /// The report stage answered `needs_review` while the PR exists and the recheck is clean.
    FalseFailure,
    /// The run ended but the flow step stayed `running`.
    RunEndedFlowRunning,
    /// A Unity reimport read as a frozen editor (silent heartbeat).
    UnityReimportNotFrozen,
    /// The editor is really stuck behind a modal window.
    UnityModalDialog,
    /// A step waits for budget forever, with nothing journaled.
    BudgetWaitSilent,
    /// The worktree cap is full of blocked tickets.
    WorktreeCapSaturated,
    /// The ticket is tied to a dead flow and skipped as `owned-by-flow`.
    DeadFlowOwnership,
    /// An add/add conflict on the `.meta` of a folder several PRs created.
    MetaAddAddConflict,
    /// `gh pr update-branch --rebase` conflicts while the final state does not.
    RebaseConflictOnly,
    /// The head moved between the read and the merge.
    HeadMovedBeforeMerge,
    /// A defect of the harness itself: a patch is proposed, never applied.
    HarnessDefect,
    /// A failure of the product, not of the pipeline.
    ProductBug,
    /// The MCP server did not answer in time (`CONNECT_TIMEOUT`).
    McpConnectTimeout,
    /// The Unity compile gate timed out.
    UnityCompileTimeout,
    /// The agent session was lost.
    SessionLost,
    /// The provider's API is overloaded or rate limited.
    ApiOverloaded,
    /// A branch changed the test infrastructure (scene, package, project setting) and broke every test.
    TestInfraRegression,
    /// The branch lags behind `main` and no longer merges or compiles with it.
    BranchBehindMain,
    /// The same failure hits several tickets: the harness is at fault, not a game ticket.
    SharedHarnessFailure,
    /// A proven defect of the game the earlier bugs did not fix.
    RealDefect,
    #[default]
    #[serde(other)]
    Other,
}

impl DoctorCategory {
    /// The wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FalseFailure => "false_failure",
            Self::RunEndedFlowRunning => "run_ended_flow_running",
            Self::UnityReimportNotFrozen => "unity_reimport_not_frozen",
            Self::UnityModalDialog => "unity_modal_dialog",
            Self::BudgetWaitSilent => "budget_wait_silent",
            Self::WorktreeCapSaturated => "worktree_cap_saturated",
            Self::DeadFlowOwnership => "dead_flow_ownership",
            Self::MetaAddAddConflict => "meta_add_add_conflict",
            Self::RebaseConflictOnly => "rebase_conflict_only",
            Self::HeadMovedBeforeMerge => "head_moved_before_merge",
            Self::HarnessDefect => "harness_defect",
            Self::ProductBug => "product_bug",
            Self::McpConnectTimeout => "mcp_connect_timeout",
            Self::UnityCompileTimeout => "unity_compile_timeout",
            Self::SessionLost => "session_lost",
            Self::ApiOverloaded => "api_overloaded",
            Self::TestInfraRegression => "test_infra_regression",
            Self::BranchBehindMain => "branch_behind_main",
            Self::SharedHarnessFailure => "shared_harness_failure",
            Self::RealDefect => "real_defect",
            Self::Other => "other",
        }
    }
}
