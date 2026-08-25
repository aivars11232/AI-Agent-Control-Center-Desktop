use crate::{
    run_coordinator::{MAX_CHANGED_FILES, MAX_CHANGED_FILE_BYTES, MAX_DIFF_BYTES},
    workspace_tools::{WorkspaceEvidenceListEntry, WorkspaceToolErrorKind, WorkspaceTools},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const WORKSPACE_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES: usize = 2 * 1024 * 1024;
const SNAPSHOT_ENTRY_LIMIT: usize = 20_000;
const SNAPSHOT_MILLIS: u64 = 5_000;
const SNAPSHOT_HASH_BYTES: u64 = 512 * 1024 * 1024;
const PER_FILE_DETAIL_BYTES: usize = 64 * 1024;
const AGGREGATE_DETAIL_BYTES: usize = 512 * 1024;
const GIT_STATUS_BYTES: usize = 4 * 1024 * 1024;
const GIT_COMMAND_MILLIS: u64 = 5_000;
const ISSUE_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceEvidenceMode {
    Git,
    Filesystem,
    NotCollected,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceEvidenceStatus {
    Complete,
    Partial,
    NotCollected,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceEvidenceReviewability {
    AgentEligible,
    HumanReviewRequired,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceEvidenceConsistency {
    ObservedDuringRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
    StatusChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceFileKind {
    File,
    Directory,
    BlockedSymlink,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceDetailKind {
    GitStaged,
    GitUnstaged,
    FilesystemPreview,
    Binary,
    Redacted,
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEvidenceLimitsV1 {
    pub snapshot_entry_limit: u64,
    pub snapshot_millis: u64,
    pub snapshot_hash_bytes: u64,
    pub persisted_change_limit: u64,
    pub persisted_path_bytes: u64,
    pub per_file_detail_bytes: u64,
    pub aggregate_detail_bytes: u64,
    pub git_status_bytes: u64,
    pub issue_limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEvidenceIssueV1 {
    pub code: String,
    pub message: String,
    pub phase: String,
    pub path: Option<String>,
    pub blocks_agent_approval: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceFileStateV1 {
    pub kind: WorkspaceFileKind,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub mode: u32,
    pub binary: Option<bool>,
    pub content_redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPathStateV1 {
    pub index_status: Option<String>,
    pub worktree_status: Option<String>,
    pub untracked: bool,
    pub conflicted: bool,
    pub previous_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceChangeEntryV1 {
    pub path: String,
    pub previous_path: Option<String>,
    pub change_kind: WorkspaceChangeKind,
    pub before: Option<WorkspaceFileStateV1>,
    pub after: Option<WorkspaceFileStateV1>,
    pub git_before: Option<GitPathStateV1>,
    pub git_after: Option<GitPathStateV1>,
    pub binary: bool,
    pub content_redacted: bool,
    pub detail_truncated: bool,
    pub human_review_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEvidenceDetailV1 {
    pub path: String,
    pub kind: WorkspaceDetailKind,
    pub content: Option<String>,
    pub original_bytes: u64,
    pub truncated: bool,
    pub redacted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceChangeSummaryV1 {
    pub total_changes: u64,
    pub retained_changes: u64,
    pub added: u64,
    pub modified: u64,
    pub deleted: u64,
    pub renamed: u64,
    pub type_changed: u64,
    pub status_changed: u64,
    pub staged: u64,
    pub unstaged: u64,
    pub untracked: u64,
    pub binary: u64,
    pub redacted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceChangeEvidenceV1 {
    pub schema_version: u32,
    pub mode: WorkspaceEvidenceMode,
    pub status: WorkspaceEvidenceStatus,
    pub reviewability: WorkspaceEvidenceReviewability,
    pub consistency: WorkspaceEvidenceConsistency,
    pub baseline_git_head: Option<String>,
    pub final_git_head: Option<String>,
    pub changes: Vec<WorkspaceChangeEntryV1>,
    pub details: Vec<WorkspaceEvidenceDetailV1>,
    pub summary: WorkspaceChangeSummaryV1,
    pub issues: Vec<WorkspaceEvidenceIssueV1>,
    pub issues_truncated: bool,
    pub before_snapshot_truncated: bool,
    pub after_snapshot_truncated: bool,
    pub changes_truncated: bool,
    pub details_truncated: bool,
    pub limits: WorkspaceEvidenceLimitsV1,
}

impl WorkspaceChangeEvidenceV1 {
    #[cfg(test)]
    pub fn complete_without_changes(mode: WorkspaceEvidenceMode) -> Self {
        debug_assert!(matches!(
            mode,
            WorkspaceEvidenceMode::Git | WorkspaceEvidenceMode::Filesystem
        ));
        Self::empty(
            mode,
            WorkspaceEvidenceStatus::Complete,
            WorkspaceEvidenceReviewability::AgentEligible,
        )
    }

    pub fn not_collected(reason: impl Into<String>) -> Self {
        let mut evidence = Self::empty(
            WorkspaceEvidenceMode::NotCollected,
            WorkspaceEvidenceStatus::NotCollected,
            WorkspaceEvidenceReviewability::Unavailable,
        );
        evidence.issues.push(issue(
            "EVIDENCE_NOT_COLLECTED",
            reason,
            "collection",
            None,
            true,
        ));
        evidence
    }

    pub fn legacy_unavailable(reason: impl Into<String>) -> Self {
        let mut evidence = Self::empty(
            WorkspaceEvidenceMode::Unavailable,
            WorkspaceEvidenceStatus::Unavailable,
            WorkspaceEvidenceReviewability::Unavailable,
        );
        evidence.issues.push(issue(
            "LEGACY_EVIDENCE_UNAVAILABLE",
            reason,
            "persistence",
            None,
            true,
        ));
        evidence
    }

    fn empty(
        mode: WorkspaceEvidenceMode,
        status: WorkspaceEvidenceStatus,
        reviewability: WorkspaceEvidenceReviewability,
    ) -> Self {
        Self {
            schema_version: WORKSPACE_EVIDENCE_SCHEMA_VERSION,
            mode,
            status,
            reviewability,
            consistency: WorkspaceEvidenceConsistency::ObservedDuringRun,
            baseline_git_head: None,
            final_git_head: None,
            changes: Vec::new(),
            details: Vec::new(),
            summary: WorkspaceChangeSummaryV1::default(),
            issues: Vec::new(),
            issues_truncated: false,
            before_snapshot_truncated: false,
            after_snapshot_truncated: false,
            changes_truncated: false,
            details_truncated: false,
            limits: EvidenceLimits::default().persisted(),
        }
    }

    pub fn is_complete_for_agent_approval(&self) -> bool {
        self.validate().is_ok()
            && self.status == WorkspaceEvidenceStatus::Complete
            && self.reviewability == WorkspaceEvidenceReviewability::AgentEligible
            && !self.issues_truncated
            && !self.before_snapshot_truncated
            && !self.after_snapshot_truncated
            && !self.changes_truncated
            && !self.details_truncated
            && !self.issues.iter().any(|entry| entry.blocks_agent_approval)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != WORKSPACE_EVIDENCE_SCHEMA_VERSION {
            return Err("workspace evidence uses an unsupported schema version".to_string());
        }
        if self.changes.len() as u64 > self.limits.persisted_change_limit
            || self.changes.len() > MAX_CHANGED_FILES
        {
            return Err("workspace evidence exceeds its retained change limit".to_string());
        }
        let path_bytes = self.changes.iter().try_fold(0usize, |total, change| {
            if !safe_evidence_path(&change.path)
                || change
                    .previous_path
                    .as_deref()
                    .is_some_and(|path| !safe_evidence_path(path))
            {
                return Err("workspace evidence contains an unsafe path".to_string());
            }
            if change.content_redacted
                && change
                    .before
                    .iter()
                    .chain(change.after.iter())
                    .any(|state| state.sha256.is_some())
            {
                return Err("redacted workspace evidence contains a persisted hash".to_string());
            }
            total
                .checked_add(change.path.len())
                .and_then(|total| {
                    total.checked_add(change.previous_path.as_ref().map_or(0, String::len))
                })
                .ok_or_else(|| "workspace evidence path size overflowed".to_string())
        })?;
        if path_bytes as u64 > self.limits.persisted_path_bytes
            || path_bytes > MAX_CHANGED_FILE_BYTES
        {
            return Err("workspace evidence exceeds its retained path budget".to_string());
        }
        let mut detail_bytes = 0usize;
        for detail in &self.details {
            if !safe_evidence_path(&detail.path) {
                return Err("workspace evidence detail contains an unsafe path".to_string());
            }
            let bytes = detail.content.as_ref().map_or(0, String::len);
            if bytes as u64 > self.limits.per_file_detail_bytes {
                return Err("workspace evidence exceeds its per-file detail budget".to_string());
            }
            detail_bytes = detail_bytes
                .checked_add(bytes)
                .ok_or_else(|| "workspace evidence detail size overflowed".to_string())?;
        }
        if detail_bytes as u64 > self.limits.aggregate_detail_bytes {
            return Err("workspace evidence exceeds its aggregate detail budget".to_string());
        }
        if self.issues.len() as u64 > self.limits.issue_limit {
            return Err("workspace evidence exceeds its issue limit".to_string());
        }
        if self.summary.retained_changes != self.changes.len() as u64
            || self.summary.total_changes < self.summary.retained_changes
        {
            return Err(
                "workspace evidence summary does not match its retained changes".to_string(),
            );
        }
        Ok(())
    }

    pub fn compatibility_paths(&self) -> Vec<String> {
        self.changes
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn compatibility_diff(&self) -> Option<String> {
        let mut output = String::new();
        for detail in &self.details {
            let Some(content) = detail.content.as_deref() else {
                continue;
            };
            let heading = format!("=== {:?}: {} ===\n", detail.kind, detail.path);
            if output.len().saturating_add(heading.len()) > MAX_DIFF_BYTES {
                break;
            }
            output.push_str(&heading);
            let remaining = MAX_DIFF_BYTES.saturating_sub(output.len());
            push_char_bounded(&mut output, content, remaining);
            if output.len() < MAX_DIFF_BYTES {
                output.push('\n');
            }
        }
        (!output.trim().is_empty()).then(|| output.trim().to_string())
    }
}

#[derive(Debug, Clone)]
struct EvidenceLimits {
    snapshot_entries: usize,
    snapshot_millis: u64,
    snapshot_hash_bytes: u64,
    retained_changes: usize,
    retained_path_bytes: usize,
    per_file_detail_bytes: usize,
    aggregate_detail_bytes: usize,
    git_status_bytes: usize,
    git_command_millis: u64,
    issue_limit: usize,
}

impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            snapshot_entries: SNAPSHOT_ENTRY_LIMIT,
            snapshot_millis: SNAPSHOT_MILLIS,
            snapshot_hash_bytes: SNAPSHOT_HASH_BYTES,
            retained_changes: MAX_CHANGED_FILES,
            retained_path_bytes: MAX_CHANGED_FILE_BYTES,
            per_file_detail_bytes: PER_FILE_DETAIL_BYTES,
            aggregate_detail_bytes: AGGREGATE_DETAIL_BYTES,
            git_status_bytes: GIT_STATUS_BYTES,
            git_command_millis: GIT_COMMAND_MILLIS,
            issue_limit: ISSUE_LIMIT,
        }
    }
}

impl EvidenceLimits {
    fn persisted(&self) -> WorkspaceEvidenceLimitsV1 {
        WorkspaceEvidenceLimitsV1 {
            snapshot_entry_limit: self.snapshot_entries as u64,
            snapshot_millis: self.snapshot_millis,
            snapshot_hash_bytes: self.snapshot_hash_bytes,
            persisted_change_limit: self.retained_changes as u64,
            persisted_path_bytes: self.retained_path_bytes as u64,
            per_file_detail_bytes: self.per_file_detail_bytes as u64,
            aggregate_detail_bytes: self.aggregate_detail_bytes as u64,
            git_status_bytes: self.git_status_bytes as u64,
            issue_limit: self.issue_limit as u64,
        }
    }
}

#[derive(Debug, Clone)]
struct SnapshotState {
    kind: WorkspaceFileKind,
    size_bytes: Option<u64>,
    mode: u32,
    modified_seconds: i64,
    modified_nanos: u64,
    changed_seconds: i64,
    changed_nanos: u64,
    device: u64,
    inode: u64,
    sha256: Option<String>,
    preview: Option<Vec<u8>>,
    preview_truncated: bool,
    binary: Option<bool>,
    sensitive: bool,
}

impl SnapshotState {
    fn persisted(&self) -> WorkspaceFileStateV1 {
        WorkspaceFileStateV1 {
            kind: self.kind,
            size_bytes: self.size_bytes,
            sha256: (!self.sensitive).then(|| self.sha256.clone()).flatten(),
            mode: self.mode,
            binary: self.binary,
            content_redacted: self.sensitive,
        }
    }

    fn materially_equals(&self, other: &Self) -> bool {
        if self.kind != other.kind || self.mode != other.mode {
            return false;
        }
        match self.kind {
            WorkspaceFileKind::Directory => true,
            WorkspaceFileKind::File => {
                if let (Some(left), Some(right)) = (&self.sha256, &other.sha256) {
                    self.size_bytes == other.size_bytes && left == right
                } else {
                    self.size_bytes == other.size_bytes
                        && self.modified_seconds == other.modified_seconds
                        && self.modified_nanos == other.modified_nanos
                        && self.changed_seconds == other.changed_seconds
                        && self.changed_nanos == other.changed_nanos
                        && self.device == other.device
                        && self.inode == other.inode
                }
            }
            WorkspaceFileKind::BlockedSymlink | WorkspaceFileKind::Unsupported => {
                self.size_bytes == other.size_bytes
                    && self.modified_seconds == other.modified_seconds
                    && self.modified_nanos == other.modified_nanos
                    && self.changed_seconds == other.changed_seconds
                    && self.changed_nanos == other.changed_nanos
                    && self.device == other.device
                    && self.inode == other.inode
            }
        }
    }
}

#[derive(Debug, Default)]
struct WorkspaceSnapshotV1 {
    entries: BTreeMap<String, SnapshotState>,
    issues: Vec<WorkspaceEvidenceIssueV1>,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitPathState {
    index_status: Option<char>,
    worktree_status: Option<char>,
    untracked: bool,
    conflicted: bool,
    previous_path: Option<String>,
}

impl GitPathState {
    fn persisted(&self) -> GitPathStateV1 {
        GitPathStateV1 {
            index_status: self.index_status.map(|value| value.to_string()),
            worktree_status: self.worktree_status.map(|value| value.to_string()),
            untracked: self.untracked,
            conflicted: self.conflicted,
            previous_path: self.previous_path.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct GitSnapshot {
    head: Option<String>,
    paths: BTreeMap<String, GitPathState>,
    issues: Vec<WorkspaceEvidenceIssueV1>,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitContext {
    filter_overrides: Vec<(String, String)>,
}

pub(crate) struct WorkspaceEvidenceBaseline {
    workspace: PathBuf,
    tools: Option<WorkspaceTools>,
    before: WorkspaceSnapshotV1,
    before_git: GitSnapshot,
    git_context: Option<GitContext>,
    mode: WorkspaceEvidenceMode,
    initial_issues: Vec<WorkspaceEvidenceIssueV1>,
    limits: EvidenceLimits,
}

impl WorkspaceEvidenceBaseline {
    pub(crate) fn begin(workspace: &Path) -> Self {
        Self::begin_with_limits(workspace, EvidenceLimits::default())
    }

    fn begin_with_limits(workspace: &Path, limits: EvidenceLimits) -> Self {
        let mut initial_issues = Vec::new();
        let tools = match WorkspaceTools::open(workspace) {
            Ok(tools) => Some(tools),
            Err(error) => {
                initial_issues.push(issue(
                    "WORKSPACE_OPEN_FAILED",
                    "The workspace could not be opened through the descriptor-confined evidence boundary.",
                    "before",
                    None,
                    true,
                ));
                let _ = error;
                None
            }
        };
        let before = tools.as_ref().map_or_else(
            || WorkspaceSnapshotV1 {
                truncated: true,
                ..WorkspaceSnapshotV1::default()
            },
            |tools| capture_workspace_snapshot(tools, "before", &limits),
        );
        let (mode, git_context) = inspect_git_context(workspace, &limits, &mut initial_issues);
        let before_git = git_context
            .as_ref()
            .map_or_else(GitSnapshot::default, |context| {
                capture_git_snapshot(workspace, context, "before", &limits)
            });
        Self {
            workspace: workspace.to_path_buf(),
            tools,
            before,
            before_git,
            git_context,
            mode,
            initial_issues,
            limits,
        }
    }

    pub(crate) fn finish(self) -> WorkspaceChangeEvidenceV1 {
        let after = self.tools.as_ref().map_or_else(
            || WorkspaceSnapshotV1 {
                truncated: true,
                ..WorkspaceSnapshotV1::default()
            },
            |tools| capture_workspace_snapshot(tools, "after", &self.limits),
        );
        let mut post_inspection_issues = Vec::new();
        let after_git_context = if self.mode == WorkspaceEvidenceMode::Git {
            let (after_mode, context) =
                inspect_git_context(&self.workspace, &self.limits, &mut post_inspection_issues);
            if after_mode != WorkspaceEvidenceMode::Git {
                None
            } else {
                context
            }
        } else {
            None
        };
        if self.git_context != after_git_context && self.mode == WorkspaceEvidenceMode::Git {
            post_inspection_issues.push(issue(
                "GIT_FILTER_CONFIGURATION_CHANGED",
                "The repository filter configuration changed while the run was active; both inspections remained helper-neutralized.",
                "after",
                None,
                true,
            ));
        }
        let after_git = after_git_context
            .as_ref()
            .map_or_else(GitSnapshot::default, |context| {
                capture_git_snapshot(&self.workspace, context, "after", &self.limits)
            });

        let mut all_issues = self.initial_issues;
        all_issues.extend(post_inspection_issues);
        all_issues.extend(self.before.issues.clone());
        all_issues.extend(after.issues.clone());
        all_issues.extend(self.before_git.issues.clone());
        all_issues.extend(after_git.issues.clone());
        if self.mode == WorkspaceEvidenceMode::Git && self.before_git.head != after_git.head {
            all_issues.push(issue(
                "GIT_HEAD_CHANGED",
                "Repository HEAD changed during the run; automatic review cannot infer the complete history operation.",
                "comparison",
                None,
                true,
            ));
        }

        let mut changes = compare_workspace_snapshots(
            &self.before,
            &after,
            &self.before_git,
            &after_git,
            self.mode,
        );
        let original_change_count = changes.len();
        let mut retained_path_bytes = 0usize;
        let mut retained = Vec::new();
        for change in changes.drain(..) {
            let path_bytes = change
                .path
                .len()
                .saturating_add(change.previous_path.as_ref().map_or(0, String::len));
            if retained.len() >= self.limits.retained_changes
                || retained_path_bytes.saturating_add(path_bytes) > self.limits.retained_path_bytes
            {
                break;
            }
            retained_path_bytes = retained_path_bytes.saturating_add(path_bytes);
            retained.push(change);
        }
        let changes_truncated = retained.len() < original_change_count;
        if changes_truncated {
            all_issues.push(issue(
                "CHANGE_RECORD_LIMIT_REACHED",
                "Workspace changes exceeded the bounded persisted change or path budget.",
                "comparison",
                None,
                true,
            ));
        }

        let (details, details_truncated, detail_issues) = collect_details(
            &self.workspace,
            after_git_context.as_ref(),
            &self.before,
            &after,
            &mut retained,
            &self.limits,
        );
        all_issues.extend(detail_issues);

        if detect_repository_mode(&self.workspace) != self.mode {
            all_issues.push(issue(
                "WORKSPACE_MODE_CHANGED",
                "The workspace Git/non-Git mode changed while the run was active.",
                "after",
                None,
                true,
            ));
        }

        let before_snapshot_truncated = self.before.truncated || self.before_git.truncated;
        let after_snapshot_truncated = after.truncated || after_git.truncated;
        let summary = summarize_changes(&retained, original_change_count);
        let mut issues_truncated = false;
        if all_issues.len() > self.limits.issue_limit {
            all_issues.truncate(self.limits.issue_limit);
            issues_truncated = true;
        }
        let partial = before_snapshot_truncated
            || after_snapshot_truncated
            || changes_truncated
            || details_truncated
            || issues_truncated
            || all_issues.iter().any(|entry| entry.blocks_agent_approval);
        let human_only = partial || retained.iter().any(|entry| entry.human_review_required);
        let status = if self.tools.is_none() {
            WorkspaceEvidenceStatus::Unavailable
        } else if partial {
            WorkspaceEvidenceStatus::Partial
        } else {
            WorkspaceEvidenceStatus::Complete
        };
        let reviewability = if status == WorkspaceEvidenceStatus::Unavailable {
            WorkspaceEvidenceReviewability::Unavailable
        } else if human_only {
            WorkspaceEvidenceReviewability::HumanReviewRequired
        } else {
            WorkspaceEvidenceReviewability::AgentEligible
        };

        WorkspaceChangeEvidenceV1 {
            schema_version: WORKSPACE_EVIDENCE_SCHEMA_VERSION,
            mode: if status == WorkspaceEvidenceStatus::Unavailable {
                WorkspaceEvidenceMode::Unavailable
            } else {
                self.mode
            },
            status,
            reviewability,
            consistency: WorkspaceEvidenceConsistency::ObservedDuringRun,
            baseline_git_head: self.before_git.head,
            final_git_head: after_git.head,
            changes: retained,
            details,
            summary,
            issues: all_issues,
            issues_truncated,
            before_snapshot_truncated,
            after_snapshot_truncated,
            changes_truncated,
            details_truncated,
            limits: self.limits.persisted(),
        }
    }
}

fn capture_workspace_snapshot(
    tools: &WorkspaceTools,
    phase: &str,
    limits: &EvidenceLimits,
) -> WorkspaceSnapshotV1 {
    let deadline = Instant::now() + Duration::from_millis(limits.snapshot_millis);
    let mut snapshot = WorkspaceSnapshotV1::default();
    let mut pending = BTreeSet::from([".".to_string()]);
    let mut hash_bytes = 0_u64;
    let mut preview_bytes = 0usize;

    while let Some(directory) = pending.pop_first() {
        if Instant::now() >= deadline || snapshot.entries.len() >= limits.snapshot_entries {
            snapshot.truncated = true;
            push_issue_bounded(
                &mut snapshot.issues,
                limits.issue_limit,
                issue(
                    "SNAPSHOT_LIMIT_REACHED",
                    "The workspace snapshot reached its entry or time bound.",
                    phase,
                    Some(directory),
                    true,
                ),
            );
            break;
        }
        let remaining = limits
            .snapshot_entries
            .saturating_sub(snapshot.entries.len());
        let listing = match tools.evidence_list(&directory, remaining.max(1), deadline) {
            Ok(listing) => listing,
            Err(error) => {
                snapshot.truncated = true;
                let code = match error.kind {
                    WorkspaceToolErrorKind::Unsupported => "SNAPSHOT_UNSUPPORTED",
                    WorkspaceToolErrorKind::Limit => "SNAPSHOT_LIMIT_REACHED",
                    _ => "SNAPSHOT_DIRECTORY_FAILED",
                };
                push_issue_bounded(
                    &mut snapshot.issues,
                    limits.issue_limit,
                    issue(
                        code,
                        "A workspace directory could not be completely inspected.",
                        phase,
                        Some(directory),
                        true,
                    ),
                );
                continue;
            }
        };
        if listing.truncated {
            snapshot.truncated = true;
            push_issue_bounded(
                &mut snapshot.issues,
                limits.issue_limit,
                issue(
                    if listing.timed_out {
                        "SNAPSHOT_TIME_LIMIT_REACHED"
                    } else {
                        "SNAPSHOT_ENTRY_LIMIT_REACHED"
                    },
                    "A workspace directory listing was explicitly truncated.",
                    phase,
                    Some(directory.clone()),
                    true,
                ),
            );
        }
        for entry in listing.entries {
            let state = snapshot_state_from_entry(
                tools,
                &entry,
                phase,
                limits,
                deadline,
                &mut hash_bytes,
                &mut preview_bytes,
                &mut snapshot.issues,
                &mut snapshot.truncated,
            );
            if state.kind == WorkspaceFileKind::Directory {
                pending.insert(entry.path.clone());
            }
            snapshot.entries.insert(entry.path, state);
        }
        if listing.truncated {
            break;
        }
    }
    snapshot
}

#[allow(clippy::too_many_arguments)]
fn snapshot_state_from_entry(
    tools: &WorkspaceTools,
    entry: &WorkspaceEvidenceListEntry,
    phase: &str,
    limits: &EvidenceLimits,
    deadline: Instant,
    hash_bytes: &mut u64,
    preview_bytes: &mut usize,
    issues: &mut Vec<WorkspaceEvidenceIssueV1>,
    truncated: &mut bool,
) -> SnapshotState {
    let kind = match entry.kind.as_str() {
        "file" => WorkspaceFileKind::File,
        "directory" => WorkspaceFileKind::Directory,
        "blocked-symlink" => WorkspaceFileKind::BlockedSymlink,
        _ => WorkspaceFileKind::Unsupported,
    };
    let sensitive = is_sensitive_path(&entry.path);
    let mut state = SnapshotState {
        kind,
        size_bytes: entry.size_bytes,
        mode: entry.mode,
        modified_seconds: entry.modified_seconds,
        modified_nanos: entry.modified_nanos,
        changed_seconds: entry.changed_seconds,
        changed_nanos: entry.changed_nanos,
        device: entry.device,
        inode: entry.inode,
        sha256: None,
        preview: None,
        preview_truncated: false,
        binary: None,
        sensitive,
    };
    if kind != WorkspaceFileKind::File {
        if matches!(
            kind,
            WorkspaceFileKind::BlockedSymlink | WorkspaceFileKind::Unsupported
        ) {
            push_issue_bounded(
                issues,
                limits.issue_limit,
                issue(
                    if kind == WorkspaceFileKind::BlockedSymlink {
                        "SYMLINK_NOT_FOLLOWED"
                    } else {
                        "SPECIAL_FILE_NOT_READ"
                    },
                    "The workspace entry was recorded as metadata and was never followed or read.",
                    phase,
                    Some(entry.path.clone()),
                    true,
                ),
            );
        }
        return state;
    }

    let remaining_hash = limits.snapshot_hash_bytes.saturating_sub(*hash_bytes);
    let preview_limit = if sensitive {
        0
    } else {
        limits
            .per_file_detail_bytes
            .min(limits.aggregate_detail_bytes.saturating_sub(*preview_bytes))
    };
    if remaining_hash == 0 || entry.size_bytes.is_some_and(|size| size > remaining_hash) {
        *truncated = true;
        state.preview_truncated = true;
        push_issue_bounded(
            issues,
            limits.issue_limit,
            issue(
                "SNAPSHOT_HASH_BUDGET_REACHED",
                "A file exceeded the remaining bounded snapshot hash budget.",
                phase,
                Some(entry.path.clone()),
                true,
            ),
        );
        return state;
    }
    match tools.evidence_file(&entry.path, remaining_hash, preview_limit, deadline) {
        Ok(file) => {
            *hash_bytes = hash_bytes.saturating_add(file.size_bytes);
            *preview_bytes = preview_bytes.saturating_add(file.preview.len());
            state.size_bytes = Some(file.size_bytes);
            state.mode = file.mode;
            state.sha256 = Some(file.sha256);
            state.binary = Some(file.binary);
            state.preview_truncated = file.preview_truncated;
            if !sensitive {
                state.preview = Some(file.preview);
            }
        }
        Err(error) => {
            *truncated = true;
            state.preview_truncated = true;
            let code = match error.kind {
                WorkspaceToolErrorKind::Conflict => "SNAPSHOT_FILE_CHANGED_DURING_READ",
                WorkspaceToolErrorKind::Limit => "SNAPSHOT_FILE_LIMIT_REACHED",
                WorkspaceToolErrorKind::Unsupported => "SNAPSHOT_FILE_UNSUPPORTED",
                _ => "SNAPSHOT_FILE_READ_FAILED",
            };
            push_issue_bounded(
                issues,
                limits.issue_limit,
                issue(
                    code,
                    "A regular file could not be completely hashed through the workspace boundary.",
                    phase,
                    Some(entry.path.clone()),
                    true,
                ),
            );
        }
    }
    state
}

fn compare_workspace_snapshots(
    before: &WorkspaceSnapshotV1,
    after: &WorkspaceSnapshotV1,
    before_git: &GitSnapshot,
    after_git: &GitSnapshot,
    mode: WorkspaceEvidenceMode,
) -> Vec<WorkspaceChangeEntryV1> {
    let mut paths = BTreeSet::new();
    paths.extend(before.entries.keys().cloned());
    paths.extend(after.entries.keys().cloned());
    let mut changes = BTreeMap::new();
    for path in paths {
        let old = before.entries.get(&path);
        let new = after.entries.get(&path);
        let kind = match (old, new) {
            (None, Some(_)) => Some(WorkspaceChangeKind::Added),
            (Some(_), None) => Some(WorkspaceChangeKind::Deleted),
            (Some(old), Some(new)) if old.kind != new.kind => {
                Some(WorkspaceChangeKind::TypeChanged)
            }
            (Some(old), Some(new)) if !old.materially_equals(new) => {
                Some(WorkspaceChangeKind::Modified)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            changes.insert(
                path.clone(),
                make_change(
                    path.clone(),
                    None,
                    kind,
                    old,
                    new,
                    before_git.paths.get(&path),
                    after_git.paths.get(&path),
                ),
            );
        }
    }

    if mode == WorkspaceEvidenceMode::Git {
        let mut git_paths = BTreeSet::new();
        git_paths.extend(before_git.paths.keys().cloned());
        git_paths.extend(after_git.paths.keys().cloned());
        for path in git_paths {
            let old = before_git.paths.get(&path);
            let new = after_git.paths.get(&path);
            if old == new || changes.contains_key(&path) {
                continue;
            }
            changes.insert(
                path.clone(),
                make_change(
                    path.clone(),
                    new.and_then(|value| value.previous_path.clone()),
                    WorkspaceChangeKind::StatusChanged,
                    before.entries.get(&path),
                    after.entries.get(&path),
                    old,
                    new,
                ),
            );
        }
        coalesce_git_renames(&mut changes, before, after, before_git, after_git);
    } else {
        coalesce_unique_content_renames(&mut changes);
    }
    changes.into_values().collect()
}

fn make_change(
    path: String,
    previous_path: Option<String>,
    change_kind: WorkspaceChangeKind,
    before: Option<&SnapshotState>,
    after: Option<&SnapshotState>,
    git_before: Option<&GitPathState>,
    git_after: Option<&GitPathState>,
) -> WorkspaceChangeEntryV1 {
    let binary = after
        .and_then(|state| state.binary)
        .or_else(|| before.and_then(|state| state.binary))
        .unwrap_or(false);
    let content_redacted = is_sensitive_path(&path)
        || previous_path.as_deref().is_some_and(is_sensitive_path)
        || after.is_some_and(|state| state.sensitive)
        || before.is_some_and(|state| state.sensitive);
    let unsupported = before.into_iter().chain(after).any(|state| {
        matches!(
            state.kind,
            WorkspaceFileKind::BlockedSymlink | WorkspaceFileKind::Unsupported
        ) || (state.kind == WorkspaceFileKind::File && state.sha256.is_none())
    });
    WorkspaceChangeEntryV1 {
        path,
        previous_path,
        change_kind,
        before: before.map(SnapshotState::persisted),
        after: after.map(SnapshotState::persisted),
        git_before: git_before.map(GitPathState::persisted),
        git_after: git_after.map(GitPathState::persisted),
        binary,
        content_redacted,
        detail_truncated: false,
        human_review_required: binary || content_redacted || unsupported,
    }
}

fn coalesce_git_renames(
    changes: &mut BTreeMap<String, WorkspaceChangeEntryV1>,
    before: &WorkspaceSnapshotV1,
    after: &WorkspaceSnapshotV1,
    before_git: &GitSnapshot,
    after_git: &GitSnapshot,
) {
    let renames = after_git
        .paths
        .iter()
        .filter_map(|(path, state)| {
            let previous = state.previous_path.as_ref()?;
            (before_git.paths.get(path) != Some(state)).then(|| (path.clone(), previous.clone()))
        })
        .collect::<Vec<_>>();
    for (path, previous) in renames {
        if !changes.contains_key(&path) && !changes.contains_key(&previous) {
            continue;
        }
        changes.remove(&previous);
        changes.insert(
            path.clone(),
            make_change(
                path.clone(),
                Some(previous.clone()),
                WorkspaceChangeKind::Renamed,
                before.entries.get(&previous),
                after.entries.get(&path),
                before_git.paths.get(&previous),
                after_git.paths.get(&path),
            ),
        );
    }
}

fn coalesce_unique_content_renames(changes: &mut BTreeMap<String, WorkspaceChangeEntryV1>) {
    let mut deleted: HashMap<(u64, String), Vec<String>> = HashMap::new();
    let mut added: HashMap<(u64, String), Vec<String>> = HashMap::new();
    for (path, change) in changes.iter() {
        let state = match change.change_kind {
            WorkspaceChangeKind::Deleted => change.before.as_ref(),
            WorkspaceChangeKind::Added => change.after.as_ref(),
            _ => None,
        };
        let Some(state) = state else { continue };
        if state.kind != WorkspaceFileKind::File {
            continue;
        }
        let (Some(size), Some(hash)) = (state.size_bytes, state.sha256.clone()) else {
            continue;
        };
        let target = if change.change_kind == WorkspaceChangeKind::Deleted {
            &mut deleted
        } else {
            &mut added
        };
        target.entry((size, hash)).or_default().push(path.clone());
    }
    let pairs = deleted
        .into_iter()
        .filter_map(|(key, old_paths)| {
            let new_paths = added.get(&key)?;
            (old_paths.len() == 1 && new_paths.len() == 1)
                .then(|| (old_paths[0].clone(), new_paths[0].clone()))
        })
        .collect::<Vec<_>>();
    for (old_path, new_path) in pairs {
        let Some(old) = changes.remove(&old_path) else {
            continue;
        };
        let Some(mut new) = changes.remove(&new_path) else {
            changes.insert(old_path, old);
            continue;
        };
        new.previous_path = Some(old.path);
        new.change_kind = WorkspaceChangeKind::Renamed;
        new.before = old.before;
        changes.insert(new_path, new);
    }
}

fn collect_details(
    workspace: &Path,
    git_context: Option<&GitContext>,
    before: &WorkspaceSnapshotV1,
    after: &WorkspaceSnapshotV1,
    changes: &mut [WorkspaceChangeEntryV1],
    limits: &EvidenceLimits,
) -> (
    Vec<WorkspaceEvidenceDetailV1>,
    bool,
    Vec<WorkspaceEvidenceIssueV1>,
) {
    let mut details = Vec::new();
    let mut aggregate_bytes = 0usize;
    let mut truncated = false;
    let mut issues = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(limits.git_command_millis);

    for change in changes {
        if aggregate_bytes >= limits.aggregate_detail_bytes || Instant::now() >= deadline {
            change.detail_truncated = true;
            change.human_review_required = true;
            truncated = true;
            continue;
        }
        if change.content_redacted {
            details.push(WorkspaceEvidenceDetailV1 {
                path: change.path.clone(),
                kind: WorkspaceDetailKind::Redacted,
                content: None,
                original_bytes: 0,
                truncated: false,
                redacted: true,
            });
            continue;
        }
        if change.binary {
            details.push(WorkspaceEvidenceDetailV1 {
                path: change.path.clone(),
                kind: WorkspaceDetailKind::Binary,
                content: None,
                original_bytes: change
                    .after
                    .as_ref()
                    .and_then(|state| state.size_bytes)
                    .or_else(|| change.before.as_ref().and_then(|state| state.size_bytes))
                    .unwrap_or(0),
                truncated: false,
                redacted: false,
            });
            continue;
        }

        let mut produced_git_detail = false;
        if let (Some(context), Some(git_after)) = (git_context, change.git_after.as_ref()) {
            for (kind, cached, needed) in [
                (
                    WorkspaceDetailKind::GitStaged,
                    true,
                    git_after.index_status.is_some(),
                ),
                (
                    WorkspaceDetailKind::GitUnstaged,
                    false,
                    git_after.worktree_status.is_some(),
                ),
            ] {
                if !needed || Instant::now() >= deadline {
                    continue;
                }
                let remaining_time = deadline.saturating_duration_since(Instant::now());
                let capture = capture_git_diff(
                    workspace,
                    context,
                    &change.path,
                    cached,
                    limits.per_file_detail_bytes,
                    remaining_time,
                );
                match capture {
                    Ok(Some(capture)) => {
                        if git_diff_is_binary(&capture.text) {
                            change.binary = true;
                            change.human_review_required = true;
                            change.detail_truncated |= capture.truncated;
                            truncated |= capture.truncated;
                            details.push(WorkspaceEvidenceDetailV1 {
                                path: change.path.clone(),
                                kind: WorkspaceDetailKind::Binary,
                                content: None,
                                original_bytes: capture.original_bytes,
                                truncated: capture.truncated,
                                redacted: false,
                            });
                            produced_git_detail = true;
                            break;
                        }
                        let (content, redacted) = redact_text(&capture.text);
                        if redacted {
                            redact_change_hashes(change);
                        }
                        let available = limits
                            .aggregate_detail_bytes
                            .saturating_sub(aggregate_bytes);
                        let mut bounded = String::new();
                        push_char_bounded(
                            &mut bounded,
                            &content,
                            limits.per_file_detail_bytes.min(available),
                        );
                        let detail_truncated = capture.truncated || bounded.len() < content.len();
                        aggregate_bytes = aggregate_bytes.saturating_add(bounded.len());
                        change.detail_truncated |= detail_truncated;
                        change.human_review_required |= detail_truncated || redacted;
                        truncated |= detail_truncated;
                        details.push(WorkspaceEvidenceDetailV1 {
                            path: change.path.clone(),
                            kind,
                            content: Some(bounded),
                            original_bytes: capture.original_bytes,
                            truncated: detail_truncated,
                            redacted,
                        });
                        produced_git_detail = true;
                    }
                    Ok(None) => {}
                    Err(code) => {
                        change.detail_truncated = true;
                        change.human_review_required = true;
                        truncated = true;
                        push_issue_bounded(
                            &mut issues,
                            limits.issue_limit,
                            issue(
                                code,
                                "A bounded Git detail could not be collected safely.",
                                "details",
                                Some(change.path.clone()),
                                true,
                            ),
                        );
                    }
                }
            }
        }
        if produced_git_detail {
            continue;
        }

        let old_path = change.previous_path.as_deref().unwrap_or(&change.path);
        let old = before.entries.get(old_path);
        let new = after.entries.get(&change.path);
        let (content, original_bytes, preview_truncated) = filesystem_preview(old, new);
        if let Some(content) = content {
            let (content, redacted) = redact_text(&content);
            if redacted {
                redact_change_hashes(change);
            }
            let available = limits
                .aggregate_detail_bytes
                .saturating_sub(aggregate_bytes);
            let mut bounded = String::new();
            push_char_bounded(
                &mut bounded,
                &content,
                limits.per_file_detail_bytes.min(available),
            );
            let detail_truncated = preview_truncated || bounded.len() < content.len();
            aggregate_bytes = aggregate_bytes.saturating_add(bounded.len());
            change.detail_truncated |= detail_truncated;
            change.human_review_required |= detail_truncated || redacted;
            truncated |= detail_truncated;
            details.push(WorkspaceEvidenceDetailV1 {
                path: change.path.clone(),
                kind: WorkspaceDetailKind::FilesystemPreview,
                content: Some(bounded),
                original_bytes,
                truncated: detail_truncated,
                redacted,
            });
        } else {
            change.human_review_required = true;
            details.push(WorkspaceEvidenceDetailV1 {
                path: change.path.clone(),
                kind: WorkspaceDetailKind::MetadataOnly,
                content: None,
                original_bytes: 0,
                truncated: false,
                redacted: false,
            });
        }
    }
    if truncated {
        push_issue_bounded(
            &mut issues,
            limits.issue_limit,
            issue(
                "DETAIL_LIMIT_REACHED",
                "Workspace detail collection reached a per-file, aggregate, or time bound.",
                "details",
                None,
                true,
            ),
        );
    }
    (details, truncated, issues)
}

fn git_diff_is_binary(value: &str) -> bool {
    value
        .lines()
        .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch")
}

fn redact_change_hashes(change: &mut WorkspaceChangeEntryV1) {
    change.content_redacted = true;
    for state in change.before.iter_mut().chain(change.after.iter_mut()) {
        state.sha256 = None;
        state.content_redacted = true;
    }
}

fn filesystem_preview(
    before: Option<&SnapshotState>,
    after: Option<&SnapshotState>,
) -> (Option<String>, u64, bool) {
    let old = before.and_then(|state| state.preview.as_deref());
    let new = after.and_then(|state| state.preview.as_deref());
    if old.is_none() && new.is_none() {
        return (None, 0, false);
    }
    let mut content = String::new();
    let mut original = 0_u64;
    if let Some(old) = old {
        original = original.saturating_add(old.len() as u64);
        content.push_str("--- before (bounded preview)\n");
        content.push_str(&String::from_utf8_lossy(old));
        content.push('\n');
    }
    if let Some(new) = new {
        original = original.saturating_add(new.len() as u64);
        content.push_str("+++ after (bounded preview)\n");
        content.push_str(&String::from_utf8_lossy(new));
        content.push('\n');
    }
    (
        Some(content),
        original,
        before.is_some_and(|state| state.preview_truncated)
            || after.is_some_and(|state| state.preview_truncated),
    )
}

fn inspect_git_context(
    workspace: &Path,
    limits: &EvidenceLimits,
    issues: &mut Vec<WorkspaceEvidenceIssueV1>,
) -> (WorkspaceEvidenceMode, Option<GitContext>) {
    match detect_repository_mode(workspace) {
        WorkspaceEvidenceMode::Git => match configured_filter_overrides(workspace, limits) {
            Ok(overrides) => (
                WorkspaceEvidenceMode::Git,
                Some(GitContext {
                    filter_overrides: overrides,
                }),
            ),
            Err(code) => {
                push_issue_bounded(
                        issues,
                        limits.issue_limit,
                        issue(
                            code,
                            "Git filters could not be neutralized safely; evidence fell back to descriptor-confined filesystem inspection.",
                            "before",
                            None,
                            true,
                        ),
                    );
                (WorkspaceEvidenceMode::Filesystem, None)
            }
        },
        WorkspaceEvidenceMode::Filesystem => (WorkspaceEvidenceMode::Filesystem, None),
        _ => {
            push_issue_bounded(
                issues,
                limits.issue_limit,
                issue(
                    "GIT_LAYOUT_UNSUPPORTED",
                    "The workspace uses a linked, symlinked, or otherwise unsupported Git metadata layout; evidence fell back to filesystem inspection.",
                    "before",
                    None,
                    true,
                ),
            );
            (WorkspaceEvidenceMode::Filesystem, None)
        }
    }
}

fn detect_repository_mode(workspace: &Path) -> WorkspaceEvidenceMode {
    match std::fs::symlink_metadata(workspace.join(".git")) {
        Ok(metadata) if metadata.file_type().is_dir() => WorkspaceEvidenceMode::Git,
        Ok(_) => WorkspaceEvidenceMode::Unavailable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            WorkspaceEvidenceMode::Filesystem
        }
        Err(_) => WorkspaceEvidenceMode::Unavailable,
    }
}

fn configured_filter_overrides(
    workspace: &Path,
    limits: &EvidenceLimits,
) -> Result<Vec<(String, String)>, &'static str> {
    let capture = run_git_command(
        workspace,
        &[],
        &[
            "config",
            "--local",
            "--includes",
            "--null",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|smudge|process|required)$",
        ],
        limits.git_status_bytes,
        Duration::from_millis(limits.git_command_millis),
    )?;
    if capture.timed_out || capture.truncated {
        return Err("GIT_FILTER_CONFIGURATION_LIMIT_REACHED");
    }
    if !matches!(capture.status, Some(0 | 1)) {
        return Err("GIT_FILTER_CONFIGURATION_FAILED");
    }
    let mut filter_names = BTreeSet::new();
    for raw in capture
        .bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
    {
        let key = std::str::from_utf8(raw)
            .map_err(|_| "GIT_FILTER_CONFIGURATION_NON_UTF8")?
            .trim()
            .to_ascii_lowercase();
        let Some(rest) = key.strip_prefix("filter.") else {
            return Err("GIT_FILTER_CONFIGURATION_INVALID");
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            return Err("GIT_FILTER_CONFIGURATION_INVALID");
        };
        if name.is_empty()
            || !matches!(field, "clean" | "smudge" | "process" | "required")
            || key.contains(['\n', '\r'])
        {
            return Err("GIT_FILTER_CONFIGURATION_INVALID");
        }
        filter_names.insert(name.to_string());
    }
    let mut overrides = Vec::new();
    for name in filter_names {
        for field in ["clean", "smudge", "process"] {
            overrides.push((format!("filter.{name}.{field}"), String::new()));
        }
        overrides.push((format!("filter.{name}.required"), "false".to_string()));
    }
    Ok(overrides)
}

fn capture_git_snapshot(
    workspace: &Path,
    context: &GitContext,
    phase: &str,
    limits: &EvidenceLimits,
) -> GitSnapshot {
    let mut snapshot = GitSnapshot::default();
    match run_git_command(
        workspace,
        &context.filter_overrides,
        &["rev-parse", "--verify", "HEAD"],
        256,
        Duration::from_millis(limits.git_command_millis),
    ) {
        Ok(capture) if capture.status == Some(0) && !capture.truncated && !capture.timed_out => {
            snapshot.head = std::str::from_utf8(&capture.bytes)
                .ok()
                .map(str::trim)
                .filter(|value| value.len() == 40 || value.len() == 64)
                .map(str::to_string);
        }
        Ok(capture) if capture.status == Some(128) => {}
        _ => push_issue_bounded(
            &mut snapshot.issues,
            limits.issue_limit,
            issue(
                "GIT_HEAD_INSPECTION_FAILED",
                "The repository HEAD could not be inspected within the Git evidence boundary.",
                phase,
                None,
                true,
            ),
        ),
    }
    let capture = match run_git_command(
        workspace,
        &context.filter_overrides,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--find-renames=50%",
            "--ignore-submodules=none",
            "--",
            ".",
        ],
        limits.git_status_bytes,
        Duration::from_millis(limits.git_command_millis),
    ) {
        Ok(capture) => capture,
        Err(code) => {
            snapshot.truncated = true;
            push_issue_bounded(
                &mut snapshot.issues,
                limits.issue_limit,
                issue(
                    code,
                    "Git status could not be started through the hardened evidence command boundary.",
                    phase,
                    None,
                    true,
                ),
            );
            return snapshot;
        }
    };
    if capture.status != Some(0) || capture.truncated || capture.timed_out {
        snapshot.truncated = true;
        push_issue_bounded(
            &mut snapshot.issues,
            limits.issue_limit,
            issue(
                if capture.timed_out {
                    "GIT_STATUS_TIME_LIMIT_REACHED"
                } else if capture.truncated {
                    "GIT_STATUS_OUTPUT_LIMIT_REACHED"
                } else {
                    "GIT_STATUS_FAILED"
                },
                "Git status did not complete inside its explicit command and output bounds.",
                phase,
                None,
                true,
            ),
        );
        return snapshot;
    }
    match parse_porcelain_v2(&capture.bytes) {
        Ok(paths) => snapshot.paths = paths,
        Err(code) => {
            snapshot.truncated = true;
            push_issue_bounded(
                &mut snapshot.issues,
                limits.issue_limit,
                issue(
                    code,
                    "Git returned an unsupported or malformed porcelain-v2 status record.",
                    phase,
                    None,
                    true,
                ),
            );
        }
    }
    snapshot
}

fn parse_porcelain_v2(bytes: &[u8]) -> Result<BTreeMap<String, GitPathState>, &'static str> {
    let mut records = bytes.split(|byte| *byte == 0).peekable();
    let mut paths = BTreeMap::new();
    while let Some(raw) = records.next() {
        if raw.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(raw).map_err(|_| "GIT_STATUS_NON_UTF8_PATH")?;
        match record.as_bytes()[0] {
            b'1' => {
                let fields = record.splitn(9, ' ').collect::<Vec<_>>();
                if fields.len() != 9 || fields[0] != "1" {
                    return Err("GIT_STATUS_MALFORMED_ORDINARY");
                }
                paths.insert(
                    fields[8].to_string(),
                    git_state(fields[1], false, false, None)?,
                );
            }
            b'2' => {
                let fields = record.splitn(10, ' ').collect::<Vec<_>>();
                if fields.len() != 10 || fields[0] != "2" {
                    return Err("GIT_STATUS_MALFORMED_RENAME");
                }
                let previous = records.next().ok_or("GIT_STATUS_MISSING_RENAME_SOURCE")?;
                let previous = std::str::from_utf8(previous)
                    .map_err(|_| "GIT_STATUS_NON_UTF8_PATH")?
                    .to_string();
                paths.insert(
                    fields[9].to_string(),
                    git_state(fields[1], false, false, Some(previous))?,
                );
            }
            b'u' => {
                let fields = record.splitn(11, ' ').collect::<Vec<_>>();
                if fields.len() != 11 || fields[0] != "u" {
                    return Err("GIT_STATUS_MALFORMED_UNMERGED");
                }
                paths.insert(
                    fields[10].to_string(),
                    git_state(fields[1], false, true, None)?,
                );
            }
            b'?' => {
                let path = record
                    .strip_prefix("? ")
                    .ok_or("GIT_STATUS_MALFORMED_UNTRACKED")?;
                paths.insert(path.to_string(), git_state("??", true, false, None)?);
            }
            b'!' | b'#' => {}
            _ => return Err("GIT_STATUS_UNKNOWN_RECORD"),
        }
    }
    Ok(paths)
}

fn git_state(
    xy: &str,
    untracked: bool,
    conflicted: bool,
    previous_path: Option<String>,
) -> Result<GitPathState, &'static str> {
    let mut chars = xy.chars();
    let index = chars.next().ok_or("GIT_STATUS_MALFORMED_XY")?;
    let worktree = chars.next().ok_or("GIT_STATUS_MALFORMED_XY")?;
    if chars.next().is_some() {
        return Err("GIT_STATUS_MALFORMED_XY");
    }
    Ok(GitPathState {
        index_status: (!untracked && index != '.').then_some(index),
        worktree_status: (!untracked && worktree != '.').then_some(worktree),
        untracked,
        conflicted,
        previous_path,
    })
}

fn capture_git_diff(
    workspace: &Path,
    context: &GitContext,
    path: &str,
    cached: bool,
    limit: usize,
    timeout: Duration,
) -> Result<Option<TextCapture>, &'static str> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames=50%",
    ];
    if cached {
        args.push("--cached");
    }
    args.extend(["--", path]);
    let capture = run_git_command(workspace, &context.filter_overrides, &args, limit, timeout)?;
    if capture.status != Some(0) || capture.timed_out {
        return Err(if capture.timed_out {
            "GIT_DIFF_TIME_LIMIT_REACHED"
        } else {
            "GIT_DIFF_FAILED"
        });
    }
    if capture.bytes.is_empty() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&capture.bytes)
        .map_err(|_| "GIT_DIFF_NON_UTF8_OUTPUT")?
        .to_string();
    Ok(Some(TextCapture {
        text,
        original_bytes: capture.original_bytes,
        truncated: capture.truncated,
    }))
}

struct GitCommandCapture {
    status: Option<i32>,
    bytes: Vec<u8>,
    original_bytes: u64,
    truncated: bool,
    timed_out: bool,
}

struct ByteCapture {
    bytes: Vec<u8>,
    original_bytes: u64,
    truncated: bool,
}

struct TextCapture {
    text: String,
    original_bytes: u64,
    truncated: bool,
}

fn run_git_command(
    workspace: &Path,
    filter_overrides: &[(String, String)],
    args: &[&str],
    stdout_limit: usize,
    timeout: Duration,
) -> Result<GitCommandCapture, &'static str> {
    if timeout.is_zero() {
        return Err("GIT_COMMAND_TIME_LIMIT_REACHED");
    }
    let mut command = Command::new("git");
    command.current_dir(workspace).args([
        "--no-pager",
        "--no-optional-locks",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.untrackedCache=false",
        "-c",
        "diff.external=",
    ]);
    for (key, value) in filter_overrides {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy();
        if matches!(
            key.as_ref(),
            "GIT_DIR"
                | "GIT_WORK_TREE"
                | "GIT_INDEX_FILE"
                | "GIT_OBJECT_DIRECTORY"
                | "GIT_ALTERNATE_OBJECT_DIRECTORIES"
                | "GIT_COMMON_DIR"
                | "GIT_CEILING_DIRECTORIES"
                | "GIT_DISCOVERY_ACROSS_FILESYSTEM"
                | "GIT_EXTERNAL_DIFF"
                | "GIT_DIFF_OPTS"
                | "GIT_ATTR_NOSYSTEM"
        ) || key.starts_with("GIT_CONFIG_KEY_")
            || key.starts_with("GIT_CONFIG_VALUE_")
            || key == "GIT_CONFIG_COUNT"
        {
            command.env_remove(key.as_ref());
        }
    }
    command.env("GIT_ATTR_NOSYSTEM", "1");
    let mut child = command.spawn().map_err(|_| "GIT_COMMAND_START_FAILED")?;
    let stdout = child.stdout.take().ok_or("GIT_COMMAND_CAPTURE_FAILED")?;
    let stderr = child.stderr.take().ok_or("GIT_COMMAND_CAPTURE_FAILED")?;
    let stdout_reader = thread::spawn(move || drain_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || drain_bounded(stderr, 64 * 1024));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().ok().and_then(|status| status.code());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GIT_COMMAND_WAIT_FAILED");
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "GIT_COMMAND_CAPTURE_FAILED")?;
    let _stderr = stderr_reader
        .join()
        .map_err(|_| "GIT_COMMAND_CAPTURE_FAILED")?;
    Ok(GitCommandCapture {
        status,
        bytes: stdout.bytes,
        original_bytes: stdout.original_bytes,
        truncated: stdout.truncated,
        timed_out,
    })
}

fn drain_bounded(mut reader: impl Read, limit: usize) -> ByteCapture {
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut original_bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                original_bytes = original_bytes.saturating_add(read as u64);
                let remaining = limit.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    }
    ByteCapture {
        truncated: original_bytes > bytes.len() as u64,
        bytes,
        original_bytes,
    }
}

fn summarize_changes(
    changes: &[WorkspaceChangeEntryV1],
    original_count: usize,
) -> WorkspaceChangeSummaryV1 {
    let mut summary = WorkspaceChangeSummaryV1 {
        total_changes: original_count as u64,
        retained_changes: changes.len() as u64,
        ..WorkspaceChangeSummaryV1::default()
    };
    for change in changes {
        match change.change_kind {
            WorkspaceChangeKind::Added => summary.added += 1,
            WorkspaceChangeKind::Modified => summary.modified += 1,
            WorkspaceChangeKind::Deleted => summary.deleted += 1,
            WorkspaceChangeKind::Renamed => summary.renamed += 1,
            WorkspaceChangeKind::TypeChanged => summary.type_changed += 1,
            WorkspaceChangeKind::StatusChanged => summary.status_changed += 1,
        }
        if change
            .git_after
            .as_ref()
            .is_some_and(|state| state.index_status.is_some())
        {
            summary.staged += 1;
        }
        if change
            .git_after
            .as_ref()
            .is_some_and(|state| state.worktree_status.is_some())
        {
            summary.unstaged += 1;
        }
        if change
            .git_after
            .as_ref()
            .is_some_and(|state| state.untracked)
        {
            summary.untracked += 1;
        }
        summary.binary += u64::from(change.binary);
        summary.redacted += u64::from(change.content_redacted);
    }
    summary
}

fn is_sensitive_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name.as_str(),
            "id_rsa" | "id_ed25519" | "credentials" | "credentials.json" | "secrets.json"
        )
        || [".pem", ".key", ".p12", ".pfx", ".kdbx"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn safe_evidence_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && Path::new(path)
            .components()
            .all(|component| match component {
                std::path::Component::Normal(value) => value != ".git",
                _ => false,
            })
}

fn redact_text(value: &str) -> (String, bool) {
    let mut output = String::with_capacity(value.len());
    let mut redacted = false;
    let mut pem_block = false;
    for segment in value.split_inclusive('\n') {
        let line = segment.trim_end_matches('\n');
        let lower = line.to_ascii_lowercase();
        let starts_pem = lower.contains("-----begin ") && lower.contains("private key-----");
        let ends_pem = lower.contains("-----end ") && lower.contains("private key-----");
        let secret_key = [
            "password",
            "passwd",
            "secret",
            "token",
            "api_key",
            "apikey",
            "authorization",
            "private_key",
            "client_secret",
            "access_key",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        let token_like = line
            .split(|character: char| {
                !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-')
            })
            .any(|word| {
                word.len() >= 32
                    && word.bytes().any(|byte| byte.is_ascii_alphabetic())
                    && word.bytes().any(|byte| byte.is_ascii_digit())
            });
        if starts_pem {
            pem_block = true;
        }
        if pem_block || secret_key || token_like {
            let prefix = if line.starts_with('+') {
                "+"
            } else if line.starts_with('-') {
                "-"
            } else {
                ""
            };
            output.push_str(prefix);
            output.push_str("[REDACTED]");
            redacted = true;
        } else {
            output.push_str(line);
        }
        if segment.ends_with('\n') {
            output.push('\n');
        }
        if ends_pem {
            pem_block = false;
        }
    }
    (output, redacted)
}

fn issue(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    path: Option<String>,
    blocks_agent_approval: bool,
) -> WorkspaceEvidenceIssueV1 {
    WorkspaceEvidenceIssueV1 {
        code: code.into(),
        message: message.into(),
        phase: phase.into(),
        path,
        blocks_agent_approval,
    }
}

fn push_issue_bounded(
    issues: &mut Vec<WorkspaceEvidenceIssueV1>,
    limit: usize,
    entry: WorkspaceEvidenceIssueV1,
) {
    if issues.len() < limit.saturating_add(1) {
        issues.push(entry);
    }
}

fn push_char_bounded(target: &mut String, value: &str, limit: usize) {
    let available = limit.saturating_sub(target.len());
    let mut end = value.len().min(available);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&value[..end]);
}

#[cfg(test)]
#[path = "workspace_evidence/tests.rs"]
mod tests;
