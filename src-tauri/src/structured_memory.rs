use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::HashSet, fmt};

pub const MAX_MEMORY_RECORDS: usize = 50_000;
pub const MAX_MEMORY_CONTENT_BYTES: usize = 32 * 1024;
pub const MAX_LEGACY_MEMORY_CONTENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PROMPT_MEMORY_RECORDS: usize = 128;
pub const MAX_PROMPT_MEMORY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryValidationError {
    pub code: &'static str,
    pub message: String,
}

impl MemoryValidationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MemoryValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MemoryValidationError {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeKind {
    Agent,
    Project,
    Task,
    Team,
}

impl MemoryScopeKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Project => "project",
            Self::Task => "task",
            Self::Team => "team",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, MemoryValidationError> {
        match value {
            "agent" => Ok(Self::Agent),
            "project" => Ok(Self::Project),
            "task" => Ok(Self::Task),
            "team" => Ok(Self::Team),
            _ => Err(MemoryValidationError::new(
                "MEMORY_STORAGE_INVALID",
                "The stored memory scope is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryScopeV1 {
    pub kind: MemoryScopeKind,
    pub agent_id: Option<i64>,
    pub workspace_id: Option<String>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub team_leader_agent_id: Option<i64>,
}

impl MemoryScopeV1 {
    #[cfg(test)]
    pub fn agent(agent_id: i64) -> Self {
        Self {
            kind: MemoryScopeKind::Agent,
            agent_id: Some(agent_id),
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
            team_leader_agent_id: None,
        }
    }

    #[cfg(test)]
    pub fn project(workspace_id: impl Into<String>) -> Self {
        Self {
            kind: MemoryScopeKind::Project,
            agent_id: None,
            workspace_id: Some(workspace_id.into()),
            task_owner_agent_id: None,
            task_id: None,
            team_leader_agent_id: None,
        }
    }

    #[cfg(test)]
    pub fn task(owner_agent_id: i64, task_id: i64) -> Self {
        Self {
            kind: MemoryScopeKind::Task,
            agent_id: None,
            workspace_id: None,
            task_owner_agent_id: Some(owner_agent_id),
            task_id: Some(task_id),
            team_leader_agent_id: None,
        }
    }

    #[cfg(test)]
    pub fn team(team_leader_agent_id: i64) -> Self {
        Self {
            kind: MemoryScopeKind::Team,
            agent_id: None,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
            team_leader_agent_id: Some(team_leader_agent_id),
        }
    }

    pub fn validate(&self) -> Result<(), MemoryValidationError> {
        if self.agent_id.is_some_and(|value| value <= 0)
            || self.task_owner_agent_id.is_some_and(|value| value <= 0)
            || self.task_id.is_some_and(|value| value <= 0)
            || self.team_leader_agent_id.is_some_and(|value| value <= 0)
            || self
                .workspace_id
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 4 * 1024)
        {
            return Err(MemoryValidationError::new(
                "MEMORY_SCOPE_INVALID",
                "Memory scope identifiers must be positive and bounded.",
            ));
        }

        let valid = match self.kind {
            MemoryScopeKind::Agent => {
                self.agent_id.is_some()
                    && self.workspace_id.is_none()
                    && self.task_owner_agent_id.is_none()
                    && self.task_id.is_none()
                    && self.team_leader_agent_id.is_none()
            }
            MemoryScopeKind::Project => {
                self.agent_id.is_none()
                    && self.workspace_id.is_some()
                    && self.task_owner_agent_id.is_none()
                    && self.task_id.is_none()
                    && self.team_leader_agent_id.is_none()
            }
            MemoryScopeKind::Task => {
                self.agent_id.is_none()
                    && self.workspace_id.is_none()
                    && self.task_owner_agent_id.is_some()
                    && self.task_id.is_some()
                    && self.team_leader_agent_id.is_none()
            }
            MemoryScopeKind::Team => {
                self.agent_id.is_none()
                    && self.workspace_id.is_none()
                    && self.task_owner_agent_id.is_none()
                    && self.task_id.is_none()
                    && self.team_leader_agent_id.is_some()
            }
        };
        if !valid {
            return Err(MemoryValidationError::new(
                "MEMORY_SCOPE_INVALID",
                "A memory record must identify exactly one agent, project, task, or team scope.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecordKind {
    Instruction,
    Fact,
    Decision,
    Summary,
}

impl MemoryRecordKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Summary => "summary",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, MemoryValidationError> {
        match value {
            "instruction" => Ok(Self::Instruction),
            "fact" => Ok(Self::Fact),
            "decision" => Ok(Self::Decision),
            "summary" => Ok(Self::Summary),
            _ => Err(MemoryValidationError::new(
                "MEMORY_STORAGE_INVALID",
                "The stored memory record kind is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProvenanceKind {
    User,
    LegacyAgentMemory,
    HandoffPromotion,
    BackupImport,
}

impl MemoryProvenanceKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::LegacyAgentMemory => "legacy_agent_memory",
            Self::HandoffPromotion => "handoff_promotion",
            Self::BackupImport => "backup_import",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, MemoryValidationError> {
        match value {
            "user" => Ok(Self::User),
            "legacy_agent_memory" => Ok(Self::LegacyAgentMemory),
            "handoff_promotion" => Ok(Self::HandoffPromotion),
            "backup_import" => Ok(Self::BackupImport),
            _ => Err(MemoryValidationError::new(
                "MEMORY_STORAGE_INVALID",
                "The stored memory provenance is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRetentionPolicy {
    Manual,
    Days7,
    Days30,
    Days90,
    TaskLifetime,
}

impl MemoryRetentionPolicy {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Days7 => "7d",
            Self::Days30 => "30d",
            Self::Days90 => "90d",
            Self::TaskLifetime => "task_lifetime",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, MemoryValidationError> {
        match value {
            "manual" => Ok(Self::Manual),
            "7d" => Ok(Self::Days7),
            "30d" => Ok(Self::Days30),
            "90d" => Ok(Self::Days90),
            "task_lifetime" => Ok(Self::TaskLifetime),
            _ => Err(MemoryValidationError::new(
                "MEMORY_STORAGE_INVALID",
                "The stored memory retention policy is invalid.",
            )),
        }
    }

    pub fn expiry_from(self, now_unix_ms: i64) -> Option<i64> {
        let days = match self {
            Self::Days7 => 7,
            Self::Days30 => 30,
            Self::Days90 => 90,
            Self::Manual | Self::TaskLifetime => return None,
        };
        now_unix_ms.checked_add(days * 24 * 60 * 60 * 1000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryRecordV1 {
    pub id: i64,
    pub scope: MemoryScopeV1,
    pub kind: MemoryRecordKind,
    pub content: String,
    pub provenance: MemoryProvenanceKind,
    pub provenance_ref: Option<String>,
    pub revision: i64,
    pub retention: MemoryRetentionPolicy,
    pub expires_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEventV1 {
    pub id: i64,
    pub record_id: Option<i64>,
    pub action: String,
    pub actor_kind: String,
    pub record_revision: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredMemorySnapshot {
    pub revision: i64,
    pub application_state_revision: i64,
    pub records: Vec<MemoryRecordV1>,
    pub recent_events: Vec<MemoryEventV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateMemoryRecordRequest {
    pub expected_revision: i64,
    pub request_id: String,
    pub scope: MemoryScopeV1,
    pub kind: MemoryRecordKind,
    pub content: String,
    pub retention: MemoryRetentionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMemoryRecordRequest {
    pub expected_revision: i64,
    pub expected_record_revision: i64,
    pub request_id: String,
    pub record_id: i64,
    pub kind: MemoryRecordKind,
    pub content: String,
    pub retention: MemoryRetentionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteMemoryRecordRequest {
    pub expected_revision: i64,
    pub expected_record_revision: i64,
    pub request_id: String,
    pub record_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySelectionContext {
    pub agent_id: i64,
    pub workspace_id: Option<String>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub team_leader_agent_ids: Vec<i64>,
}

impl MemorySelectionContext {
    pub fn validate(&self) -> Result<(), MemoryValidationError> {
        if self.agent_id <= 0
            || self.task_owner_agent_id.is_some() != self.task_id.is_some()
            || self.task_owner_agent_id.is_some_and(|value| value <= 0)
            || self.task_id.is_some_and(|value| value <= 0)
            || self
                .team_leader_agent_ids
                .iter()
                .any(|identifier| *identifier <= 0)
            || self
                .workspace_id
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value.len() > 4 * 1024)
        {
            return Err(MemoryValidationError::new(
                "MEMORY_CONTEXT_INVALID",
                "The memory selection context is malformed.",
            ));
        }
        let unique: HashSet<_> = self.team_leader_agent_ids.iter().collect();
        if unique.len() != self.team_leader_agent_ids.len() {
            return Err(MemoryValidationError::new(
                "MEMORY_CONTEXT_INVALID",
                "The team visibility chain contains duplicate agents.",
            ));
        }
        Ok(())
    }

    pub fn permits(&self, scope: &MemoryScopeV1) -> bool {
        match scope.kind {
            MemoryScopeKind::Agent => scope.agent_id == Some(self.agent_id),
            MemoryScopeKind::Project => scope.workspace_id == self.workspace_id,
            MemoryScopeKind::Task => {
                scope.task_owner_agent_id == self.task_owner_agent_id
                    && scope.task_id == self.task_id
            }
            MemoryScopeKind::Team => scope
                .team_leader_agent_id
                .is_some_and(|leader| self.team_leader_agent_ids.contains(&leader)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptMemoryRecordV1 {
    pub id: i64,
    pub scope: MemoryScopeV1,
    pub kind: MemoryRecordKind,
    pub content: String,
    pub provenance: MemoryProvenanceKind,
    pub provenance_ref: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPromptBundleV1 {
    pub schema_version: i64,
    pub context: MemorySelectionContext,
    pub records: Vec<PromptMemoryRecordV1>,
    pub omitted_record_count: usize,
}

impl MemoryPromptBundleV1 {
    pub fn canonical_json(&self) -> Result<String, MemoryValidationError> {
        serde_json::to_string(self).map_err(|_| {
            MemoryValidationError::new(
                "MEMORY_BUNDLE_INVALID",
                "The selected prompt-memory bundle could not be serialized.",
            )
        })
    }

    pub fn sha256(&self) -> Result<String, MemoryValidationError> {
        Ok(format!(
            "{:x}",
            Sha256::digest(self.canonical_json()?.as_bytes())
        ))
    }
}

pub fn validate_create_request(
    request: &CreateMemoryRecordRequest,
) -> Result<(), MemoryValidationError> {
    validate_revision_and_request(request.expected_revision, &request.request_id)?;
    request.scope.validate()?;
    validate_memory_content(&request.content, false)?;
    validate_retention_scope(request.retention, &request.scope)
}

pub fn validate_update_request(
    request: &UpdateMemoryRecordRequest,
) -> Result<(), MemoryValidationError> {
    validate_revision_and_request(request.expected_revision, &request.request_id)?;
    if request.record_id <= 0 || request.expected_record_revision <= 0 {
        return Err(MemoryValidationError::new(
            "MEMORY_REQUEST_INVALID",
            "The record identifier and expected record revision must be positive.",
        ));
    }
    validate_memory_content(&request.content, false)
}

pub fn validate_delete_request(
    request: &DeleteMemoryRecordRequest,
) -> Result<(), MemoryValidationError> {
    validate_revision_and_request(request.expected_revision, &request.request_id)?;
    if request.record_id <= 0 || request.expected_record_revision <= 0 {
        return Err(MemoryValidationError::new(
            "MEMORY_REQUEST_INVALID",
            "The record identifier and expected record revision must be positive.",
        ));
    }
    Ok(())
}

pub fn validate_memory_record(record: &MemoryRecordV1) -> Result<(), MemoryValidationError> {
    if record.id <= 0
        || record.revision <= 0
        || record.created_at_unix_ms < 0
        || record.updated_at_unix_ms < record.created_at_unix_ms
        || record.expires_at_unix_ms.is_some_and(|value| value < 0)
        || record
            .provenance_ref
            .as_ref()
            .is_some_and(|value| value.len() > 4 * 1024)
    {
        return Err(MemoryValidationError::new(
            "MEMORY_STORAGE_INVALID",
            "The stored memory record metadata is invalid.",
        ));
    }
    record.scope.validate()?;
    validate_memory_content(
        &record.content,
        record.provenance == MemoryProvenanceKind::LegacyAgentMemory,
    )?;
    validate_retention_scope(record.retention, &record.scope)
}

pub fn build_prompt_bundle(
    records: &[MemoryRecordV1],
    context: MemorySelectionContext,
    now_unix_ms: i64,
) -> Result<MemoryPromptBundleV1, MemoryValidationError> {
    context.validate()?;
    if now_unix_ms < 0 {
        return Err(MemoryValidationError::new(
            "MEMORY_CONTEXT_INVALID",
            "The prompt-memory selection timestamp must be non-negative.",
        ));
    }

    let mut selected: Vec<&MemoryRecordV1> = records
        .iter()
        .filter(|record| {
            context.permits(&record.scope)
                && record
                    .expires_at_unix_ms
                    .map_or(true, |expires| expires > now_unix_ms)
        })
        .collect();
    selected.sort_by(|left, right| compare_prompt_order(left, right));

    let mut bundle = MemoryPromptBundleV1 {
        schema_version: 1,
        context,
        records: Vec::new(),
        omitted_record_count: 0,
    };
    for record in selected {
        validate_memory_record(record)?;
        let prompt_record = PromptMemoryRecordV1 {
            id: record.id,
            scope: record.scope.clone(),
            kind: record.kind,
            content: record.content.clone(),
            provenance: record.provenance,
            provenance_ref: record.provenance_ref.clone(),
            revision: record.revision,
        };
        if bundle.records.len() >= MAX_PROMPT_MEMORY_RECORDS {
            bundle.omitted_record_count += 1;
            continue;
        }
        bundle.records.push(prompt_record);
        if bundle.canonical_json()?.len() > MAX_PROMPT_MEMORY_BYTES {
            bundle.records.pop();
            bundle.omitted_record_count += 1;
        }
    }
    Ok(bundle)
}

fn compare_prompt_order(left: &MemoryRecordV1, right: &MemoryRecordV1) -> Ordering {
    scope_rank(left.scope.kind)
        .cmp(&scope_rank(right.scope.kind))
        .then_with(|| left.created_at_unix_ms.cmp(&right.created_at_unix_ms))
        .then_with(|| left.id.cmp(&right.id))
}

fn scope_rank(kind: MemoryScopeKind) -> u8 {
    match kind {
        MemoryScopeKind::Agent => 0,
        MemoryScopeKind::Team => 1,
        MemoryScopeKind::Project => 2,
        MemoryScopeKind::Task => 3,
    }
}

fn validate_revision_and_request(
    expected_revision: i64,
    request_id: &str,
) -> Result<(), MemoryValidationError> {
    if expected_revision < 0 || request_id.trim().is_empty() || request_id.len() > 128 {
        return Err(MemoryValidationError::new(
            "MEMORY_REQUEST_INVALID",
            "The expected revision must be non-negative and requestId must be 1 to 128 bytes.",
        ));
    }
    Ok(())
}

fn validate_memory_content(content: &str, legacy: bool) -> Result<(), MemoryValidationError> {
    let maximum = if legacy {
        MAX_LEGACY_MEMORY_CONTENT_BYTES
    } else {
        MAX_MEMORY_CONTENT_BYTES
    };
    if content.trim().is_empty() || content.len() > maximum {
        return Err(MemoryValidationError::new(
            "MEMORY_CONTENT_INVALID",
            format!("Memory content must be non-empty and no more than {maximum} bytes."),
        ));
    }
    Ok(())
}

fn validate_retention_scope(
    retention: MemoryRetentionPolicy,
    scope: &MemoryScopeV1,
) -> Result<(), MemoryValidationError> {
    if retention == MemoryRetentionPolicy::TaskLifetime && scope.kind != MemoryScopeKind::Task {
        return Err(MemoryValidationError::new(
            "MEMORY_RETENTION_INVALID",
            "Task-lifetime retention is only valid for task-scoped memory.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: i64, scope: MemoryScopeV1, content: &str) -> MemoryRecordV1 {
        MemoryRecordV1 {
            id,
            scope,
            kind: MemoryRecordKind::Fact,
            content: content.to_string(),
            provenance: MemoryProvenanceKind::User,
            provenance_ref: None,
            revision: 1,
            retention: MemoryRetentionPolicy::Manual,
            expires_at_unix_ms: None,
            created_at_unix_ms: id,
            updated_at_unix_ms: id,
        }
    }

    #[test]
    fn task_0018_memory_scope_validation_is_exact() {
        let mut malformed = MemoryScopeV1::agent(7);
        malformed.workspace_id = Some("project-a".to_string());
        assert_eq!(
            malformed.validate().unwrap_err().code,
            "MEMORY_SCOPE_INVALID"
        );
        assert!(MemoryScopeV1::task(4, 9).validate().is_ok());
    }

    #[test]
    fn task_0018_memory_selection_does_not_leak_projects_or_tasks() {
        let records = vec![
            record(1, MemoryScopeV1::agent(10), "agent"),
            record(2, MemoryScopeV1::agent(11), "other agent"),
            record(3, MemoryScopeV1::project("project-a"), "project a"),
            record(4, MemoryScopeV1::project("project-b"), "project b"),
            record(5, MemoryScopeV1::task(10, 5), "task 5"),
            record(6, MemoryScopeV1::task(10, 6), "task 6"),
            record(7, MemoryScopeV1::team(20), "team"),
            record(8, MemoryScopeV1::team(21), "other team"),
        ];
        let bundle = build_prompt_bundle(
            &records,
            MemorySelectionContext {
                agent_id: 10,
                workspace_id: Some("project-a".to_string()),
                task_owner_agent_id: Some(10),
                task_id: Some(5),
                team_leader_agent_ids: vec![20],
            },
            100,
        )
        .unwrap();
        let ids: Vec<i64> = bundle.records.iter().map(|record| record.id).collect();
        assert_eq!(ids, vec![1, 7, 3, 5]);
        assert!(!bundle.canonical_json().unwrap().contains("project b"));
        assert!(!bundle.canonical_json().unwrap().contains("task 6"));
    }

    #[test]
    fn task_0018_memory_bundle_is_bounded_deterministic_and_hashed() {
        let records: Vec<_> = (1..=200)
            .map(|id| record(id, MemoryScopeV1::agent(10), &"x".repeat(1024)))
            .collect();
        let context = MemorySelectionContext {
            agent_id: 10,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
            team_leader_agent_ids: vec![],
        };
        let first = build_prompt_bundle(&records, context.clone(), 100).unwrap();
        let second = build_prompt_bundle(&records, context, 100).unwrap();
        assert!(first.records.len() < records.len());
        assert!(first.canonical_json().unwrap().len() <= MAX_PROMPT_MEMORY_BYTES);
        assert_eq!(first.sha256().unwrap(), second.sha256().unwrap());
    }

    #[test]
    fn task_0018_memory_expired_records_are_not_retrieved() {
        let mut expired = record(1, MemoryScopeV1::agent(10), "expired");
        expired.expires_at_unix_ms = Some(100);
        let bundle = build_prompt_bundle(
            &[expired],
            MemorySelectionContext {
                agent_id: 10,
                workspace_id: None,
                task_owner_agent_id: None,
                task_id: None,
                team_leader_agent_ids: vec![],
            },
            100,
        )
        .unwrap();
        assert!(bundle.records.is_empty());
    }
}
