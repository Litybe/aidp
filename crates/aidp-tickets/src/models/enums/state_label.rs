//! The state labels of a GitHub issue (`aidp:*`), the vocabulary of the plan's §3.1.

/// One state an issue can show through its single `aidp:*` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateLabel {
    Triage,
    Ready,
    Queued,
    InProgress,
    Review,
    Fixing,
    ToTest,
    Testing,
    AwaitingMerge,
    Done,
    Blocked,
    Skip,
}

impl StateLabel {
    /// Every state, in board order.
    pub const ALL: [StateLabel; 12] = [
        StateLabel::Triage,
        StateLabel::Ready,
        StateLabel::Queued,
        StateLabel::InProgress,
        StateLabel::Review,
        StateLabel::Fixing,
        StateLabel::ToTest,
        StateLabel::Testing,
        StateLabel::AwaitingMerge,
        StateLabel::Done,
        StateLabel::Blocked,
        StateLabel::Skip,
    ];

    /// The states a flow sets while the ticket stays `in-progress`.
    pub const PHASES: [StateLabel; 4] = [
        StateLabel::Review,
        StateLabel::Fixing,
        StateLabel::ToTest,
        StateLabel::Testing,
    ];

    /// The part after the label prefix, e.g. `awaiting-merge`.
    pub const fn suffix(self) -> &'static str {
        match self {
            StateLabel::Triage => "triage",
            StateLabel::Ready => "ready",
            StateLabel::Queued => "queued",
            StateLabel::InProgress => "in-progress",
            StateLabel::Review => "review",
            StateLabel::Fixing => "fixing",
            StateLabel::ToTest => "to-test",
            StateLabel::Testing => "testing",
            StateLabel::AwaitingMerge => "awaiting-merge",
            StateLabel::Done => "done",
            StateLabel::Blocked => "blocked",
            StateLabel::Skip => "skip",
        }
    }

    /// The label name under `prefix`, e.g. `aidp:ready`.
    pub fn label(self, prefix: &str) -> String {
        format!("{prefix}{}", self.suffix())
    }

    /// The state whose suffix is `suffix`, ignoring case.
    pub fn from_suffix(suffix: &str) -> Option<StateLabel> {
        let wanted = suffix.trim();
        Self::ALL
            .into_iter()
            .find(|state| state.suffix().eq_ignore_ascii_case(wanted))
    }

    /// The colour the label is created with (hex, no `#`).
    pub const fn color(self) -> &'static str {
        match self {
            StateLabel::Triage => "fef2c0",
            StateLabel::Ready => "bfd4f2",
            StateLabel::Queued => "d4c5f9",
            StateLabel::InProgress => "fbca04",
            StateLabel::Review => "5319e7",
            StateLabel::Fixing => "c2e0c6",
            StateLabel::ToTest => "ffd8b1",
            StateLabel::Testing => "f9d0c4",
            StateLabel::AwaitingMerge => "0052cc",
            StateLabel::Done => "0e8a16",
            StateLabel::Blocked => "d93f0b",
            StateLabel::Skip => "cccccc",
        }
    }

    /// The description the label is created with.
    pub const fn description(self) -> &'static str {
        match self {
            StateLabel::Triage => "aidp : issue incomplète, pas de critères CA-n",
            StateLabel::Ready => "aidp : prête, candidate au dispatch",
            StateLabel::Queued => "aidp : retenue, attend un slot, l'éditeur ou le budget",
            StateLabel::InProgress => "aidp : réalisation en cours",
            StateLabel::Review => "aidp : revue en cours",
            StateLabel::Fixing => "aidp : correction après revue ou test",
            StateLabel::ToTest => "aidp : développement fini, attend la voie de test Unity",
            StateLabel::Testing => "aidp : test dans Unity en cours",
            StateLabel::AwaitingMerge => "aidp : PR prête, attend le merge",
            StateLabel::Done => "aidp : PR mergée, issue fermée",
            StateLabel::Blocked => "aidp : besoin d'un humain, raison en commentaire",
            StateLabel::Skip => "aidp : jamais dispatchée",
        }
    }
}
