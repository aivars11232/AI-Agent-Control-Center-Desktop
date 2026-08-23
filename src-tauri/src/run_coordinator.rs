use serde::{Deserialize, Serialize};

pub const MAX_RUN_REQUEST_ID_BYTES: usize = 128;
pub const MAX_PROGRESS_MESSAGE_BYTES: usize = 8 * 1024;
pub const MAX_PROGRESS_EVENTS: i64 = 256;
pub const MAX_PROGRESS_BYTES: i64 = 512 * 1024;
pub const MAX_STDOUT_CAPTURE_BYTES: usize = 1024 * 1024;
pub const MAX_STDERR_CAPTURE_BYTES: usize = 512 * 1024;
pub const MAX_OLLAMA_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OLLAMA_CONVERSATION_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SUMMARY_BYTES: usize = 128 * 1024;
pub const MAX_ERROR_BYTES: usize = 64 * 1024;
pub const MAX_DIFF_CHARS: usize = 120_000;
pub const MAX_DIFF_BYTES: usize = 512 * 1024;
pub const MAX_SNAPSHOT_FILES: usize = 20_000;
pub const MAX_SNAPSHOT_MILLIS: u64 = 5_000;
pub const MAX_CHANGED_FILES: usize = 250;
pub const MAX_CHANGED_FILE_BYTES: usize = 256 * 1024;
pub const MAX_RETAINED_ATTEMPTS: i64 = 1_000;
pub const MAX_RETAINED_PAYLOAD_BYTES: i64 = 256 * 1024 * 1024;
pub const MAX_RECENT_ATTEMPTS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAttemptMode {
    Execute,
    Review,
}

