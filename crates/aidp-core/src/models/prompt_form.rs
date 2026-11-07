/// How a stage prompt asks the agent to deliver its reply envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptForm {
    /// Returned as the final answer, schema-validated by the Workflow runtime.
    #[default]
    Workflow,
    /// Written to `<stage_dir>/reply.md`, validated by the engine afterwards.
    File,
}
