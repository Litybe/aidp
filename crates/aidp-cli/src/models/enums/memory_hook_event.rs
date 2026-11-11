use clap::ValueEnum;

/// The Claude Code event a memory hook answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MemoryHookEvent {
    SessionStart,
    Prompt,
    Stop,
    PreCompact,
    SessionEnd,
    SubagentStart,
}