impl RunAttemptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Review => "review",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, String> {
        match value {
            "execute" => Ok(Self::Execute),
            "review" => Ok(Self::Review),
            _ => Err("Stored run mode is invalid.".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAttemptStatus {
    Admitted,
    Starting,
    Dispatching,
    Running,
    CancelRequested,
    Succeeded,
    Cancelled,
    TimedOut,
    StartupFailed,
    Failed,
    Interrupted,
}

impl RunAttemptStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Starting => "starting",
            Self::Dispatching => "dispatching",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Succeeded => "succeeded",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::StartupFailed => "startup_failed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_storage(value: &str) -> Result<Self, String> {
        match value {
            "admitted" => Ok(Self::Admitted),
            "starting" => Ok(Self::Starting),
            "dispatching" => Ok(Self::Dispatching),
            "running" => Ok(Self::Running),
            "cancel_requested" => Ok(Self::CancelRequested),
            "succeeded" => Ok(Self::Succeeded),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "startup_failed" => Ok(Self::StartupFailed),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err("Stored run status is invalid.".to_string()),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Cancelled
                | Self::TimedOut
                | Self::StartupFailed
                | Self::Failed
                | Self::Interrupted
        )
    }

    pub fn may_transition_to(self, next: Self) -> bool {
        if self.is_terminal() {
            return false;
        }
        match self {
            Self::Admitted => matches!(
                next,
                Self::Starting | Self::CancelRequested | Self::StartupFailed | Self::Interrupted
            ),
            Self::Starting => matches!(
                next,
                Self::Dispatching | Self::CancelRequested | Self::StartupFailed | Self::Interrupted
            ),
            Self::Dispatching => matches!(
                next,
                Self::Running | Self::CancelRequested | Self::StartupFailed | Self::Interrupted
            ),
            Self::Running => matches!(
                next,
                Self::CancelRequested
                    | Self::Succeeded
                    | Self::TimedOut
                    | Self::Failed
                    | Self::Interrupted
            ),
            Self::CancelRequested => matches!(next, Self::Cancelled | Self::Interrupted),
            Self::Succeeded
            | Self::Cancelled
            | Self::TimedOut
            | Self::StartupFailed
            | Self::Failed
            | Self::Interrupted => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunTruncationEvidence {
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub summary_truncated: bool,
    pub diff_truncated: bool,
    pub changed_files_truncated: bool,
    pub progress_truncated: bool,
    pub before_snapshot_truncated: bool,
    pub after_snapshot_truncated: bool,
    pub original_stdout_bytes: u64,
    pub original_stderr_bytes: u64,
    pub original_summary_bytes: u64,
    pub original_diff_bytes: u64,
    pub original_changed_file_count: u64,
    pub omitted_progress_event_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAttemptProjection {
    pub id: i64,
    pub request_id: String,
    pub agent_id: i64,
    pub task_owner_agent_id: i64,
    pub task_id: i64,
    pub task_title: String,
    pub run_mode: RunAttemptMode,
    pub status: RunAttemptStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workspace_id: Option<String>,
    pub approval_id: Option<i64>,
    pub admitted_at_unix_ms: i64,
    pub started_at_unix_ms: Option<i64>,
    pub cancel_requested_at_unix_ms: Option<i64>,
    pub completed_at_unix_ms: Option<i64>,
    pub duration_seconds: Option<u64>,
    pub output_summary: Option<String>,
    pub stderr_excerpt: Option<String>,
    pub response_id: Option<String>,
    pub usage: RunUsage,
    pub changed_files: Vec<String>,
    pub diff: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub progress_event_count: u64,
    pub recovery_disposition: Option<String>,
    pub truncation: RunTruncationEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEventProjection {
    pub coordinator_revision: i64,
    pub attempt_id: i64,
    pub request_id: String,
    pub sequence: i64,
    pub kind: String,
    pub status: RunAttemptStatus,
    pub message: String,
    pub message_truncated: bool,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunCoordinatorSnapshot {
    pub revision: i64,
    pub active_attempt: Option<RunAttemptProjection>,
    pub recent_attempts: Vec<RunAttemptProjection>,
    pub retained_attempt_count: u64,
    pub retained_payload_bytes: u64,
    pub pruned_attempt_count: u64,
    pub last_pruned_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunCompletion {
    pub status: RunAttemptStatus,
    pub output_summary: Option<String>,
    pub stderr_excerpt: Option<String>,
    pub response_id: Option<String>,
    pub runtime_model: Option<String>,
    pub usage: RunUsage,
    pub changed_files: Vec<String>,
    pub diff: Option<String>,
    pub duration_seconds: u64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub truncation: RunTruncationEvidence,
    pub recovery_disposition: Option<String>,
}

impl RunCompletion {
    pub fn terminal_error(
        status: RunAttemptStatus,
        code: impl Into<String>,
        message: impl Into<String>,
        duration_seconds: u64,
    ) -> Self {
        debug_assert!(status.is_terminal());
        let message = message.into();
        let bounded = BoundedText::from_text(&message, MAX_ERROR_BYTES);
        Self {
            status,
            output_summary: None,
            stderr_excerpt: None,
            response_id: None,
            runtime_model: None,
            usage: RunUsage {
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
            },
            changed_files: Vec::new(),
            diff: None,
            duration_seconds,
            error_code: Some(code.into()),
            error_message: Some(bounded.as_str().to_string()),
            truncation: RunTruncationEvidence {
                summary_truncated: bounded.truncated(),
                original_summary_bytes: bounded.original_bytes() as u64,
                ..RunTruncationEvidence::default()
            },
            recovery_disposition: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedText {
    text: String,
    limit: usize,
    original_bytes: usize,
    truncated: bool,
}

impl BoundedText {
    pub fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            original_bytes: 0,
            truncated: false,
        }
    }

    pub fn from_text(value: &str, limit: usize) -> Self {
        let mut bounded = Self::new(limit);
        bounded.push_str(value);
        bounded
    }

    pub fn push_str(&mut self, value: &str) {
        self.original_bytes = self.original_bytes.saturating_add(value.len());
        if value.is_empty() {
            return;
        }
        let available = self.limit.saturating_sub(self.text.len());
        if available == 0 {
            self.truncated = true;
            return;
        }
        let mut end = value.len().min(available);
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&value[..end]);
        if end < value.len() {
            self.truncated = true;
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }

    pub fn original_bytes(&self) -> usize {
        self.original_bytes
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPaths {
    pub paths: Vec<String>,
    pub original_count: usize,
    pub truncated: bool,
}

pub fn bound_paths(values: Vec<String>) -> BoundedPaths {
    let original_count = values.len();
    let mut retained_bytes = 0usize;
    let mut paths = Vec::new();
    for value in values {
        if paths.len() >= MAX_CHANGED_FILES {
            break;
        }
        let next = retained_bytes.saturating_add(value.len());
        if next > MAX_CHANGED_FILE_BYTES {
            break;
        }
        retained_bytes = next;
        paths.push(value);
    }
    BoundedPaths {
        truncated: paths.len() < original_count,
        paths,
        original_count,
    }
}

pub fn bound_diff(value: Option<String>) -> (Option<String>, usize, bool) {
    let Some(value) = value else {
        return (None, 0, false);
    };
    let original_bytes = value.len();
    let by_chars: String = value.chars().take(MAX_DIFF_CHARS).collect();
    let bounded = BoundedText::from_text(&by_chars, MAX_DIFF_BYTES);
    let truncated = value.chars().count() > MAX_DIFF_CHARS || bounded.truncated();
    (Some(bounded.into_string()), original_bytes, truncated)
}

pub fn validate_request_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > MAX_RUN_REQUEST_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err("The run request identifier is invalid.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_0005_attempt_state_machine_rejects_illegal_and_terminal_transitions() {
        assert!(RunAttemptStatus::Admitted.may_transition_to(RunAttemptStatus::Starting));
        assert!(RunAttemptStatus::Starting.may_transition_to(RunAttemptStatus::Dispatching));
        assert!(RunAttemptStatus::Dispatching.may_transition_to(RunAttemptStatus::Running));
        assert!(RunAttemptStatus::Running.may_transition_to(RunAttemptStatus::Succeeded));
        assert!(RunAttemptStatus::Running.may_transition_to(RunAttemptStatus::CancelRequested));
        assert!(RunAttemptStatus::CancelRequested.may_transition_to(RunAttemptStatus::Cancelled));
        assert!(!RunAttemptStatus::Admitted.may_transition_to(RunAttemptStatus::Succeeded));
        assert!(!RunAttemptStatus::Succeeded.may_transition_to(RunAttemptStatus::Failed));
        assert!(!RunAttemptStatus::Cancelled.may_transition_to(RunAttemptStatus::Running));
    }

    #[test]
    fn task_0005_bounded_text_is_utf8_safe_and_reports_original_size() {
        let mut bounded = BoundedText::new(5);
        bounded.push_str("ab");
        bounded.push_str("€cd");
        assert_eq!(bounded.as_str(), "ab€");
        assert_eq!(bounded.original_bytes(), 7);
        assert!(bounded.truncated());
        assert!(std::str::from_utf8(bounded.as_str().as_bytes()).is_ok());
    }

    #[test]
    fn task_0005_changed_file_and_diff_bounds_are_visible() {
        let paths = (0..=MAX_CHANGED_FILES)
            .map(|index| format!("src/{index}.rs"))
            .collect();
        let bounded = bound_paths(paths);
        assert_eq!(bounded.paths.len(), MAX_CHANGED_FILES);
        assert_eq!(bounded.original_count, MAX_CHANGED_FILES + 1);
        assert!(bounded.truncated);

        let value = "x".repeat(MAX_DIFF_CHARS + 1);
        let (diff, original_bytes, truncated) = bound_diff(Some(value));
        assert_eq!(diff.unwrap().chars().count(), MAX_DIFF_CHARS);
        assert_eq!(original_bytes, MAX_DIFF_CHARS + 1);
        assert!(truncated);
    }

    #[test]
    fn task_0005_request_ids_are_bounded_and_unambiguous() {
        assert!(validate_request_id("task:41:550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_request_id("").is_err());
        assert!(validate_request_id("contains space").is_err());
        assert!(validate_request_id(&"a".repeat(MAX_RUN_REQUEST_ID_BYTES + 1)).is_err());
    }
}
