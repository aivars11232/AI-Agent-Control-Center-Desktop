use crate::app_state::{
    application_state_from_legacy_backup, validate_application_state, AgentTask, ApplicationState,
    StateValidationError, CURRENT_SCHEMA_VERSION, MAX_STATE_BYTES,
};
use crate::reminder_scheduler::{
    validate_scheduled_item, DeliveryMode, ScheduledItemV1, MAX_SCHEDULED_ITEMS,
};
use crate::structured_memory::{validate_memory_record, MemoryRecordV1, MAX_MEMORY_RECORDS};
use serde::{
    de::{Error as DeError, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{Map, Number, Value};
use std::{fmt, fmt::Write as _};

pub const BACKUP_FORMAT: &str = "ai-agent-control-center-portable-backup";
pub const BACKUP_VERSION: i64 = 4;
pub const MAX_BACKUP_BYTES: usize = MAX_STATE_BYTES;
pub const MAX_BACKUP_JSON_DEPTH: usize = 128;
pub const MAX_MAINTENANCE_ROWS_PER_DOMAIN: i64 = 500;
pub const MAX_MAINTENANCE_EVIDENCE_ROWS: i64 = 100;
pub const MAINTENANCE_INTERVAL_SECONDS: u64 = 15 * 60;
pub const MAINTENANCE_BACKLOG_INTERVAL_SECONDS: u64 = 60;
pub const MONITORING_PAGE_LIMIT: i64 = 100;

const OMITTED_DOMAINS: [&str; 11] = [
    "providerCredentials",
    "authorizationIntents",
    "runReservations",
    "runAttempts",
    "reviewFlows",
    "portalSessions",
    "portalNotificationGrants",
    "notificationDeliveryEvidence",
    "managementHandoffs",
    "voiceRuntimeSessions",
    "systemActionAudit",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackupEnvelopeV3 {
    format: String,
    version: i64,
    exported_at_unix_ms: i64,
    source_schema_version: i64,
    data: ApplicationState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackupEnvelopeV4 {
    format: String,
    version: i64,
    exported_at_unix_ms: i64,
    source_schema_version: i64,
    data: ApplicationState,
    scheduled_items: Vec<ScheduledItemV1>,
    memory_records: Vec<MemoryRecordV1>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecordCounts {
    pub agents: usize,
    pub tasks: usize,
    pub activity: usize,
    pub models: usize,
    pub approval_history: usize,
    pub reminders: usize,
    pub workspaces: usize,
    pub memory_records: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupSanitizationCounts {
    pub held_tasks: usize,
    pub expired_approvals: usize,
    pub cleared_task_evidence: usize,
    pub disabled_voice_runtime: bool,
    pub portal_deliveries_disabled: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupExport {
    pub file_name: String,
    pub backup_json: String,
    pub byte_length: usize,
    pub counts: BackupRecordCounts,
    pub sanitizations: BackupSanitizationCounts,
    pub omitted_domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportPreview {
    pub format_version: i64,
    pub source_schema_version: Option<i64>,
    pub byte_length: usize,
    pub counts: BackupRecordCounts,
    pub sanitizations: BackupSanitizationCounts,
    pub omitted_domains: Vec<String>,
    pub replaces_current_state: bool,
    pub clears_run_and_review_history: bool,
    pub security_change_summary: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPruneCounts {
    pub tasks: i64,
    pub attempts: i64,
    pub review_flows: i64,
    pub activity: i64,
    pub approvals: i64,
    pub reminders: i64,
    pub system_action_audits: i64,
    pub memory_records: i64,
    pub reminder_occurrences: i64,
    pub management_handoffs: i64,
}

impl RetentionPruneCounts {
    pub fn application_rows(&self) -> i64 {
        self.tasks
            .saturating_add(self.activity)
            .saturating_add(self.approvals)
            .saturating_add(self.reminders)
            .saturating_add(self.memory_records)
            .saturating_add(self.reminder_occurrences)
            .saturating_add(self.management_handoffs)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionMaintenanceResult {
    pub lifecycle_revision: i64,
    pub application_state_revision: i64,
    pub trigger_kind: String,
    pub status: String,
    pub started_at_unix_ms: i64,
    pub completed_at_unix_ms: i64,
    pub task_cutoff_unix_ms: Option<i64>,
    pub activity_cutoff_unix_ms: Option<i64>,
    pub pruned: RetentionPruneCounts,
    pub skipped_protected: i64,
    pub backlog_remaining: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonitoringRevision {
    pub application_state: i64,
    pub task_orchestration: i64,
    pub run_coordinator: i64,
    pub review_orchestration: i64,
    pub data_lifecycle: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringCounts {
    pub configured_agents: i64,
    pub active_agents: i64,
    pub total_tasks: i64,
    pub running_tasks: i64,
    pub pending_tasks: i64,
    pub blocked_tasks: i64,
    pub completed_tasks: i64,
    pub failed_tasks: i64,
    pub activity_entries: i64,
    pub pending_approvals: i64,
    pub upcoming_reminders: i64,
    pub retained_run_attempts: i64,
    pub active_run_attempts: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataLifecycleSummary {
    pub task_retention: String,
    pub activity_retention: String,
    pub last_observed_at_unix_ms: Option<i64>,
    pub last_success_at_unix_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub total_runs: i64,
    pub total_pruned: RetentionPruneCounts,
    pub inferred_timestamp_count: i64,
    pub latest_run: Option<RetentionMaintenanceResult>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringSnapshot {
    pub authoritative: bool,
    pub generated_at_unix_ms: i64,
    pub revision: MonitoringRevision,
    pub counts: MonitoringCounts,
    pub lifecycle: DataLifecycleSummary,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringTaskRecord {
    pub owner_agent_id: i64,
    pub owner_name: String,
    pub owner_role: String,
    pub executor_name: Option<String>,
    pub created_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub task: AgentTask,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringTaskPage {
    pub authoritative: bool,
    pub revision: MonitoringRevision,
    pub offset: i64,
    pub limit: i64,
    pub total: i64,
    pub records: Vec<MonitoringTaskRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringActivityRecord {
    pub owner_agent_id: i64,
    pub owner_name: String,
    pub owner_role: String,
    pub entry_id: i64,
    pub message: String,
    pub created_at: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringActivityPage {
    pub authoritative: bool,
    pub revision: MonitoringRevision,
    pub offset: i64,
    pub limit: i64,
    pub total: i64,
    pub records: Vec<MonitoringActivityRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringMutationResult {
    pub deleted_count: i64,
    pub snapshot: MonitoringSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct BackupCandidate {
    pub state: ApplicationState,
    pub format_version: i64,
    pub source_schema_version: Option<i64>,
    pub source_kind: &'static str,
    pub byte_length: usize,
    pub sanitizations: BackupSanitizationCounts,
    pub scheduled_items: Vec<ScheduledItemV1>,
    pub memory_records: Vec<MemoryRecordV1>,
}

#[cfg(test)]
pub fn build_backup_export(
    state: &ApplicationState,
    exported_at_unix_ms: i64,
) -> Result<BackupExport, StateValidationError> {
    build_backup_export_with_domains(state, &[], &[], exported_at_unix_ms)
}

pub(crate) fn build_backup_export_with_domains(
    state: &ApplicationState,
    scheduled_items: &[ScheduledItemV1],
    memory_records: &[MemoryRecordV1],
    exported_at_unix_ms: i64,
) -> Result<BackupExport, StateValidationError> {
    if exported_at_unix_ms < 0 {
        return Err(StateValidationError::new(
            "backup.exportedAtUnixMs",
            "export time must be non-negative",
        ));
    }
    if scheduled_items.len() > MAX_SCHEDULED_ITEMS || memory_records.len() > MAX_MEMORY_RECORDS {
        return Err(StateValidationError::new(
            "backup",
            "portable reminder or memory count exceeds the supported limit",
        ));
    }
    let (mut portable_state, mut sanitizations) = sanitize_portable_state(state.clone())?;
    portable_state.reminders.clear();
    for agent in &mut portable_state.agents {
        agent.memory.clear();
    }
    let mut portable_scheduled_items = scheduled_items.to_vec();
    for item in &mut portable_scheduled_items {
        validate_scheduled_item(item)
            .map_err(|error| StateValidationError::new("backup.scheduledItems", error.message))?;
        if item.delivery_mode == DeliveryMode::Portal {
            sanitizations.portal_deliveries_disabled += 1;
        }
        item.delivery_mode = DeliveryMode::InApp;
        item.schedule_fingerprint = None;
    }
    let portable_memory_records = memory_records
        .iter()
        .filter(|record| {
            record
                .expires_at_unix_ms
                .map_or(true, |expires| expires > exported_at_unix_ms)
        })
        .cloned()
        .collect::<Vec<_>>();
    for record in &portable_memory_records {
        validate_memory_record(record)
            .map_err(|error| StateValidationError::new("backup.memoryRecords", error.message))?;
    }
    let counts = record_counts(
        &portable_state,
        portable_scheduled_items.len(),
        portable_memory_records.len(),
    );
    let envelope = BackupEnvelopeV4 {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_VERSION,
        exported_at_unix_ms,
        source_schema_version: CURRENT_SCHEMA_VERSION,
        data: portable_state,
        scheduled_items: portable_scheduled_items,
        memory_records: portable_memory_records,
    };
    let backup_json = serde_json::to_string(&envelope).map_err(|error| {
        StateValidationError::new("backup", format!("backup could not be encoded: {error}"))
    })?;
    if backup_json.len() > MAX_BACKUP_BYTES {
        return Err(StateValidationError::new(
            "backup",
            format!("backup exceeds {MAX_BACKUP_BYTES} bytes"),
        ));
    }
    let file_name = format!(
        "ai-agent-control-center-backup-{}.json",
        date_from_unix_ms(exported_at_unix_ms)
    );
    Ok(BackupExport {
        file_name,
        byte_length: backup_json.len(),
        backup_json,
        counts,
        sanitizations,
        omitted_domains: omitted_domains(),
    })
}

pub(crate) fn parse_backup_candidate(
    backup_json: &str,
    current: &ApplicationState,
) -> Result<BackupCandidate, StateValidationError> {
    if backup_json.len() > MAX_BACKUP_BYTES {
        return Err(StateValidationError::new(
            "backup",
            format!("backup exceeds {MAX_BACKUP_BYTES} bytes"),
        ));
    }
    let value = parse_json_without_duplicate_keys(backup_json)?;
    let version = value.get("version").and_then(Value::as_i64).unwrap_or(2);
    let (state, source_schema_version, source_kind, mut scheduled_items, memory_records) =
        match version {
            BACKUP_VERSION => {
                let envelope: BackupEnvelopeV4 =
                    serde_json::from_value(value).map_err(|error| {
                        StateValidationError::new(
                            "backup",
                            format!("backup version 4 does not match its strict schema: {error}"),
                        )
                    })?;
                if envelope.format != BACKUP_FORMAT {
                    return Err(StateValidationError::new(
                        "backup.format",
                        "backup format discriminator is unsupported",
                    ));
                }
                if envelope.version != BACKUP_VERSION {
                    return Err(StateValidationError::new(
                        "backup.version",
                        "backup version is unsupported",
                    ));
                }
                if envelope.exported_at_unix_ms < 0 {
                    return Err(StateValidationError::new(
                        "backup.exportedAtUnixMs",
                        "export time must be non-negative",
                    ));
                }
                if !(1..=CURRENT_SCHEMA_VERSION).contains(&envelope.source_schema_version) {
                    return Err(StateValidationError::new(
                        "backup.sourceSchemaVersion",
                        "backup source schema is unsupported",
                    ));
                }
                (
                    envelope.data,
                    Some(envelope.source_schema_version),
                    "backup_v4",
                    envelope.scheduled_items,
                    envelope.memory_records,
                )
            }
            3 => {
                let envelope: BackupEnvelopeV3 =
                    serde_json::from_value(value).map_err(|error| {
                        StateValidationError::new(
                            "backup",
                            format!("backup version 3 does not match its strict schema: {error}"),
                        )
                    })?;
                if envelope.format != BACKUP_FORMAT || envelope.version != 3 {
                    return Err(StateValidationError::new(
                        "backup.format",
                        "backup version 3 discriminator is unsupported",
                    ));
                }
                if envelope.exported_at_unix_ms < 0
                    || !(1..=CURRENT_SCHEMA_VERSION).contains(&envelope.source_schema_version)
                {
                    return Err(StateValidationError::new(
                        "backup",
                        "backup version 3 metadata is unsupported",
                    ));
                }
                (
                    envelope.data,
                    Some(envelope.source_schema_version),
                    "backup_v3",
                    Vec::new(),
                    Vec::new(),
                )
            }
            2 => {
                let normalized_json = serde_json::to_string(&value).map_err(|error| {
                    StateValidationError::new(
                        "backup",
                        format!("legacy backup could not be normalized: {error}"),
                    )
                })?;
                (
                    application_state_from_legacy_backup(&normalized_json, current)?,
                    Some(2),
                    "legacy_backup",
                    Vec::new(),
                    Vec::new(),
                )
            }
            _ => {
                return Err(StateValidationError::new(
                    "backup.version",
                    "only portable backup versions 4 and 3 and legacy version 2 are supported",
                ));
            }
        };
    if version == BACKUP_VERSION
        && (!state.reminders.is_empty()
            || state.agents.iter().any(|agent| !agent.memory.is_empty()))
    {
        return Err(StateValidationError::new(
            "backup.data",
            "backup version 4 must store reminders and memory only in their structured domains",
        ));
    }
    if scheduled_items.len() > MAX_SCHEDULED_ITEMS || memory_records.len() > MAX_MEMORY_RECORDS {
        return Err(StateValidationError::new(
            "backup",
            "portable reminder or memory count exceeds the supported limit",
        ));
    }
    let portal_deliveries_disabled = scheduled_items
        .iter()
        .filter(|item| item.delivery_mode == DeliveryMode::Portal)
        .count();
    let mut item_ids = std::collections::HashSet::new();
    for item in &mut scheduled_items {
        if !item_ids.insert(item.id) {
            return Err(StateValidationError::new(
                "backup.scheduledItems",
                "a portable scheduled item identifier is duplicated",
            ));
        }
        validate_scheduled_item(item)
            .map_err(|error| StateValidationError::new("backup.scheduledItems", error.message))?;
        item.delivery_mode = DeliveryMode::InApp;
        item.schedule_fingerprint = None;
    }
    let mut memory_ids = std::collections::HashSet::new();
    for record in &memory_records {
        if !memory_ids.insert(record.id) {
            return Err(StateValidationError::new(
                "backup.memoryRecords",
                "a portable memory record identifier is duplicated",
            ));
        }
        validate_memory_record(record)
            .map_err(|error| StateValidationError::new("backup.memoryRecords", error.message))?;
    }
    let (state, mut sanitizations) = sanitize_portable_state(state)?;
    sanitizations.portal_deliveries_disabled = portal_deliveries_disabled;
    Ok(BackupCandidate {
        state,
        format_version: version,
        source_schema_version,
        source_kind,
        byte_length: backup_json.len(),
        sanitizations,
        scheduled_items,
        memory_records,
    })
}

pub(crate) fn preview_for_candidate(
    candidate: &BackupCandidate,
    security_change_summary: Option<String>,
) -> BackupImportPreview {
    BackupImportPreview {
        format_version: candidate.format_version,
        source_schema_version: candidate.source_schema_version,
        byte_length: candidate.byte_length,
        counts: record_counts(
            &candidate.state,
            candidate.scheduled_items.len(),
            candidate.memory_records.len(),
        ),
        sanitizations: candidate.sanitizations.clone(),
        omitted_domains: omitted_domains(),
        replaces_current_state: true,
        clears_run_and_review_history: true,
        security_change_summary,
    }
}

pub(crate) fn import_confirmation_message(preview: &BackupImportPreview) -> String {
    let security = preview
        .security_change_summary
        .as_deref()
        .unwrap_or("No protected security configuration increase was detected.");
    format!(
        "Replace current portable application data with backup version {}?\n\nAgents: {}\nTasks: {}\nActivity entries: {}\nApproval history: {}\nReminders/events: {}\nMemory records: {}\n\n{} task(s) will be held, {} approval record(s) will be expired, and current run/review/handoff and notification-delivery history will be cleared.\n\nSecurity: {}",
        preview.format_version,
        preview.counts.agents,
        preview.counts.tasks,
        preview.counts.activity,
        preview.counts.approval_history,
        preview.counts.reminders,
        preview.counts.memory_records,
        preview.sanitizations.held_tasks,
        preview.sanitizations.expired_approvals,
        security,
    )
}

fn sanitize_portable_state(
    mut state: ApplicationState,
) -> Result<(ApplicationState, BackupSanitizationCounts), StateValidationError> {
    let mut sanitizations = BackupSanitizationCounts::default();
    let mut held_tasks = Vec::new();

    for request in &mut state.approval_requests {
        if matches!(request.status.as_str(), "Pending" | "Approved") {
            request.status = "Expired".to_string();
            request.consumed_at = None;
            sanitizations.expired_approvals += 1;
        }
    }

    for (agent_index, agent) in state.agents.iter_mut().enumerate() {
        for (task_index, task) in agent.tasks.iter_mut().enumerate() {
            let had_evidence = task.response_id.is_some()
                || task.runtime_model.is_some()
                || task.total_tokens.is_some()
                || !task.changed_files.is_empty()
                || task.diff.is_some()
                || task.workspace_changes.is_some()
                || task.duration_seconds.is_some()
                || task.routed_from_agent_id.is_some()
                || task.routing_reason.is_some()
                || task.routing_evidence.is_some()
                || task.review_agent_id.is_some()
                || task.review_status != "Not Requested"
                || task.review_result.is_some()
                || task.review_model.is_some()
                || task.review_duration_seconds.is_some()
                || task.reviewed_at.is_some();
            if had_evidence {
                sanitizations.cleared_task_evidence += 1;
            }

            let nonterminal = matches!(
                task.status.as_str(),
                "Pending" | "Running" | "Blocked" | "Under Review"
            );
            if nonterminal {
                task.status = "Pending".to_string();
                task.phase = "Assigned".to_string();
                task.completed_at = None;
                task.result = None;
                task.queue_state = "held".to_string();
                held_tasks.push((
                    task.created_at.clone(),
                    agent.id,
                    task.id,
                    agent_index,
                    task_index,
                ));
                sanitizations.held_tasks += 1;
            } else {
                task.queue_state = "notQueued".to_string();
                task.enqueue_sequence = None;
            }

            task.response_id = None;
            task.runtime_model = None;
            task.total_tokens = None;
            task.changed_files.clear();
            task.diff = None;
            task.workspace_changes = None;
            task.duration_seconds = None;
            task.routed_from_agent_id = None;
            task.routing_reason = None;
            task.routing_evidence = None;
            task.review_agent_id = None;
            task.review_status = "Not Requested".to_string();
            task.review_result = None;
            task.review_model = None;
            task.review_duration_seconds = None;
            task.reviewed_at = None;
        }
    }

    held_tasks.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    for (position, (_, _, _, agent_index, task_index)) in held_tasks.into_iter().enumerate() {
        state.agents[agent_index].tasks[task_index].enqueue_sequence = Some(position as i64 + 1);
    }

    sanitizations.disabled_voice_runtime =
        state.preferences.background_voice_enabled || state.preferences.voice_state != "VOICE_OFF";
    state.preferences.background_voice_enabled = false;
    state.preferences.voice_state = "VOICE_OFF".to_string();

    validate_application_state(&state)?;
    Ok((state, sanitizations))
}

fn record_counts(
    state: &ApplicationState,
    scheduled_item_count: usize,
    memory_record_count: usize,
) -> BackupRecordCounts {
    BackupRecordCounts {
        agents: state.agents.len(),
        tasks: state.agents.iter().map(|agent| agent.tasks.len()).sum(),
        activity: state.agents.iter().map(|agent| agent.activity.len()).sum(),
        models: state.models.len(),
        approval_history: state.approval_requests.len(),
        reminders: if scheduled_item_count == 0 {
            state.reminders.len()
        } else {
            scheduled_item_count
        },
        workspaces: state.preferences.workspaces.len(),
        memory_records: memory_record_count,
    }
}

fn omitted_domains() -> Vec<String> {
    OMITTED_DOMAINS
        .iter()
        .map(|domain| (*domain).to_string())
        .collect()
}

struct NoDuplicateValue(Value);

impl<'de> Deserialize<'de> for NoDuplicateValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor { depth: 0 })
    }
}

struct NoDuplicateVisitor {
    depth: usize,
}

impl<'de> Visitor<'de> for NoDuplicateVisitor {
    type Value = NoDuplicateValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(NoDuplicateValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        self.visit_string(value.to_string())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= MAX_BACKUP_JSON_DEPTH {
            return Err(A::Error::custom(
                "backup JSON nesting exceeds the supported depth",
            ));
        }
        let mut values = Vec::new();
        while let Some(NoDuplicateValue(value)) = sequence.next_element_seed(NoDuplicateSeed {
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(NoDuplicateValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= MAX_BACKUP_JSON_DEPTH {
            return Err(A::Error::custom(
                "backup JSON nesting exceeds the supported depth",
            ));
        }
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate JSON key: {key}")));
            }
            let NoDuplicateValue(value) = map.next_value_seed(NoDuplicateSeed {
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(NoDuplicateValue(Value::Object(values)))
    }
}

struct NoDuplicateSeed {
    depth: usize,
}

impl<'de> serde::de::DeserializeSeed<'de> for NoDuplicateSeed {
    type Value = NoDuplicateValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor { depth: self.depth })
    }
}

fn parse_json_without_duplicate_keys(input: &str) -> Result<Value, StateValidationError> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let NoDuplicateValue(value) =
        NoDuplicateValue::deserialize(&mut deserializer).map_err(|error| {
            StateValidationError::new(
                "backup",
                format!("backup must be one strict JSON value: {error}"),
            )
        })?;
    deserializer.end().map_err(|error| {
        StateValidationError::new(
            "backup",
            format!("backup contains trailing content: {error}"),
        )
    })?;
    if !value.is_object() {
        return Err(StateValidationError::new(
            "backup",
            "backup must be one JSON object",
        ));
    }
    Ok(value)
}

fn date_from_unix_ms(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let mut output = String::with_capacity(10);
    let _ = write!(&mut output, "{year:04}-{month:02}-{day:02}");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::default_application_state;

    #[test]
    fn task_0014_backup_v3_round_trip_is_strict_and_deterministic() {
        let state = default_application_state().unwrap();
        let export = build_backup_export(&state, 1_777_000_000_000).unwrap();
        assert!(export.backup_json.len() <= MAX_BACKUP_BYTES);
        assert_eq!(export.counts.agents, 11);

        let candidate = parse_backup_candidate(&export.backup_json, &state).unwrap();
        let second = build_backup_export(&candidate.state, 1_777_000_000_000).unwrap();
        assert_eq!(export.backup_json, second.backup_json);
    }

    #[test]
    fn task_0014_backup_sanitizes_active_work_and_authority_evidence() {
        let current = default_application_state().unwrap();
        let mut unsafe_state = current.clone();
        let owner_id = unsafe_state.agents[0].id;
        unsafe_state.agents[0]
            .tasks
            .push(crate::app_state::AgentTask {
                id: 100,
                title: "Unsafe active task".to_string(),
                category: "Development".to_string(),
                priority: "High".to_string(),
                assigned_agent_id: owner_id,
                status: "Running".to_string(),
                phase: "Specialist Work".to_string(),
                created_at: "2026-08-26T10:00:00.000Z".to_string(),
                completed_at: None,
                result: Some("untrusted output".to_string()),
                response_id: Some("response".to_string()),
                runtime_model: Some("model".to_string()),
                total_tokens: Some(12),
                workspace_id: None,
                specialist_request: None,
                changed_files: vec!["secret.txt".to_string()],
                diff: Some("diff".to_string()),
                workspace_changes: None,
                duration_seconds: Some(1.0),
                routing_mode: "selected".to_string(),
                routed_from_agent_id: None,
                routing_reason: Some("reason".to_string()),
                queue_state: "running".to_string(),
                enqueue_sequence: Some(1),
                routing_evidence: None,
                review_agent_id: None,
                review_status: "Running".to_string(),
                review_result: None,
                review_model: None,
                review_duration_seconds: None,
                reviewed_at: None,
            });
        unsafe_state
            .approval_requests
            .push(crate::app_state::ApprovalRequest {
                id: 100,
                agent_id: owner_id,
                task_id: Some(100),
                title: "Untrusted approved record".to_string(),
                reason: "Portable data cannot carry authority".to_string(),
                status: "Approved".to_string(),
                created_at: "2026-08-26T10:00:00.000Z".to_string(),
                resolved_at: None,
                risk_level: "High".to_string(),
                scopes: vec!["files".to_string()],
                workspace_id: None,
                task_snapshot: "Unsafe active task".to_string(),
                expires_at: "2026-08-26T10:30:00.000Z".to_string(),
                consumed_at: None,
            });

        let export = build_backup_export(&unsafe_state, 1_777_000_000_000).unwrap();
        assert_eq!(export.sanitizations.held_tasks, 1);
        assert_eq!(export.sanitizations.expired_approvals, 1);
        let candidate = parse_backup_candidate(&export.backup_json, &current).unwrap();
        let task = &candidate.state.agents[0].tasks[0];
        assert_eq!(task.status, "Pending");
        assert_eq!(task.queue_state, "held");
        assert_eq!(task.phase, "Assigned");
        assert!(task.response_id.is_none());
        assert!(task.changed_files.is_empty());
        assert_eq!(candidate.sanitizations.held_tasks, 1);
        assert_eq!(candidate.state.approval_requests[0].status, "Expired");
    }

    #[test]
    fn task_0014_backup_rejects_duplicate_trailing_and_future_data() {
        let state = default_application_state().unwrap();
        let duplicate =
            r#"{"version":2,"version":2,"agents":[],"models":[],"approvalRequests":[]}"#;
        assert!(parse_backup_candidate(duplicate, &state)
            .unwrap_err()
            .message
            .contains("duplicate"));

        let trailing = r#"{"version":2,"agents":[],"models":[],"approvalRequests":[]} true"#;
        assert!(parse_backup_candidate(trailing, &state)
            .unwrap_err()
            .message
            .contains("trailing"));

        let future = r#"{"version":99}"#;
        assert_eq!(
            parse_backup_candidate(future, &state).unwrap_err().path,
            "backup.version"
        );

        let oversized = " ".repeat(MAX_BACKUP_BYTES + 1);
        assert!(parse_backup_candidate(&oversized, &state)
            .unwrap_err()
            .message
            .contains("exceeds"));

        let export = build_backup_export(&state, 1_777_000_000_000).unwrap();
        let mut unknown: Value = serde_json::from_str(&export.backup_json).unwrap();
        unknown["trustedSession"] = Value::Bool(true);
        assert!(
            parse_backup_candidate(&serde_json::to_string(&unknown).unwrap(), &state)
                .unwrap_err()
                .message
                .contains("unknown field")
        );
    }
}
