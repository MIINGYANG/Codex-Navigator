use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: Option<PathBuf>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub identity: SessionIdentity,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionKind {
    Main,
    Subagent,
    #[default]
    Unknown,
}

impl SessionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "MAIN",
            Self::Subagent => "SUBAGENT",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionIdentity {
    pub kind: SessionKind,
    pub parent_id: Option<String>,
    pub agent_label: Option<String>,
    /// Persisted session lineage; not a parent-question relationship.
    pub has_fork_lineage: bool,
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
    pub identity: SessionIdentity,
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
    pub events: Vec<SessionEvent>,
    pub parse_stats: ParseStats,
    pub revision: u64,
}

impl Session {
    pub fn visible_events(&self) -> impl Iterator<Item = &SessionEvent> {
        self.events.iter().filter(|event| {
            event.turn_index.is_none_or(|index| {
                self.turns
                    .get(index)
                    .is_some_and(|turn| turn.status != TurnStatus::RolledBack)
            })
        })
    }

    pub fn event_summary(&self) -> SessionEventSummary {
        let mut summary = SessionEventSummary::default();
        for event in self.visible_events() {
            match event.kind.as_str() {
                "commit" => {
                    summary.commit_count += 1;
                    summary.last_commit = Some(event.clone());
                }
                "compaction" => summary.compaction_count += 1,
                _ => {}
            }
        }
        summary
    }

    pub fn latest_active(&self) -> Option<usize> {
        self.turns
            .iter()
            .rposition(|t| t.status != TurnStatus::RolledBack)
    }
}

/// Observed persisted evidence only; missing metadata is never inferred from prose.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SessionEvent {
    pub id: String,
    pub kind: String,
    pub turn_index: Option<usize>,
    pub timestamp: Option<String>,
    pub source: String,
    pub hash: Option<String>,
    /// Working directory explicitly associated with the successful Git command.
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub version: Option<String>,
    pub summary: Option<String>,
    pub trigger: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SessionEventSummary {
    pub commit_count: usize,
    pub compaction_count: usize,
    pub last_commit: Option<SessionEvent>,
}

#[derive(Clone, Debug, Default)]
pub struct UserPrompt {
    pub text: String,
    pub preview: String,
    pub images_count: usize,
    /// Text bytes no longer retained; preview remains available if the body is evicted.
    pub omitted_bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Turn {
    pub ordinal: usize,
    pub id: Option<String>,
    /// Only an explicit persisted parent_turn_id, never root_turn_id or inferred text.
    pub parent_turn_id: Option<String>,
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
    AgentMessage {
        text: String,
        phase: Option<String>,
    },
    ToolCall {
        name: String,
        summary: String,
    },
    ToolOutput {
        summary: String,
        is_error: bool,
    },
    FileActivity {
        path: String,
        kind: String,
    },
    Notice {
        text: String,
    },
    /// The activity body was evicted to keep newer content within the memory budget.
    Omitted,
}

impl TurnItem {
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::AgentMessage { text, phase } => {
                text.len() + phase.as_ref().map_or(0, String::len)
            }
            Self::ToolCall { name, summary } => name.len() + summary.len(),
            Self::ToolOutput { summary, .. } => summary.len(),
            Self::FileActivity { path, kind } => path.len() + kind.len(),
            Self::Notice { text } => text.len(),
            Self::Omitted => 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActivitySummary {
    pub commands: usize,
    pub tool_calls: usize,
    pub files_read: usize,
    pub files_changed: usize,
    /// Observed activity errors, independent of the turn's lifecycle or answer quality.
    pub errors: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TurnStatus {
    InProgress,
    Completed,
    Failed,
    Interrupted,
    #[default]
    Unknown,
    RolledBack,
}

impl TurnStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::InProgress => "incomplete",
            Self::Completed => "completed",
            Self::Failed => "execution error",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
            Self::RolledBack => "rolled back",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::InProgress => "…",
            Self::Completed => "✓",
            Self::Failed => "✕",
            Self::Interrupted => "⊘",
            Self::Unknown => "?",
            Self::RolledBack => "↶",
        }
    }
}
