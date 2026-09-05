use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: Option<PathBuf>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionSummary {
    pub id: String,
    pub path: PathBuf,
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub first_prompt: Option<String>,
    /// None means the lightweight discovery scan has not counted the full file.
    pub turn_count: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct ParseStats {
    pub records: usize,
    pub malformed_records: usize,
    pub skipped_oversize_records: usize,
    pub omitted_text_bytes: usize,
    pub unknown_records: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    pub meta: SessionMeta,
    pub turns: Vec<Turn>,
    pub parse_stats: ParseStats,
    pub revision: u64,
}

impl Session {
    pub fn latest_active(&self) -> Option<usize> {
        self.turns
            .iter()
            .rposition(|t| t.status != TurnStatus::RolledBack)
    }
}

#[derive(Clone, Debug, Default)]
pub struct UserPrompt {
    pub text: String,
    pub preview: String,
    pub images_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Turn {
    pub ordinal: usize,
    pub id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub prompt: UserPrompt,
    pub items: Vec<TurnItem>,
    pub status: TurnStatus,
    pub activity: ActivitySummary,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnItem {
    AgentMessage { text: String, phase: Option<String> },
    ToolCall { name: String, summary: String },
    ToolOutput { summary: String, is_error: bool },
    FileActivity { path: String, kind: String },
    Notice { text: String },
}

#[derive(Clone, Debug, Default)]
pub struct ActivitySummary {
    pub commands: usize,
    pub tool_calls: usize,
    pub files_read: usize,
    pub files_changed: usize,
    pub errors: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TurnStatus {
    InProgress,
    Completed,
    Failed,
    #[default]
    Unknown,
    RolledBack,
}

impl TurnStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::InProgress => "incomplete",
            Self::Completed => "completed",
            Self::Failed => "detected failure",
            Self::Unknown => "unknown",
            Self::RolledBack => "rolled back",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::InProgress => "…",
            Self::Completed => "✓",
            Self::Failed => "✕",
            Self::Unknown => "?",
            Self::RolledBack => "↶",
        }
    }
}
