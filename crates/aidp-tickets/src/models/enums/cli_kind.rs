use crate::model::{IssueType, Source, Status};
use crate::models::constants::cli::{
    ISSUE_BUG, ISSUE_SPIKE, ISSUE_STORY, ISSUE_TASK, SOURCE_EXPECTED, SOURCE_GITHUB, SOURCE_LOCAL,
    STATUS_BLOCKED, STATUS_CANCELED, STATUS_CANCELLED, STATUS_DONE, STATUS_EXPECTED,
    STATUS_IN_PROGRESS, STATUS_IN_PROGRESS_FLAT, STATUS_IN_PROGRESS_SNAKE, STATUS_IN_REVIEW,
    STATUS_IN_REVIEW_FLAT, STATUS_IN_REVIEW_SNAKE, STATUS_QUEUED, STATUS_TODO,
};
use std::str::FromStr;

/// Status a user can type on the command line, with its tolerant spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusToken {
    Todo,
    Queued,
    InProgress,
    InReview,
    Done,
    Blocked,
    Cancelled,
}

impl StatusToken {
    /// The ticket status this token names.
    pub fn status(self) -> Status {
        match self {
            StatusToken::Todo => Status::Todo,
            StatusToken::Queued => Status::Queued,
            StatusToken::InProgress => Status::InProgress,
            StatusToken::InReview => Status::InReview,
            StatusToken::Done => Status::Done,
            StatusToken::Blocked => Status::Blocked,
            StatusToken::Cancelled => Status::Cancelled,
        }
    }
}

impl FromStr for StatusToken {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            STATUS_TODO => Ok(StatusToken::Todo),
            STATUS_QUEUED => Ok(StatusToken::Queued),
            STATUS_IN_PROGRESS | STATUS_IN_PROGRESS_SNAKE | STATUS_IN_PROGRESS_FLAT => {
                Ok(StatusToken::InProgress)
            }
            STATUS_IN_REVIEW | STATUS_IN_REVIEW_SNAKE | STATUS_IN_REVIEW_FLAT => {
                Ok(StatusToken::InReview)
            }
            STATUS_DONE => Ok(StatusToken::Done),
            STATUS_BLOCKED => Ok(StatusToken::Blocked),
            STATUS_CANCELLED | STATUS_CANCELED => Ok(StatusToken::Cancelled),
            _ => anyhow::bail!("unknown status `{s}`: expected {STATUS_EXPECTED}"),
        }
    }
}

/// Ticket source a user can type on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceToken {
    Github,
    Local,
}

impl SourceToken {
    /// The ticket source this token names.
    pub fn source(self) -> Source {
        match self {
            SourceToken::Github => Source::Github,
            SourceToken::Local => Source::Local,
        }
    }
}

impl FromStr for SourceToken {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            SOURCE_GITHUB => Ok(SourceToken::Github),
            SOURCE_LOCAL => Ok(SourceToken::Local),
            _ => anyhow::bail!("unknown source `{s}`: expected {SOURCE_EXPECTED}"),
        }
    }
}

/// Lowercase name of an [`IssueType`], as routing rules match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueTypeName(pub IssueType);

impl IssueTypeName {
    /// The lowercase name of the wrapped issue type.
    pub fn as_str(self) -> &'static str {
        match self.0 {
            IssueType::Story => ISSUE_STORY,
            IssueType::Task => ISSUE_TASK,
            IssueType::Bug => ISSUE_BUG,
            IssueType::Spike => ISSUE_SPIKE,
        }
    }
}
