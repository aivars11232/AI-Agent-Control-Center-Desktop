use crate::agent_registry::{
    authority_for_role, template_summaries, AgentRegistrySnapshot, CreateAgentRequest,
    DeleteAgentRequest, RestoreAgentTemplateRequest, UpdateAgentRequest,
};
use crate::app_state::{
    application_state_from_legacy, default_application_state, validate_application_state,
    ActivityEntry, Agent, AgentApprovals, AgentCapabilities, AgentPerformance, AgentTask,
    AppPreferences, ApplicationState, ApprovalRequest, HistoryRetentionDays, LegacyRendererState,
    ModelDefinition, Reminder, StateValidationError, WorkspaceDefinition, CURRENT_SCHEMA_VERSION,
    MAX_SAFE_INTEGER,
};
use crate::authorization::{
    build_approval_confirmation, dialog_literal, format_unix_ms, ApprovalConfirmation,
    ApprovalResolution, AuthorizationGrant, AuthorizationOutcome,
};
use crate::data_lifecycle::{
    build_backup_export_with_domains, parse_backup_candidate, preview_for_candidate, BackupExport,
    BackupImportPreview, DataLifecycleSummary, MonitoringActivityPage, MonitoringActivityRecord,
    MonitoringCounts, MonitoringMutationResult, MonitoringRevision, MonitoringSnapshot,
    MonitoringTaskPage, MonitoringTaskRecord, RetentionMaintenanceResult, RetentionPruneCounts,
    MAX_MAINTENANCE_EVIDENCE_ROWS, MAX_MAINTENANCE_ROWS_PER_DOMAIN, MONITORING_PAGE_LIMIT,
};
use crate::management_handoffs::{
    ManagementHandoffKind, ManagementHandoffSnapshot, ManagementHandoffSource, ManagementHandoffV1,
    ManagementOwnerRole, NewManagementHandoff, MAX_HANDOFF_SUMMARY_BYTES,
};
use crate::policy::{evaluate_policy, ActionIntent, PolicyDisposition, PolicyEvaluation};
use crate::provider_runtime::{resolve_model_identity, ProviderRegistrySnapshot};
use crate::reminder_scheduler::{
    portal_policy_fingerprint, recurrence_resolution, schedule_fingerprint, system_time_zone_name,
    validate_create_request as validate_create_scheduled_item,
    validate_update_request as validate_update_scheduled_item, CreateScheduledItemRequest,
    DeleteScheduledItemRequest, DeliveryMode, DstResolution, PrivacyMode, RecurrenceKind,
    RecurrenceRuleV1, ReminderDeliveryJob, ReminderOccurrenceV1, ReminderSchedulerSnapshot,
    ScheduleStatus, ScheduledItemKind, ScheduledItemV1, SetScheduledItemStatusRequest,
    UpdateScheduledItemRequest, MAX_SCHEDULED_ITEMS,
};
use crate::review_orchestration::{
    build_review_request, human_review_result, next_required_level, parse_review_result,
    required_levels_for_role, select_reviewer, HumanReviewDecisionRequest, ReviewActor,
    ReviewFlowProjection, ReviewIntentContext, ReviewLevel, ReviewOrchestrationSnapshot,
    ReviewRequestV1, ReviewResultV1, ReviewStageAttemptProjection, ReviewStageStart, ReviewVerdict,
    StartReviewStageRequest, MAX_REVISION_ROUNDS, MAX_STAGE_ATTEMPTS, REVIEW_PIPELINE_VERSION,
};
use crate::run_coordinator::{
    bound_diff, bound_paths, validate_request_id, BoundedText, RunAttemptMode,
    RunAttemptProjection, RunAttemptStatus, RunCompletion, RunCoordinatorSnapshot,
    RunEventProjection, RunTruncationEvidence, RunUsage, MAX_ERROR_BYTES, MAX_PROGRESS_BYTES,
    MAX_PROGRESS_EVENTS, MAX_PROGRESS_MESSAGE_BYTES, MAX_RECENT_ATTEMPTS, MAX_RETAINED_ATTEMPTS,
    MAX_RETAINED_PAYLOAD_BYTES, MAX_STDERR_CAPTURE_BYTES, MAX_SUMMARY_BYTES,
};
use crate::specialist_capabilities::{
    canonical_specialist_result_json, parse_specialist_request_json, parse_specialist_result_json,
    SpecialistKind, SpecialistRunContractV1,
};
use crate::structured_memory::{
    build_prompt_bundle, validate_create_request as validate_create_memory,
    validate_delete_request as validate_delete_memory,
    validate_update_request as validate_update_memory, CreateMemoryRecordRequest,
    DeleteMemoryRecordRequest, MemoryEventV1, MemoryProvenanceKind, MemoryRecordKind,
    MemoryRecordV1, MemoryRetentionPolicy, MemoryScopeKind, MemoryScopeV1, MemorySelectionContext,
    StructuredMemorySnapshot, UpdateMemoryRecordRequest, MAX_MEMORY_RECORDS,
};
use crate::system_actions::{
    sha256_hex, validate_audit_write, AuditWrite, SystemActionAuditPage, SystemActionAuditRecord,
    MAX_SYSTEM_ACTION_AUDITS, MAX_SYSTEM_ACTION_AUDIT_PAGE,
};
use crate::task_orchestration::{
    route_task, CreateRoutedTaskRequest, QueueDisposition, RerouteTaskRequest, RoutingTaskInput,
    SetTaskQueueDispositionRequest, TaskOrchestrationSnapshot, TaskQueueEntry,
};
use crate::workspace_evidence::{
    WorkspaceChangeEvidenceV1, MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_application_state.sql");
const AUTHORIZATION_MIGRATION: &str =
    include_str!("../migrations/0002_authoritative_approvals.sql");
const RUN_COORDINATION_MIGRATION: &str = include_str!("../migrations/0003_run_coordination.sql");
const AGENT_REGISTRY_MIGRATION: &str = include_str!("../migrations/0004_agent_registry.sql");
const TASK_ORCHESTRATION_MIGRATION: &str =
    include_str!("../migrations/0005_task_orchestration.sql");
const REVIEW_ORCHESTRATION_MIGRATION: &str =
    include_str!("../migrations/0006_review_orchestration.sql");
const WORKSPACE_EVIDENCE_MIGRATION: &str =
    include_str!("../migrations/0007_workspace_evidence.sql");
const DATA_LIFECYCLE_MIGRATION: &str = include_str!("../migrations/0008_data_lifecycle.sql");
const SYSTEM_ACTION_GATEWAY_MIGRATION: &str =
    include_str!("../migrations/0009_system_action_gateway.sql");
const SPECIALIST_CAPABILITIES_MIGRATION: &str =
    include_str!("../migrations/0010_specialist_capabilities.sql");
const REMINDERS_MEMORY_HANDOFFS_MIGRATION: &str =
    include_str!("../migrations/0011_reminders_memory_handoffs.sql");
const MAX_AUTHORIZATION_RECORDS: i64 = 10_000;

#[derive(Debug, Clone)]
pub struct RunAdmission {
    pub attempt: RunAttemptProjection,
    pub authorization: Option<AuthorizationGrant>,
    pub application_state: ApplicationState,
    pub review_request_json: Option<String>,
    pub memory_bundle_json: String,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl PersistenceError {
    pub fn new(code: &str, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            recoverable,
        }
    }

    fn database(error: rusqlite::Error) -> Self {
        Self::new(
            "DATABASE_ERROR",
            format!("The application database operation failed: {error}"),
            true,
        )
    }

    fn database_or_corrupt(error: rusqlite::Error) -> Self {
        if matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        ) {
            return Self::new(
                "DATABASE_CORRUPT",
                "The application database could not be validated and was not modified.",
                false,
            );
        }
        Self::database(error)
    }

    fn validation(error: StateValidationError) -> Self {
        Self::new(
            "STATE_VALIDATION_FAILED",
            format!(
                "Stored application state is invalid at {}: {}",
                error.path, error.message
            ),
            true,
        )
    }
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PersistenceError {}

type PersistenceResult<T> = Result<T, PersistenceError>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationInfo {
    pub source_kind: Option<String>,
    pub source_version: Option<i64>,
    pub migrated_at_unix_ms: Option<i64>,
    pub legacy_cleanup_acknowledged: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateEnvelope {
    pub schema_version: i64,
    pub revision: i64,
    pub state: ApplicationState,
    pub migration: MigrationInfo,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveReceipt {
    pub schema_version: i64,
    pub revision: i64,
}

#[derive(Debug, Clone)]
struct ApplicationMeta {
    initialized: bool,
    state_revision: i64,
    source_kind: Option<String>,
    source_version: Option<i64>,
    migrated_at_unix_ms: Option<i64>,
    legacy_cleanup_ack_at_unix_ms: Option<i64>,
}

#[derive(Debug)]
pub struct StateRepository {
    connection: Connection,
}

#[derive(Clone)]
pub struct PersistenceService {
    repository: Arc<Mutex<Result<StateRepository, PersistenceError>>>,
}

impl PersistenceService {
    pub fn new(repository: Result<StateRepository, PersistenceError>) -> Self {
        Self {
            repository: Arc::new(Mutex::new(repository)),
        }
    }

    pub async fn load(&self) -> PersistenceResult<Option<StateEnvelope>> {
        self.run(|repository| repository.load()).await
    }

    pub async fn initialize(
        &self,
        legacy: LegacyRendererState,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.migrate_legacy(&legacy))
            .await
    }

    pub async fn save(
        &self,
        expected_revision: i64,
        state: ApplicationState,
        security_change_confirmed: bool,
    ) -> PersistenceResult<SaveReceipt> {
        self.run(move |repository| {
            repository.save(expected_revision, &state, security_change_confirmed)
        })
        .await
    }

    pub async fn security_change_summary(
        &self,
        expected_revision: i64,
        state: ApplicationState,
    ) -> PersistenceResult<Option<String>> {
        self.run(move |repository| repository.security_change_summary(expected_revision, &state))
            .await
    }

    pub async fn reset(
        &self,
        expected_revision: i64,
        confirmation: String,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.reset(expected_revision, &confirmation))
            .await
    }

    pub async fn export_backup(&self) -> PersistenceResult<BackupExport> {
        self.run(StateRepository::export_backup).await
    }

    pub async fn preview_backup_import(
        &self,
        expected_revision: i64,
        backup_json: String,
    ) -> PersistenceResult<BackupImportPreview> {
        self.run(move |repository| {
            repository.preview_backup_import(expected_revision, &backup_json)
        })
        .await
    }

    pub async fn apply_backup_import(
        &self,
        expected_revision: i64,
        backup_json: String,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.apply_backup_import(expected_revision, &backup_json))
            .await
    }

    pub async fn acknowledge_legacy_cleanup(
        &self,
        expected_revision: i64,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.acknowledge_legacy_cleanup(expected_revision))
            .await
    }

    pub async fn agent_registry_snapshot(&self) -> PersistenceResult<AgentRegistrySnapshot> {
        self.run(StateRepository::agent_registry_snapshot).await
    }

    pub async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.create_agent(request))
            .await
    }

    pub async fn update_agent(
        &self,
        request: UpdateAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.update_agent(request))
            .await
    }

    pub async fn delete_agent(
        &self,
        request: DeleteAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.delete_agent(request))
            .await
    }

    pub async fn restore_agent_template(
        &self,
        request: RestoreAgentTemplateRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.restore_agent_template(request))
            .await
    }

    pub async fn reminder_scheduler_snapshot(
        &self,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        self.run(StateRepository::reminder_scheduler_snapshot).await
    }

    pub async fn create_scheduled_item(
        &self,
        request: CreateScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        self.run(move |repository| repository.create_scheduled_item(request))
            .await
    }

    pub async fn update_scheduled_item(
        &self,
        request: UpdateScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        self.run(move |repository| repository.update_scheduled_item(request))
            .await
    }

    pub async fn set_scheduled_item_status(
        &self,
        request: SetScheduledItemStatusRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        self.run(move |repository| repository.set_scheduled_item_status(request))
            .await
    }

    pub async fn delete_scheduled_item(
        &self,
        request: DeleteScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        self.run(move |repository| repository.delete_scheduled_item(request))
            .await
    }

    pub async fn structured_memory_snapshot(&self) -> PersistenceResult<StructuredMemorySnapshot> {
        self.run(StateRepository::structured_memory_snapshot).await
    }

    pub async fn create_memory_record(
        &self,
        request: CreateMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        self.run(move |repository| repository.create_memory_record(request))
            .await
    }

    pub async fn update_memory_record(
        &self,
        request: UpdateMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        self.run(move |repository| repository.update_memory_record(request))
            .await
    }

    pub async fn delete_memory_record(
        &self,
        request: DeleteMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        self.run(move |repository| repository.delete_memory_record(request))
            .await
    }

    pub async fn management_handoff_snapshot(
        &self,
    ) -> PersistenceResult<ManagementHandoffSnapshot> {
        self.run(StateRepository::management_handoff_snapshot).await
    }

    pub(crate) async fn scan_due_reminders(
        &self,
        now_unix_ms: i64,
    ) -> PersistenceResult<Vec<ReminderDeliveryJob>> {
        self.run(move |repository| repository.scan_due_reminders(now_unix_ms))
            .await
    }

    pub(crate) async fn finish_reminder_delivery(
        &self,
        occurrence_id: i64,
        accepted: bool,
        detail: Option<String>,
    ) -> PersistenceResult<()> {
        self.run(move |repository| {
            repository.finish_reminder_delivery(occurrence_id, accepted, detail.as_deref())
        })
        .await
    }

    pub async fn create_routed_task(
        &self,
        request: CreateRoutedTaskRequest,
        providers: ProviderRegistrySnapshot,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.create_routed_task(request, &providers))
            .await
    }

    pub async fn reroute_task(
        &self,
        request: RerouteTaskRequest,
        providers: ProviderRegistrySnapshot,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.reroute_task(request, &providers))
            .await
    }

    pub async fn set_task_queue_disposition(
        &self,
        request: SetTaskQueueDispositionRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.set_task_queue_disposition(request))
            .await
    }

    pub async fn task_orchestration_snapshot(
        &self,
    ) -> PersistenceResult<TaskOrchestrationSnapshot> {
        self.run(StateRepository::task_orchestration_snapshot).await
    }

    pub async fn review_orchestration_snapshot(
        &self,
    ) -> PersistenceResult<ReviewOrchestrationSnapshot> {
        self.run(StateRepository::review_orchestration_snapshot)
            .await
    }

    pub async fn start_review_stage(
        &self,
        request: StartReviewStageRequest,
        providers: ProviderRegistrySnapshot,
    ) -> PersistenceResult<ReviewStageStart> {
        self.run(move |repository| repository.start_review_stage(request, &providers))
            .await
    }

    pub async fn human_review_confirmation(
        &self,
        request: HumanReviewDecisionRequest,
    ) -> PersistenceResult<ApprovalConfirmation> {
        self.run(move |repository| repository.human_review_confirmation(&request))
            .await
    }

    pub async fn record_human_review_decision(
        &self,
        request: HumanReviewDecisionRequest,
    ) -> PersistenceResult<ReviewOrchestrationSnapshot> {
        self.run(move |repository| repository.record_human_review_decision(request))
            .await
    }

    pub async fn request_authorization(
        &self,
        intent: ActionIntent,
    ) -> PersistenceResult<AuthorizationOutcome> {
        self.run(move |repository| repository.request_authorization(&intent))
            .await
    }

    pub async fn write_system_action_audit(
        &self,
        write: AuditWrite,
    ) -> PersistenceResult<SystemActionAuditRecord> {
        self.run(move |repository| repository.write_system_action_audit(&write))
            .await
    }

    pub async fn system_action_audit(
        &self,
        request_id: String,
    ) -> PersistenceResult<Option<SystemActionAuditRecord>> {
        self.run(move |repository| repository.system_action_audit(&request_id))
            .await
    }

    pub async fn query_system_action_audits(
        &self,
        limit: i64,
    ) -> PersistenceResult<SystemActionAuditPage> {
        self.run(move |repository| repository.query_system_action_audits(limit))
            .await
    }

    pub async fn resolve_approval(
        &self,
        approval_id: i64,
        resolution: ApprovalResolution,
        native_confirmed: bool,
    ) -> PersistenceResult<ApprovalRequest> {
        self.run(move |repository| {
            repository.resolve_approval(approval_id, resolution, native_confirmed)
        })
        .await
    }

    pub async fn run_data_lifecycle_maintenance(
        &self,
        trigger_kind: String,
    ) -> PersistenceResult<RetentionMaintenanceResult> {
        self.run(move |repository| {
            let timestamp = now_unix_ms()?;
            repository.run_data_lifecycle_maintenance(&trigger_kind, timestamp)
        })
        .await
    }

    pub async fn monitoring_snapshot(&self) -> PersistenceResult<MonitoringSnapshot> {
        self.run(StateRepository::monitoring_snapshot).await
    }

    pub async fn query_monitoring_tasks(
        &self,
        expected_revision: MonitoringRevision,
        status: Option<String>,
        category: Option<String>,
        offset: i64,
        limit: i64,
    ) -> PersistenceResult<MonitoringTaskPage> {
        self.run(move |repository| {
            repository.query_monitoring_tasks(
                &expected_revision,
                status.as_deref(),
                category.as_deref(),
                offset,
                limit,
            )
        })
        .await
    }

    pub async fn query_monitoring_activity(
        &self,
        expected_revision: MonitoringRevision,
        offset: i64,
        limit: i64,
    ) -> PersistenceResult<MonitoringActivityPage> {
        self.run(move |repository| {
            repository.query_monitoring_activity(&expected_revision, offset, limit)
        })
        .await
    }

    pub async fn delete_monitoring_activity(
        &self,
        expected_revision: MonitoringRevision,
        owner_agent_id: i64,
        entry_id: i64,
    ) -> PersistenceResult<MonitoringMutationResult> {
        self.run(move |repository| {
            repository.delete_monitoring_activity(&expected_revision, owner_agent_id, entry_id)
        })
        .await
    }

    pub async fn clear_monitoring_activity(
        &self,
        expected_revision: MonitoringRevision,
    ) -> PersistenceResult<MonitoringMutationResult> {
        self.run(move |repository| repository.clear_monitoring_activity(&expected_revision))
            .await
    }

    pub async fn approval_confirmation(
        &self,
        approval_id: i64,
    ) -> PersistenceResult<ApprovalConfirmation> {
        self.run(move |repository| repository.approval_confirmation(approval_id))
            .await
    }

    pub async fn authorize_intent(
        &self,
        intent: ActionIntent,
    ) -> PersistenceResult<AuthorizationGrant> {
        self.run(move |repository| repository.authorize_intent(&intent))
            .await
    }

    pub async fn authorize_intent_and_state(
        &self,
        intent: ActionIntent,
    ) -> PersistenceResult<(AuthorizationGrant, ApplicationState)> {
        self.run(move |repository| repository.authorize_intent_and_state(&intent))
            .await
    }

    pub async fn run_snapshot(&self) -> PersistenceResult<RunCoordinatorSnapshot> {
        self.run(StateRepository::run_snapshot).await
    }

    pub async fn admit_run(
        &self,
        request_id: String,
        intent: ActionIntent,
    ) -> PersistenceResult<RunAdmission> {
        self.run(move |repository| repository.admit_run(&request_id, &intent))
            .await
    }

    pub async fn prepare_run_attempt(
        &self,
        attempt_id: i64,
        provider: String,
        model: String,
        workspace_id: Option<String>,
    ) -> PersistenceResult<RunAttemptProjection> {
        self.run(move |repository| {
            repository.prepare_run_attempt(attempt_id, &provider, &model, workspace_id.as_deref())
        })
        .await
    }

    pub async fn request_run_cancellation(
        &self,
        attempt_id: i64,
    ) -> PersistenceResult<RunAttemptProjection> {
        self.run(move |repository| repository.request_run_cancellation(attempt_id))
            .await
    }

    pub async fn complete_run(
        &self,
        attempt_id: i64,
        completion: RunCompletion,
    ) -> PersistenceResult<RunAttemptProjection> {
        self.run(move |repository| repository.complete_run(attempt_id, &completion))
            .await
    }

    pub(crate) fn run_snapshot_blocking(&self) -> PersistenceResult<RunCoordinatorSnapshot> {
        self.with_repository(StateRepository::run_snapshot)
    }

    pub(crate) fn mark_run_dispatching_blocking(
        &self,
        attempt_id: i64,
    ) -> PersistenceResult<RunAttemptProjection> {
        self.with_repository(|repository| repository.mark_run_dispatching(attempt_id))
    }

    pub(crate) fn mark_run_started_blocking(
        &self,
        attempt_id: i64,
    ) -> PersistenceResult<RunAttemptProjection> {
        self.with_repository(|repository| repository.mark_run_started(attempt_id))
    }

    pub(crate) fn record_run_event_blocking(
        &self,
        attempt_id: i64,
        kind: &str,
        message: &str,
    ) -> PersistenceResult<RunEventProjection> {
        self.with_repository(|repository| repository.record_run_event(attempt_id, kind, message))
    }

    async fn run<T, F>(&self, operation: F) -> PersistenceResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut StateRepository) -> PersistenceResult<T> + Send + 'static,
    {
        let service = self.clone();
        tauri::async_runtime::spawn_blocking(move || service.with_repository(operation))
            .await
            .map_err(|_| {
                PersistenceError::new(
                    "PERSISTENCE_WORKER_STOPPED",
                    "The application persistence worker stopped unexpectedly.",
                    true,
                )
            })?
    }

    fn with_repository<T, F>(&self, operation: F) -> PersistenceResult<T>
    where
        F: FnOnce(&mut StateRepository) -> PersistenceResult<T>,
    {
        let mut repository = self.repository.lock().map_err(|_| {
            PersistenceError::new(
                "PERSISTENCE_LOCK_FAILED",
                "The application persistence service is unavailable.",
                false,
            )
        })?;
        match &mut *repository {
            Ok(repository) => operation(repository),
            Err(error) => Err(error.clone()),
        }
    }
}

impl StateRepository {
    pub fn open(path: &Path) -> PersistenceResult<Self> {
        let mut repository = Self::open_and_migrate(path)?;

        // A database whose migrations completed but that was never initialized
        // holds no user state: agent rows and the `initialized = 1` flag are
        // written in the same transaction as the first migrated or fresh state.
        // If such a shell was produced by an earlier build whose migration DDL
        // has since been corrected in place (for example the TASK-0021
        // `registry_issue` CHECK), the current binary neither re-runs the
        // migration nor can write against the stale schema, so startup is
        // permanently stuck. Rebuild the empty shell from scratch instead.
        if repository.is_superseded_uninitialized_shell()? {
            log::warn!(
                "rebuilding an uninitialized application database whose schema \
                 predates the current migrations"
            );
            drop(repository);
            remove_database_artifacts(path)?;
            repository = Self::open_and_migrate(path)?;
            if repository.is_superseded_uninitialized_shell()? {
                return Err(PersistenceError::new(
                    "SCHEMA_REBUILD_FAILED",
                    "The application database could not be rebuilt to the current schema.",
                    false,
                ));
            }
        }

        repository.reconcile_interrupted_runs()?;
        repository.reconcile_dispatched_system_actions()?;
        repository.reconcile_reserved_reminder_deliveries()?;
        let timestamp = now_unix_ms()?;
        if let Err(error) = repository.run_data_lifecycle_maintenance("startup", timestamp) {
            log::warn!("data lifecycle startup maintenance failed: {}", error.code);
        }
        Ok(repository)
    }

    fn open_and_migrate(path: &Path) -> PersistenceResult<Self> {
        prepare_private_database_file(path)?;
        let connection = Connection::open(path).map_err(PersistenceError::database)?;
        let mut repository = Self { connection };
        repository.configure_connection_preflight()?;
        repository.verify_integrity()?;
        repository.verify_supported_schema_version()?;
        repository.configure_write_durability(false)?;
        repository.apply_migrations()?;
        Ok(repository)
    }

    /// Definitions of every named schema object (tables, indexes, triggers,
    /// views) that carries explicit SQL, ordered by name. Two databases that
    /// ran the same migration statements produce byte-identical definitions.
    fn schema_object_definitions(&self) -> PersistenceResult<Vec<(String, String)>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT name, sql FROM sqlite_master
                 WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .map_err(PersistenceError::database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(PersistenceError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(PersistenceError::database)?;
        Ok(rows)
    }

    /// True only for a database that finished migrating, was never initialized,
    /// and whose persisted schema differs from a freshly migrated schema. An
    /// initialized database is never inspected and never rebuilt.
    fn is_superseded_uninitialized_shell(&self) -> PersistenceResult<bool> {
        let initialized: bool = self
            .connection
            .query_row(
                "SELECT initialized FROM application_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if initialized {
            return Ok(false);
        }
        Ok(self.schema_object_definitions()? != expected_migrated_schema_objects()?)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> PersistenceResult<Self> {
        Self::open_in_memory_internal()
    }

    /// Test-only: force an authoritative approval to be expired on the wall
    /// clock while still satisfying the `expires_at_unix_ms > created_at_unix_ms`
    /// table constraint, so the fail-closed expiry path can be exercised without
    /// waiting out a real retention window.
    #[cfg(test)]
    pub(crate) fn force_expire_approval_for_tests(
        &mut self,
        approval_id: i64,
    ) -> PersistenceResult<()> {
        self.connection
            .execute(
                "UPDATE approval_requests
                 SET created_at_unix_ms = 1, expires_at_unix_ms = 2
                 WHERE id = ?1",
                [approval_id],
            )
            .map_err(PersistenceError::database)?;
        Ok(())
    }

    fn open_in_memory_internal() -> PersistenceResult<Self> {
        let connection = Connection::open_in_memory().map_err(PersistenceError::database)?;
        let mut repository = Self { connection };
        repository.configure_connection_preflight()?;
        repository.verify_integrity()?;
        repository.verify_supported_schema_version()?;
        repository.configure_write_durability(true)?;
        repository.apply_migrations()?;
        repository.reconcile_interrupted_runs()?;
        repository.reconcile_dispatched_system_actions()?;
        repository.reconcile_reserved_reminder_deliveries()?;
        Ok(repository)
    }

    pub fn load(&mut self) -> PersistenceResult<Option<StateEnvelope>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if !meta.initialized {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(None);
        }
        let state = read_application_state(&transaction)?;
        validate_application_state(&state).map_err(PersistenceError::validation)?;
        let envelope = StateEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            revision: meta.state_revision,
            state,
            migration: migration_info(&meta),
        };
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(Some(envelope))
    }

    pub fn agent_registry_snapshot(&mut self) -> PersistenceResult<AgentRegistrySnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if !meta.initialized {
            return Err(PersistenceError::new(
                "APPLICATION_STATE_UNINITIALIZED",
                "Application state must be initialized before loading the agent registry.",
                true,
            ));
        }
        let state = read_application_state(&transaction)?;
        let defaults = default_application_state().map_err(PersistenceError::validation)?;
        let snapshot = AgentRegistrySnapshot {
            revision: meta.state_revision,
            templates: template_summaries(&defaults.agents, &state.agents),
        };
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    fn mutate_agent_registry<F>(
        &mut self,
        expected_revision: i64,
        operation: F,
    ) -> PersistenceResult<StateEnvelope>
    where
        F: FnOnce(&Transaction<'_>, &mut ApplicationState) -> PersistenceResult<()>,
    {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        ensure_run_mutation_idle(&transaction)?;
        let mut state = read_application_state(&transaction)?;
        operation(&transaction, &mut state)?;
        validate_application_state(&state).map_err(PersistenceError::validation)?;
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )?;
        let revision = next_revision(meta.state_revision)?;
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [revision],
            )
            .map_err(PersistenceError::database)?;
        advance_task_orchestration_revision(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "AGENT_REGISTRY_MUTATION_FAILED",
                "Agent registry state was not available after the mutation committed.",
                false,
            )
        })
    }

    pub fn create_agent(
        &mut self,
        request: CreateAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.mutate_agent_registry(request.expected_revision, move |transaction, state| {
            let authority_level = authority_for_role(&request.role).ok_or_else(|| {
                PersistenceError::new("AGENT_ROLE_INVALID", "Select a supported agent role.", true)
            })?;
            let id = allocate_agent_id(transaction)?;
            let reports_to = if request.role == "Supervisor" {
                None
            } else {
                request.reports_to
            };
            state.agents.push(Agent {
                id,
                template_key: None,
                registry_state: "active".to_string(),
                registry_issue: None,
                deleted_at_unix_ms: None,
                name: request.name.trim().to_string(),
                description: request.description.trim().to_string(),
                status: state.preferences.default_agent_status.clone(),
                role: request.role,
                category: request.category,
                reports_to,
                authority_level,
                model: state.preferences.default_model.clone(),
                memory: String::new(),
                tasks: Vec::new(),
                activity: Vec::new(),
                performance: state.preferences.default_performance.clone(),
                capabilities: AgentCapabilities {
                    files: "read".to_string(),
                    internet: "none".to_string(),
                    clipboard: "none".to_string(),
                    terminal: "none".to_string(),
                    system: "none".to_string(),
                },
                approvals: AgentApprovals {
                    files: "ask".to_string(),
                    internet: "ask".to_string(),
                    clipboard: "ask".to_string(),
                    terminal: "ask".to_string(),
                    system: "ask".to_string(),
                },
            });
            Ok(())
        })
    }

    pub fn update_agent(
        &mut self,
        request: UpdateAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        self.mutate_agent_registry(request.expected_revision, move |_transaction, state| {
            let authority_level = authority_for_role(&request.role).ok_or_else(|| {
                PersistenceError::new("AGENT_ROLE_INVALID", "Select a supported agent role.", true)
            })?;
            let agent = state
                .agents
                .iter_mut()
                .find(|agent| agent.id == request.agent_id)
                .ok_or_else(|| {
                    PersistenceError::new(
                        "AGENT_NOT_FOUND",
                        "The selected agent no longer exists.",
                        true,
                    )
                })?;
            if agent.registry_state == "deleted" {
                return Err(PersistenceError::new(
                    "AGENT_DELETED",
                    "Deleted agents cannot be edited.",
                    true,
                ));
            }
            agent.name = request.name.trim().to_string();
            agent.description = request.description.trim().to_string();
            agent.role = request.role;
            agent.category = request.category;
            agent.reports_to = if agent.role == "Supervisor" {
                None
            } else {
                request.reports_to
            };
            agent.authority_level = authority_level;
            agent.registry_state = "active".to_string();
            agent.registry_issue = None;
            agent.deleted_at_unix_ms = None;
            Ok(())
        })
    }

    pub fn delete_agent(
        &mut self,
        request: DeleteAgentRequest,
    ) -> PersistenceResult<StateEnvelope> {
        let timestamp = now_unix_ms()?;
        self.mutate_agent_registry(request.expected_revision, move |transaction, state| {
            let target_index = state
                .agents
                .iter()
                .position(|agent| agent.id == request.agent_id)
                .ok_or_else(|| {
                    PersistenceError::new(
                        "AGENT_NOT_FOUND",
                        "The selected agent no longer exists.",
                        true,
                    )
                })?;
            if state.agents[target_index].registry_state == "deleted" {
                return Err(PersistenceError::new(
                    "AGENT_ALREADY_DELETED",
                    "The selected agent is already deleted.",
                    true,
                ));
            }

            let direct_report_ids = state
                .agents
                .iter()
                .filter(|agent| {
                    agent.registry_state == "active" && agent.reports_to == Some(request.agent_id)
                })
                .map(|agent| agent.id)
                .collect::<Vec<_>>();
            if !direct_report_ids.is_empty() && request.replacement_manager_id.is_none() {
                return Err(PersistenceError::new(
                    "AGENT_REASSIGNMENT_REQUIRED",
                    "Reassign this agent's direct reports before deleting it.",
                    true,
                ));
            }
            if request.replacement_manager_id == Some(request.agent_id) {
                return Err(PersistenceError::new(
                    "AGENT_REASSIGNMENT_INVALID",
                    "The deleted agent cannot be its own replacement manager.",
                    true,
                ));
            }
            if let Some(replacement_manager_id) = request.replacement_manager_id {
                for agent in &mut state.agents {
                    if direct_report_ids.contains(&agent.id) {
                        agent.reports_to = Some(replacement_manager_id);
                    }
                }
            }

            let resolved_at = format_unix_ms(timestamp);
            for agent in &mut state.agents {
                if agent.performance.redirect_agent_id == Some(request.agent_id) {
                    agent.performance.redirect_agent_id = None;
                    agent.performance.overflow_action = "queue".to_string();
                }
                for task in &mut agent.tasks {
                    let nonterminal = !matches!(task.status.as_str(), "Completed" | "Failed");
                    let deleted_owner = agent.id == request.agent_id && nonterminal;
                    if deleted_owner {
                        task.status = "Failed".to_string();
                        task.phase = "Failed".to_string();
                        task.queue_state = "notQueued".to_string();
                        task.enqueue_sequence = None;
                        task.routing_evidence = None;
                        task.completed_at = Some(resolved_at.clone());
                        task.result =
                            Some("Task closed because its owning agent was deleted.".to_string());
                    } else if task.assigned_agent_id == request.agent_id && nonterminal {
                        if task.enqueue_sequence.is_none() {
                            task.enqueue_sequence = Some(allocate_enqueue_sequence(transaction)?);
                        }
                        task.assigned_agent_id = agent.id;
                        task.status = "Pending".to_string();
                        task.phase = "Assigned".to_string();
                        task.queue_state = "queued".to_string();
                        task.routed_from_agent_id = Some(request.agent_id);
                        task.routing_reason = Some(
                            "Executor reset because the previously assigned agent was deleted."
                                .to_string(),
                        );
                        task.routing_evidence = None;
                    } else if task.routed_from_agent_id == Some(request.agent_id) {
                        task.routed_from_agent_id = None;
                    }
                    if deleted_owner
                        || (task.review_agent_id == Some(request.agent_id)
                            && !matches!(
                                task.review_status.as_str(),
                                "Approved" | "Changes Requested"
                            ))
                    {
                        task.review_agent_id = None;
                        task.review_status = "Not Requested".to_string();
                        task.review_result = None;
                        task.review_model = None;
                        task.review_duration_seconds = None;
                        task.reviewed_at = None;
                    }
                }
            }
            if state.preferences.default_performance.redirect_agent_id == Some(request.agent_id) {
                state.preferences.default_performance.redirect_agent_id = None;
                state.preferences.default_performance.overflow_action = "queue".to_string();
            }
            let target = &mut state.agents[target_index];
            target.registry_state = "deleted".to_string();
            target.registry_issue = None;
            target.deleted_at_unix_ms = Some(timestamp);
            target.status = "Paused".to_string();
            target.reports_to = None;

            transaction
                .execute(
                    "UPDATE approval_requests
                     SET status = 'Expired', resolved_at = COALESCE(resolved_at, ?1)
                     WHERE agent_id = ?2 AND status IN ('Pending', 'Approved')",
                    params![resolved_at, request.agent_id],
                )
                .map_err(PersistenceError::database)?;
            transaction
                .execute(
                    "UPDATE reminders
                     SET subject_agent_id = CASE WHEN subject_agent_id = ?1 THEN NULL
                                                 ELSE subject_agent_id END,
                         task_owner_agent_id = CASE WHEN task_owner_agent_id = ?1 THEN NULL
                                                    ELSE task_owner_agent_id END,
                         task_id = CASE WHEN task_owner_agent_id = ?1 THEN NULL ELSE task_id END,
                         scheduler_agent_id = CASE WHEN scheduler_agent_id = ?1 THEN NULL
                                                   ELSE scheduler_agent_id END,
                         delivery_mode = CASE WHEN scheduler_agent_id = ?1 THEN 'in_app'
                                              ELSE delivery_mode END,
                         schedule_fingerprint = CASE WHEN scheduler_agent_id = ?1 THEN NULL
                                                     ELSE schedule_fingerprint END,
                         authorization_kind = CASE WHEN scheduler_agent_id = ?1 THEN NULL
                                                   ELSE authorization_kind END,
                         approval_id = CASE WHEN scheduler_agent_id = ?1 THEN NULL
                                            ELSE approval_id END,
                         authorization_policy_fingerprint =
                             CASE WHEN scheduler_agent_id = ?1 THEN NULL
                                  ELSE authorization_policy_fingerprint END,
                         revision = revision + 1,
                         updated_at_unix_ms = ?2
                     WHERE subject_agent_id = ?1 OR task_owner_agent_id = ?1
                        OR scheduler_agent_id = ?1",
                    params![request.agent_id, timestamp],
                )
                .map_err(PersistenceError::database)?;
            Ok(())
        })
    }

    pub fn restore_agent_template(
        &mut self,
        request: RestoreAgentTemplateRequest,
    ) -> PersistenceResult<StateEnvelope> {
        let defaults = default_application_state().map_err(PersistenceError::validation)?;
        let template = defaults
            .agents
            .iter()
            .find(|agent| agent.template_key.as_deref() == Some(request.template_key.as_str()))
            .cloned()
            .ok_or_else(|| {
                PersistenceError::new(
                    "AGENT_TEMPLATE_UNKNOWN",
                    "The requested agent template is not recognized.",
                    true,
                )
            })?;
        self.mutate_agent_registry(request.expected_revision, move |transaction, state| {
            let existing_index = state.agents.iter().position(|agent| {
                agent.template_key.as_deref() == Some(request.template_key.as_str())
            });
            if existing_index
                .and_then(|index| state.agents.get(index))
                .is_some_and(|agent| agent.registry_state == "active")
            {
                return Err(PersistenceError::new(
                    "AGENT_TEMPLATE_ACTIVE",
                    "That agent template already has an active instance.",
                    true,
                ));
            }

            let reports_to = if template.role == "Supervisor" {
                None
            } else if request.reports_to.is_some() {
                request.reports_to
            } else {
                let manager_template_key = template
                    .reports_to
                    .and_then(|manager_id| defaults.agents.iter().find(|agent| agent.id == manager_id))
                    .and_then(|agent| agent.template_key.as_deref())
                    .ok_or_else(|| {
                        PersistenceError::new(
                            "AGENT_TEMPLATE_MANAGER_MISSING",
                            "Select a compatible manager before restoring this template.",
                            true,
                        )
                    })?;
                Some(
                    state
                        .agents
                        .iter()
                        .find(|agent| {
                            agent.registry_state == "active"
                                && agent.template_key.as_deref() == Some(manager_template_key)
                        })
                        .map(|agent| agent.id)
                        .ok_or_else(|| {
                            PersistenceError::new(
                                "AGENT_TEMPLATE_MANAGER_MISSING",
                                "Restore the template's manager or select a compatible manager first.",
                                true,
                            )
                        })?,
                )
            };

            let redirect_agent_id = template.performance.redirect_agent_id.and_then(|template_id| {
                defaults
                    .agents
                    .iter()
                    .find(|agent| agent.id == template_id)
                    .and_then(|agent| agent.template_key.as_deref())
                    .and_then(|template_key| {
                        state
                            .agents
                            .iter()
                            .find(|agent| {
                                agent.registry_state == "active"
                                    && agent.template_key.as_deref() == Some(template_key)
                            })
                            .map(|agent| agent.id)
                    })
            });

            let mut restored = template.clone();
            restored.reports_to = reports_to;
            restored.performance.redirect_agent_id = redirect_agent_id;
            restored.registry_state = "active".to_string();
            restored.registry_issue = None;
            restored.deleted_at_unix_ms = None;
            if let Some(index) = existing_index {
                restored.id = state.agents[index].id;
                restored.tasks = std::mem::take(&mut state.agents[index].tasks);
                restored.activity = std::mem::take(&mut state.agents[index].activity);
                restored.memory.clear();
                state.agents[index] = restored;
            } else {
                restored.id = allocate_agent_id(transaction)?;
                state.agents.push(restored);
            }
            Ok(())
        })
    }

    pub fn reminder_scheduler_snapshot(&mut self) -> PersistenceResult<ReminderSchedulerSnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "loading the reminder scheduler")?;
        let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn create_scheduled_item(
        &mut self,
        request: CreateScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        let resolution = validate_create_scheduled_item(&request)
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let request_fingerprint =
            persistence_request_fingerprint(&request, "REMINDER_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "creating a scheduled item")?;
        if reminder_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let (revision, next_item_id, _next_occurrence_id): (i64, i64, i64) = transaction
            .query_row(
                "SELECT revision, next_reminder_id, next_occurrence_id
                 FROM reminder_scheduler_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "REMINDER_REVISION_CONFLICT",
        )?;
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM reminders", [], |row| row.get(0))
            .map_err(PersistenceError::database)?;
        if count >= MAX_SCHEDULED_ITEMS as i64 {
            return Err(PersistenceError::new(
                "REMINDER_CAPACITY_EXCEEDED",
                "The reminder scheduler already contains its maximum of 10000 items.",
                true,
            ));
        }
        validate_schedule_links(
            &transaction,
            request.subject_agent_id,
            request.workspace_id.as_deref(),
            request.task_owner_agent_id,
            request.task_id,
        )?;
        let event_end = request
            .event_end_local
            .as_deref()
            .map(|value| crate::reminder_scheduler::resolve_local_due_at(value, &request.time_zone))
            .transpose()
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let fingerprint = schedule_fingerprint(&request, &resolution)
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        let position: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM reminders",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let portal_policy = (request.delivery_mode == DeliveryMode::Portal)
            .then(|| portal_policy_fingerprint(&fingerprint));
        let new_revision = next_revision(revision)?;
        let created_at = format_unix_ms(timestamp);
        transaction
            .execute(
                "INSERT INTO reminders
                 (id, position, revision, kind, title, notes, local_due_at, time_zone,
                  due_at, due_at_unix_ms, event_end_local, event_end_unix_ms,
                  dst_resolution, status, recurrence_kind, recurrence_interval,
                  recurrence_limit, recurrence_until_unix_ms, next_occurrence_sequence,
                  missed_occurrence_count, delivery_mode, privacy_mode, schedule_fingerprint,
                  authorization_kind, approval_id, authorization_policy_fingerprint,
                  subject_agent_id, workspace_id, task_owner_agent_id, task_id,
                  scheduler_agent_id, created_at, created_at_unix_ms, updated_at_unix_ms)
                 VALUES
                 (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                  'scheduled', ?13, ?14, ?15, ?16, 0, 0, ?17, ?18, ?19, ?20,
                  ?21, ?22, ?23, ?24, ?25, ?26,
                  (SELECT id FROM agents WHERE template_key = 'event-reminder'
                   AND registry_state = 'active' ORDER BY id LIMIT 1),
                  ?27, ?28, ?28)",
                params![
                    next_item_id,
                    position,
                    request.kind.as_storage_value(),
                    request.title.trim(),
                    request.notes,
                    resolution.local_due_at,
                    resolution.time_zone,
                    resolution.due_at,
                    resolution.due_at_unix_ms,
                    event_end.as_ref().map(|value| value.local_due_at.as_str()),
                    event_end.as_ref().map(|value| value.due_at_unix_ms),
                    resolution.dst_resolution.as_storage_value(),
                    request.recurrence.kind.as_storage_value(),
                    request.recurrence.interval,
                    request.recurrence.occurrence_limit,
                    request.recurrence.until_unix_ms,
                    request.delivery_mode.as_storage_value(),
                    request.privacy_mode.as_storage_value(),
                    fingerprint,
                    portal_policy.as_ref().map(|_| "policy_allow"),
                    Option::<i64>::None,
                    portal_policy.as_deref(),
                    request.subject_agent_id,
                    request.workspace_id,
                    request.task_owner_agent_id,
                    request.task_id,
                    created_at,
                    timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta
                 SET revision = ?1, next_reminder_id = ?2
                 WHERE singleton = 1",
                params![new_revision, next_revision(next_item_id)?],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_reminder_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            Some(next_item_id),
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.reminder_scheduler_snapshot()
    }

    pub fn update_scheduled_item(
        &mut self,
        request: UpdateScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        let request_fingerprint =
            persistence_request_fingerprint(&request, "REMINDER_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "updating a scheduled item")?;
        if reminder_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM reminder_scheduler_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "REMINDER_REVISION_CONFLICT",
        )?;
        let (stored_kind, item_revision): (String, i64) = transaction
            .query_row(
                "SELECT kind, revision FROM reminders WHERE id = ?1",
                [request.item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(PersistenceError::database)?
            .ok_or_else(|| {
                PersistenceError::new(
                    "REMINDER_NOT_FOUND",
                    "The selected scheduled item no longer exists.",
                    true,
                )
            })?;
        if item_revision != request.expected_item_revision {
            return Err(PersistenceError::new(
                "REMINDER_ITEM_REVISION_CONFLICT",
                "The scheduled item changed; refresh it before editing.",
                true,
            ));
        }
        let kind = ScheduledItemKind::from_storage_value(&stored_kind)
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        let resolution = validate_update_scheduled_item(&request, kind)
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        validate_schedule_links(
            &transaction,
            request.subject_agent_id,
            request.workspace_id.as_deref(),
            request.task_owner_agent_id,
            request.task_id,
        )?;
        let event_end = request
            .event_end_local
            .as_deref()
            .map(|value| crate::reminder_scheduler::resolve_local_due_at(value, &request.time_zone))
            .transpose()
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let create_shape = CreateScheduledItemRequest {
            expected_revision: request.expected_revision,
            request_id: request.request_id.clone(),
            kind,
            title: request.title.clone(),
            notes: request.notes.clone(),
            local_due_at: request.local_due_at.clone(),
            time_zone: request.time_zone.clone(),
            event_end_local: request.event_end_local.clone(),
            recurrence: request.recurrence.clone(),
            delivery_mode: request.delivery_mode,
            privacy_mode: request.privacy_mode,
            subject_agent_id: request.subject_agent_id,
            workspace_id: request.workspace_id.clone(),
            task_owner_agent_id: request.task_owner_agent_id,
            task_id: request.task_id,
        };
        let fingerprint = schedule_fingerprint(&create_shape, &resolution)
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        let portal_policy = (request.delivery_mode == DeliveryMode::Portal)
            .then(|| portal_policy_fingerprint(&fingerprint));
        let new_revision = next_revision(revision)?;
        let new_item_revision = next_revision(item_revision)?;
        transaction
            .execute(
                "UPDATE reminders
                 SET revision = ?1, title = ?2, notes = ?3, local_due_at = ?4,
                     time_zone = ?5, due_at = ?6, due_at_unix_ms = ?7,
                     event_end_local = ?8, event_end_unix_ms = ?9,
                     dst_resolution = ?10, status = 'scheduled',
                     recurrence_kind = ?11, recurrence_interval = ?12,
                     recurrence_limit = ?13, recurrence_until_unix_ms = ?14,
                     next_occurrence_sequence = 0, missed_occurrence_count = 0,
                     delivery_mode = ?15, privacy_mode = ?16,
                     schedule_fingerprint = ?17, authorization_kind = ?18,
                     approval_id = ?19, authorization_policy_fingerprint = ?20,
                     subject_agent_id = ?21, workspace_id = ?22,
                     task_owner_agent_id = ?23, task_id = ?24,
                     schedule_issue_code = NULL, schedule_issue_message = NULL,
                     resolved_at_unix_ms = NULL, updated_at_unix_ms = ?25
                 WHERE id = ?26",
                params![
                    new_item_revision,
                    request.title.trim(),
                    request.notes,
                    resolution.local_due_at,
                    resolution.time_zone,
                    resolution.due_at,
                    resolution.due_at_unix_ms,
                    event_end.as_ref().map(|value| value.local_due_at.as_str()),
                    event_end.as_ref().map(|value| value.due_at_unix_ms),
                    resolution.dst_resolution.as_storage_value(),
                    request.recurrence.kind.as_storage_value(),
                    request.recurrence.interval,
                    request.recurrence.occurrence_limit,
                    request.recurrence.until_unix_ms,
                    request.delivery_mode.as_storage_value(),
                    request.privacy_mode.as_storage_value(),
                    fingerprint,
                    portal_policy.as_ref().map(|_| "policy_allow"),
                    Option::<i64>::None,
                    portal_policy.as_deref(),
                    request.subject_agent_id,
                    request.workspace_id,
                    request.task_owner_agent_id,
                    request.task_id,
                    timestamp,
                    request.item_id,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta SET revision = ?1 WHERE singleton = 1",
                [new_revision],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_reminder_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            Some(request.item_id),
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.reminder_scheduler_snapshot()
    }

    pub fn set_scheduled_item_status(
        &mut self,
        request: SetScheduledItemStatusRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        if request.expected_revision < 0
            || request.expected_item_revision <= 0
            || request.item_id <= 0
            || request.request_id.trim().is_empty()
            || request.request_id.len() > 128
            || !matches!(
                request.status,
                ScheduleStatus::Scheduled | ScheduleStatus::Completed | ScheduleStatus::Dismissed
            )
        {
            return Err(PersistenceError::new(
                "REMINDER_REQUEST_INVALID",
                "The scheduled-item status request is invalid.",
                true,
            ));
        }
        let request_fingerprint =
            persistence_request_fingerprint(&request, "REMINDER_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "changing a scheduled-item status")?;
        if reminder_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM reminder_scheduler_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "REMINDER_REVISION_CONFLICT",
        )?;
        let item_revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM reminders WHERE id = ?1",
                [request.item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        let item_revision = item_revision.ok_or_else(|| {
            PersistenceError::new(
                "REMINDER_NOT_FOUND",
                "The selected scheduled item no longer exists.",
                true,
            )
        })?;
        if item_revision != request.expected_item_revision {
            return Err(PersistenceError::new(
                "REMINDER_ITEM_REVISION_CONFLICT",
                "The scheduled item changed; refresh it before changing its status.",
                true,
            ));
        }
        let new_revision = next_revision(revision)?;
        transaction
            .execute(
                "UPDATE reminders
                 SET revision = revision + 1, status = ?1,
                     resolved_at_unix_ms = CASE WHEN ?1 IN ('completed', 'dismissed')
                                                THEN ?2 ELSE NULL END,
                     updated_at_unix_ms = ?2
                 WHERE id = ?3",
                params![
                    request.status.as_storage_value(),
                    timestamp,
                    request.item_id
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta SET revision = ?1 WHERE singleton = 1",
                [new_revision],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_reminder_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            Some(request.item_id),
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.reminder_scheduler_snapshot()
    }

    pub fn delete_scheduled_item(
        &mut self,
        request: DeleteScheduledItemRequest,
    ) -> PersistenceResult<ReminderSchedulerSnapshot> {
        if request.expected_revision < 0
            || request.expected_item_revision <= 0
            || request.item_id <= 0
            || request.request_id.trim().is_empty()
            || request.request_id.len() > 128
        {
            return Err(PersistenceError::new(
                "REMINDER_REQUEST_INVALID",
                "The scheduled-item delete request is invalid.",
                true,
            ));
        }
        let request_fingerprint =
            persistence_request_fingerprint(&request, "REMINDER_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "deleting a scheduled item")?;
        if reminder_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM reminder_scheduler_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "REMINDER_REVISION_CONFLICT",
        )?;
        let changed = transaction
            .execute(
                "DELETE FROM reminders WHERE id = ?1 AND revision = ?2",
                params![request.item_id, request.expected_item_revision],
            )
            .map_err(PersistenceError::database)?;
        if changed == 0 {
            return Err(PersistenceError::new(
                "REMINDER_ITEM_REVISION_CONFLICT",
                "The scheduled item is absent or changed; refresh before deleting it.",
                true,
            ));
        }
        let new_revision = next_revision(revision)?;
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta SET revision = ?1 WHERE singleton = 1",
                [new_revision],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_reminder_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            None,
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.reminder_scheduler_snapshot()
    }

    pub fn structured_memory_snapshot(&mut self) -> PersistenceResult<StructuredMemorySnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "loading structured memory")?;
        let snapshot = read_structured_memory_snapshot(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn create_memory_record(
        &mut self,
        request: CreateMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        validate_create_memory(&request)
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let request_fingerprint =
            persistence_request_fingerprint(&request, "MEMORY_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "creating a memory record")?;
        if memory_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_structured_memory_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let (revision, next_record_id, next_event_id): (i64, i64, i64) = transaction
            .query_row(
                "SELECT revision, next_record_id, next_event_id
                 FROM structured_memory_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "MEMORY_REVISION_CONFLICT",
        )?;
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
            .map_err(PersistenceError::database)?;
        if count >= MAX_MEMORY_RECORDS as i64 {
            return Err(PersistenceError::new(
                "MEMORY_CAPACITY_EXCEEDED",
                "Structured memory already contains its maximum of 50000 records.",
                true,
            ));
        }
        validate_memory_scope_references(&transaction, &request.scope)?;
        let expires_at = request.retention.expiry_from(timestamp);
        transaction
            .execute(
                "INSERT INTO memory_records
                 (id, scope_kind, agent_id, workspace_id, task_owner_agent_id, task_id,
                  team_leader_agent_id, record_kind, content, provenance_kind,
                  provenance_ref, revision, retention_policy, expires_at_unix_ms,
                  created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'user', NULL, 1,
                         ?10, ?11, ?12, ?12)",
                params![
                    next_record_id,
                    request.scope.kind.as_storage_value(),
                    request.scope.agent_id,
                    request.scope.workspace_id,
                    request.scope.task_owner_agent_id,
                    request.scope.task_id,
                    request.scope.team_leader_agent_id,
                    request.kind.as_storage_value(),
                    request.content,
                    request.retention.as_storage_value(),
                    expires_at,
                    timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "INSERT INTO memory_events
                 (id, record_id, action, actor_kind, record_revision, created_at_unix_ms)
                 VALUES (?1, ?2, 'created', 'human', 1, ?3)",
                params![next_event_id, next_record_id, timestamp],
            )
            .map_err(PersistenceError::database)?;
        let new_revision = next_revision(revision)?;
        transaction
            .execute(
                "UPDATE structured_memory_meta
                 SET revision = ?1, next_record_id = ?2, next_event_id = ?3
                 WHERE singleton = 1",
                params![
                    new_revision,
                    next_revision(next_record_id)?,
                    next_revision(next_event_id)?,
                ],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_memory_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            Some(next_record_id),
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.structured_memory_snapshot()
    }

    pub fn update_memory_record(
        &mut self,
        request: UpdateMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        validate_update_memory(&request)
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let request_fingerprint =
            persistence_request_fingerprint(&request, "MEMORY_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "updating a memory record")?;
        if memory_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_structured_memory_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let (revision, next_event_id): (i64, i64) = transaction
            .query_row(
                "SELECT revision, next_event_id FROM structured_memory_meta
                 WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "MEMORY_REVISION_CONFLICT",
        )?;
        let stored_scope_kind: Option<String> = transaction
            .query_row(
                "SELECT scope_kind FROM memory_records
                 WHERE id = ?1 AND revision = ?2",
                params![request.record_id, request.expected_record_revision],
                |row| row.get(0),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        let stored_scope_kind = stored_scope_kind.ok_or_else(|| {
            PersistenceError::new(
                "MEMORY_RECORD_REVISION_CONFLICT",
                "The memory record is absent or changed; refresh it before editing.",
                true,
            )
        })?;
        if request.retention == MemoryRetentionPolicy::TaskLifetime && stored_scope_kind != "task" {
            return Err(PersistenceError::new(
                "MEMORY_RETENTION_INVALID",
                "Task-lifetime retention is only valid for task-scoped memory.",
                true,
            ));
        }
        let new_record_revision = next_revision(request.expected_record_revision)?;
        transaction
            .execute(
                "UPDATE memory_records
                 SET record_kind = ?1, content = ?2, revision = ?3,
                     retention_policy = ?4, expires_at_unix_ms = ?5,
                     updated_at_unix_ms = ?6
                 WHERE id = ?7",
                params![
                    request.kind.as_storage_value(),
                    request.content,
                    new_record_revision,
                    request.retention.as_storage_value(),
                    request.retention.expiry_from(timestamp),
                    timestamp,
                    request.record_id,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "INSERT INTO memory_events
                 (id, record_id, action, actor_kind, record_revision, created_at_unix_ms)
                 VALUES (?1, ?2, 'updated', 'human', ?3, ?4)",
                params![
                    next_event_id,
                    request.record_id,
                    new_record_revision,
                    timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        let new_revision = next_revision(revision)?;
        transaction
            .execute(
                "UPDATE structured_memory_meta
                 SET revision = ?1, next_event_id = ?2 WHERE singleton = 1",
                params![new_revision, next_revision(next_event_id)?],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_memory_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            Some(request.record_id),
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.structured_memory_snapshot()
    }

    pub fn delete_memory_record(
        &mut self,
        request: DeleteMemoryRecordRequest,
    ) -> PersistenceResult<StructuredMemorySnapshot> {
        validate_delete_memory(&request)
            .map_err(|error| PersistenceError::new(error.code, error.message, true))?;
        let request_fingerprint =
            persistence_request_fingerprint(&request, "MEMORY_REQUEST_INVALID")?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "deleting a memory record")?;
        if memory_request_is_duplicate(&transaction, &request.request_id, &request_fingerprint)? {
            let snapshot = read_structured_memory_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(snapshot);
        }
        let (revision, next_event_id): (i64, i64) = transaction
            .query_row(
                "SELECT revision, next_event_id FROM structured_memory_meta
                 WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        ensure_subsystem_revision(
            revision,
            request.expected_revision,
            "MEMORY_REVISION_CONFLICT",
        )?;
        let changed = transaction
            .execute(
                "DELETE FROM memory_records WHERE id = ?1 AND revision = ?2",
                params![request.record_id, request.expected_record_revision],
            )
            .map_err(PersistenceError::database)?;
        if changed == 0 {
            return Err(PersistenceError::new(
                "MEMORY_RECORD_REVISION_CONFLICT",
                "The memory record is absent or changed; refresh it before deleting.",
                true,
            ));
        }
        transaction
            .execute(
                "INSERT INTO memory_events
                 (id, record_id, action, actor_kind, record_revision, created_at_unix_ms)
                 VALUES (?1, ?2, 'deleted', 'human', ?3, ?4)",
                params![
                    next_event_id,
                    request.record_id,
                    request.expected_record_revision,
                    timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        let new_revision = next_revision(revision)?;
        transaction
            .execute(
                "UPDATE structured_memory_meta
                 SET revision = ?1, next_event_id = ?2 WHERE singleton = 1",
                params![new_revision, next_revision(next_event_id)?],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        record_memory_request(
            &transaction,
            &request.request_id,
            &request_fingerprint,
            new_revision,
            None,
            timestamp,
        )?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.structured_memory_snapshot()
    }

    pub fn management_handoff_snapshot(&mut self) -> PersistenceResult<ManagementHandoffSnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "loading management handoffs")?;
        let snapshot = read_management_handoff_snapshot(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn scan_due_reminders(
        &mut self,
        now_unix_ms: i64,
    ) -> PersistenceResult<Vec<ReminderDeliveryJob>> {
        if now_unix_ms < 0 {
            return Err(PersistenceError::new(
                "REMINDER_SCAN_INVALID",
                "The reminder scan timestamp must be non-negative.",
                false,
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_application_initialized(&transaction, "scanning reminders")?;
        let (scheduler_revision, mut next_occurrence_id): (i64, i64) = transaction
            .query_row(
                "SELECT revision, next_occurrence_id FROM reminder_scheduler_meta
                 WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        let snapshot = read_reminder_scheduler_snapshot(&transaction)?;
        let mut jobs = Vec::new();
        let mut changed = false;
        let mut processed = 0_usize;
        for item in snapshot.items.iter().filter(|item| {
            item.status == ScheduleStatus::Scheduled
                && item.due_at_unix_ms.is_some_and(|due| due <= now_unix_ms)
        }) {
            if processed >= 1_000 {
                break;
            }
            let mut sequence = item.next_occurrence_sequence;
            let mut due_at_unix_ms = item
                .due_at_unix_ms
                .expect("filtered scheduled reminder has an instant");
            let mut next_resolution = None;
            let mut recurrence_error = None;
            let mut item_missed = 0_i64;
            loop {
                if processed >= 1_000 {
                    break;
                }
                let missed = i64::from(due_at_unix_ms < now_unix_ms);
                item_missed = item_missed.saturating_add(missed);
                let occurrence_key = format!(
                    "reminder-v1:{}:{}:{}:{}",
                    item.id, item.revision, sequence, due_at_unix_ms
                );
                let occurrence_status = if item.delivery_mode == DeliveryMode::Portal {
                    "reserved"
                } else {
                    "in_app_due"
                };
                let inserted = transaction
                    .execute(
                        "INSERT OR IGNORE INTO reminder_occurrences
                         (id, reminder_id, schedule_revision, occurrence_sequence,
                          occurrence_key, due_at_unix_ms, status, missed_count,
                          first_missed_at_unix_ms, last_missed_at_unix_ms,
                          portal_notification_id, created_at_unix_ms, updated_at_unix_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                                 CASE WHEN ?8 > 0 THEN ?9 END,
                                 CASE WHEN ?8 > 0 THEN ?9 END,
                                 CASE WHEN ?7 = 'reserved' THEN ?5 END, ?9, ?9)",
                        params![
                            next_occurrence_id,
                            item.id,
                            item.revision,
                            sequence,
                            occurrence_key,
                            due_at_unix_ms,
                            occurrence_status,
                            missed,
                            now_unix_ms,
                        ],
                    )
                    .map_err(PersistenceError::database)?;
                if inserted == 1 {
                    processed += 1;
                    if item.delivery_mode == DeliveryMode::Portal {
                        let (title, body) = match item.privacy_mode {
                            PrivacyMode::Generic => (
                                "AI Agent Control Center reminder".to_string(),
                                "A scheduled item is due. Open the app to inspect it.".to_string(),
                            ),
                            PrivacyMode::Title => {
                                (item.title.clone(), "A scheduled item is due.".to_string())
                            }
                        };
                        jobs.push(ReminderDeliveryJob {
                            occurrence_id: next_occurrence_id,
                            notification_id: occurrence_key,
                            title,
                            body,
                        });
                    }
                    next_occurrence_id = next_revision(next_occurrence_id)?;
                }

                let next_sequence = next_revision(sequence)?;
                match recurrence_resolution(
                    &item.local_due_at,
                    &item.time_zone,
                    &item.recurrence,
                    next_sequence,
                ) {
                    Ok(resolution) => next_resolution = resolution,
                    Err(error) => {
                        sequence = next_sequence;
                        recurrence_error = Some(error);
                        break;
                    }
                }
                match next_resolution.as_ref() {
                    Some(next) if next.due_at_unix_ms <= now_unix_ms => {
                        sequence = next_sequence;
                        due_at_unix_ms = next.due_at_unix_ms;
                    }
                    _ => {
                        sequence = next_sequence;
                        break;
                    }
                }
            }

            if let Some(error) = recurrence_error {
                transaction
                    .execute(
                        "UPDATE reminders
                         SET status = 'needs_attention', next_occurrence_sequence = ?1,
                             schedule_issue_code = ?2, schedule_issue_message = ?3,
                             missed_occurrence_count = missed_occurrence_count + ?4,
                             updated_at_unix_ms = ?5
                         WHERE id = ?6",
                        params![
                            sequence,
                            error.code,
                            error.message,
                            item_missed,
                            now_unix_ms,
                            item.id,
                        ],
                    )
                    .map_err(PersistenceError::database)?;
                changed = true;
                continue;
            }

            if processed >= 1_000
                && next_resolution
                    .as_ref()
                    .is_some_and(|next| next.due_at_unix_ms <= now_unix_ms)
            {
                transaction
                    .execute(
                        "UPDATE reminders
                         SET status = 'needs_attention', schedule_issue_code = 'MISSED_EVENT_LIMIT',
                             schedule_issue_message =
                                 'More than 1000 due occurrences require review before scheduling continues.',
                             missed_occurrence_count = missed_occurrence_count + ?1,
                             updated_at_unix_ms = ?2
                         WHERE id = ?3",
                        params![item_missed, now_unix_ms, item.id],
                    )
                    .map_err(PersistenceError::database)?;
                changed = true;
                break;
            }

            match next_resolution {
                Some(next) => {
                    transaction
                        .execute(
                            "UPDATE reminders
                             SET due_at = ?1, due_at_unix_ms = ?2, dst_resolution = ?3,
                                 next_occurrence_sequence = ?4, status = 'scheduled',
                                 missed_occurrence_count = missed_occurrence_count + ?5,
                                 updated_at_unix_ms = ?6
                             WHERE id = ?7",
                            params![
                                next.due_at,
                                next.due_at_unix_ms,
                                next.dst_resolution.as_storage_value(),
                                sequence,
                                item_missed,
                                now_unix_ms,
                                item.id,
                            ],
                        )
                        .map_err(PersistenceError::database)?;
                }
                None => {
                    transaction
                        .execute(
                            "UPDATE reminders
                             SET status = 'due', missed_occurrence_count = missed_occurrence_count + ?1,
                                 updated_at_unix_ms = ?2
                             WHERE id = ?3",
                            params![item_missed, now_unix_ms, item.id],
                        )
                        .map_err(PersistenceError::database)?;
                }
            }
            changed = true;
        }
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta
                 SET next_occurrence_id = ?1, last_scan_at_unix_ms = ?2,
                     last_error_code = NULL, last_error_message = NULL
                 WHERE singleton = 1",
                params![next_occurrence_id, now_unix_ms],
            )
            .map_err(PersistenceError::database)?;
        if changed {
            transaction
                .execute(
                    "UPDATE reminder_scheduler_meta SET revision = ?1 WHERE singleton = 1",
                    [next_revision(scheduler_revision)?],
                )
                .map_err(PersistenceError::database)?;
            advance_application_revision(&transaction)?;
        }
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(jobs)
    }

    pub fn finish_reminder_delivery(
        &mut self,
        occurrence_id: i64,
        accepted: bool,
        detail: Option<&str>,
    ) -> PersistenceResult<()> {
        if occurrence_id <= 0 || detail.is_some_and(|value| value.len() > 4 * 1024) {
            return Err(PersistenceError::new(
                "REMINDER_DELIVERY_INVALID",
                "The notification delivery result is invalid.",
                false,
            ));
        }
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM reminder_occurrences WHERE id = ?1",
                [occurrence_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        match status.as_deref() {
            Some("reserved") => {}
            Some("portal_accepted") if accepted => {
                transaction.commit().map_err(PersistenceError::database)?;
                return Ok(());
            }
            Some(_) => {
                return Err(PersistenceError::new(
                    "REMINDER_DELIVERY_STATE_CONFLICT",
                    "The notification delivery is no longer reserved.",
                    true,
                ));
            }
            None => {
                return Err(PersistenceError::new(
                    "REMINDER_OCCURRENCE_NOT_FOUND",
                    "The reminder occurrence no longer exists.",
                    true,
                ));
            }
        }
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM reminder_scheduler_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE reminder_occurrences
                 SET status = ?1, detail_code = ?2, detail_message = ?3,
                     updated_at_unix_ms = ?4
                 WHERE id = ?5",
                params![
                    if accepted {
                        "portal_accepted"
                    } else {
                        "failed"
                    },
                    if accepted {
                        None
                    } else {
                        Some("PORTAL_DELIVERY_FAILED")
                    },
                    detail,
                    timestamp,
                    occurrence_id,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE reminder_scheduler_meta SET revision = ?1 WHERE singleton = 1",
                [next_revision(revision)?],
            )
            .map_err(PersistenceError::database)?;
        advance_application_revision(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)
    }

    pub fn create_routed_task(
        &mut self,
        request: CreateRoutedTaskRequest,
        providers: &ProviderRegistrySnapshot,
    ) -> PersistenceResult<StateEnvelope> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, request.expected_revision)?;
        let mut state = read_application_state(&transaction)?;
        let owner_index = state
            .agents
            .iter()
            .position(|agent| agent.id == request.task_owner_agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_OWNER_NOT_FOUND",
                    "The selected task owner no longer exists.",
                    true,
                )
            })?;
        if state.agents[owner_index].registry_state != "active" {
            return Err(PersistenceError::new(
                "TASK_OWNER_INACTIVE",
                "New tasks require an active registry owner.",
                true,
            ));
        }
        let (task_id, enqueue_sequence) = allocate_task_and_enqueue_sequence(&transaction)?;
        let mut task = AgentTask {
            id: task_id,
            title: request.title.trim().to_string(),
            category: request.category,
            priority: request.priority,
            assigned_agent_id: request.task_owner_agent_id,
            status: "Pending".to_string(),
            phase: "Assigned".to_string(),
            created_at: format_unix_ms(timestamp),
            completed_at: None,
            result: None,
            response_id: None,
            runtime_model: None,
            total_tokens: None,
            workspace_id: Some(request.workspace_id),
            specialist_request: request.specialist_request,
            changed_files: Vec::new(),
            diff: None,
            workspace_changes: None,
            duration_seconds: None,
            routing_mode: request.routing_mode,
            routed_from_agent_id: None,
            routing_reason: None,
            queue_state: "queued".to_string(),
            enqueue_sequence: Some(enqueue_sequence),
            routing_evidence: None,
            review_agent_id: None,
            review_status: "Not Requested".to_string(),
            review_result: None,
            review_model: None,
            review_duration_seconds: None,
            reviewed_at: None,
        };
        let input = RoutingTaskInput {
            task_owner_agent_id: request.task_owner_agent_id,
            task: task.clone(),
            preferred_agent_id: request.preferred_agent_id,
            selected_agent_id: request.selected_agent_id,
        };
        let evidence = route_task(&state, providers, &input).map_err(routing_error)?;
        task.assigned_agent_id = evidence.winning_agent_id;
        task.routing_reason = Some(evidence.reason.clone());
        task.routing_evidence = Some(evidence);
        state.agents[owner_index].tasks.push(task.clone());
        validate_application_state(&state).map_err(PersistenceError::validation)?;

        let position: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1
                 FROM agent_tasks WHERE owner_agent_id = ?1",
                [request.task_owner_agent_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        write_task(
            &transaction,
            request.task_owner_agent_id,
            position as usize,
            &task,
            (Some(timestamp), None),
            timestamp,
        )?;
        let owner_role = management_owner_role(&state.agents[owner_index].role);
        insert_management_handoff(
            &transaction,
            NewManagementHandoff {
                task_owner_agent_id: request.task_owner_agent_id,
                task_id,
                kind: ManagementHandoffKind::TaskPlan,
                from_agent_id: Some(request.task_owner_agent_id),
                to_agent_id: Some(task.assigned_agent_id),
                owner_role,
                revision_round: 0,
                run_attempt_id: None,
                review_flow_id: None,
                review_stage_attempt_id: None,
                source: ManagementHandoffSource::TaskOrchestration,
                summary: format!("Plan created for task: {}", task.title),
                payload: serde_json::json!({
                    "title": task.title,
                    "category": task.category,
                    "priority": task.priority,
                    "workspaceId": task.workspace_id,
                    "routingMode": task.routing_mode,
                    "queueState": task.queue_state,
                }),
                idempotency_key: format!(
                    "task-orchestration:plan:{}:{}",
                    request.task_owner_agent_id, task_id
                ),
            },
            timestamp,
        )?;
        insert_management_handoff(
            &transaction,
            NewManagementHandoff {
                task_owner_agent_id: request.task_owner_agent_id,
                task_id,
                kind: ManagementHandoffKind::Assignment,
                from_agent_id: Some(request.task_owner_agent_id),
                to_agent_id: Some(task.assigned_agent_id),
                owner_role,
                revision_round: 0,
                run_attempt_id: None,
                review_flow_id: None,
                review_stage_attempt_id: None,
                source: ManagementHandoffSource::TaskOrchestration,
                summary: "Task assigned through deterministic routing.".to_string(),
                payload: serde_json::json!({
                    "assignedAgentId": task.assigned_agent_id,
                    "routingReason": task.routing_reason,
                    "routingEvidence": task.routing_evidence,
                }),
                idempotency_key: format!(
                    "task-orchestration:assignment:{}:{}:initial",
                    request.task_owner_agent_id, task_id
                ),
            },
            timestamp,
        )?;
        finish_task_orchestration_mutation(&transaction, meta.state_revision)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "TASK_CREATE_FAILED",
                "Application state was unavailable after task creation.",
                false,
            )
        })
    }

    pub fn reroute_task(
        &mut self,
        request: RerouteTaskRequest,
        providers: &ProviderRegistrySnapshot,
    ) -> PersistenceResult<StateEnvelope> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, request.expected_revision)?;
        ensure_task_has_no_active_run(&transaction, request.task_owner_agent_id, request.task_id)?;
        ensure_task_has_no_active_review_flow(
            &transaction,
            request.task_owner_agent_id,
            request.task_id,
        )?;
        let mut state = read_application_state(&transaction)?;
        let (owner_index, task_index) =
            find_task_indexes(&state, request.task_owner_agent_id, request.task_id)?;
        if state.agents[owner_index].registry_state != "active" {
            return Err(PersistenceError::new(
                "TASK_OWNER_INACTIVE",
                "Queued tasks can only be rerouted while their owner is active.",
                true,
            ));
        }
        let current = state.agents[owner_index].tasks[task_index].clone();
        if !matches!(current.queue_state.as_str(), "queued" | "held") {
            return Err(PersistenceError::new(
                "TASK_QUEUE_LOCKED",
                "Only queued or held tasks can be rerouted.",
                true,
            ));
        }
        let mut task = current.clone();
        task.title = request.title.trim().to_string();
        task.category = request.category;
        task.priority = request.priority;
        task.workspace_id = Some(request.workspace_id);
        task.specialist_request = request.specialist_request;
        task.routing_mode = request.routing_mode;
        let input = RoutingTaskInput {
            task_owner_agent_id: request.task_owner_agent_id,
            task: task.clone(),
            preferred_agent_id: request.preferred_agent_id,
            selected_agent_id: request.selected_agent_id,
        };
        let evidence = route_task(&state, providers, &input).map_err(routing_error)?;
        if current.assigned_agent_id != evidence.winning_agent_id {
            task.routed_from_agent_id = Some(current.assigned_agent_id);
        }
        task.assigned_agent_id = evidence.winning_agent_id;
        task.routing_reason = Some(evidence.reason.clone());
        task.routing_evidence = Some(evidence);
        state.agents[owner_index].tasks[task_index] = task.clone();
        validate_application_state(&state).map_err(PersistenceError::validation)?;
        let routing_evidence_json =
            serde_json::to_string(&task.routing_evidence).map_err(|_| {
                PersistenceError::new(
                    "ROUTING_EVIDENCE_INVALID",
                    "Task routing evidence could not be normalized.",
                    false,
                )
            })?;
        let specialist_request_json = task
            .specialist_request
            .as_ref()
            .map(|request| request.canonical_json())
            .transpose()
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        transaction
            .execute(
                "UPDATE agent_tasks
                 SET title = ?1, category = ?2, priority = ?3, workspace_id = ?4,
                     routing_mode = ?5, assigned_agent_id = ?6,
                     routed_from_agent_id = ?7, routing_reason = ?8,
                     routing_evidence_json = ?9, specialist_request_json = ?10
                 WHERE owner_agent_id = ?11 AND id = ?12",
                params![
                    task.title,
                    task.category,
                    task.priority,
                    task.workspace_id,
                    task.routing_mode,
                    task.assigned_agent_id,
                    task.routed_from_agent_id,
                    task.routing_reason,
                    routing_evidence_json,
                    specialist_request_json,
                    request.task_owner_agent_id,
                    request.task_id
                ],
            )
            .map_err(PersistenceError::database)?;
        insert_management_handoff(
            &transaction,
            NewManagementHandoff {
                task_owner_agent_id: request.task_owner_agent_id,
                task_id: request.task_id,
                kind: ManagementHandoffKind::Assignment,
                from_agent_id: task.routed_from_agent_id,
                to_agent_id: Some(task.assigned_agent_id),
                owner_role: management_owner_role(&state.agents[owner_index].role),
                revision_round: 0,
                run_attempt_id: None,
                review_flow_id: None,
                review_stage_attempt_id: None,
                source: ManagementHandoffSource::TaskOrchestration,
                summary: "Task assignment changed through deterministic rerouting.".to_string(),
                payload: serde_json::json!({
                    "assignedAgentId": task.assigned_agent_id,
                    "routedFromAgentId": task.routed_from_agent_id,
                    "routingReason": task.routing_reason,
                    "routingEvidence": task.routing_evidence,
                }),
                idempotency_key: format!(
                    "task-orchestration:assignment:{}:{}:revision:{}",
                    request.task_owner_agent_id, request.task_id, meta.state_revision
                ),
            },
            timestamp,
        )?;
        expire_task_approvals(&transaction, request.task_owner_agent_id, request.task_id)?;
        finish_task_orchestration_mutation(&transaction, meta.state_revision)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "TASK_REROUTE_FAILED",
                "Application state was unavailable after task rerouting.",
                false,
            )
        })
    }

    pub fn set_task_queue_disposition(
        &mut self,
        request: SetTaskQueueDispositionRequest,
    ) -> PersistenceResult<StateEnvelope> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, request.expected_revision)?;
        ensure_task_has_no_active_run(&transaction, request.task_owner_agent_id, request.task_id)?;
        ensure_review_queue_mutation_allowed(
            &transaction,
            request.task_owner_agent_id,
            request.task_id,
        )?;
        let mut state = read_application_state(&transaction)?;
        let (owner_index, task_index) =
            find_task_indexes(&state, request.task_owner_agent_id, request.task_id)?;
        let task = &mut state.agents[owner_index].tasks[task_index];
        match request.disposition {
            QueueDisposition::Hold => {
                if task.queue_state != "queued" {
                    return Err(PersistenceError::new(
                        "TASK_QUEUE_STATE_CONFLICT",
                        "Only a queued task can be held.",
                        true,
                    ));
                }
                task.queue_state = "held".to_string();
                task.status = "Blocked".to_string();
                task.phase = "Supervisor Approval".to_string();
            }
            QueueDisposition::Resume => {
                if task.queue_state != "held" {
                    return Err(PersistenceError::new(
                        "TASK_QUEUE_STATE_CONFLICT",
                        "Only a held task can be resumed.",
                        true,
                    ));
                }
                task.queue_state = "queued".to_string();
                task.status = "Pending".to_string();
                task.phase = "Assigned".to_string();
            }
            QueueDisposition::ResetTerminal => {
                if task.queue_state != "notQueued"
                    || !matches!(task.status.as_str(), "Completed" | "Failed")
                {
                    return Err(PersistenceError::new(
                        "TASK_QUEUE_STATE_CONFLICT",
                        "Only a terminal task outside the queue can be reset.",
                        true,
                    ));
                }
                task.enqueue_sequence = Some(allocate_enqueue_sequence(&transaction)?);
                task.queue_state = "queued".to_string();
                task.status = "Pending".to_string();
                task.phase = "Assigned".to_string();
                task.completed_at = None;
                task.result = None;
                task.response_id = None;
                task.runtime_model = None;
                task.total_tokens = None;
                task.changed_files.clear();
                task.diff = None;
                task.workspace_changes = None;
                task.duration_seconds = None;
                task.review_agent_id = None;
                task.review_status = "Not Requested".to_string();
                task.review_result = None;
                task.review_model = None;
                task.review_duration_seconds = None;
                task.reviewed_at = None;
            }
        }
        validate_application_state(&state).map_err(PersistenceError::validation)?;
        let task = &state.agents[owner_index].tasks[task_index];
        transaction
            .execute(
                "UPDATE agent_tasks
                 SET queue_state = ?1, enqueue_sequence = ?2, status = ?3, phase = ?4,
                     completed_at = ?5, result = ?6, response_id = ?7,
                     runtime_model = ?8, total_tokens = ?9, diff = ?10,
                     workspace_evidence_json = ?11,
                     duration_seconds = ?12, review_agent_id = ?13,
                     review_status = ?14, review_result = ?15, review_model = ?16,
                     review_duration_seconds = ?17, reviewed_at = ?18
                 WHERE owner_agent_id = ?19 AND id = ?20",
                params![
                    task.queue_state,
                    task.enqueue_sequence,
                    task.status,
                    task.phase,
                    task.completed_at,
                    task.result,
                    task.response_id,
                    task.runtime_model,
                    task.total_tokens,
                    task.diff,
                    task.workspace_changes
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|_| PersistenceError::new(
                            "INVALID_WORKSPACE_EVIDENCE",
                            "Task workspace evidence could not be normalized.",
                            false,
                        ))?,
                    task.duration_seconds,
                    task.review_agent_id,
                    task.review_status,
                    task.review_result,
                    task.review_model,
                    task.review_duration_seconds,
                    task.reviewed_at,
                    request.task_owner_agent_id,
                    request.task_id
                ],
            )
            .map_err(PersistenceError::database)?;
        if request.disposition == QueueDisposition::ResetTerminal {
            transaction
                .execute(
                    "DELETE FROM task_changed_files
                     WHERE owner_agent_id = ?1 AND task_id = ?2",
                    params![request.task_owner_agent_id, request.task_id],
                )
                .map_err(PersistenceError::database)?;
        }
        expire_task_approvals(&transaction, request.task_owner_agent_id, request.task_id)?;
        finish_task_orchestration_mutation(&transaction, meta.state_revision)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "TASK_QUEUE_UPDATE_FAILED",
                "Application state was unavailable after the queue update.",
                false,
            )
        })
    }

    pub fn task_orchestration_snapshot(&mut self) -> PersistenceResult<TaskOrchestrationSnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM task_orchestration_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let mut execute_queue = read_task_queue_entries(&transaction, "queued")?;
        for (position, entry) in execute_queue.iter_mut().enumerate() {
            entry.queue_position = Some(position as i64 + 1);
        }
        let held_tasks = read_task_queue_entries(&transaction, "held")?;
        let active_entries = read_active_task_queue_entries(&transaction)?;
        if active_entries.len() > 1 {
            return Err(PersistenceError::new(
                "TASK_QUEUE_CORRUPT",
                "More than one execute task is marked active.",
                false,
            ));
        }
        let active_execute = active_entries.into_iter().next();
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(TaskOrchestrationSnapshot {
            revision,
            execute_queue,
            held_tasks,
            active_execute,
        })
    }

    pub fn review_orchestration_snapshot(
        &mut self,
    ) -> PersistenceResult<ReviewOrchestrationSnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        let snapshot = read_review_orchestration_snapshot(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn start_review_stage(
        &mut self,
        request: StartReviewStageRequest,
        providers: &ProviderRegistrySnapshot,
    ) -> PersistenceResult<ReviewStageStart> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        ensure_expected_review_revision(&transaction, request.expected_revision)?;
        ensure_task_has_no_active_run(&transaction, request.task_owner_agent_id, request.task_id)?;
        let flow = read_active_review_flow_binding(
            &transaction,
            request.task_owner_agent_id,
            request.task_id,
        )?;
        if flow.state == "awaiting_human" {
            let snapshot = read_review_orchestration_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(ReviewStageStart {
                snapshot,
                stage: None,
                context: None,
                blocked_code: flow.last_error_code,
                blocked_message: flow.last_error_message,
            });
        }
        if flow.state != "awaiting_review" {
            return Err(PersistenceError::new(
                "REVIEW_STATE_CONFLICT",
                "The review flow is not waiting for a reviewer stage.",
                true,
            ));
        }
        let level = flow.current_level.ok_or_else(|| {
            PersistenceError::new(
                "REVIEW_LEDGER_INVALID",
                "The active review flow has no current review level.",
                false,
            )
        })?;
        let attempt_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM review_stage_attempts
                 WHERE flow_id = ?1 AND revision_round = ?2 AND level = ?3
                   AND actor = 'agent'",
                params![flow.id, flow.revision_round, level.as_storage()],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if attempt_count >= MAX_STAGE_ATTEMPTS {
            mark_review_awaiting_human(
                &transaction,
                &flow,
                "REVIEW_STAGE_ATTEMPTS_EXHAUSTED",
                "The current review stage exhausted its three bounded agent attempts.",
                timestamp,
            )?;
            let snapshot = read_review_orchestration_snapshot(&transaction)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(ReviewStageStart {
                snapshot,
                stage: None,
                context: None,
                blocked_code: Some("REVIEW_STAGE_ATTEMPTS_EXHAUSTED".to_string()),
                blocked_message: Some(
                    "The current review stage exhausted its three bounded agent attempts."
                        .to_string(),
                ),
            });
        }

        let state = read_application_state(&transaction)?;
        let prior_reviewer_ids = read_prior_reviewer_ids(&transaction, flow.id, level)?;
        let owner = state
            .agents
            .iter()
            .find(|agent| agent.id == request.task_owner_agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_OWNER_NOT_FOUND",
                    "The task owner no longer exists.",
                    true,
                )
            })?;
        let task = owner
            .tasks
            .iter()
            .find(|task| task.id == request.task_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_NOT_FOUND",
                    "The selected task no longer exists.",
                    true,
                )
            })?;
        let required_template_key = (level == ReviewLevel::Senior
            && task
                .specialist_request
                .as_ref()
                .is_some_and(|specialist| specialist.kind() == SpecialistKind::Coding))
        .then_some("debugging");
        let reviewer = match select_reviewer(
            &state,
            providers,
            flow.executor_agent_id,
            level,
            &prior_reviewer_ids,
            required_template_key,
        ) {
            Ok(reviewer) => reviewer.clone(),
            Err(error) => {
                mark_review_awaiting_human(
                    &transaction,
                    &flow,
                    &error.code,
                    &error.message,
                    timestamp,
                )?;
                let snapshot = read_review_orchestration_snapshot(&transaction)?;
                transaction.commit().map_err(PersistenceError::database)?;
                return Ok(ReviewStageStart {
                    snapshot,
                    stage: None,
                    context: None,
                    blocked_code: Some(error.code),
                    blocked_message: Some(error.message),
                });
            }
        };
        let executor = state
            .agents
            .iter()
            .find(|agent| agent.id == flow.executor_agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "REVIEW_EXECUTOR_NOT_FOUND",
                    "The task executor no longer exists.",
                    true,
                )
            })?;
        let execution_attempt_id = flow.latest_execution_attempt_id.ok_or_else(|| {
            PersistenceError::new(
                "REVIEW_EVIDENCE_MISSING",
                "The review flow has no successful execution evidence.",
                false,
            )
        })?;
        let execution = read_run_attempt(&transaction, execution_attempt_id)?;
        if execution.status != RunAttemptStatus::Succeeded
            || execution.run_mode != RunAttemptMode::Execute
        {
            return Err(PersistenceError::new(
                "REVIEW_EVIDENCE_INVALID",
                "The bound review evidence is not a successful execution attempt.",
                false,
            ));
        }
        let stage_attempt_id = allocate_review_stage_id(&transaction)?;
        let review_request = build_review_request(
            flow.id,
            stage_attempt_id,
            flow.revision_round,
            level,
            task,
            executor,
            &execution,
        )
        .map_err(review_protocol_error)?;
        let request_json = serde_json::to_string(&review_request).map_err(|_| {
            PersistenceError::new(
                "REVIEW_REQUEST_INVALID",
                "The authoritative review request could not be stored.",
                false,
            )
        })?;
        transaction
            .execute(
                "INSERT INTO review_stage_attempts
                 (id, flow_id, revision_round, level, attempt_number, actor,
                  reviewer_agent_id, state, request_json, request_fingerprint,
                  created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'agent', ?6, 'pending', ?7, ?8, ?9)",
                params![
                    stage_attempt_id,
                    flow.id,
                    flow.revision_round,
                    level.as_storage(),
                    attempt_count + 1,
                    reviewer.id,
                    request_json,
                    review_request.request_fingerprint,
                    timestamp
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE review_flows
                 SET state = 'review_pending', last_error_code = NULL,
                     last_error_message = NULL, updated_at_unix_ms = ?1
                 WHERE id = ?2 AND state = 'awaiting_review'",
                params![timestamp, flow.id],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "UPDATE agent_tasks
                 SET status = 'Under Review', phase = ?1, review_agent_id = ?2,
                     review_status = 'Pending', review_model = NULL,
                     review_duration_seconds = NULL, reviewed_at = NULL
                 WHERE owner_agent_id = ?3 AND id = ?4",
                params![
                    level.task_phase(),
                    reviewer.id,
                    request.task_owner_agent_id,
                    request.task_id
                ],
            )
            .map_err(PersistenceError::database)?;
        advance_review_revision(&transaction)?;
        advance_task_orchestration_revision(&transaction)?;
        let snapshot = read_review_orchestration_snapshot(&transaction)?;
        let stage = snapshot
            .flows
            .iter()
            .find(|candidate| candidate.id == flow.id)
            .and_then(|candidate| {
                candidate
                    .stages
                    .iter()
                    .find(|stage| stage.id == stage_attempt_id)
            })
            .cloned();
        let context = Some(review_request.intent_context());
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(ReviewStageStart {
            snapshot,
            stage,
            context,
            blocked_code: None,
            blocked_message: None,
        })
    }

    pub fn human_review_confirmation(
        &mut self,
        request: &HumanReviewDecisionRequest,
    ) -> PersistenceResult<ApprovalConfirmation> {
        validate_human_review_request(request)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        ensure_expected_review_revision(&transaction, request.expected_revision)?;
        let flow = read_active_review_flow_binding(
            &transaction,
            request.task_owner_agent_id,
            request.task_id,
        )?;
        ensure_human_review_flow(&flow, request.flow_id)?;
        let task_title: String = transaction
            .query_row(
                "SELECT title FROM agent_tasks WHERE owner_agent_id = ?1 AND id = ?2",
                params![request.task_owner_agent_id, request.task_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(ApprovalConfirmation {
            title: "Confirm human review decision".to_string(),
            message: format!(
                "Task: {} (ID {})\nReview flow: {}\nRevision round: {} of {}\nDecision: {}\nFeedback: {}\n\nThis trusted decision can complete the task or queue another execution revision. Continue?",
                dialog_literal(&task_title),
                request.task_id,
                request.flow_id,
                flow.revision_round,
                MAX_REVISION_ROUNDS,
                match request.verdict {
                    ReviewVerdict::Approved => "Approve",
                    ReviewVerdict::ChangesRequested => "Request changes",
                },
                if request.feedback.trim().is_empty() {
                    "None".to_string()
                } else {
                    dialog_literal(request.feedback.trim())
                }
            ),
        })
    }

    pub fn record_human_review_decision(
        &mut self,
        request: HumanReviewDecisionRequest,
    ) -> PersistenceResult<ReviewOrchestrationSnapshot> {
        validate_human_review_request(&request)?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        ensure_expected_review_revision(&transaction, request.expected_revision)?;
        ensure_task_has_no_active_run(&transaction, request.task_owner_agent_id, request.task_id)?;
        let flow = read_active_review_flow_binding(
            &transaction,
            request.task_owner_agent_id,
            request.task_id,
        )?;
        ensure_human_review_flow(&flow, request.flow_id)?;
        let state = read_application_state(&transaction)?;
        let owner = state
            .agents
            .iter()
            .find(|agent| agent.id == request.task_owner_agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_OWNER_NOT_FOUND",
                    "The task owner no longer exists.",
                    true,
                )
            })?;
        let task = owner
            .tasks
            .iter()
            .find(|task| task.id == request.task_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_NOT_FOUND",
                    "The selected task no longer exists.",
                    true,
                )
            })?;
        let executor = state
            .agents
            .iter()
            .find(|agent| agent.id == flow.executor_agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "REVIEW_EXECUTOR_NOT_FOUND",
                    "The task executor no longer exists.",
                    true,
                )
            })?;
        let execution_attempt_id = flow.latest_execution_attempt_id.ok_or_else(|| {
            PersistenceError::new(
                "REVIEW_EVIDENCE_MISSING",
                "The review flow has no successful execution evidence.",
                false,
            )
        })?;
        let execution = read_run_attempt(&transaction, execution_attempt_id)?;
        if execution.run_mode != RunAttemptMode::Execute
            || !matches!(
                execution.status,
                RunAttemptStatus::Succeeded | RunAttemptStatus::Interrupted
            )
        {
            return Err(PersistenceError::new(
                "REVIEW_EVIDENCE_INVALID",
                "Trusted human review requires bound execution or explicit legacy-recovery evidence.",
                false,
            ));
        }
        let level = flow.current_level.unwrap_or(ReviewLevel::Supervisor);
        let prior_attempts: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM review_stage_attempts
                 WHERE flow_id = ?1 AND revision_round = ?2 AND level = ?3",
                params![flow.id, flow.revision_round, level.as_storage()],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if prior_attempts > MAX_STAGE_ATTEMPTS {
            return Err(PersistenceError::new(
                "HUMAN_REVIEW_ALREADY_RECORDED",
                "A human decision has already been recorded for this review stage.",
                true,
            ));
        }
        let stage_attempt_id = allocate_review_stage_id(&transaction)?;
        let review_request = build_review_request(
            flow.id,
            stage_attempt_id,
            flow.revision_round,
            level,
            task,
            executor,
            &execution,
        )
        .map_err(review_protocol_error)?;
        let result = human_review_result(&review_request, request.verdict, &request.feedback)
            .map_err(review_protocol_error)?;
        let request_json = serde_json::to_string(&review_request).map_err(|_| {
            PersistenceError::new(
                "REVIEW_REQUEST_INVALID",
                "The human review request could not be stored.",
                false,
            )
        })?;
        let result_json = serde_json::to_string(&result).map_err(|_| {
            PersistenceError::new(
                "REVIEW_RESULT_INVALID",
                "The human review result could not be stored.",
                false,
            )
        })?;
        transaction
            .execute(
                "INSERT INTO review_stage_attempts
                 (id, flow_id, revision_round, level, attempt_number, actor,
                  reviewer_agent_id, state, request_json, request_fingerprint,
                  result_json, verdict, feedback, created_at_unix_ms,
                  started_at_unix_ms, completed_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'human', NULL, ?6, ?7, ?8,
                         ?9, ?10, ?11, ?12, ?12, ?12)",
                params![
                    stage_attempt_id,
                    flow.id,
                    flow.revision_round,
                    level.as_storage(),
                    prior_attempts + 1,
                    result.verdict.as_storage(),
                    request_json,
                    review_request.request_fingerprint,
                    result_json,
                    result.verdict.as_storage(),
                    result.feedback,
                    timestamp
                ],
            )
            .map_err(PersistenceError::database)?;
        apply_review_verdict(
            &transaction,
            &flow,
            level,
            &result,
            ReviewActor::Human,
            None,
            None,
            None,
            timestamp,
        )?;
        insert_management_handoff(
            &transaction,
            NewManagementHandoff {
                task_owner_agent_id: request.task_owner_agent_id,
                task_id: request.task_id,
                kind: ManagementHandoffKind::HumanOverride,
                from_agent_id: None,
                to_agent_id: Some(flow.executor_agent_id),
                owner_role: ManagementOwnerRole::Human,
                revision_round: flow.revision_round,
                run_attempt_id: Some(execution_attempt_id),
                review_flow_id: Some(flow.id),
                review_stage_attempt_id: Some(stage_attempt_id),
                source: ManagementHandoffSource::HumanDecision,
                summary: format!("Human review decision: {}", result.verdict.as_storage()),
                payload: serde_json::json!({
                    "level": level.as_storage(),
                    "verdict": result.verdict.as_storage(),
                    "feedback": result.feedback,
                    "requestFingerprint": review_request.request_fingerprint,
                }),
                idempotency_key: format!(
                    "human-review:{}:{}:{}",
                    flow.id, flow.revision_round, stage_attempt_id
                ),
            },
            timestamp,
        )?;
        let snapshot = read_review_orchestration_snapshot(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn initialize_fresh(&mut self) -> PersistenceResult<StateEnvelope> {
        let state = default_application_state().map_err(PersistenceError::validation)?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if meta.initialized {
            return Err(PersistenceError::new(
                "ALREADY_INITIALIZED",
                "Application state has already been initialized.",
                true,
            ));
        }
        write_application_state(&transaction, &state, "fresh", &HashMap::new(), true)?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = 1, source_kind = 'fresh',
                     source_version = NULL, migrated_at_unix_ms = ?1,
                     legacy_cleanup_ack_at_unix_ms = NULL
                 WHERE singleton = 1",
                [timestamp],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "INITIALIZATION_FAILED",
                "Application state did not become available after initialization.",
                false,
            )
        })
    }

    pub fn migrate_legacy(
        &mut self,
        legacy: &LegacyRendererState,
    ) -> PersistenceResult<StateEnvelope> {
        if let Some(envelope) = self.load()? {
            return Ok(envelope);
        }
        if legacy.is_empty() {
            return self.initialize_fresh();
        }

        let state = application_state_from_legacy(legacy).map_err(PersistenceError::validation)?;
        let timestamp = now_unix_ms()?;
        let approval_origins = state
            .approval_requests
            .iter()
            .map(|request| (request.id, "legacy_local_storage".to_string()))
            .collect::<HashMap<_, _>>();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if meta.initialized {
            return Err(PersistenceError::new(
                "ALREADY_INITIALIZED",
                "Application state was initialized by another database connection.",
                true,
            ));
        }
        write_application_state(
            &transaction,
            &state,
            "legacy_local_storage",
            &approval_origins,
            true,
        )?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = 1,
                     source_kind = 'legacy_local_storage', source_version = 0,
                     migrated_at_unix_ms = ?1, legacy_cleanup_ack_at_unix_ms = NULL
                 WHERE singleton = 1",
                [timestamp],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "MIGRATION_FAILED",
                "Migrated application state did not become available after commit.",
                false,
            )
        })
    }

    pub fn acknowledge_legacy_cleanup(
        &mut self,
        expected_revision: i64,
    ) -> PersistenceResult<StateEnvelope> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        if meta.source_kind.as_deref() != Some("legacy_local_storage") {
            return Err(PersistenceError::new(
                "LEGACY_CLEANUP_NOT_APPLICABLE",
                "The current state was not migrated from renderer local storage.",
                true,
            ));
        }
        if meta.legacy_cleanup_ack_at_unix_ms.is_none() {
            transaction
                .execute(
                    "UPDATE application_meta
                     SET legacy_cleanup_ack_at_unix_ms = ?1 WHERE singleton = 1",
                    [timestamp],
                )
                .map_err(PersistenceError::database)?;
        }
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "LEGACY_CLEANUP_ACK_FAILED",
                "Application state was unavailable after recording legacy cleanup.",
                false,
            )
        })
    }

    pub fn export_backup(&mut self) -> PersistenceResult<BackupExport> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if !meta.initialized {
            return Err(PersistenceError::new(
                "APPLICATION_STATE_UNINITIALIZED",
                "Application state must be initialized before exporting a backup.",
                true,
            ));
        }
        let state = read_application_state(&transaction)?;
        let reminders = read_reminder_scheduler_snapshot(&transaction)?;
        let memory = read_structured_memory_snapshot(&transaction)?;
        let backup =
            build_backup_export_with_domains(&state, &reminders.items, &memory.records, timestamp)
                .map_err(PersistenceError::validation)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(backup)
    }

    pub fn run_data_lifecycle_maintenance(
        &mut self,
        trigger_kind: &str,
        timestamp: i64,
    ) -> PersistenceResult<RetentionMaintenanceResult> {
        if !matches!(
            trigger_kind,
            "startup" | "interval" | "settings" | "import" | "test"
        ) {
            return Err(PersistenceError::new(
                "DATA_LIFECYCLE_TRIGGER_INVALID",
                "Data lifecycle maintenance received an unsupported trigger.",
                false,
            ));
        }
        if timestamp < 0 {
            return Err(PersistenceError::new(
                "CLOCK_UNAVAILABLE",
                "Data lifecycle maintenance requires a non-negative backend timestamp.",
                false,
            ));
        }

        match self.run_data_lifecycle_maintenance_transaction(trigger_kind, timestamp) {
            Ok(result) => Ok(result),
            Err(error) => {
                let _ = self.record_data_lifecycle_failure(trigger_kind, timestamp, &error);
                Err(error)
            }
        }
    }

    fn run_data_lifecycle_maintenance_transaction(
        &mut self,
        trigger_kind: &str,
        timestamp: i64,
    ) -> PersistenceResult<RetentionMaintenanceResult> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let application_meta = application_meta_from(&transaction)?;
        let (lifecycle_revision, last_observed_at): (i64, Option<i64>) = transaction
            .query_row(
                "SELECT revision, last_observed_at_unix_ms
                 FROM data_lifecycle_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        let next_lifecycle_revision = next_revision(lifecycle_revision)?;
        let clock_rollback = last_observed_at.is_some_and(|last| timestamp < last);
        let (task_cutoff, activity_cutoff) = if application_meta.initialized && !clock_rollback {
            (
                retention_cutoff(&transaction, "task_retention", timestamp)?,
                retention_cutoff(&transaction, "activity_retention", timestamp)?,
            )
        } else {
            (None, None)
        };

        let pruned = if clock_rollback {
            RetentionPruneCounts::default()
        } else {
            prune_retention_rows(&transaction, task_cutoff, activity_cutoff, timestamp)?
        };
        let skipped_protected = if clock_rollback {
            0
        } else {
            count_retention_protected(&transaction, task_cutoff, activity_cutoff)?
        };
        let backlog_remaining = if clock_rollback {
            false
        } else {
            retention_backlog_exists(&transaction, task_cutoff, activity_cutoff, timestamp)?
        };

        let mut application_state_revision = application_meta.state_revision;
        if pruned.application_rows() > 0 {
            application_state_revision = next_revision(application_state_revision)?;
            transaction
                .execute(
                    "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                    [application_state_revision],
                )
                .map_err(PersistenceError::database)?;
        }
        if pruned.tasks > 0 {
            advance_task_orchestration_revision(&transaction)?;
        }
        if pruned.attempts > 0 {
            update_run_retention_meta(&transaction, pruned.attempts, timestamp)?;
        }
        if pruned.review_flows > 0 {
            advance_review_revision(&transaction)?;
        }

        let status = if clock_rollback {
            "clock_rollback"
        } else {
            "succeeded"
        };
        let error_code = clock_rollback.then(|| "CLOCK_ROLLBACK".to_string());
        let error_message = clock_rollback.then(|| {
            "Backend time moved behind the last observed maintenance time; age-based pruning was skipped."
                .to_string()
        });
        insert_data_lifecycle_run(
            &transaction,
            next_lifecycle_revision,
            application_state_revision,
            trigger_kind,
            status,
            timestamp,
            timestamp,
            task_cutoff,
            activity_cutoff,
            &pruned,
            skipped_protected,
            backlog_remaining,
            error_code.as_deref(),
            error_message.as_deref(),
        )?;
        trim_data_lifecycle_runs(&transaction)?;
        transaction
            .execute(
                "UPDATE data_lifecycle_meta
                 SET revision = ?1,
                     last_observed_at_unix_ms = CASE
                         WHEN last_observed_at_unix_ms IS NULL
                              OR last_observed_at_unix_ms < ?2 THEN ?2
                         ELSE last_observed_at_unix_ms
                     END,
                     last_started_at_unix_ms = ?2,
                     last_completed_at_unix_ms = ?2,
                     last_success_at_unix_ms = CASE WHEN ?3 = 0 THEN ?2 ELSE last_success_at_unix_ms END,
                     last_error_code = ?4,
                     last_error_message = ?5,
                     total_runs = total_runs + 1,
                     total_pruned_tasks = total_pruned_tasks + ?6,
                     total_pruned_attempts = total_pruned_attempts + ?7,
                     total_pruned_review_flows = total_pruned_review_flows + ?8,
                     total_pruned_activity = total_pruned_activity + ?9,
                     total_pruned_approvals = total_pruned_approvals + ?10,
                     total_pruned_reminders = total_pruned_reminders + ?11,
                     total_pruned_system_action_audits =
                         total_pruned_system_action_audits + ?12,
                     total_pruned_memory_records = total_pruned_memory_records + ?13,
                     total_pruned_reminder_occurrences =
                         total_pruned_reminder_occurrences + ?14,
                     total_pruned_management_handoffs =
                         total_pruned_management_handoffs + ?15
                 WHERE singleton = 1",
                params![
                    next_lifecycle_revision,
                    timestamp,
                    clock_rollback as i64,
                    error_code,
                    error_message,
                    pruned.tasks,
                    pruned.attempts,
                    pruned.review_flows,
                    pruned.activity,
                    pruned.approvals,
                    pruned.reminders,
                    pruned.system_action_audits,
                    pruned.memory_records,
                    pruned.reminder_occurrences,
                    pruned.management_handoffs,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;

        Ok(RetentionMaintenanceResult {
            lifecycle_revision: next_lifecycle_revision,
            application_state_revision,
            trigger_kind: trigger_kind.to_string(),
            status: status.to_string(),
            started_at_unix_ms: timestamp,
            completed_at_unix_ms: timestamp,
            task_cutoff_unix_ms: task_cutoff,
            activity_cutoff_unix_ms: activity_cutoff,
            pruned,
            skipped_protected,
            backlog_remaining,
            error_code,
            error_message,
        })
    }

    fn record_data_lifecycle_failure(
        &mut self,
        trigger_kind: &str,
        timestamp: i64,
        error: &PersistenceError,
    ) -> PersistenceResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let lifecycle_revision: i64 = transaction
            .query_row(
                "SELECT revision FROM data_lifecycle_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let next_lifecycle_revision = next_revision(lifecycle_revision)?;
        let application_state_revision = application_meta_from(&transaction)?.state_revision;
        let message = bounded_lifecycle_message(&error.message);
        insert_data_lifecycle_run(
            &transaction,
            next_lifecycle_revision,
            application_state_revision,
            trigger_kind,
            "failed",
            timestamp,
            timestamp,
            None,
            None,
            &RetentionPruneCounts::default(),
            0,
            false,
            Some(&error.code),
            Some(&message),
        )?;
        trim_data_lifecycle_runs(&transaction)?;
        transaction
            .execute(
                "UPDATE data_lifecycle_meta
                 SET revision = ?1,
                     last_observed_at_unix_ms = CASE
                         WHEN last_observed_at_unix_ms IS NULL
                              OR last_observed_at_unix_ms < ?2 THEN ?2
                         ELSE last_observed_at_unix_ms
                     END,
                     last_started_at_unix_ms = ?2,
                     last_completed_at_unix_ms = ?2,
                     last_error_code = ?3,
                     last_error_message = ?4,
                     total_runs = total_runs + 1
                 WHERE singleton = 1",
                params![next_lifecycle_revision, timestamp, error.code, message],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)
    }

    pub fn monitoring_snapshot(&mut self) -> PersistenceResult<MonitoringSnapshot> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let snapshot = read_monitoring_snapshot(&transaction, timestamp)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(snapshot)
    }

    pub fn query_monitoring_tasks(
        &mut self,
        expected_revision: &MonitoringRevision,
        status: Option<&str>,
        category: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> PersistenceResult<MonitoringTaskPage> {
        validate_monitoring_page(offset, limit)?;
        let offset_usize = usize::try_from(offset).map_err(|_| {
            PersistenceError::new(
                "MONITORING_PAGE_INVALID",
                "Task monitoring offset exceeds this platform's supported range.",
                true,
            )
        })?;
        let limit_usize = usize::try_from(limit).map_err(|_| {
            PersistenceError::new(
                "MONITORING_PAGE_INVALID",
                "Task monitoring limit exceeds this platform's supported range.",
                true,
            )
        })?;
        if status.is_some_and(|value| {
            !matches!(
                value,
                "Pending" | "Running" | "Blocked" | "Under Review" | "Completed" | "Failed"
            )
        }) {
            return Err(PersistenceError::new(
                "MONITORING_FILTER_INVALID",
                "Task monitoring received an unsupported status filter.",
                true,
            ));
        }
        if category.is_some_and(|value| {
            !matches!(
                value,
                "Development"
                    | "Research"
                    | "Browsing"
                    | "Finance"
                    | "Business"
                    | "Communication"
                    | "System Control"
                    | "General"
            )
        }) {
            return Err(PersistenceError::new(
                "MONITORING_FILTER_INVALID",
                "Task monitoring received an unsupported category filter.",
                true,
            ));
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let revision = monitoring_revision(&transaction)?;
        ensure_monitoring_revision(&revision, expected_revision)?;
        let state = read_application_state(&transaction)?;
        let agent_names = state
            .agents
            .iter()
            .map(|agent| (agent.id, agent.name.clone()))
            .collect::<HashMap<_, _>>();
        let mut timestamp_statement = transaction
            .prepare(
                "SELECT owner_agent_id, id, created_at_unix_ms, completed_at_unix_ms
                 FROM agent_tasks",
            )
            .map_err(PersistenceError::database)?;
        let timestamp_rows = timestamp_statement
            .query_map([], |row| {
                Ok((
                    (row.get::<_, i64>(0)?, row.get::<_, i64>(1)?),
                    (
                        row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        row.get::<_, Option<i64>>(3)?,
                    ),
                ))
            })
            .map_err(PersistenceError::database)?;
        let mut timestamps = HashMap::new();
        for row in timestamp_rows {
            let (key, value) = row.map_err(PersistenceError::database)?;
            timestamps.insert(key, value);
        }
        drop(timestamp_statement);

        let mut records = state
            .agents
            .iter()
            .flat_map(|owner| {
                owner.tasks.iter().filter_map(|task| {
                    if status.is_some_and(|value| task.status != value)
                        || category.is_some_and(|value| task.category != value)
                    {
                        return None;
                    }
                    let (created_at_unix_ms, completed_at_unix_ms) = timestamps
                        .get(&(owner.id, task.id))
                        .copied()
                        .unwrap_or((0, None));
                    Some(MonitoringTaskRecord {
                        owner_agent_id: owner.id,
                        owner_name: owner.name.clone(),
                        owner_role: owner.role.clone(),
                        executor_name: agent_names.get(&task.assigned_agent_id).cloned(),
                        created_at_unix_ms,
                        completed_at_unix_ms,
                        task: task.clone(),
                    })
                })
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            monitoring_queue_rank(&left.task)
                .cmp(&monitoring_queue_rank(&right.task))
                .then_with(|| {
                    left.task
                        .enqueue_sequence
                        .unwrap_or(MAX_SAFE_INTEGER)
                        .cmp(&right.task.enqueue_sequence.unwrap_or(MAX_SAFE_INTEGER))
                })
                .then_with(|| right.created_at_unix_ms.cmp(&left.created_at_unix_ms))
                .then_with(|| left.owner_agent_id.cmp(&right.owner_agent_id))
                .then_with(|| left.task.id.cmp(&right.task.id))
        });
        let total = i64::try_from(records.len()).map_err(|_| {
            PersistenceError::new(
                "MONITORING_RESULT_TOO_LARGE",
                "Task monitoring result count exceeded the supported numeric range.",
                false,
            )
        })?;
        let records = records
            .into_iter()
            .skip(offset_usize)
            .take(limit_usize)
            .collect();
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(MonitoringTaskPage {
            authoritative: true,
            revision,
            offset,
            limit,
            total,
            records,
        })
    }

    pub fn query_monitoring_activity(
        &mut self,
        expected_revision: &MonitoringRevision,
        offset: i64,
        limit: i64,
    ) -> PersistenceResult<MonitoringActivityPage> {
        validate_monitoring_page(offset, limit)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let revision = monitoring_revision(&transaction)?;
        ensure_monitoring_revision(&revision, expected_revision)?;
        let total: i64 = transaction
            .query_row("SELECT COUNT(*) FROM agent_activity", [], |row| row.get(0))
            .map_err(PersistenceError::database)?;
        let mut statement = transaction
            .prepare(
                "SELECT activity.owner_agent_id, agent.name, agent.role, activity.id,
                        activity.message, activity.created_at,
                        COALESCE(activity.created_at_unix_ms, 0)
                 FROM agent_activity AS activity
                 JOIN agents AS agent ON agent.id = activity.owner_agent_id
                 ORDER BY activity.created_at_unix_ms DESC,
                          activity.owner_agent_id, activity.id DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(PersistenceError::database)?;
        let rows = statement
            .query_map(params![limit, offset], |row| {
                Ok(MonitoringActivityRecord {
                    owner_agent_id: row.get(0)?,
                    owner_name: row.get(1)?,
                    owner_role: row.get(2)?,
                    entry_id: row.get(3)?,
                    message: row.get(4)?,
                    created_at: row.get(5)?,
                    created_at_unix_ms: row.get(6)?,
                })
            })
            .map_err(PersistenceError::database)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(PersistenceError::database)?);
        }
        drop(statement);
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(MonitoringActivityPage {
            authoritative: true,
            revision,
            offset,
            limit,
            total,
            records,
        })
    }

    pub fn delete_monitoring_activity(
        &mut self,
        expected_revision: &MonitoringRevision,
        owner_agent_id: i64,
        entry_id: i64,
    ) -> PersistenceResult<MonitoringMutationResult> {
        if owner_agent_id <= 0 || entry_id <= 0 {
            return Err(PersistenceError::new(
                "MONITORING_ACTIVITY_ID_INVALID",
                "Activity deletion requires positive owner and entry identifiers.",
                true,
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let revision = monitoring_revision(&transaction)?;
        ensure_monitoring_revision(&revision, expected_revision)?;
        let deleted = transaction
            .execute(
                "DELETE FROM agent_activity WHERE owner_agent_id = ?1 AND id = ?2",
                params![owner_agent_id, entry_id],
            )
            .map_err(PersistenceError::database)? as i64;
        if deleted == 0 {
            return Err(PersistenceError::new(
                "MONITORING_ACTIVITY_NOT_FOUND",
                "The selected local activity entry no longer exists.",
                true,
            ));
        }
        advance_monitoring_activity_revision(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(MonitoringMutationResult {
            deleted_count: deleted,
            snapshot: self.monitoring_snapshot()?,
        })
    }

    pub fn clear_monitoring_activity(
        &mut self,
        expected_revision: &MonitoringRevision,
    ) -> PersistenceResult<MonitoringMutationResult> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let revision = monitoring_revision(&transaction)?;
        ensure_monitoring_revision(&revision, expected_revision)?;
        let deleted = transaction
            .execute("DELETE FROM agent_activity", [])
            .map_err(PersistenceError::database)? as i64;
        if deleted > 0 {
            advance_monitoring_activity_revision(&transaction)?;
        }
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(MonitoringMutationResult {
            deleted_count: deleted,
            snapshot: self.monitoring_snapshot()?,
        })
    }

    pub fn preview_backup_import(
        &mut self,
        expected_revision: i64,
        backup_json: &str,
    ) -> PersistenceResult<BackupImportPreview> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        let current = read_application_state(&transaction)?;
        let candidate =
            parse_backup_candidate(backup_json, &current).map_err(PersistenceError::validation)?;
        let security_change_summary = protected_security_change_summary(&current, &candidate.state);
        let preview = preview_for_candidate(&candidate, security_change_summary);
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(preview)
    }

    pub fn apply_backup_import(
        &mut self,
        expected_revision: i64,
        backup_json: &str,
    ) -> PersistenceResult<StateEnvelope> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        ensure_run_mutation_idle(&transaction)?;
        let current = read_application_state(&transaction)?;
        let candidate =
            parse_backup_candidate(backup_json, &current).map_err(PersistenceError::validation)?;
        clear_run_coordination(&transaction)?;
        let approval_origins = candidate
            .state
            .approval_requests
            .iter()
            .map(|request| (request.id, "legacy_backup".to_string()))
            .collect::<HashMap<_, _>>();
        write_application_state(
            &transaction,
            &candidate.state,
            candidate.source_kind,
            &approval_origins,
            true,
        )?;
        if candidate.format_version == 4 {
            write_portable_task18_domains(
                &transaction,
                &candidate.scheduled_items,
                &candidate.memory_records,
                timestamp,
            )?;
        }
        let revision = next_revision(meta.state_revision)?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = ?1, source_kind = ?2,
                     source_version = ?3, migrated_at_unix_ms = ?4,
                     legacy_cleanup_ack_at_unix_ms = NULL
                 WHERE singleton = 1",
                params![
                    revision,
                    candidate.source_kind,
                    candidate.format_version,
                    timestamp
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        if let Err(error) = self.run_data_lifecycle_maintenance("import", timestamp) {
            log::warn!(
                "post-import data lifecycle maintenance failed: {}",
                error.code
            );
        }
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "BACKUP_IMPORT_FAILED",
                "Imported application state did not become available after commit.",
                false,
            )
        })
    }

    pub fn save(
        &mut self,
        expected_revision: i64,
        state: &ApplicationState,
        security_change_confirmed: bool,
    ) -> PersistenceResult<SaveReceipt> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let current = read_application_state(&transaction)?;
        ensure_agent_registry_structure_unchanged(&current, state)?;
        let protected_state = protect_run_owned_state(&transaction, &current, state, timestamp)?;
        let retention_changed = current.task_retention_days != protected_state.task_retention_days
            || current.activity_retention_days != protected_state.activity_retention_days;
        validate_application_state(&protected_state).map_err(PersistenceError::validation)?;
        if let Some(summary) = protected_security_change_summary(&current, &protected_state) {
            if !security_change_confirmed {
                return Err(PersistenceError::new(
                    "NATIVE_CONFIRMATION_REQUIRED",
                    format!(
                        "A protected security change requires trusted desktop confirmation: {summary}"
                    ),
                    true,
                ));
            }
        }
        write_application_state(
            &transaction,
            &protected_state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )?;
        let revision = next_revision(meta.state_revision)?;
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [revision],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        let revision = if retention_changed {
            if let Err(error) = self.run_data_lifecycle_maintenance("settings", timestamp) {
                log::warn!(
                    "post-settings data lifecycle maintenance failed: {}",
                    error.code
                );
            }
            self.connection
                .query_meta()
                .map_err(PersistenceError::database)?
                .state_revision
        } else {
            revision
        };
        Ok(SaveReceipt {
            schema_version: CURRENT_SCHEMA_VERSION,
            revision,
        })
    }

    pub fn security_change_summary(
        &mut self,
        expected_revision: i64,
        state: &ApplicationState,
    ) -> PersistenceResult<Option<String>> {
        validate_application_state(state).map_err(PersistenceError::validation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        let current = read_application_state(&transaction)?;
        let summary = protected_security_change_summary(&current, state);
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(summary)
    }

    pub fn reset(
        &mut self,
        expected_revision: i64,
        confirmation: &str,
    ) -> PersistenceResult<StateEnvelope> {
        if confirmation != "RESET" {
            return Err(PersistenceError::new(
                "RESET_CONFIRMATION_REQUIRED",
                "Reset requires the exact confirmation text.",
                true,
            ));
        }
        let state = default_application_state().map_err(PersistenceError::validation)?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        ensure_run_mutation_idle(&transaction)?;
        clear_run_coordination(&transaction)?;
        write_application_state(&transaction, &state, "fresh", &HashMap::new(), true)?;
        let revision = next_revision(meta.state_revision)?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = ?1, source_kind = 'reset',
                     source_version = NULL, migrated_at_unix_ms = ?2,
                     legacy_cleanup_ack_at_unix_ms = NULL
                 WHERE singleton = 1",
                params![revision, timestamp],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        self.load()?.ok_or_else(|| {
            PersistenceError::new(
                "RESET_FAILED",
                "Application state did not become available after reset.",
                false,
            )
        })
    }

    pub fn schema_version(&self) -> PersistenceResult<i64> {
        self.connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(PersistenceError::database)
    }

    pub fn request_authorization(
        &mut self,
        intent: &ActionIntent,
    ) -> PersistenceResult<AuthorizationOutcome> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let state = read_application_state(&transaction)?;
        ensure_review_intent_binding(&transaction, intent)?;
        let evaluation = evaluate_policy(&state, intent).map_err(policy_denial)?;
        if evaluation.disposition == PolicyDisposition::Allow {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(AuthorizationOutcome::allowed(&evaluation));
        }

        if let Some(approval_id) =
            find_matching_active_approval(&transaction, &evaluation, timestamp)?
        {
            let approval = read_approval_request(&transaction, approval_id)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(AuthorizationOutcome::approval_required(
                approval,
                &evaluation,
            ));
        }

        let approval = insert_authoritative_approval(&transaction, intent, &evaluation, timestamp)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(AuthorizationOutcome::approval_required(
            approval,
            &evaluation,
        ))
    }

    pub fn system_action_audit(
        &mut self,
        request_id: &str,
    ) -> PersistenceResult<Option<SystemActionAuditRecord>> {
        if request_id.is_empty() || request_id.len() > 128 {
            return Err(PersistenceError::new(
                "INVALID_VOICE_REQUEST_ID",
                "The voice request identifier is empty or malformed.",
                true,
            ));
        }
        self.connection
            .query_row(
                SYSTEM_ACTION_AUDIT_SELECT_BY_REQUEST,
                [request_id],
                read_system_action_audit_row,
            )
            .optional()
            .map_err(PersistenceError::database)
    }

    pub fn query_system_action_audits(
        &mut self,
        limit: i64,
    ) -> PersistenceResult<SystemActionAuditPage> {
        if !(1..=MAX_SYSTEM_ACTION_AUDIT_PAGE).contains(&limit) {
            return Err(PersistenceError::new(
                "SYSTEM_ACTION_AUDIT_LIMIT_INVALID",
                format!(
                    "System-action audit queries require a limit from 1 to {MAX_SYSTEM_ACTION_AUDIT_PAGE}."
                ),
                true,
            ));
        }
        let mut statement = self
            .connection
            .prepare(SYSTEM_ACTION_AUDIT_SELECT_RECENT)
            .map_err(PersistenceError::database)?;
        let records = collect_rows(statement.query_map([limit], read_system_action_audit_row))?;
        Ok(SystemActionAuditPage { records, limit })
    }

    pub fn write_system_action_audit(
        &mut self,
        write: &AuditWrite,
    ) -> PersistenceResult<SystemActionAuditRecord> {
        validate_audit_write(write)
            .map_err(|error| PersistenceError::new(&error.code, error.message, true))?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        let existing = transaction
            .query_row(
                SYSTEM_ACTION_AUDIT_SELECT_BY_REQUEST,
                [write.request_id.as_str()],
                read_system_action_audit_row,
            )
            .optional()
            .map_err(PersistenceError::database)?;

        let audit_id = if let Some(existing) = existing {
            ensure_system_action_audit_binding(&existing, write)?;
            if existing.status != write.status {
                ensure_system_action_audit_transition(&existing.status, &write.status)?;
            }
            transaction
                .execute(
                    "UPDATE system_action_audits
                     SET task_owner_agent_id = ?1, task_id = ?2, approval_id = ?3,
                         authorization_kind = ?4, intent_fingerprint_sha256 = ?5,
                         policy_fingerprint_sha256 = ?6, status = ?7,
                         detail_code = ?8, detail_message = ?9,
                         updated_at_unix_ms = ?10
                     WHERE id = ?11",
                    params![
                        write.task_owner_agent_id,
                        write.task_id,
                        write.approval_id,
                        write.authorization_kind,
                        write.intent_fingerprint_sha256,
                        write.policy_fingerprint_sha256,
                        write.status,
                        write.detail_code,
                        write.detail_message,
                        timestamp,
                        existing.id,
                    ],
                )
                .map_err(PersistenceError::database)?;
            existing.id
        } else {
            transaction
                .execute(
                    "INSERT INTO system_action_audits
                     (request_id, request_fingerprint, intent_kind, risk_class,
                      target_kind, target_id, agent_id, task_owner_agent_id, task_id,
                      approval_id, authorization_kind, intent_fingerprint_sha256,
                      policy_fingerprint_sha256, status, detail_code, detail_message,
                      content_sha256, content_length, created_at_unix_ms, updated_at_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                             ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?19)",
                    params![
                        write.request_id,
                        write.request_fingerprint,
                        write.intent_kind,
                        write.risk_class,
                        write.target_kind,
                        write.target_id,
                        write.agent_id,
                        write.task_owner_agent_id,
                        write.task_id,
                        write.approval_id,
                        write.authorization_kind,
                        write.intent_fingerprint_sha256,
                        write.policy_fingerprint_sha256,
                        write.status,
                        write.detail_code,
                        write.detail_message,
                        write.content_sha256,
                        write.content_length,
                        timestamp,
                    ],
                )
                .map_err(PersistenceError::database)?;
            transaction.last_insert_rowid()
        };
        enforce_system_action_audit_cap(&transaction, audit_id, timestamp)?;
        let record = transaction
            .query_row(
                SYSTEM_ACTION_AUDIT_SELECT_BY_ID,
                [audit_id],
                read_system_action_audit_row,
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(record)
    }

    pub fn resolve_approval(
        &mut self,
        approval_id: i64,
        resolution: ApprovalResolution,
        native_confirmed: bool,
    ) -> PersistenceResult<ApprovalRequest> {
        if approval_id <= 0 || approval_id > MAX_SAFE_INTEGER {
            return Err(PersistenceError::new(
                "APPROVAL_NOT_FOUND",
                "The requested approval does not exist.",
                true,
            ));
        }
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let stored = read_stored_approval(&transaction, approval_id)?;
        ensure_pending_authoritative_approval(&stored, timestamp)?;

        if resolution == ApprovalResolution::Approve {
            if !native_confirmed {
                return Err(PersistenceError::new(
                    "NATIVE_CONFIRMATION_REQUIRED",
                    "Approval requires confirmation in a trusted desktop dialog.",
                    true,
                ));
            }
            let intent: ActionIntent = serde_json::from_str(&stored.intent_json).map_err(|_| {
                PersistenceError::new(
                    "MALFORMED_APPROVAL",
                    "The stored approval intent is invalid and cannot authorize an action.",
                    false,
                )
            })?;
            let state = read_application_state(&transaction)?;
            ensure_review_intent_binding(&transaction, &intent)?;
            let evaluation = evaluate_policy(&state, &intent).map_err(policy_denial)?;
            ensure_evaluation_matches(&stored, &evaluation)?;
        }

        let (status, resolved_at) = match resolution {
            ApprovalResolution::Approve => ("Approved", format_unix_ms(timestamp)),
            ApprovalResolution::Deny => ("Denied", format_unix_ms(timestamp)),
        };
        let changed = transaction
            .execute(
                "UPDATE approval_requests
                 SET status = ?1, resolved_at = ?2, resolved_at_unix_ms = ?3
                 WHERE id = ?4 AND authoritative = 1 AND status = 'Pending'
                   AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?3",
                params![status, resolved_at, timestamp, approval_id],
            )
            .map_err(PersistenceError::database)?;
        if changed != 1 {
            return Err(PersistenceError::new(
                "APPROVAL_STATE_CHANGED",
                "The approval changed before the decision could be recorded.",
                true,
            ));
        }
        let approval = read_approval_request(&transaction, approval_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(approval)
    }

    pub fn approval_confirmation(
        &mut self,
        approval_id: i64,
    ) -> PersistenceResult<ApprovalConfirmation> {
        if approval_id <= 0 || approval_id > MAX_SAFE_INTEGER {
            return Err(PersistenceError::new(
                "APPROVAL_NOT_FOUND",
                "The requested approval does not exist.",
                true,
            ));
        }
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let stored = read_stored_approval(&transaction, approval_id)?;
        ensure_pending_authoritative_approval(&stored, timestamp)?;
        let intent: ActionIntent = serde_json::from_str(&stored.intent_json).map_err(|_| {
            PersistenceError::new(
                "MALFORMED_APPROVAL",
                "The stored approval intent is invalid and cannot be confirmed.",
                false,
            )
        })?;
        let state = read_application_state(&transaction)?;
        ensure_review_intent_binding(&transaction, &intent)?;
        let evaluation = evaluate_policy(&state, &intent).map_err(policy_denial)?;
        ensure_evaluation_matches(&stored, &evaluation)?;
        let approval = read_approval_request(&transaction, approval_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(build_approval_confirmation(&approval, &intent))
    }

    pub fn authorize_intent(
        &mut self,
        intent: &ActionIntent,
    ) -> PersistenceResult<AuthorizationGrant> {
        self.authorize_intent_and_state(intent)
            .map(|(grant, _)| grant)
    }

    pub fn authorize_intent_and_state(
        &mut self,
        intent: &ActionIntent,
    ) -> PersistenceResult<(AuthorizationGrant, ApplicationState)> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let state = read_application_state(&transaction)?;
        ensure_review_intent_binding(&transaction, intent)?;
        let evaluation = evaluate_policy(&state, intent).map_err(policy_denial)?;
        if evaluation.disposition == PolicyDisposition::Allow {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok((
                AuthorizationGrant::policy_allowed_with_evidence(&evaluation),
                state,
            ));
        }

        let Some(approval_id) =
            find_matching_approved_approval(&transaction, &evaluation, timestamp)?
        else {
            let error = missing_approval_error(&transaction, &evaluation, timestamp)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Err(error);
        };
        let consumed_at = format_unix_ms(timestamp);
        let changed = transaction
            .execute(
                "UPDATE approval_requests
                 SET consumed_at = ?1, consumed_at_unix_ms = ?2
                 WHERE id = ?3 AND authoritative = 1 AND status = 'Approved'
                   AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?2
                   AND intent_fingerprint = ?4 AND policy_fingerprint = ?5
                   AND workspace_fingerprint = ?6",
                params![
                    consumed_at,
                    timestamp,
                    approval_id,
                    evaluation.intent_fingerprint,
                    evaluation.policy_fingerprint,
                    evaluation.workspace_fingerprint
                ],
            )
            .map_err(PersistenceError::database)?;
        if changed != 1 {
            return Err(PersistenceError::new(
                "APPROVAL_STATE_CHANGED",
                "The approval changed before it could be consumed.",
                true,
            ));
        }
        let approval = read_approval_request(&transaction, approval_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok((AuthorizationGrant::consumed(approval, &evaluation), state))
    }

    pub fn run_snapshot(&mut self) -> PersistenceResult<RunCoordinatorSnapshot> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(PersistenceError::database)?;
        let (
            revision,
            active_attempt_id,
            retained_attempt_count,
            retained_payload_bytes,
            pruned_attempt_count,
            last_pruned_at_unix_ms,
        ) = transaction
            .query_row(
                "SELECT revision, active_attempt_id, retained_attempt_count,
                        retained_payload_bytes, pruned_attempt_count, last_pruned_at_unix_ms
                 FROM run_coordinator_meta WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .map_err(PersistenceError::database)?;
        let active_attempt = active_attempt_id
            .map(|attempt_id| read_run_attempt(&transaction, attempt_id))
            .transpose()?;
        let recent_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT id FROM run_attempts
                     WHERE status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                      'failed', 'interrupted')
                     ORDER BY id DESC LIMIT ?1",
                )
                .map_err(PersistenceError::database)?;
            collect_rows(statement.query_map([MAX_RECENT_ATTEMPTS], |row| row.get(0)))?
        };
        let recent_attempts = recent_ids
            .into_iter()
            .map(|attempt_id| read_run_attempt(&transaction, attempt_id))
            .collect::<PersistenceResult<Vec<_>>>()?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(RunCoordinatorSnapshot {
            revision,
            active_attempt,
            recent_attempts,
            retained_attempt_count: nonnegative_u64(retained_attempt_count)?,
            retained_payload_bytes: nonnegative_u64(retained_payload_bytes)?,
            pruned_attempt_count: nonnegative_u64(pruned_attempt_count)?,
            last_pruned_at_unix_ms,
        })
    }

    pub fn admit_run(
        &mut self,
        request_id: &str,
        intent: &ActionIntent,
    ) -> PersistenceResult<RunAdmission> {
        validate_request_id(request_id)
            .map_err(|message| PersistenceError::new("INVALID_RUN_REQUEST_ID", message, true))?;
        let (agent_id, task_owner_agent_id, task_id, run_mode, review_context) =
            run_intent_parts(intent)?;
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_state_initialized(&transaction)?;
        expire_authoritative_approvals(&transaction, timestamp)?;
        let state = read_application_state(&transaction)?;
        let intent_json = serde_json::to_string(intent).map_err(|_| {
            PersistenceError::new(
                "INVALID_INTENT",
                "The run intent could not be normalized for admission.",
                true,
            )
        })?;

        let duplicate: Option<(i64, String)> = transaction
            .query_row(
                "SELECT id, intent_json FROM run_attempts WHERE request_id = ?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        if let Some((attempt_id, stored_intent)) = duplicate {
            if stored_intent != intent_json {
                return Err(PersistenceError::new(
                    "RUN_IDEMPOTENCY_CONFLICT",
                    "The run request identifier is already bound to a different action.",
                    false,
                ));
            }
            let attempt = read_run_attempt(&transaction, attempt_id)?;
            let authorization = attempt
                .approval_id
                .map(|approval_id| read_approval_request(&transaction, approval_id))
                .transpose()?
                .map(|approval| AuthorizationGrant {
                    approval: Some(approval),
                    evidence: None,
                });
            let review_request_json = attempt
                .review_stage_attempt_id
                .map(|stage_id| read_review_request_json(&transaction, stage_id))
                .transpose()?;
            let memory_bundle_json: String = transaction
                .query_row(
                    "SELECT memory_bundle_json FROM run_attempts WHERE id = ?1",
                    [attempt_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(PersistenceError::database)?
                .unwrap_or_else(|| {
                    "{\"schemaVersion\":1,\"records\":[],\"omittedRecordCount\":0}".to_string()
                });
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(RunAdmission {
                attempt,
                authorization,
                application_state: state,
                review_request_json,
                memory_bundle_json,
                duplicate: true,
            });
        }

        ensure_review_intent_binding(&transaction, intent)?;

        let active_attempt_id: Option<i64> = transaction
            .query_row(
                "SELECT active_attempt_id FROM run_coordinator_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let orphan_nonterminal: Option<i64> = transaction
            .query_row(
                "SELECT id FROM run_attempts
                 WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                  'cancel_requested')
                 ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        if active_attempt_id.is_some() || orphan_nonterminal.is_some() {
            transaction.commit().map_err(PersistenceError::database)?;
            return Err(PersistenceError::new(
                "RUN_BUSY",
                "Another AI run is active. Stop it or wait for it to finish before retrying.",
                true,
            ));
        }

        if run_mode == RunAttemptMode::Execute {
            ensure_execute_queue_head(&transaction, task_owner_agent_id, task_id)?;
        }

        let evaluation = evaluate_policy(&state, intent).map_err(policy_denial)?;
        let approval_id = if evaluation.disposition == PolicyDisposition::ApprovalRequired {
            let Some(approval_id) =
                find_matching_approved_approval(&transaction, &evaluation, timestamp)?
            else {
                let error = missing_approval_error(&transaction, &evaluation, timestamp)?;
                transaction.commit().map_err(PersistenceError::database)?;
                return Err(error);
            };
            Some(approval_id)
        } else {
            None
        };
        let task = state
            .agents
            .iter()
            .find(|agent| agent.id == task_owner_agent_id)
            .and_then(|owner| owner.tasks.iter().find(|task| task.id == task_id))
            .ok_or_else(|| {
                PersistenceError::new(
                    "TASK_NOT_FOUND",
                    "The selected task no longer exists.",
                    true,
                )
            })?;
        let specialist_contract_json = if run_mode == RunAttemptMode::Execute {
            task.specialist_request
                .as_ref()
                .map(|request| {
                    let agent = state
                        .agents
                        .iter()
                        .find(|agent| agent.id == agent_id)
                        .ok_or_else(|| {
                            PersistenceError::new(
                                "AGENT_NOT_FOUND",
                                "The selected agent no longer exists.",
                                true,
                            )
                        })?;
                    let model = resolve_model_identity(
                        &state.models,
                        &agent.model,
                        &state.preferences.active_ai_provider,
                    )
                    .map_err(|error| {
                        PersistenceError::new(error.code.as_str(), error.message, false)
                    })?;
                    let contract = SpecialistRunContractV1::for_request(
                        request,
                        model.provider_id.to_string(),
                        model.runtime_model,
                        approval_id,
                    )
                    .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
                    serde_json::to_string(&contract).map_err(|_| {
                        PersistenceError::new(
                            "SPECIALIST_CONTRACT_INVALID",
                            "The specialist run contract could not be normalized.",
                            false,
                        )
                    })
                })
                .transpose()?
        } else {
            None
        };
        let memory_bundle = build_prompt_bundle(
            &read_structured_memory_snapshot(&transaction)?.records,
            MemorySelectionContext {
                agent_id,
                workspace_id: task.workspace_id.clone(),
                task_owner_agent_id: Some(task_owner_agent_id),
                task_id: Some(task_id),
                team_leader_agent_ids: management_chain_for_agent(&state, agent_id)?,
            },
            timestamp,
        )
        .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        let memory_bundle_json = memory_bundle
            .canonical_json()
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        let memory_bundle_sha256 = memory_bundle
            .sha256()
            .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
        transaction
            .execute(
                "INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                 workspace_fingerprint, agent_id, task_owner_agent_id, task_id, task_title,
                  run_mode, review_flow_id, review_stage_attempt_id, review_revision_round,
                  status, workspace_id, approval_id, task_status_before,
                  task_phase_before, review_status_before, admitted_at_unix_ms,
                  specialist_contract_json, memory_bundle_json, memory_bundle_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                         ?13, 'admitted', ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    request_id,
                    intent_json,
                    evaluation.intent_fingerprint,
                    evaluation.policy_fingerprint,
                    evaluation.workspace_fingerprint,
                    agent_id,
                    task_owner_agent_id,
                    task_id,
                    task.title,
                    run_mode.as_str(),
                    review_context.as_ref().map(|context| context.flow_id),
                    review_context
                        .as_ref()
                        .map(|context| context.stage_attempt_id),
                    review_context
                        .as_ref()
                        .map(|context| context.revision_round),
                    evaluation.workspace_id,
                    approval_id,
                    task.status,
                    task.phase,
                    task.review_status,
                    timestamp,
                    specialist_contract_json,
                    memory_bundle_json,
                    memory_bundle_sha256,
                ],
            )
            .map_err(PersistenceError::database)?;
        let attempt_id = transaction.last_insert_rowid();
        if let Some(context) = &review_context {
            let changed = transaction
                .execute(
                    "UPDATE review_stage_attempts
                     SET state = 'admitted', run_attempt_id = ?1
                     WHERE id = ?2 AND flow_id = ?3 AND revision_round = ?4
                       AND level = ?5 AND reviewer_agent_id = ?6
                       AND request_fingerprint = ?7 AND state = 'pending'",
                    params![
                        attempt_id,
                        context.stage_attempt_id,
                        context.flow_id,
                        context.revision_round,
                        context.level.as_storage(),
                        agent_id,
                        context.request_fingerprint
                    ],
                )
                .map_err(PersistenceError::database)?;
            if changed != 1 {
                return Err(PersistenceError::new(
                    "REVIEW_STAGE_STATE_CONFLICT",
                    "The bound review stage changed before run admission completed.",
                    true,
                ));
            }
            advance_review_revision(&transaction)?;
        }
        if let Some(approval_id) = approval_id {
            transaction
                .execute(
                    "INSERT INTO run_approval_reservations
                     (attempt_id, approval_id, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                    params![attempt_id, approval_id, timestamp],
                )
                .map_err(PersistenceError::database)?;
        }
        transaction
            .execute(
                "UPDATE run_coordinator_meta
                 SET active_attempt_id = ?1, revision = revision + 1
                 WHERE singleton = 1 AND active_attempt_id IS NULL",
                [attempt_id],
            )
            .map_err(PersistenceError::database)?;
        if run_mode == RunAttemptMode::Execute {
            let changed = transaction
                .execute(
                    "UPDATE agent_tasks SET queue_state = 'admitted'
                     WHERE owner_agent_id = ?1 AND id = ?2 AND queue_state = 'queued'",
                    params![task_owner_agent_id, task_id],
                )
                .map_err(PersistenceError::database)?;
            if changed != 1 {
                return Err(PersistenceError::new(
                    "TASK_QUEUE_STATE_CONFLICT",
                    "The queue head changed before execution admission completed.",
                    true,
                ));
            }
            advance_task_orchestration_revision(&transaction)?;
        }
        refresh_run_retention_meta(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        let authorization = approval_id
            .map(|id| read_approval_request(&transaction, id))
            .transpose()?
            .map(|approval| AuthorizationGrant {
                approval: Some(approval),
                evidence: Some(crate::authorization::AuthorizationEvidence::from(
                    &evaluation,
                )),
            });
        let application_state = read_application_state(&transaction)?;
        let review_request_json = review_context
            .as_ref()
            .map(|context| read_review_request_json(&transaction, context.stage_attempt_id))
            .transpose()?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(RunAdmission {
            attempt,
            authorization,
            application_state,
            review_request_json,
            memory_bundle_json,
            duplicate: false,
        })
    }

    pub fn prepare_run_attempt(
        &mut self,
        attempt_id: i64,
        provider: &str,
        model: &str,
        workspace_id: Option<&str>,
    ) -> PersistenceResult<RunAttemptProjection> {
        validate_run_label("provider", provider)?;
        validate_run_label("model", model)?;
        if let Some(workspace_id) = workspace_id {
            validate_run_label("workspace identifier", workspace_id)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let current = read_run_attempt(&transaction, attempt_id)?;
        if current.status == RunAttemptStatus::Starting
            && current.provider.as_deref() == Some(provider)
            && current.model.as_deref() == Some(model)
            && current.workspace_id.as_deref() == workspace_id
        {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(current);
        }
        ensure_active_attempt(&transaction, attempt_id)?;
        ensure_run_transition(current.status, RunAttemptStatus::Starting)?;
        transaction
            .execute(
                "UPDATE run_attempts
                 SET status = 'starting', provider = ?1, model = ?2, workspace_id = ?3
                 WHERE id = ?4 AND status = 'admitted'",
                params![provider, model, workspace_id, attempt_id],
            )
            .map_err(PersistenceError::database)?;
        advance_run_revision(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(attempt)
    }

    pub fn mark_run_dispatching(
        &mut self,
        attempt_id: i64,
    ) -> PersistenceResult<RunAttemptProjection> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let current = read_run_attempt(&transaction, attempt_id)?;
        if matches!(
            current.status,
            RunAttemptStatus::Dispatching
                | RunAttemptStatus::Running
                | RunAttemptStatus::CancelRequested
        ) {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(current);
        }
        ensure_active_attempt(&transaction, attempt_id)?;
        ensure_run_transition(current.status, RunAttemptStatus::Dispatching)?;
        transaction
            .execute(
                "UPDATE run_attempts SET status = 'dispatching'
                 WHERE id = ?1 AND status = 'starting'",
                [attempt_id],
            )
            .map_err(PersistenceError::database)?;
        advance_run_revision(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(attempt)
    }

    pub fn mark_run_started(&mut self, attempt_id: i64) -> PersistenceResult<RunAttemptProjection> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        ensure_active_attempt(&transaction, attempt_id)?;
        let current = read_run_attempt(&transaction, attempt_id)?;
        if current.started_at_unix_ms.is_some() {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(current);
        }
        if !matches!(
            current.status,
            RunAttemptStatus::Dispatching | RunAttemptStatus::CancelRequested
        ) {
            ensure_run_transition(current.status, RunAttemptStatus::Running)?;
        }
        consume_reserved_run_approval(&transaction, attempt_id, timestamp)?;
        let next_status = if current.status == RunAttemptStatus::CancelRequested {
            RunAttemptStatus::CancelRequested
        } else {
            RunAttemptStatus::Running
        };
        transaction
            .execute(
                "UPDATE run_attempts SET status = ?1, started_at_unix_ms = ?2
                 WHERE id = ?3 AND started_at_unix_ms IS NULL",
                params![next_status.as_str(), timestamp, attempt_id],
            )
            .map_err(PersistenceError::database)?;
        project_run_started_to_task(&transaction, &current, timestamp)?;
        if current.run_mode == RunAttemptMode::Execute {
            advance_task_orchestration_revision(&transaction)?;
        }
        advance_run_revision(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(attempt)
    }

    pub fn request_run_cancellation(
        &mut self,
        attempt_id: i64,
    ) -> PersistenceResult<RunAttemptProjection> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let current = read_run_attempt(&transaction, attempt_id)?;
        if current.status.is_terminal() || current.status == RunAttemptStatus::CancelRequested {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(current);
        }
        ensure_active_attempt(&transaction, attempt_id)?;
        ensure_run_transition(current.status, RunAttemptStatus::CancelRequested)?;
        let cancellation_disposition = if matches!(
            current.status,
            RunAttemptStatus::Dispatching | RunAttemptStatus::Running
        ) {
            "manual_review_required"
        } else {
            "safe_to_retry"
        };
        transaction
            .execute(
                "UPDATE run_attempts
                 SET status = 'cancel_requested', cancel_requested_at_unix_ms = ?1,
                     recovery_disposition = ?2
                 WHERE id = ?3 AND status = ?4",
                params![
                    timestamp,
                    cancellation_disposition,
                    attempt_id,
                    current.status.as_str()
                ],
            )
            .map_err(PersistenceError::database)?;
        advance_run_revision(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(attempt)
    }

    pub fn record_run_event(
        &mut self,
        attempt_id: i64,
        kind: &str,
        message: &str,
    ) -> PersistenceResult<RunEventProjection> {
        if !matches!(kind, "status" | "progress" | "complete" | "error") {
            return Err(PersistenceError::new(
                "INVALID_RUN_EVENT",
                "The run event kind is invalid.",
                true,
            ));
        }
        let timestamp = now_unix_ms()?;
        let bounded = BoundedText::from_text(message, MAX_PROGRESS_MESSAGE_BYTES);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        if attempt.status.is_terminal() {
            return Err(PersistenceError::new(
                "RUN_EVENT_STALE",
                "A completed run cannot accept additional events.",
                true,
            ));
        }
        ensure_active_attempt(&transaction, attempt_id)?;
        let (event_count, progress_bytes, omitted_count, next_sequence): (i64, i64, i64, i64) =
            transaction
                .query_row(
                    "SELECT progress_event_count, progress_bytes,
                            omitted_progress_event_count,
                            COALESCE((SELECT MAX(sequence) FROM run_events
                                      WHERE attempt_id = run_attempts.id), 0) + 1
                     FROM run_attempts WHERE id = ?1",
                    [attempt_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(PersistenceError::database)?;
        let bounded_bytes = i64::try_from(bounded.as_str().len()).unwrap_or(i64::MAX);
        let omitted = event_count >= MAX_PROGRESS_EVENTS
            || progress_bytes.saturating_add(bounded_bytes) > MAX_PROGRESS_BYTES;
        let (sequence, projected_message, projected_truncated) = if omitted {
            transaction
                .execute(
                    "UPDATE run_attempts
                     SET progress_truncated = 1,
                         omitted_progress_event_count = omitted_progress_event_count + 1
                     WHERE id = ?1",
                    [attempt_id],
                )
                .map_err(PersistenceError::database)?;
            (
                next_sequence.saturating_add(omitted_count),
                "Additional run progress was omitted after reaching the storage bound.".to_string(),
                true,
            )
        } else {
            transaction
                .execute(
                    "INSERT INTO run_events
                     (attempt_id, sequence, kind, message, message_truncated,
                      created_at_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        attempt_id,
                        next_sequence,
                        kind,
                        bounded.as_str(),
                        bounded.truncated() as i64,
                        timestamp
                    ],
                )
                .map_err(PersistenceError::database)?;
            transaction
                .execute(
                    "UPDATE run_attempts
                     SET progress_event_count = progress_event_count + 1,
                         progress_bytes = progress_bytes + ?1,
                         payload_bytes = payload_bytes + ?1,
                         progress_truncated = MAX(progress_truncated, ?2)
                     WHERE id = ?3",
                    params![bounded_bytes, bounded.truncated() as i64, attempt_id],
                )
                .map_err(PersistenceError::database)?;
            (
                next_sequence,
                bounded.as_str().to_string(),
                bounded.truncated(),
            )
        };
        let revision = advance_run_revision(&transaction)?;
        refresh_run_retention_meta(&transaction)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(RunEventProjection {
            coordinator_revision: revision,
            attempt_id,
            request_id: attempt.request_id,
            sequence,
            kind: kind.to_string(),
            status: attempt.status,
            message: projected_message,
            message_truncated: projected_truncated,
            created_at_unix_ms: timestamp,
        })
    }

    pub fn complete_run(
        &mut self,
        attempt_id: i64,
        completion: &RunCompletion,
    ) -> PersistenceResult<RunAttemptProjection> {
        if !completion.status.is_terminal() {
            return Err(PersistenceError::new(
                "INVALID_RUN_COMPLETION",
                "Run completion requires a terminal status.",
                false,
            ));
        }
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let current = read_run_attempt(&transaction, attempt_id)?;
        if current.status.is_terminal() {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(current);
        }
        ensure_active_attempt(&transaction, attempt_id)?;
        let terminal_status = if current.status == RunAttemptStatus::CancelRequested {
            RunAttemptStatus::Cancelled
        } else {
            completion.status
        };
        let recovery_disposition = completion
            .recovery_disposition
            .as_deref()
            .or(current.recovery_disposition.as_deref());
        ensure_run_transition(current.status, terminal_status)?;

        let summary = completion
            .output_summary
            .as_deref()
            .map(|value| BoundedText::from_text(value, MAX_SUMMARY_BYTES));
        let stderr = completion
            .stderr_excerpt
            .as_deref()
            .map(|value| BoundedText::from_text(value, MAX_STDERR_CAPTURE_BYTES));
        let error_message = completion
            .error_message
            .as_deref()
            .map(|value| BoundedText::from_text(value, MAX_ERROR_BYTES));
        let bounded_paths = bound_paths(completion.changed_files.clone());
        let (diff, original_diff_bytes, diff_truncated) = bound_diff(completion.diff.clone());
        let changed_files_json = serde_json::to_string(&bounded_paths.paths).map_err(|_| {
            PersistenceError::new(
                "INVALID_RUN_COMPLETION",
                "Changed-file evidence could not be normalized.",
                false,
            )
        })?;
        completion.workspace_changes.validate().map_err(|message| {
            PersistenceError::new(
                "INVALID_WORKSPACE_EVIDENCE",
                format!("Workspace evidence is invalid: {message}."),
                false,
            )
        })?;
        let workspace_evidence_json = serde_json::to_string(&completion.workspace_changes)
            .map_err(|_| {
                PersistenceError::new(
                    "INVALID_WORKSPACE_EVIDENCE",
                    "Workspace evidence could not be normalized.",
                    false,
                )
            })?;
        if workspace_evidence_json.len() > MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES {
            return Err(PersistenceError::new(
                "RUN_OUTPUT_TOO_LARGE",
                "Workspace evidence exceeds the persisted payload bound.",
                false,
            ));
        }
        let specialist_result_json = if terminal_status != RunAttemptStatus::Succeeded {
            None
        } else {
            match (
                current.specialist_contract.as_ref(),
                completion.specialist_result.as_ref(),
            ) {
                (Some(contract), Some(result)) if contract.kind == result.kind() => Some(
                    canonical_specialist_result_json(result)
                        .map_err(|error| PersistenceError::new(error.code, error.message, false))?,
                ),
                (Some(_), Some(_)) => {
                    return Err(PersistenceError::new(
                        "SPECIALIST_RESULT_MISMATCH",
                        "The specialist result kind does not match its immutable run contract.",
                        false,
                    ))
                }
                (Some(_), None) => {
                    return Err(PersistenceError::new(
                        "SPECIALIST_RESULT_REQUIRED",
                        "A successful specialist run requires a validated structured result.",
                        false,
                    ))
                }
                (None, Some(_)) => {
                    return Err(PersistenceError::new(
                        "SPECIALIST_RESULT_UNBOUND",
                        "A specialist result cannot be stored without an immutable run contract.",
                        false,
                    ))
                }
                (None, None) => None,
            }
        };
        let mut truncation = completion.truncation.clone();
        if let Some(summary) = &summary {
            truncation.summary_truncated |= summary.truncated();
            truncation.original_summary_bytes = truncation
                .original_summary_bytes
                .max(summary.original_bytes() as u64);
        }
        if let Some(stderr) = &stderr {
            truncation.stderr_truncated |= stderr.truncated();
            truncation.original_stderr_bytes = truncation
                .original_stderr_bytes
                .max(stderr.original_bytes() as u64);
        }
        if let Some(error) = &error_message {
            truncation.summary_truncated |= error.truncated();
            truncation.original_summary_bytes = truncation
                .original_summary_bytes
                .max(error.original_bytes() as u64);
        }
        truncation.diff_truncated |= diff_truncated;
        truncation.original_diff_bytes = truncation
            .original_diff_bytes
            .max(original_diff_bytes as u64);
        truncation.changed_files_truncated |= bounded_paths.truncated;
        truncation.original_changed_file_count = truncation
            .original_changed_file_count
            .max(bounded_paths.original_count as u64);
        let progress_evidence: (i64, i64, i64) = transaction
            .query_row(
                "SELECT progress_truncated, omitted_progress_event_count, progress_bytes
                 FROM run_attempts WHERE id = ?1",
                [attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(PersistenceError::database)?;
        truncation.progress_truncated |= progress_evidence.0 != 0;
        truncation.omitted_progress_event_count = truncation
            .omitted_progress_event_count
            .max(nonnegative_u64(progress_evidence.1)?);

        let summary_text = summary.as_ref().map(|value| value.as_str());
        let stderr_text = stderr.as_ref().map(|value| value.as_str());
        let error_text = error_message.as_ref().map(|value| value.as_str());
        let payload_bytes = run_payload_bytes(
            [
                summary_text,
                stderr_text,
                completion.response_id.as_deref(),
                diff.as_deref(),
                completion.error_code.as_deref(),
                error_text,
                specialist_result_json.as_deref(),
            ],
            &changed_files_json,
            &workspace_evidence_json,
        )?
        .checked_add(progress_evidence.2)
        .ok_or_else(|| {
            PersistenceError::new(
                "RUN_OUTPUT_TOO_LARGE",
                "Run output size could not be represented safely.",
                false,
            )
        })?;
        transaction
            .execute(
                "UPDATE run_attempts
                 SET status = ?1, completed_at_unix_ms = ?2, duration_seconds = ?3,
                     output_summary = ?4, stderr_excerpt = ?5, response_id = ?6,
                     model = COALESCE(?7, model), input_tokens = ?8, output_tokens = ?9,
                     total_tokens = ?10, changed_files_json = ?11, diff = ?12,
                     error_code = ?13, error_message = ?14, payload_bytes = ?15,
                     stdout_truncated = ?16, stderr_truncated = ?17,
                     summary_truncated = ?18, diff_truncated = ?19,
                     changed_files_truncated = ?20, progress_truncated = ?21,
                     before_snapshot_truncated = ?22, after_snapshot_truncated = ?23,
                     original_stdout_bytes = ?24, original_stderr_bytes = ?25,
                     original_summary_bytes = ?26, original_diff_bytes = ?27,
                     original_changed_file_count = ?28,
                     omitted_progress_event_count = ?29, recovery_disposition = ?30,
                     workspace_evidence_json = ?31, specialist_result_json = ?32
                 WHERE id = ?33 AND status = ?34",
                params![
                    terminal_status.as_str(),
                    timestamp,
                    bounded_i64(completion.duration_seconds),
                    summary_text,
                    stderr_text,
                    completion.response_id,
                    completion.runtime_model,
                    optional_bounded_i64(completion.usage.input_tokens),
                    optional_bounded_i64(completion.usage.output_tokens),
                    optional_bounded_i64(completion.usage.total_tokens),
                    changed_files_json,
                    diff,
                    completion.error_code,
                    error_text,
                    payload_bytes,
                    truncation.stdout_truncated as i64,
                    truncation.stderr_truncated as i64,
                    truncation.summary_truncated as i64,
                    truncation.diff_truncated as i64,
                    truncation.changed_files_truncated as i64,
                    truncation.progress_truncated as i64,
                    truncation.before_snapshot_truncated as i64,
                    truncation.after_snapshot_truncated as i64,
                    bounded_i64(truncation.original_stdout_bytes),
                    bounded_i64(truncation.original_stderr_bytes),
                    bounded_i64(truncation.original_summary_bytes),
                    bounded_i64(truncation.original_diff_bytes),
                    bounded_i64(truncation.original_changed_file_count),
                    bounded_i64(truncation.omitted_progress_event_count),
                    recovery_disposition,
                    &workspace_evidence_json,
                    specialist_result_json,
                    attempt_id,
                    current.status.as_str()
                ],
            )
            .map_err(PersistenceError::database)?;
        let completed_attempt = read_run_attempt(&transaction, attempt_id)?;
        project_run_completion_to_task(
            &transaction,
            &completed_attempt,
            terminal_status,
            summary_text,
            completion.response_id.as_deref(),
            completion
                .runtime_model
                .as_deref()
                .or(current.model.as_deref()),
            completion.usage.total_tokens,
            &bounded_paths.paths,
            diff.as_deref(),
            &workspace_evidence_json,
            completion.duration_seconds,
            timestamp,
            recovery_disposition,
        )?;
        project_run_completion_to_queue(
            &transaction,
            &completed_attempt,
            terminal_status,
            recovery_disposition,
        )?;
        if completed_attempt.run_mode == RunAttemptMode::Execute
            && terminal_status != RunAttemptStatus::Succeeded
        {
            project_execution_failure_to_review_flow(
                &transaction,
                &completed_attempt,
                terminal_status,
                recovery_disposition,
                timestamp,
            )?;
        }
        let owner_role: String = transaction
            .query_row(
                "SELECT role FROM agents WHERE id = ?1",
                [completed_attempt.task_owner_agent_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let memory_bundle_sha256: Option<String> = transaction
            .query_row(
                "SELECT memory_bundle_sha256 FROM run_attempts WHERE id = ?1",
                [attempt_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let review_verdict = completed_attempt
            .review_stage_attempt_id
            .map(|stage_id| {
                transaction
                    .query_row(
                        "SELECT verdict FROM review_stage_attempts WHERE id = ?1",
                        [stage_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .map_err(PersistenceError::database)
            })
            .transpose()?
            .flatten();
        let handoff_kind = match (
            completed_attempt.run_mode,
            terminal_status,
            review_verdict.as_deref(),
        ) {
            (RunAttemptMode::Execute, RunAttemptStatus::Succeeded, _) => {
                ManagementHandoffKind::ExecutionEvidence
            }
            (RunAttemptMode::Review, RunAttemptStatus::Succeeded, Some("approved")) => {
                ManagementHandoffKind::ReviewDecision
            }
            (RunAttemptMode::Review, RunAttemptStatus::Succeeded, Some("changes_requested")) => {
                ManagementHandoffKind::RevisionRequest
            }
            _ => ManagementHandoffKind::Failure,
        };
        let handoff_summary = BoundedText::from_text(
            summary_text
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| terminal_status.as_str()),
            MAX_HANDOFF_SUMMARY_BYTES,
        );
        let handoff_error_code = completion
            .error_code
            .as_deref()
            .map(|value| BoundedText::from_text(value, 4 * 1024).into_string());
        insert_management_handoff(
            &transaction,
            NewManagementHandoff {
                task_owner_agent_id: completed_attempt.task_owner_agent_id,
                task_id: completed_attempt.task_id,
                kind: handoff_kind,
                from_agent_id: Some(completed_attempt.agent_id),
                to_agent_id: Some(completed_attempt.task_owner_agent_id),
                owner_role: management_owner_role(&owner_role),
                revision_round: completed_attempt.review_revision_round.unwrap_or(0),
                run_attempt_id: Some(attempt_id),
                review_flow_id: completed_attempt.review_flow_id,
                review_stage_attempt_id: completed_attempt.review_stage_attempt_id,
                source: if completed_attempt.run_mode == RunAttemptMode::Review {
                    ManagementHandoffSource::ReviewOrchestration
                } else {
                    ManagementHandoffSource::RunCoordinator
                },
                summary: handoff_summary.as_str().to_string(),
                payload: serde_json::json!({
                    "terminalStatus": terminal_status.as_str(),
                    "reviewVerdict": review_verdict,
                    "summaryTruncatedForHandoff": handoff_summary.truncated(),
                    "errorCode": handoff_error_code,
                    "memoryBundleSha256": memory_bundle_sha256,
                    "recoveryDisposition": recovery_disposition,
                    "runEvidence": {
                        "attemptId": attempt_id,
                        "workspaceEvidenceSha256": sha256_hex(workspace_evidence_json.as_bytes()),
                        "workspaceMode": completion.workspace_changes.mode,
                        "workspaceStatus": completion.workspace_changes.status,
                        "workspaceReviewability": completion.workspace_changes.reviewability,
                        "workspaceSummary": completion.workspace_changes.summary,
                        "workspaceIssueCount": completion.workspace_changes.issues.len(),
                        "workspaceIssuesTruncated": completion.workspace_changes.issues_truncated,
                        "retainedChangedFileCount": bounded_paths.paths.len(),
                        "originalChangedFileCount": bounded_paths.original_count,
                        "changedFilesTruncated": bounded_paths.truncated,
                        "fullEvidenceLocation": "run_attempt",
                    },
                }),
                idempotency_key: format!("run-coordinator:completion:{attempt_id}"),
            },
            timestamp,
        )?;
        transaction
            .execute(
                "DELETE FROM run_approval_reservations WHERE attempt_id = ?1",
                [attempt_id],
            )
            .map_err(PersistenceError::database)?;
        if current.run_mode == RunAttemptMode::Execute {
            advance_task_orchestration_revision(&transaction)?;
        }
        transaction
            .execute(
                "UPDATE run_coordinator_meta
                 SET active_attempt_id = NULL, revision = revision + 1
                 WHERE singleton = 1 AND active_attempt_id = ?1",
                [attempt_id],
            )
            .map_err(PersistenceError::database)?;
        prune_run_history(&transaction, timestamp)?;
        refresh_run_retention_meta(&transaction)?;
        let attempt = read_run_attempt(&transaction, attempt_id)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(attempt)
    }

    fn configure_connection_preflight(&self) -> PersistenceResult<()> {
        self.connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(PersistenceError::database_or_corrupt)?;
        self.connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA trusted_schema = OFF;
                 PRAGMA cell_size_check = ON;
                 PRAGMA mmap_size = 0;",
            )
            .map_err(PersistenceError::database_or_corrupt)?;
        Ok(())
    }

    fn configure_write_durability(&self, in_memory: bool) -> PersistenceResult<()> {
        self.connection
            .execute_batch(
                "PRAGMA secure_delete = ON;
                 PRAGMA synchronous = FULL;",
            )
            .map_err(PersistenceError::database_or_corrupt)?;
        if !in_memory {
            let mode: String = self
                .connection
                .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
                .map_err(PersistenceError::database_or_corrupt)?;
            if !mode.eq_ignore_ascii_case("delete") {
                return Err(PersistenceError::new(
                    "JOURNAL_MODE_UNAVAILABLE",
                    "SQLite could not enable rollback-journal mode.",
                    false,
                ));
            }
        }
        Ok(())
    }

    fn verify_supported_schema_version(&self) -> PersistenceResult<()> {
        let version = self.schema_version()?;
        if version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::new(
                "UNSUPPORTED_NEWER_SCHEMA",
                format!(
                    "Database schema {version} is newer than supported schema {CURRENT_SCHEMA_VERSION}."
                ),
                false,
            ));
        }
        Ok(())
    }

    fn verify_integrity(&self) -> PersistenceResult<()> {
        let result: String = self
            .connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(|_| {
                PersistenceError::new(
                    "DATABASE_CORRUPT",
                    "The application database could not be validated and was not modified.",
                    false,
                )
            })?;
        if result != "ok" {
            return Err(PersistenceError::new(
                "DATABASE_CORRUPT",
                "The application database failed its integrity check and was not modified.",
                false,
            ));
        }
        Ok(())
    }

    fn apply_migrations(&mut self) -> PersistenceResult<()> {
        self.verify_supported_schema_version()?;
        while self.schema_version()? < CURRENT_SCHEMA_VERSION {
            match self.schema_version()? {
                0 => self.apply_migration(1, "initial_application_state", INITIAL_MIGRATION)?,
                1 => self.apply_migration(
                    2,
                    "authoritative_approval_lifecycle",
                    AUTHORIZATION_MIGRATION,
                )?,
                2 => self.apply_migration(
                    3,
                    "authoritative_run_coordination",
                    RUN_COORDINATION_MIGRATION,
                )?,
                3 => self.apply_migration(4, "dynamic_agent_registry", AGENT_REGISTRY_MIGRATION)?,
                4 => self.apply_migration(
                    5,
                    "authoritative_task_orchestration",
                    TASK_ORCHESTRATION_MIGRATION,
                )?,
                5 => self.apply_migration(
                    6,
                    "structured_review_orchestration",
                    REVIEW_ORCHESTRATION_MIGRATION,
                )?,
                6 => self.apply_migration(
                    7,
                    "structured_workspace_evidence",
                    WORKSPACE_EVIDENCE_MIGRATION,
                )?,
                7 => self.apply_migration(
                    8,
                    "data_lifecycle_and_monitoring",
                    DATA_LIFECYCLE_MIGRATION,
                )?,
                8 => self.apply_migration(
                    9,
                    "system_action_policy_gateway",
                    SYSTEM_ACTION_GATEWAY_MIGRATION,
                )?,
                9 => self.apply_migration(
                    10,
                    "bounded_specialist_capabilities",
                    SPECIALIST_CAPABILITIES_MIGRATION,
                )?,
                10 => self.apply_migration(
                    11,
                    "reminders_memory_management_handoffs",
                    REMINDERS_MEMORY_HANDOFFS_MIGRATION,
                )?,
                version => {
                    return Err(PersistenceError::new(
                        "MIGRATION_PATH_MISSING",
                        format!("No migration path exists from database schema {version}."),
                        false,
                    ));
                }
            }
        }
        self.verify_migration_ledger()
    }

    fn apply_migration(
        &mut self,
        target_version: i64,
        name: &str,
        sql: &str,
    ) -> PersistenceResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let locked_version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(PersistenceError::database)?;
        if locked_version > CURRENT_SCHEMA_VERSION {
            return Err(PersistenceError::new(
                "UNSUPPORTED_NEWER_SCHEMA",
                format!(
                    "Database schema {locked_version} is newer than supported schema {CURRENT_SCHEMA_VERSION}."
                ),
                false,
            ));
        }
        if locked_version == target_version - 1 {
            let timestamp = now_unix_ms()?;
            transaction
                .execute_batch(sql)
                .map_err(PersistenceError::database)?;
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at_unix_ms)
                     VALUES (?1, ?2, ?3)",
                    params![target_version, name, timestamp],
                )
                .map_err(PersistenceError::database)?;
            transaction
                .pragma_update(None, "user_version", target_version)
                .map_err(PersistenceError::database)?;
        } else if locked_version < target_version {
            return Err(PersistenceError::new(
                "MIGRATION_LEDGER_MISMATCH",
                format!(
                    "Migration {target_version} cannot be applied from database schema {locked_version}."
                ),
                false,
            ));
        }
        transaction.commit().map_err(PersistenceError::database)
    }

    fn verify_migration_ledger(&self) -> PersistenceResult<()> {
        let version = self.schema_version()?;
        let ledger_version: Option<i64> = self
            .connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .map_err(PersistenceError::database)?;
        if version != CURRENT_SCHEMA_VERSION || ledger_version != Some(CURRENT_SCHEMA_VERSION) {
            return Err(PersistenceError::new(
                "MIGRATION_LEDGER_MISMATCH",
                "Database schema version and migration ledger do not agree.",
                false,
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn simulate_interrupted_save(
        &mut self,
        expected_revision: i64,
        state: &ApplicationState,
    ) -> PersistenceResult<()> {
        validate_application_state(state).map_err(PersistenceError::validation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        clear_application_state(&transaction, true)?;
        Err(PersistenceError::new(
            "INJECTED_INTERRUPTION",
            "Injected interruption before commit.",
            true,
        ))
    }

    #[cfg(test)]
    fn simulate_interrupted_migration(
        &mut self,
        legacy: &LegacyRendererState,
    ) -> PersistenceResult<()> {
        let state = application_state_from_legacy(legacy).map_err(PersistenceError::validation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if meta.initialized {
            return Err(PersistenceError::new(
                "ALREADY_INITIALIZED",
                "Application state has already been initialized.",
                true,
            ));
        }
        write_application_state(
            &transaction,
            &state,
            "legacy_local_storage",
            &HashMap::new(),
            true,
        )?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = 1,
                     source_kind = 'legacy_local_storage', source_version = 0
                 WHERE singleton = 1",
                [],
            )
            .map_err(PersistenceError::database)?;
        Err(PersistenceError::new(
            "INJECTED_MIGRATION_INTERRUPTION",
            "Injected interruption before migration commit.",
            true,
        ))
    }

    fn reconcile_interrupted_runs(&mut self) -> PersistenceResult<()> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        if !meta.initialized {
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(());
        }

        let interrupted_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT id FROM run_attempts
                     WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                      'cancel_requested')
                     ORDER BY id",
                )
                .map_err(PersistenceError::database)?;
            collect_rows(statement.query_map([], |row| row.get(0)))?
        };
        let mut changed = false;
        let mut reconciled_tasks = HashSet::new();
        for attempt_id in interrupted_ids {
            let attempt = read_run_attempt(&transaction, attempt_id)?;
            reconciled_tasks.insert((
                attempt.task_owner_agent_id,
                attempt.task_id,
                attempt.run_mode.as_str().to_string(),
            ));
            let safe_to_retry = matches!(
                attempt.status,
                RunAttemptStatus::Admitted | RunAttemptStatus::Starting
            );
            if !safe_to_retry {
                invalidate_reserved_run_approval(&transaction, attempt_id, timestamp)?;
            }
            let recovery_disposition = if safe_to_retry {
                "safe_to_retry"
            } else {
                "manual_review_required"
            };
            let message = if safe_to_retry {
                "The application restarted before provider dispatch. The task is safe to retry."
            } else {
                "The application restarted after dispatch could have begun. Inspect the workspace before retrying."
            };
            transaction
                .execute(
                    "UPDATE run_attempts
                     SET status = 'interrupted', completed_at_unix_ms = ?1,
                         error_code = 'RUN_INTERRUPTED', error_message = ?2,
                         recovery_disposition = ?3, payload_bytes = ?4
                     WHERE id = ?5 AND status = ?6",
                    params![
                        timestamp,
                        message,
                        recovery_disposition,
                        message.len() as i64,
                        attempt_id,
                        attempt.status.as_str()
                    ],
                )
                .map_err(PersistenceError::database)?;
            let recovered = read_run_attempt(&transaction, attempt_id)?;
            if recovered.run_mode == RunAttemptMode::Review
                && recovered.review_stage_attempt_id.is_some()
            {
                project_review_run_completion(
                    &transaction,
                    &recovered,
                    RunAttemptStatus::Interrupted,
                    recovered.output_summary.as_deref(),
                    recovered.model.as_deref(),
                    recovered.duration_seconds.unwrap_or_default(),
                    timestamp,
                    Some(recovery_disposition),
                )?;
            } else {
                project_recovered_attempt_to_task(&transaction, &recovered, safe_to_retry)?;
                if recovered.run_mode == RunAttemptMode::Execute {
                    project_execution_failure_to_review_flow(
                        &transaction,
                        &recovered,
                        RunAttemptStatus::Interrupted,
                        Some(recovery_disposition),
                        timestamp,
                    )?;
                }
            }
            transaction
                .execute(
                    "DELETE FROM run_approval_reservations WHERE attempt_id = ?1",
                    [attempt_id],
                )
                .map_err(PersistenceError::database)?;
            changed = true;
        }

        let legacy_running_tasks = {
            let mut statement = transaction
                .prepare(
                    "SELECT owner_agent_id, id, title, assigned_agent_id, status, phase,
                            review_agent_id, review_status
                     FROM agent_tasks AS task
                     WHERE status = 'Running' OR review_status = 'Running'
                        OR EXISTS (
                            SELECT 1 FROM review_flows AS flow
                            WHERE flow.task_owner_agent_id = task.owner_agent_id
                              AND flow.task_id = task.id
                              AND flow.state = 'awaiting_human'
                              AND flow.latest_execution_attempt_id IS NULL
                        )",
                )
                .map_err(PersistenceError::database)?;
            collect_rows(statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }))?
        };
        for (
            owner_id,
            task_id,
            title,
            assigned_id,
            status,
            phase,
            review_agent_id,
            review_status,
        ) in legacy_running_tasks
        {
            let legacy_review_flow = transaction
                .query_row(
                    "SELECT id, executor_agent_id FROM review_flows
                     WHERE task_owner_agent_id = ?1 AND task_id = ?2
                       AND state = 'awaiting_human'
                       AND latest_execution_attempt_id IS NULL",
                    params![owner_id, task_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(PersistenceError::database)?;
            let mode = if legacy_review_flow.is_some() {
                RunAttemptMode::Execute
            } else if review_status == "Running" {
                RunAttemptMode::Review
            } else {
                RunAttemptMode::Execute
            };
            if reconciled_tasks.contains(&(owner_id, task_id, mode.as_str().to_string())) {
                continue;
            }
            let request_id = format!(
                "recovery:{owner_id}:{task_id}:{}:{timestamp}",
                mode.as_str()
            );
            let agent_id = if let Some((_, executor_agent_id)) = legacy_review_flow {
                executor_agent_id
            } else if mode == RunAttemptMode::Review {
                review_agent_id.unwrap_or(assigned_id)
            } else {
                assigned_id
            };
            let message = if legacy_review_flow.is_some() {
                "A legacy in-progress review had no durable execution evidence. Trusted human adjudication is required."
            } else {
                "A legacy in-progress task had no durable attempt record. Inspect the workspace before retrying."
            };
            transaction
                .execute(
                    "INSERT INTO run_attempts
                     (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                      workspace_fingerprint, agent_id, task_owner_agent_id, task_id,
                      task_title, run_mode, status, task_status_before, task_phase_before,
                      review_status_before, admitted_at_unix_ms, completed_at_unix_ms,
                      output_summary, error_code, error_message, payload_bytes,
                      summary_truncated, before_snapshot_truncated,
                      after_snapshot_truncated, recovery_disposition)
                     VALUES (?1, '{}', 'legacy-reconciliation', 'legacy-reconciliation',
                             'legacy-reconciliation', ?2, ?3, ?4, ?5, ?6, 'interrupted',
                             ?7, ?8, ?9, ?10, ?10, ?11, 'RUN_INTERRUPTED', ?11, ?12,
                             ?13, ?13, ?13,
                             'manual_review_required')",
                    params![
                        request_id,
                        agent_id,
                        owner_id,
                        task_id,
                        title,
                        mode.as_str(),
                        status,
                        phase,
                        review_status,
                        timestamp,
                        message,
                        message.len() as i64,
                        legacy_review_flow.is_some() as i64
                    ],
                )
                .map_err(PersistenceError::database)?;
            let synthetic = read_run_attempt(&transaction, transaction.last_insert_rowid())?;
            if let Some((flow_id, _)) = legacy_review_flow {
                transaction
                    .execute(
                        "UPDATE review_flows
                         SET latest_execution_attempt_id = ?1,
                             updated_at_unix_ms = ?2
                         WHERE id = ?3 AND state = 'awaiting_human'
                           AND latest_execution_attempt_id IS NULL",
                        params![synthetic.id, timestamp, flow_id],
                    )
                    .map_err(PersistenceError::database)?;
                transaction
                    .execute(
                        "UPDATE agent_tasks
                         SET status = 'Under Review', phase = 'Supervisor Approval',
                             completed_at = NULL, queue_state = 'notQueued',
                             enqueue_sequence = NULL, review_status = 'Failed',
                             review_result = ?1
                         WHERE owner_agent_id = ?2 AND id = ?3",
                        params![message, owner_id, task_id],
                    )
                    .map_err(PersistenceError::database)?;
                advance_review_revision(&transaction)?;
            } else {
                project_recovered_attempt_to_task(&transaction, &synthetic, false)?;
            }
            changed = true;
        }

        transaction
            .execute(
                "DELETE FROM run_approval_reservations
                 WHERE attempt_id NOT IN (
                     SELECT id FROM run_attempts
                     WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                      'cancel_requested')
                 )",
                [],
            )
            .map_err(PersistenceError::database)?;
        if changed {
            transaction
                .execute(
                    "UPDATE run_coordinator_meta
                     SET active_attempt_id = NULL, revision = revision + 1
                     WHERE singleton = 1",
                    [],
                )
                .map_err(PersistenceError::database)?;
            advance_task_orchestration_revision(&transaction)?;
            prune_run_history(&transaction, timestamp)?;
            refresh_run_retention_meta(&transaction)?;
        }
        transaction.commit().map_err(PersistenceError::database)
    }

    fn reconcile_dispatched_system_actions(&mut self) -> PersistenceResult<()> {
        let timestamp = now_unix_ms()?;
        self.connection
            .execute(
                "UPDATE system_action_audits
                 SET status = 'uncertain',
                     detail_code = 'SYSTEM_ACTION_DISPATCH_INTERRUPTED',
                     detail_message = 'The application restarted after dispatch; the action was not retried.',
                     updated_at_unix_ms = ?1
                 WHERE status = 'dispatched'",
                [timestamp],
            )
            .map_err(PersistenceError::database)?;
        Ok(())
    }

    fn reconcile_reserved_reminder_deliveries(&mut self) -> PersistenceResult<()> {
        let timestamp = now_unix_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let changed = transaction
            .execute(
                "UPDATE reminder_occurrences
                 SET status = 'uncertain',
                     detail_code = 'NOTIFICATION_DISPATCH_INTERRUPTED',
                     detail_message =
                         'The application restarted after notification dispatch began; the notification was not retried.',
                     updated_at_unix_ms = ?1
                 WHERE status = 'reserved'",
                [timestamp],
            )
            .map_err(PersistenceError::database)?;
        if changed > 0 {
            transaction
                .execute(
                    "UPDATE reminder_scheduler_meta SET revision = revision + 1
                     WHERE singleton = 1",
                    [],
                )
                .map_err(PersistenceError::database)?;
            advance_application_revision(&transaction)?;
        }
        transaction.commit().map_err(PersistenceError::database)
    }
}

#[derive(Debug, Clone)]
struct ActiveReviewFlowBinding {
    id: i64,
    task_owner_agent_id: i64,
    task_id: i64,
    executor_agent_id: i64,
    state: String,
    revision_round: i64,
    current_level: Option<ReviewLevel>,
    required_levels: Vec<ReviewLevel>,
    latest_execution_attempt_id: Option<i64>,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
}

fn review_protocol_error(
    error: crate::review_orchestration::ReviewProtocolError,
) -> PersistenceError {
    PersistenceError::new(&error.code, error.message, false)
}

fn parse_review_level(value: Option<String>) -> PersistenceResult<Option<ReviewLevel>> {
    value
        .map(|value| ReviewLevel::from_storage(&value).map_err(review_protocol_error))
        .transpose()
}

fn parse_required_review_levels(value: &str) -> PersistenceResult<Vec<ReviewLevel>> {
    serde_json::from_str::<Vec<String>>(value)
        .map_err(|_| {
            PersistenceError::new(
                "REVIEW_LEDGER_INVALID",
                "Stored required review levels are invalid.",
                false,
            )
        })?
        .into_iter()
        .map(|level| ReviewLevel::from_storage(&level).map_err(review_protocol_error))
        .collect()
}

fn read_active_review_flow_binding(
    connection: &Connection,
    task_owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<ActiveReviewFlowBinding> {
    let stored = connection
        .query_row(
            "SELECT id, task_owner_agent_id, task_id, executor_agent_id, state,
                    revision_round, current_level, required_levels_json,
                    latest_execution_attempt_id, last_error_code, last_error_message
             FROM review_flows
             WHERE task_owner_agent_id = ?1 AND task_id = ?2
               AND state NOT IN ('completed', 'failed', 'cancelled')",
            params![task_owner_agent_id, task_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                "REVIEW_FLOW_NOT_FOUND",
                "The selected task has no active review flow.",
                true,
            ),
            other => PersistenceError::database(other),
        })?;
    Ok(ActiveReviewFlowBinding {
        id: stored.0,
        task_owner_agent_id: stored.1,
        task_id: stored.2,
        executor_agent_id: stored.3,
        state: stored.4,
        revision_round: stored.5,
        current_level: parse_review_level(stored.6)?,
        required_levels: parse_required_review_levels(&stored.7)?,
        latest_execution_attempt_id: stored.8,
        last_error_code: stored.9,
        last_error_message: stored.10,
    })
}

fn read_review_orchestration_snapshot(
    connection: &Connection,
) -> PersistenceResult<ReviewOrchestrationSnapshot> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM review_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let flow_rows = {
        let mut statement = connection
            .prepare(
                "SELECT id, task_owner_agent_id, task_id, executor_agent_id, state,
                        revision_round, max_revisions, current_level, required_levels_json,
                        latest_execution_attempt_id, review_mode, last_error_code,
                        last_error_message, created_at_unix_ms, updated_at_unix_ms,
                        completed_at_unix_ms
                 FROM review_flows ORDER BY id DESC",
            )
            .map_err(PersistenceError::database)?;
        collect_rows(statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, i64>(14)?,
                row.get::<_, Option<i64>>(15)?,
            ))
        }))?
    };
    let mut flows = Vec::with_capacity(flow_rows.len());
    for row in flow_rows {
        let stage_rows = {
            let mut statement = connection
                .prepare(
                    "SELECT id, revision_round, level, attempt_number, actor,
                            reviewer_agent_id, state, request_fingerprint, verdict,
                            feedback, run_attempt_id, error_code, error_message,
                            created_at_unix_ms, started_at_unix_ms, completed_at_unix_ms
                     FROM review_stage_attempts WHERE flow_id = ?1 ORDER BY id",
                )
                .map_err(PersistenceError::database)?;
            collect_rows(statement.query_map([row.0], |stage| {
                Ok((
                    stage.get::<_, i64>(0)?,
                    stage.get::<_, i64>(1)?,
                    stage.get::<_, String>(2)?,
                    stage.get::<_, i64>(3)?,
                    stage.get::<_, String>(4)?,
                    stage.get::<_, Option<i64>>(5)?,
                    stage.get::<_, String>(6)?,
                    stage.get::<_, String>(7)?,
                    stage.get::<_, Option<String>>(8)?,
                    stage.get::<_, Option<String>>(9)?,
                    stage.get::<_, Option<i64>>(10)?,
                    stage.get::<_, Option<String>>(11)?,
                    stage.get::<_, Option<String>>(12)?,
                    stage.get::<_, i64>(13)?,
                    stage.get::<_, Option<i64>>(14)?,
                    stage.get::<_, Option<i64>>(15)?,
                ))
            }))?
        };
        let stages = stage_rows
            .into_iter()
            .map(|stage| {
                let actor = match stage.4.as_str() {
                    "agent" => ReviewActor::Agent,
                    "human" => ReviewActor::Human,
                    _ => {
                        return Err(PersistenceError::new(
                            "REVIEW_LEDGER_INVALID",
                            "Stored review actor is invalid.",
                            false,
                        ))
                    }
                };
                let verdict = match stage.8.as_deref() {
                    Some("approved") => Some(ReviewVerdict::Approved),
                    Some("changes_requested") => Some(ReviewVerdict::ChangesRequested),
                    None => None,
                    _ => {
                        return Err(PersistenceError::new(
                            "REVIEW_LEDGER_INVALID",
                            "Stored review verdict is invalid.",
                            false,
                        ))
                    }
                };
                Ok(ReviewStageAttemptProjection {
                    id: stage.0,
                    flow_id: row.0,
                    revision_round: stage.1,
                    level: ReviewLevel::from_storage(&stage.2).map_err(review_protocol_error)?,
                    attempt_number: stage.3,
                    actor,
                    reviewer_agent_id: stage.5,
                    state: stage.6,
                    request_fingerprint: stage.7,
                    verdict,
                    feedback: stage.9,
                    run_attempt_id: stage.10,
                    error_code: stage.11,
                    error_message: stage.12,
                    created_at_unix_ms: stage.13,
                    started_at_unix_ms: stage.14,
                    completed_at_unix_ms: stage.15,
                })
            })
            .collect::<PersistenceResult<Vec<_>>>()?;
        flows.push(ReviewFlowProjection {
            id: row.0,
            task_owner_agent_id: row.1,
            task_id: row.2,
            executor_agent_id: row.3,
            state: row.4,
            revision_round: row.5,
            max_revisions: row.6,
            current_level: parse_review_level(row.7)?,
            required_levels: parse_required_review_levels(&row.8)?,
            latest_execution_attempt_id: row.9,
            review_mode: row.10,
            last_error_code: row.11,
            last_error_message: row.12,
            created_at_unix_ms: row.13,
            updated_at_unix_ms: row.14,
            completed_at_unix_ms: row.15,
            stages,
        });
    }
    Ok(ReviewOrchestrationSnapshot { revision, flows })
}

fn ensure_expected_review_revision(
    connection: &Connection,
    expected_revision: i64,
) -> PersistenceResult<()> {
    let current: i64 = connection
        .query_row(
            "SELECT revision FROM review_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if expected_revision != current {
        return Err(PersistenceError::new(
            "STALE_REVIEW_REVISION",
            format!(
                "The review flow changed before this action (expected {expected_revision}, current {current})."
            ),
            true,
        ));
    }
    Ok(())
}

fn advance_review_revision(connection: &Connection) -> PersistenceResult<i64> {
    let current: i64 = connection
        .query_row(
            "SELECT revision FROM review_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let next = next_revision(current)?;
    connection
        .execute(
            "UPDATE review_orchestration_meta SET revision = ?1 WHERE singleton = 1",
            [next],
        )
        .map_err(PersistenceError::database)?;
    Ok(next)
}

fn allocate_review_stage_id(connection: &Connection) -> PersistenceResult<i64> {
    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(id), 0) FROM review_stage_attempts",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if current >= MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "REVIEW_STAGE_ID_EXHAUSTED",
            "No additional JavaScript-safe review-stage identifiers are available.",
            false,
        ));
    }
    Ok(current + 1)
}

fn read_prior_reviewer_ids(
    connection: &Connection,
    flow_id: i64,
    current_level: ReviewLevel,
) -> PersistenceResult<HashSet<i64>> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT reviewer_agent_id FROM review_stage_attempts
             WHERE flow_id = ?1 AND reviewer_agent_id IS NOT NULL AND level <> ?2",
        )
        .map_err(PersistenceError::database)?;
    Ok(collect_rows(
        statement.query_map(params![flow_id, current_level.as_storage()], |row| {
            row.get(0)
        }),
    )?
    .into_iter()
    .collect())
}

fn mark_review_awaiting_human(
    connection: &Connection,
    flow: &ActiveReviewFlowBinding,
    code: &str,
    message: &str,
    timestamp: i64,
) -> PersistenceResult<()> {
    connection
        .execute(
            "UPDATE review_flows
             SET state = 'awaiting_human', last_error_code = ?1,
                 last_error_message = ?2, updated_at_unix_ms = ?3
             WHERE id = ?4 AND state NOT IN ('completed', 'failed', 'cancelled')",
            params![code, message, timestamp, flow.id],
        )
        .map_err(PersistenceError::database)?;
    connection
        .execute(
            "UPDATE agent_tasks
             SET status = 'Under Review', phase = 'Supervisor Approval',
                 review_status = 'Failed', review_result = ?1
             WHERE owner_agent_id = ?2 AND id = ?3",
            params![message, flow.task_owner_agent_id, flow.task_id],
        )
        .map_err(PersistenceError::database)?;
    advance_review_revision(connection)?;
    advance_task_orchestration_revision(connection)?;
    Ok(())
}

fn validate_human_review_request(request: &HumanReviewDecisionRequest) -> PersistenceResult<()> {
    if !(0..=MAX_SAFE_INTEGER).contains(&request.expected_revision)
        || !(1..=MAX_SAFE_INTEGER).contains(&request.task_owner_agent_id)
        || !(1..=MAX_SAFE_INTEGER).contains(&request.task_id)
        || !(1..=MAX_SAFE_INTEGER).contains(&request.flow_id)
        || request.feedback.len() > 32 * 1024
        || request
            .feedback
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        || (request.verdict == ReviewVerdict::Approved && !request.feedback.trim().is_empty())
        || (request.verdict == ReviewVerdict::ChangesRequested
            && request.feedback.trim().is_empty())
    {
        return Err(PersistenceError::new(
            "INVALID_HUMAN_REVIEW_DECISION",
            "Human approval requires empty feedback; requesting changes requires bounded non-empty feedback.",
            true,
        ));
    }
    Ok(())
}

fn ensure_human_review_flow(
    flow: &ActiveReviewFlowBinding,
    expected_flow_id: i64,
) -> PersistenceResult<()> {
    if flow.id != expected_flow_id || flow.state != "awaiting_human" {
        return Err(PersistenceError::new(
            "HUMAN_REVIEW_STATE_CONFLICT",
            "The selected review flow is not awaiting a trusted human decision.",
            true,
        ));
    }
    Ok(())
}

fn ensure_review_intent_binding(
    connection: &Connection,
    intent: &ActionIntent,
) -> PersistenceResult<()> {
    let ActionIntent::RunTask {
        agent_id,
        task_owner_agent_id,
        task_id,
        run_mode,
        review_context,
    } = intent
    else {
        return Ok(());
    };
    match (run_mode, review_context) {
        (crate::policy::RunMode::Execute, None) => Ok(()),
        (crate::policy::RunMode::Review, Some(context)) => {
            let matches_binding: bool = connection
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1
                         FROM review_flows AS flow
                         JOIN review_stage_attempts AS stage ON stage.flow_id = flow.id
                         WHERE flow.id = ?1 AND stage.id = ?2
                           AND flow.task_owner_agent_id = ?3 AND flow.task_id = ?4
                           AND flow.state = 'review_pending'
                           AND flow.revision_round = ?5 AND flow.current_level = ?6
                           AND stage.revision_round = ?5 AND stage.level = ?6
                           AND stage.reviewer_agent_id = ?7 AND stage.actor = 'agent'
                           AND stage.state = 'pending'
                           AND stage.request_fingerprint = ?8
                     )",
                    params![
                        context.flow_id,
                        context.stage_attempt_id,
                        task_owner_agent_id,
                        task_id,
                        context.revision_round,
                        context.level.as_storage(),
                        agent_id,
                        context.request_fingerprint
                    ],
                    |row| row.get(0),
                )
                .map_err(PersistenceError::database)?;
            if !matches_binding {
                return Err(PersistenceError::new(
                    "REVIEW_INTENT_STALE",
                    "The review intent does not match the exact pending backend review stage.",
                    true,
                ));
            }
            Ok(())
        }
        _ => Err(PersistenceError::new(
            "INVALID_REVIEW_CONTEXT",
            "Execution must not carry review context and review must carry a bound context.",
            true,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_review_verdict(
    connection: &Connection,
    flow: &ActiveReviewFlowBinding,
    level: ReviewLevel,
    result: &ReviewResultV1,
    actor: ReviewActor,
    reviewer_agent_id: Option<i64>,
    runtime_model: Option<&str>,
    duration_seconds: Option<u64>,
    timestamp: i64,
) -> PersistenceResult<()> {
    let result_json = serde_json::to_string(result).map_err(|_| {
        PersistenceError::new(
            "REVIEW_RESULT_INVALID",
            "The structured review result could not be projected.",
            false,
        )
    })?;
    let reviewed_at = format_unix_ms(timestamp);
    match result.verdict {
        ReviewVerdict::Approved => {
            if let Some(next_level) = next_required_level(&flow.required_levels, level) {
                connection
                    .execute(
                        "UPDATE review_flows
                         SET state = 'awaiting_review', current_level = ?1,
                             last_error_code = NULL, last_error_message = NULL,
                             updated_at_unix_ms = ?2
                         WHERE id = ?3",
                        params![next_level.as_storage(), timestamp, flow.id],
                    )
                    .map_err(PersistenceError::database)?;
                connection
                    .execute(
                        "UPDATE agent_tasks
                         SET status = 'Under Review', phase = ?1, completed_at = NULL,
                             review_agent_id = ?2, review_status = 'Pending',
                             review_result = ?3, review_model = ?4,
                             review_duration_seconds = ?5, reviewed_at = ?6
                         WHERE owner_agent_id = ?7 AND id = ?8",
                        params![
                            next_level.task_phase(),
                            reviewer_agent_id,
                            result_json,
                            runtime_model,
                            duration_seconds.map(|value| value as f64),
                            reviewed_at,
                            flow.task_owner_agent_id,
                            flow.task_id
                        ],
                    )
                    .map_err(PersistenceError::database)?;
            } else {
                connection
                    .execute(
                        "UPDATE review_flows
                         SET state = 'completed', current_level = NULL,
                             last_error_code = NULL, last_error_message = NULL,
                             updated_at_unix_ms = ?1, completed_at_unix_ms = ?1
                         WHERE id = ?2",
                        params![timestamp, flow.id],
                    )
                    .map_err(PersistenceError::database)?;
                connection
                    .execute(
                        "UPDATE agent_tasks
                         SET status = 'Completed', phase = 'Finished', completed_at = ?1,
                             queue_state = 'notQueued', enqueue_sequence = NULL,
                             review_agent_id = ?2, review_status = 'Approved',
                             review_result = ?3, review_model = ?4,
                             review_duration_seconds = ?5, reviewed_at = ?1
                         WHERE owner_agent_id = ?6 AND id = ?7",
                        params![
                            reviewed_at,
                            reviewer_agent_id,
                            result_json,
                            runtime_model,
                            duration_seconds.map(|value| value as f64),
                            flow.task_owner_agent_id,
                            flow.task_id
                        ],
                    )
                    .map_err(PersistenceError::database)?;
            }
        }
        ReviewVerdict::ChangesRequested if flow.revision_round < MAX_REVISION_ROUNDS => {
            let enqueue_sequence = allocate_enqueue_sequence(connection)?;
            let next_level = flow.required_levels.first().copied();
            connection
                .execute(
                    "UPDATE review_flows
                     SET state = 'revision_queued', revision_round = revision_round + 1,
                         current_level = ?1, last_error_code = NULL,
                         last_error_message = NULL, updated_at_unix_ms = ?2
                     WHERE id = ?3",
                    params![next_level.map(ReviewLevel::as_storage), timestamp, flow.id],
                )
                .map_err(PersistenceError::database)?;
            connection
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Pending', phase = 'Assigned', completed_at = NULL,
                         queue_state = 'queued', enqueue_sequence = ?1,
                         review_agent_id = ?2, review_status = 'Changes Requested',
                         review_result = ?3, review_model = ?4,
                         review_duration_seconds = ?5, reviewed_at = ?6
                     WHERE owner_agent_id = ?7 AND id = ?8",
                    params![
                        enqueue_sequence,
                        reviewer_agent_id,
                        result.feedback,
                        runtime_model,
                        duration_seconds.map(|value| value as f64),
                        reviewed_at,
                        flow.task_owner_agent_id,
                        flow.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
            expire_task_approvals(connection, flow.task_owner_agent_id, flow.task_id)?;
        }
        ReviewVerdict::ChangesRequested if actor == ReviewActor::Agent => {
            connection
                .execute(
                    "UPDATE review_flows
                     SET state = 'awaiting_human',
                         last_error_code = 'REVIEW_REVISION_LIMIT_REACHED',
                         last_error_message = 'Three revision executions were reviewed without approval.',
                         updated_at_unix_ms = ?1
                     WHERE id = ?2",
                    params![timestamp, flow.id],
                )
                .map_err(PersistenceError::database)?;
            connection
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Under Review', phase = 'Supervisor Approval',
                         completed_at = NULL, review_agent_id = ?1,
                         review_status = 'Changes Requested', review_result = ?2,
                         review_model = ?3, review_duration_seconds = ?4, reviewed_at = ?5
                     WHERE owner_agent_id = ?6 AND id = ?7",
                    params![
                        reviewer_agent_id,
                        result.feedback,
                        runtime_model,
                        duration_seconds.map(|value| value as f64),
                        reviewed_at,
                        flow.task_owner_agent_id,
                        flow.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
        }
        ReviewVerdict::ChangesRequested => {
            connection
                .execute(
                    "UPDATE review_flows
                     SET state = 'failed', current_level = NULL,
                         last_error_code = 'REVIEW_REJECTED_AT_REVISION_LIMIT',
                         last_error_message = ?1, updated_at_unix_ms = ?2,
                         completed_at_unix_ms = ?2
                     WHERE id = ?3",
                    params![result.feedback, timestamp, flow.id],
                )
                .map_err(PersistenceError::database)?;
            connection
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Failed', phase = 'Failed', completed_at = ?1,
                         queue_state = 'notQueued', enqueue_sequence = NULL,
                         review_agent_id = NULL, review_status = 'Changes Requested',
                         review_result = ?2, review_model = NULL,
                         review_duration_seconds = NULL, reviewed_at = ?1
                     WHERE owner_agent_id = ?3 AND id = ?4",
                    params![
                        reviewed_at,
                        result.feedback,
                        flow.task_owner_agent_id,
                        flow.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
        }
    }
    advance_review_revision(connection)?;
    advance_task_orchestration_revision(connection)?;
    Ok(())
}

#[derive(Debug)]
struct StoredRunAttempt {
    id: i64,
    request_id: String,
    agent_id: i64,
    task_owner_agent_id: i64,
    task_id: i64,
    task_title: String,
    run_mode: String,
    status: String,
    provider: Option<String>,
    model: Option<String>,
    workspace_id: Option<String>,
    approval_id: Option<i64>,
    review_flow_id: Option<i64>,
    review_stage_attempt_id: Option<i64>,
    review_revision_round: Option<i64>,
    admitted_at_unix_ms: i64,
    started_at_unix_ms: Option<i64>,
    cancel_requested_at_unix_ms: Option<i64>,
    completed_at_unix_ms: Option<i64>,
    duration_seconds: Option<i64>,
    output_summary: Option<String>,
    stderr_excerpt: Option<String>,
    response_id: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    changed_files_json: String,
    diff: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    progress_event_count: i64,
    recovery_disposition: Option<String>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    summary_truncated: bool,
    diff_truncated: bool,
    changed_files_truncated: bool,
    progress_truncated: bool,
    before_snapshot_truncated: bool,
    after_snapshot_truncated: bool,
    original_stdout_bytes: i64,
    original_stderr_bytes: i64,
    original_summary_bytes: i64,
    original_diff_bytes: i64,
    original_changed_file_count: i64,
    omitted_progress_event_count: i64,
    workspace_evidence_json: Option<String>,
    specialist_contract_json: Option<String>,
    specialist_result_json: Option<String>,
}

fn map_stored_run_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRunAttempt> {
    Ok(StoredRunAttempt {
        id: row.get(0)?,
        request_id: row.get(1)?,
        agent_id: row.get(2)?,
        task_owner_agent_id: row.get(3)?,
        task_id: row.get(4)?,
        task_title: row.get(5)?,
        run_mode: row.get(6)?,
        status: row.get(7)?,
        provider: row.get(8)?,
        model: row.get(9)?,
        workspace_id: row.get(10)?,
        approval_id: row.get(11)?,
        review_flow_id: row.get(12)?,
        review_stage_attempt_id: row.get(13)?,
        review_revision_round: row.get(14)?,
        admitted_at_unix_ms: row.get(15)?,
        started_at_unix_ms: row.get(16)?,
        cancel_requested_at_unix_ms: row.get(17)?,
        completed_at_unix_ms: row.get(18)?,
        duration_seconds: row.get(19)?,
        output_summary: row.get(20)?,
        stderr_excerpt: row.get(21)?,
        response_id: row.get(22)?,
        input_tokens: row.get(23)?,
        output_tokens: row.get(24)?,
        total_tokens: row.get(25)?,
        changed_files_json: row.get(26)?,
        diff: row.get(27)?,
        error_code: row.get(28)?,
        error_message: row.get(29)?,
        progress_event_count: row.get(30)?,
        recovery_disposition: row.get(31)?,
        stdout_truncated: row.get::<_, i64>(32)? != 0,
        stderr_truncated: row.get::<_, i64>(33)? != 0,
        summary_truncated: row.get::<_, i64>(34)? != 0,
        diff_truncated: row.get::<_, i64>(35)? != 0,
        changed_files_truncated: row.get::<_, i64>(36)? != 0,
        progress_truncated: row.get::<_, i64>(37)? != 0,
        before_snapshot_truncated: row.get::<_, i64>(38)? != 0,
        after_snapshot_truncated: row.get::<_, i64>(39)? != 0,
        original_stdout_bytes: row.get(40)?,
        original_stderr_bytes: row.get(41)?,
        original_summary_bytes: row.get(42)?,
        original_diff_bytes: row.get(43)?,
        original_changed_file_count: row.get(44)?,
        omitted_progress_event_count: row.get(45)?,
        workspace_evidence_json: row.get(46)?,
        specialist_contract_json: row.get(47)?,
        specialist_result_json: row.get(48)?,
    })
}

const RUN_ATTEMPT_PROJECTION_QUERY: &str =
    "SELECT id, request_id, agent_id, task_owner_agent_id, task_id, task_title,
            run_mode, status, provider, model, workspace_id, approval_id,
            review_flow_id, review_stage_attempt_id, review_revision_round,
            admitted_at_unix_ms, started_at_unix_ms, cancel_requested_at_unix_ms,
            completed_at_unix_ms, duration_seconds, output_summary, stderr_excerpt,
            response_id, input_tokens, output_tokens, total_tokens, changed_files_json,
            diff, error_code, error_message, progress_event_count, recovery_disposition,
            stdout_truncated, stderr_truncated, summary_truncated, diff_truncated,
            changed_files_truncated, progress_truncated, before_snapshot_truncated,
            after_snapshot_truncated, original_stdout_bytes, original_stderr_bytes,
            original_summary_bytes, original_diff_bytes, original_changed_file_count,
            omitted_progress_event_count, workspace_evidence_json,
            specialist_contract_json, specialist_result_json
     FROM run_attempts WHERE id = ?1";

fn read_run_attempt(
    connection: &Connection,
    attempt_id: i64,
) -> PersistenceResult<RunAttemptProjection> {
    let stored = connection
        .query_row(
            RUN_ATTEMPT_PROJECTION_QUERY,
            [attempt_id],
            map_stored_run_attempt,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                "RUN_NOT_FOUND",
                "The requested run attempt does not exist.",
                true,
            ),
            other => PersistenceError::database(other),
        })?;
    let run_mode = RunAttemptMode::from_storage(&stored.run_mode)
        .map_err(|message| PersistenceError::new("RUN_LEDGER_INVALID", message, false))?;
    let status = RunAttemptStatus::from_storage(&stored.status)
        .map_err(|message| PersistenceError::new("RUN_LEDGER_INVALID", message, false))?;
    let changed_files =
        serde_json::from_str::<Vec<String>>(&stored.changed_files_json).map_err(|_| {
            PersistenceError::new(
                "RUN_LEDGER_INVALID",
                "Stored changed-file evidence is invalid.",
                false,
            )
        })?;
    let workspace_changes = match stored.workspace_evidence_json.as_deref() {
        Some(json) => {
            if json.len() > MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES {
                return Err(PersistenceError::new(
                    "RUN_LEDGER_INVALID",
                    "Stored workspace evidence exceeds its payload bound.",
                    false,
                ));
            }
            let evidence =
                serde_json::from_str::<WorkspaceChangeEvidenceV1>(json).map_err(|_| {
                    PersistenceError::new(
                        "RUN_LEDGER_INVALID",
                        "Stored workspace evidence is invalid.",
                        false,
                    )
                })?;
            evidence.validate().map_err(|message| {
                PersistenceError::new(
                    "RUN_LEDGER_INVALID",
                    format!("Stored workspace evidence is invalid: {message}."),
                    false,
                )
            })?;
            evidence
        }
        None => WorkspaceChangeEvidenceV1::legacy_unavailable(
            "This run predates structured workspace evidence persistence.",
        ),
    };
    let specialist_contract = stored
        .specialist_contract_json
        .as_deref()
        .map(|json| {
            let contract = serde_json::from_str::<SpecialistRunContractV1>(json).map_err(|_| {
                PersistenceError::new(
                    "RUN_LEDGER_INVALID",
                    "Stored specialist contract is invalid.",
                    false,
                )
            })?;
            contract.validate().map_err(|error| {
                PersistenceError::new("RUN_LEDGER_INVALID", error.message, false)
            })?;
            Ok(contract)
        })
        .transpose()?;
    let specialist_result = stored
        .specialist_result_json
        .as_deref()
        .map(|json| {
            parse_specialist_result_json(json)
                .map_err(|error| PersistenceError::new("RUN_LEDGER_INVALID", error.message, false))
        })
        .transpose()?;
    if specialist_result
        .as_ref()
        .zip(specialist_contract.as_ref())
        .is_some_and(|(result, contract)| result.kind() != contract.kind)
        || (specialist_result.is_some() && specialist_contract.is_none())
    {
        return Err(PersistenceError::new(
            "RUN_LEDGER_INVALID",
            "Stored specialist result does not match its immutable run contract.",
            false,
        ));
    }
    Ok(RunAttemptProjection {
        id: stored.id,
        request_id: stored.request_id,
        agent_id: stored.agent_id,
        task_owner_agent_id: stored.task_owner_agent_id,
        task_id: stored.task_id,
        task_title: stored.task_title,
        run_mode,
        status,
        provider: stored.provider,
        model: stored.model,
        workspace_id: stored.workspace_id,
        approval_id: stored.approval_id,
        specialist_contract,
        specialist_result,
        review_flow_id: stored.review_flow_id,
        review_stage_attempt_id: stored.review_stage_attempt_id,
        review_revision_round: stored.review_revision_round,
        admitted_at_unix_ms: stored.admitted_at_unix_ms,
        started_at_unix_ms: stored.started_at_unix_ms,
        cancel_requested_at_unix_ms: stored.cancel_requested_at_unix_ms,
        completed_at_unix_ms: stored.completed_at_unix_ms,
        duration_seconds: stored.duration_seconds.map(nonnegative_u64).transpose()?,
        output_summary: stored.output_summary,
        stderr_excerpt: stored.stderr_excerpt,
        response_id: stored.response_id,
        usage: RunUsage {
            input_tokens: stored.input_tokens.map(nonnegative_u64).transpose()?,
            output_tokens: stored.output_tokens.map(nonnegative_u64).transpose()?,
            total_tokens: stored.total_tokens.map(nonnegative_u64).transpose()?,
        },
        changed_files,
        diff: stored.diff,
        workspace_changes,
        error_code: stored.error_code,
        error_message: stored.error_message,
        progress_event_count: nonnegative_u64(stored.progress_event_count)?,
        recovery_disposition: stored.recovery_disposition,
        truncation: RunTruncationEvidence {
            stdout_truncated: stored.stdout_truncated,
            stderr_truncated: stored.stderr_truncated,
            summary_truncated: stored.summary_truncated,
            diff_truncated: stored.diff_truncated,
            changed_files_truncated: stored.changed_files_truncated,
            progress_truncated: stored.progress_truncated,
            before_snapshot_truncated: stored.before_snapshot_truncated,
            after_snapshot_truncated: stored.after_snapshot_truncated,
            original_stdout_bytes: nonnegative_u64(stored.original_stdout_bytes)?,
            original_stderr_bytes: nonnegative_u64(stored.original_stderr_bytes)?,
            original_summary_bytes: nonnegative_u64(stored.original_summary_bytes)?,
            original_diff_bytes: nonnegative_u64(stored.original_diff_bytes)?,
            original_changed_file_count: nonnegative_u64(stored.original_changed_file_count)?,
            omitted_progress_event_count: nonnegative_u64(stored.omitted_progress_event_count)?,
        },
    })
}

fn read_review_request_json(
    connection: &Connection,
    stage_attempt_id: i64,
) -> PersistenceResult<String> {
    connection
        .query_row(
            "SELECT request_json FROM review_stage_attempts WHERE id = ?1",
            [stage_attempt_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                "REVIEW_STAGE_NOT_FOUND",
                "The bound review stage no longer exists.",
                true,
            ),
            other => PersistenceError::database(other),
        })
}

fn nonnegative_u64(value: i64) -> PersistenceResult<u64> {
    u64::try_from(value).map_err(|_| {
        PersistenceError::new(
            "RUN_LEDGER_INVALID",
            "Stored run metadata contains a negative value.",
            false,
        )
    })
}

fn bounded_i64(value: u64) -> i64 {
    value.min(MAX_SAFE_INTEGER as u64) as i64
}

fn optional_bounded_i64(value: Option<u64>) -> Option<i64> {
    value.map(bounded_i64)
}

fn run_intent_parts(
    intent: &ActionIntent,
) -> PersistenceResult<(i64, i64, i64, RunAttemptMode, Option<ReviewIntentContext>)> {
    match intent {
        ActionIntent::RunTask {
            agent_id,
            task_owner_agent_id,
            task_id,
            run_mode,
            review_context,
        } => Ok((
            *agent_id,
            *task_owner_agent_id,
            *task_id,
            match run_mode {
                crate::policy::RunMode::Execute => RunAttemptMode::Execute,
                crate::policy::RunMode::Review => RunAttemptMode::Review,
            },
            review_context.clone(),
        )),
        _ => Err(PersistenceError::new(
            "INVALID_RUN_INTENT",
            "Only a task execution or review can be admitted as an AI run.",
            true,
        )),
    }
}

fn ensure_execute_queue_head(
    transaction: &Transaction<'_>,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<()> {
    let queue_state: Option<String> = transaction
        .query_row(
            "SELECT queue_state FROM agent_tasks
             WHERE owner_agent_id = ?1 AND id = ?2",
            params![owner_agent_id, task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    let Some(queue_state) = queue_state else {
        return Err(PersistenceError::new(
            "TASK_NOT_FOUND",
            "The selected task no longer exists.",
            true,
        ));
    };
    if queue_state != "queued" {
        return Err(PersistenceError::new(
            "TASK_NOT_QUEUED",
            "Only a queued execute task can be admitted.",
            true,
        ));
    }
    let head: Option<(i64, i64)> = transaction
        .query_row(
            "SELECT owner_agent_id, id FROM agent_tasks
             WHERE queue_state = 'queued'
             ORDER BY CASE priority
                 WHEN 'Critical' THEN 0 WHEN 'High' THEN 1
                 WHEN 'Normal' THEN 2 ELSE 3 END,
                 enqueue_sequence, owner_agent_id, id
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    if head != Some((owner_agent_id, task_id)) {
        return Err(PersistenceError::new(
            "QUEUE_HEAD_REQUIRED",
            "Only queue position 1 can enter the global execute slot.",
            true,
        ));
    }
    Ok(())
}

fn advance_task_orchestration_revision(transaction: &Connection) -> PersistenceResult<i64> {
    let revision: i64 = transaction
        .query_row(
            "SELECT revision FROM task_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let next = next_revision(revision)?;
    transaction
        .execute(
            "UPDATE task_orchestration_meta SET revision = ?1 WHERE singleton = 1",
            [next],
        )
        .map_err(PersistenceError::database)?;
    Ok(next)
}

fn validate_run_label(label: &str, value: &str) -> PersistenceResult<()> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(PersistenceError::new(
            "INVALID_RUN_METADATA",
            format!("The run {label} is invalid."),
            true,
        ));
    }
    Ok(())
}

fn ensure_run_transition(
    current: RunAttemptStatus,
    next: RunAttemptStatus,
) -> PersistenceResult<()> {
    if !current.may_transition_to(next) {
        return Err(PersistenceError::new(
            "RUN_STATE_CONFLICT",
            format!(
                "The run cannot transition from {} to {}.",
                current.as_str(),
                next.as_str()
            ),
            true,
        ));
    }
    Ok(())
}

fn ensure_active_attempt(transaction: &Transaction<'_>, attempt_id: i64) -> PersistenceResult<()> {
    let active_attempt_id: Option<i64> = transaction
        .query_row(
            "SELECT active_attempt_id FROM run_coordinator_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if active_attempt_id != Some(attempt_id) {
        return Err(PersistenceError::new(
            "RUN_NOT_ACTIVE",
            "The requested run is not the authoritative active attempt.",
            true,
        ));
    }
    Ok(())
}

fn advance_run_revision(transaction: &Transaction<'_>) -> PersistenceResult<i64> {
    let current: i64 = transaction
        .query_row(
            "SELECT revision FROM run_coordinator_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let revision = next_revision(current)?;
    transaction
        .execute(
            "UPDATE run_coordinator_meta SET revision = ?1 WHERE singleton = 1",
            [revision],
        )
        .map_err(PersistenceError::database)?;
    Ok(revision)
}

fn consume_reserved_run_approval(
    transaction: &Transaction<'_>,
    attempt_id: i64,
    timestamp: i64,
) -> PersistenceResult<()> {
    let approval_id: Option<i64> = transaction
        .query_row(
            "SELECT approval_id FROM run_approval_reservations WHERE attempt_id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    let Some(approval_id) = approval_id else {
        return Ok(());
    };
    expire_authoritative_approvals(transaction, timestamp)?;
    let consumed_at = format_unix_ms(timestamp);
    let changed = transaction
        .execute(
            "UPDATE approval_requests
             SET consumed_at = ?1, consumed_at_unix_ms = ?2
             WHERE id = ?3 AND authoritative = 1 AND status = 'Approved'
               AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?2
               AND intent_fingerprint = (
                   SELECT intent_fingerprint FROM run_attempts WHERE id = ?4)
               AND policy_fingerprint = (
                   SELECT policy_fingerprint FROM run_attempts WHERE id = ?4)
               AND workspace_fingerprint = (
                   SELECT workspace_fingerprint FROM run_attempts WHERE id = ?4)",
            params![consumed_at, timestamp, approval_id, attempt_id],
        )
        .map_err(PersistenceError::database)?;
    if changed != 1 {
        return Err(PersistenceError::new(
            "APPROVAL_STATE_CHANGED",
            "The reserved approval changed before provider startup and was not consumed.",
            true,
        ));
    }
    transaction
        .execute(
            "DELETE FROM run_approval_reservations WHERE attempt_id = ?1",
            [attempt_id],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn invalidate_reserved_run_approval(
    transaction: &Transaction<'_>,
    attempt_id: i64,
    timestamp: i64,
) -> PersistenceResult<()> {
    let resolved_at = format_unix_ms(timestamp);
    transaction
        .execute(
            "UPDATE approval_requests
             SET status = 'Expired', resolved_at = COALESCE(resolved_at, ?1),
                 resolved_at_unix_ms = COALESCE(resolved_at_unix_ms, ?2)
             WHERE id = (SELECT approval_id FROM run_approval_reservations
                         WHERE attempt_id = ?3)
               AND authoritative = 1 AND status IN ('Pending', 'Approved')
               AND consumed_at_unix_ms IS NULL",
            params![resolved_at, timestamp, attempt_id],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn project_run_started_to_task(
    transaction: &Transaction<'_>,
    attempt: &RunAttemptProjection,
    timestamp: i64,
) -> PersistenceResult<()> {
    let changed = match attempt.run_mode {
        RunAttemptMode::Execute => {
            let changed = transaction
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Running', phase = 'Specialist Work', completed_at = NULL,
                         queue_state = 'running'
                     WHERE owner_agent_id = ?1 AND id = ?2 AND queue_state = 'admitted'",
                    params![attempt.task_owner_agent_id, attempt.task_id],
                )
                .map_err(PersistenceError::database)?;
            let flow_changed = transaction
                .execute(
                    "UPDATE review_flows
                     SET state = 'awaiting_execution', updated_at_unix_ms = ?1
                     WHERE task_owner_agent_id = ?2 AND task_id = ?3
                       AND state = 'revision_queued'",
                    params![timestamp, attempt.task_owner_agent_id, attempt.task_id],
                )
                .map_err(PersistenceError::database)?;
            if flow_changed == 1 {
                advance_review_revision(transaction)?;
            }
            changed
        }
        RunAttemptMode::Review => {
            let flow_id = attempt.review_flow_id.ok_or_else(|| {
                PersistenceError::new(
                    "REVIEW_BINDING_MISSING",
                    "The review run has no bound flow.",
                    false,
                )
            })?;
            let stage_id = attempt.review_stage_attempt_id.ok_or_else(|| {
                PersistenceError::new(
                    "REVIEW_BINDING_MISSING",
                    "The review run has no bound stage.",
                    false,
                )
            })?;
            let level: String = transaction
                .query_row(
                    "SELECT level FROM review_stage_attempts
                     WHERE id = ?1 AND flow_id = ?2 AND run_attempt_id = ?3
                       AND state = 'admitted'",
                    params![stage_id, flow_id, attempt.id],
                    |row| row.get(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                        "REVIEW_STAGE_STATE_CONFLICT",
                        "The admitted review stage no longer matches this run.",
                        true,
                    ),
                    other => PersistenceError::database(other),
                })?;
            let level = ReviewLevel::from_storage(&level).map_err(review_protocol_error)?;
            let stage_changed = transaction
                .execute(
                    "UPDATE review_stage_attempts
                     SET state = 'running', started_at_unix_ms = ?1
                     WHERE id = ?2 AND state = 'admitted'",
                    params![timestamp, stage_id],
                )
                .map_err(PersistenceError::database)?;
            let flow_changed = transaction
                .execute(
                    "UPDATE review_flows
                     SET state = 'reviewing', updated_at_unix_ms = ?1
                     WHERE id = ?2 AND state = 'review_pending'",
                    params![timestamp, flow_id],
                )
                .map_err(PersistenceError::database)?;
            if stage_changed != 1 || flow_changed != 1 {
                return Err(PersistenceError::new(
                    "REVIEW_STAGE_STATE_CONFLICT",
                    "The admitted review flow changed before startup could be recorded.",
                    true,
                ));
            }
            advance_review_revision(transaction)?;
            transaction
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Under Review', phase = ?1,
                         review_agent_id = ?2, review_status = 'Running'
                     WHERE owner_agent_id = ?3 AND id = ?4",
                    params![
                        level.task_phase(),
                        attempt.agent_id,
                        attempt.task_owner_agent_id,
                        attempt.task_id
                    ],
                )
                .map_err(PersistenceError::database)?
        }
    };
    if changed != 1 {
        return Err(PersistenceError::new(
            "TASK_STATE_CONFLICT",
            "The run task disappeared before startup could be recorded.",
            true,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ReviewAfterExecution {
    required: bool,
    next_level: Option<ReviewLevel>,
}

fn project_execution_success_to_review_flow(
    connection: &Connection,
    attempt: &RunAttemptProjection,
    configured_review_mode: &str,
    timestamp: i64,
) -> PersistenceResult<ReviewAfterExecution> {
    let active_flow_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM review_flows
             WHERE task_owner_agent_id = ?1 AND task_id = ?2
               AND state NOT IN ('completed', 'failed', 'cancelled')",
            params![attempt.task_owner_agent_id, attempt.task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    if active_flow_id.is_none() && configured_review_mode == "off" {
        return Ok(ReviewAfterExecution {
            required: false,
            next_level: None,
        });
    }
    let (flow_id, required_levels) = if let Some(flow_id) = active_flow_id {
        let flow = read_active_review_flow_binding(
            connection,
            attempt.task_owner_agent_id,
            attempt.task_id,
        )?;
        if flow.id != flow_id
            || flow.executor_agent_id != attempt.agent_id
            || !matches!(
                flow.state.as_str(),
                "awaiting_execution" | "revision_queued"
            )
        {
            return Err(PersistenceError::new(
                "REVIEW_FLOW_STATE_CONFLICT",
                "The revision execution does not match the active review flow.",
                true,
            ));
        }
        (flow.id, flow.required_levels)
    } else {
        if !matches!(configured_review_mode, "manual" | "automatic") {
            return Err(PersistenceError::new(
                "REVIEW_MODE_INVALID",
                "The configured review mode is invalid.",
                false,
            ));
        }
        let state = read_application_state(connection)?;
        let executor = state
            .agents
            .iter()
            .find(|agent| agent.id == attempt.agent_id)
            .ok_or_else(|| {
                PersistenceError::new(
                    "REVIEW_EXECUTOR_NOT_FOUND",
                    "The task executor no longer exists.",
                    true,
                )
            })?;
        let required_levels =
            required_levels_for_role(&executor.role).map_err(review_protocol_error)?;
        let required_levels_json = serde_json::to_string(
            &required_levels
                .iter()
                .map(|level| level.as_storage())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| {
            PersistenceError::new(
                "REVIEW_REQUEST_INVALID",
                "The required review levels could not be stored.",
                false,
            )
        })?;
        connection
            .execute(
                "INSERT INTO review_flows
                 (task_owner_agent_id, task_id, executor_agent_id, pipeline_version,
                  state, revision_round, max_revisions, required_levels_json,
                  current_level, latest_execution_attempt_id, review_mode,
                  last_error_code, last_error_message, created_at_unix_ms,
                  updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, 'awaiting_execution', 0, ?5, ?6, NULL,
                         ?7, ?8, NULL, NULL, ?9, ?9)",
                params![
                    attempt.task_owner_agent_id,
                    attempt.task_id,
                    attempt.agent_id,
                    REVIEW_PIPELINE_VERSION,
                    MAX_REVISION_ROUNDS,
                    required_levels_json,
                    attempt.id,
                    configured_review_mode,
                    timestamp
                ],
            )
            .map_err(PersistenceError::database)?;
        (connection.last_insert_rowid(), required_levels)
    };
    let next_level = required_levels.first().copied();
    let awaiting_human = next_level.is_none();
    connection
        .execute(
            "UPDATE review_flows
             SET state = ?1, current_level = ?2, latest_execution_attempt_id = ?3,
                 last_error_code = ?4, last_error_message = ?5,
                 updated_at_unix_ms = ?6
             WHERE id = ?7",
            params![
                if awaiting_human {
                    "awaiting_human"
                } else {
                    "awaiting_review"
                },
                next_level.map(ReviewLevel::as_storage),
                attempt.id,
                if awaiting_human {
                    Some("HUMAN_REVIEW_REQUIRED")
                } else {
                    None
                },
                if awaiting_human {
                    Some("The executor is the Supervisor, so a trusted human decision is required.")
                } else {
                    None
                },
                timestamp,
                flow_id
            ],
        )
        .map_err(PersistenceError::database)?;
    advance_review_revision(connection)?;
    Ok(ReviewAfterExecution {
        required: true,
        next_level,
    })
}

fn project_execution_failure_to_review_flow(
    connection: &Connection,
    attempt: &RunAttemptProjection,
    status: RunAttemptStatus,
    recovery_disposition: Option<&str>,
    timestamp: i64,
) -> PersistenceResult<()> {
    let flow_id: Option<i64> = connection
        .query_row(
            "SELECT id FROM review_flows
             WHERE task_owner_agent_id = ?1 AND task_id = ?2
               AND state IN ('revision_queued', 'awaiting_execution')",
            params![attempt.task_owner_agent_id, attempt.task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    let Some(flow_id) = flow_id else {
        return Ok(());
    };
    let safe_to_retry = status == RunAttemptStatus::StartupFailed
        || (matches!(
            status,
            RunAttemptStatus::Cancelled | RunAttemptStatus::Interrupted
        ) && recovery_disposition == Some("safe_to_retry"));
    if safe_to_retry {
        connection
            .execute(
                "UPDATE review_flows
                 SET state = 'revision_queued',
                     last_error_code = 'REVISION_EXECUTION_RETRYABLE',
                     last_error_message = 'The revision execution stopped before dispatch and is safe to retry.',
                     updated_at_unix_ms = ?1
                 WHERE id = ?2",
                params![timestamp, flow_id],
            )
            .map_err(PersistenceError::database)?;
        connection
            .execute(
                "UPDATE agent_tasks
                 SET status = 'Pending', phase = 'Assigned', completed_at = NULL,
                     queue_state = 'queued'
                 WHERE owner_agent_id = ?1 AND id = ?2",
                params![attempt.task_owner_agent_id, attempt.task_id],
            )
            .map_err(PersistenceError::database)?;
    } else {
        let enqueue_sequence: Option<i64> = connection
            .query_row(
                "SELECT enqueue_sequence FROM agent_tasks
                 WHERE owner_agent_id = ?1 AND id = ?2",
                params![attempt.task_owner_agent_id, attempt.task_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let enqueue_sequence = match enqueue_sequence {
            Some(sequence) => sequence,
            None => allocate_enqueue_sequence(connection)?,
        };
        connection
            .execute(
                "UPDATE review_flows
                 SET state = 'awaiting_human',
                     last_error_code = 'REVISION_EXECUTION_UNCERTAIN',
                     last_error_message = 'The revision execution may have changed the workspace and requires human inspection.',
                     updated_at_unix_ms = ?1
                 WHERE id = ?2",
                params![timestamp, flow_id],
            )
            .map_err(PersistenceError::database)?;
        connection
            .execute(
                "UPDATE agent_tasks
                 SET status = 'Blocked', phase = 'Supervisor Approval', completed_at = NULL,
                     queue_state = 'held', enqueue_sequence = ?1,
                     review_status = 'Failed',
                     review_result = 'Revision execution outcome is uncertain; inspect the workspace before continuing.'
                 WHERE owner_agent_id = ?2 AND id = ?3",
                params![enqueue_sequence, attempt.task_owner_agent_id, attempt.task_id],
            )
            .map_err(PersistenceError::database)?;
    }
    advance_review_revision(connection)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn project_run_completion_to_task(
    transaction: &Transaction<'_>,
    attempt: &RunAttemptProjection,
    status: RunAttemptStatus,
    summary: Option<&str>,
    response_id: Option<&str>,
    runtime_model: Option<&str>,
    total_tokens: Option<u64>,
    changed_files: &[String],
    diff: Option<&str>,
    workspace_evidence_json: &str,
    duration_seconds: u64,
    timestamp: i64,
    recovery_disposition: Option<&str>,
) -> PersistenceResult<()> {
    let completed_at = format_unix_ms(timestamp);
    if attempt.run_mode == RunAttemptMode::Review {
        return project_review_run_completion(
            transaction,
            attempt,
            status,
            summary,
            runtime_model,
            duration_seconds,
            timestamp,
            recovery_disposition,
        );
    }
    transaction
        .execute(
            "UPDATE agent_tasks
             SET workspace_evidence_json = ?1, diff = ?2
             WHERE owner_agent_id = ?3 AND id = ?4",
            params![
                workspace_evidence_json,
                diff,
                attempt.task_owner_agent_id,
                attempt.task_id
            ],
        )
        .map_err(PersistenceError::database)?;
    match status {
        RunAttemptStatus::Succeeded => {
            let review_mode: String = transaction
                .query_row(
                    "SELECT review_mode FROM preferences WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(PersistenceError::database)?;
            let review = project_execution_success_to_review_flow(
                transaction,
                attempt,
                &review_mode,
                timestamp,
            )?;
            let awaiting_human = review.required && review.next_level.is_none();
            transaction
                    .execute(
                        "UPDATE agent_tasks
                         SET status = ?1, phase = ?2, completed_at = ?3, result = ?4,
                             response_id = ?5, runtime_model = ?6, total_tokens = ?7,
                             diff = ?8, duration_seconds = ?9, review_agent_id = NULL,
                             review_status = ?10, review_result = ?11,
                             review_model = NULL, review_duration_seconds = NULL,
                             reviewed_at = NULL
                         WHERE owner_agent_id = ?12 AND id = ?13",
                        params![
                            if review.required {
                                "Under Review"
                            } else {
                                "Completed"
                            },
                            if let Some(level) = review.next_level {
                                level.task_phase()
                            } else if awaiting_human {
                                "Supervisor Approval"
                            } else {
                                "Finished"
                            },
                            if review.required {
                                None::<String>
                            } else {
                                Some(completed_at.clone())
                            },
                            summary,
                            response_id,
                            runtime_model,
                            optional_bounded_i64(total_tokens),
                            diff,
                            duration_seconds as f64,
                            if review.required {
                                "Pending"
                            } else {
                                "Not Requested"
                            },
                            if awaiting_human {
                                Some("A trusted human decision is required for Supervisor-executed work.")
                            } else {
                                None
                            },
                            attempt.task_owner_agent_id,
                            attempt.task_id
                        ],
                    )
                    .map_err(PersistenceError::database)?;
        }
        RunAttemptStatus::Cancelled | RunAttemptStatus::StartupFailed => {
            transaction
                .execute(
                    "UPDATE agent_tasks
                         SET status = 'Pending', phase = 'Assigned', completed_at = NULL
                         WHERE owner_agent_id = ?1 AND id = ?2",
                    params![attempt.task_owner_agent_id, attempt.task_id],
                )
                .map_err(PersistenceError::database)?;
        }
        RunAttemptStatus::Interrupted => {
            let safe = recovery_disposition == Some("safe_to_retry");
            transaction
                .execute(
                    "UPDATE agent_tasks SET status = ?1, phase = ?2, completed_at = NULL
                         WHERE owner_agent_id = ?3 AND id = ?4",
                    params![
                        if safe { "Pending" } else { "Blocked" },
                        if safe {
                            "Assigned"
                        } else {
                            "Supervisor Approval"
                        },
                        attempt.task_owner_agent_id,
                        attempt.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
        }
        RunAttemptStatus::TimedOut | RunAttemptStatus::Failed => {
            transaction
                .execute(
                    "UPDATE agent_tasks
                         SET status = 'Failed', phase = 'Failed', completed_at = ?1
                         WHERE owner_agent_id = ?2 AND id = ?3",
                    params![completed_at, attempt.task_owner_agent_id, attempt.task_id],
                )
                .map_err(PersistenceError::database)?;
        }
        _ => {
            return Err(PersistenceError::new(
                "INVALID_RUN_COMPLETION",
                "The execution completion status is not terminal.",
                false,
            ));
        }
    }
    transaction
        .execute(
            "DELETE FROM task_changed_files
             WHERE owner_agent_id = ?1 AND task_id = ?2",
            params![attempt.task_owner_agent_id, attempt.task_id],
        )
        .map_err(PersistenceError::database)?;
    for (position, path) in changed_files.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO task_changed_files
                 (owner_agent_id, task_id, position, path)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    attempt.task_owner_agent_id,
                    attempt.task_id,
                    position as i64,
                    path
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn project_review_run_completion(
    connection: &Connection,
    attempt: &RunAttemptProjection,
    status: RunAttemptStatus,
    summary: Option<&str>,
    runtime_model: Option<&str>,
    duration_seconds: u64,
    timestamp: i64,
    recovery_disposition: Option<&str>,
) -> PersistenceResult<()> {
    let flow_id = attempt.review_flow_id.ok_or_else(|| {
        PersistenceError::new(
            "REVIEW_BINDING_MISSING",
            "The completed review run has no bound flow.",
            false,
        )
    })?;
    let stage_id = attempt.review_stage_attempt_id.ok_or_else(|| {
        PersistenceError::new(
            "REVIEW_BINDING_MISSING",
            "The completed review run has no bound stage.",
            false,
        )
    })?;
    let flow =
        read_active_review_flow_binding(connection, attempt.task_owner_agent_id, attempt.task_id)?;
    if flow.id != flow_id
        || attempt.review_revision_round != Some(flow.revision_round)
        || !matches!(flow.state.as_str(), "review_pending" | "reviewing")
    {
        return Err(PersistenceError::new(
            "REVIEW_FLOW_STATE_CONFLICT",
            "The completed review run no longer matches the active review flow.",
            false,
        ));
    }
    let (request_json, stage_level, stage_state, attempt_number): (String, String, String, i64) =
        connection
            .query_row(
                "SELECT request_json, level, state, attempt_number
                 FROM review_stage_attempts
                 WHERE id = ?1 AND flow_id = ?2 AND run_attempt_id = ?3
                   AND reviewer_agent_id = ?4",
                params![stage_id, flow_id, attempt.id, attempt.agent_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                    "REVIEW_STAGE_STATE_CONFLICT",
                    "The completed review run no longer matches its stage attempt.",
                    false,
                ),
                other => PersistenceError::database(other),
            })?;
    if !matches!(stage_state.as_str(), "admitted" | "running") {
        return Err(PersistenceError::new(
            "REVIEW_STAGE_STATE_CONFLICT",
            "The review stage is already terminal or has an invalid state.",
            false,
        ));
    }
    let level = ReviewLevel::from_storage(&stage_level).map_err(review_protocol_error)?;
    let review_request: ReviewRequestV1 = serde_json::from_str(&request_json).map_err(|_| {
        PersistenceError::new(
            "REVIEW_LEDGER_INVALID",
            "The stored review request is invalid.",
            false,
        )
    })?;
    if review_request.flow_id != flow_id
        || review_request.stage_attempt_id != stage_id
        || review_request.revision_round != flow.revision_round
        || review_request.level != level
        || review_request.request_fingerprint
            != read_review_request_fingerprint(connection, stage_id)?
    {
        return Err(PersistenceError::new(
            "REVIEW_LEDGER_INVALID",
            "The stored review request does not match its normalized stage binding.",
            false,
        ));
    }

    if status == RunAttemptStatus::Succeeded
        && !attempt.truncation.summary_truncated
        && !attempt.truncation.stdout_truncated
    {
        match parse_review_result(summary.unwrap_or_default(), &review_request) {
            Ok(result) => {
                let result_json = serde_json::to_string(&result).map_err(|_| {
                    PersistenceError::new(
                        "REVIEW_RESULT_INVALID",
                        "The structured review result could not be stored.",
                        false,
                    )
                })?;
                let changed = connection
                    .execute(
                        "UPDATE review_stage_attempts
                         SET state = ?1, result_json = ?2, verdict = ?3, feedback = ?4,
                             completed_at_unix_ms = ?5
                         WHERE id = ?6 AND state IN ('admitted', 'running')",
                        params![
                            result.verdict.as_storage(),
                            result_json,
                            result.verdict.as_storage(),
                            result.feedback,
                            timestamp,
                            stage_id
                        ],
                    )
                    .map_err(PersistenceError::database)?;
                if changed != 1 {
                    return Err(PersistenceError::new(
                        "REVIEW_STAGE_STATE_CONFLICT",
                        "The review stage changed before its verdict was recorded.",
                        true,
                    ));
                }
                return apply_review_verdict(
                    connection,
                    &flow,
                    level,
                    &result,
                    ReviewActor::Agent,
                    Some(attempt.agent_id),
                    runtime_model,
                    Some(duration_seconds),
                    timestamp,
                );
            }
            Err(error) => {
                return project_review_stage_failure(
                    connection,
                    &flow,
                    stage_id,
                    level,
                    attempt_number,
                    "invalid",
                    &error.code,
                    &error.message,
                    false,
                    timestamp,
                )
            }
        }
    }

    let uncertain = recovery_disposition != Some("safe_to_retry")
        && matches!(
            status,
            RunAttemptStatus::Cancelled | RunAttemptStatus::Interrupted
        );
    let (stage_terminal_state, code, message) = if status == RunAttemptStatus::Succeeded {
        (
            "invalid",
            "REVIEW_RESULT_TRUNCATED",
            "The review response was truncated and cannot support a verdict.",
        )
    } else {
        (
            match status {
                RunAttemptStatus::Cancelled => "cancelled",
                RunAttemptStatus::Interrupted => "interrupted",
                _ => "failed",
            },
            attempt.error_code.as_deref().unwrap_or("REVIEW_RUN_FAILED"),
            attempt
                .error_message
                .as_deref()
                .unwrap_or("The review run did not produce a valid structured result."),
        )
    };
    project_review_stage_failure(
        connection,
        &flow,
        stage_id,
        level,
        attempt_number,
        stage_terminal_state,
        code,
        message,
        uncertain,
        timestamp,
    )
}

fn read_review_request_fingerprint(
    connection: &Connection,
    stage_attempt_id: i64,
) -> PersistenceResult<String> {
    connection
        .query_row(
            "SELECT request_fingerprint FROM review_stage_attempts WHERE id = ?1",
            [stage_attempt_id],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)
}

#[allow(clippy::too_many_arguments)]
fn project_review_stage_failure(
    connection: &Connection,
    flow: &ActiveReviewFlowBinding,
    stage_id: i64,
    level: ReviewLevel,
    attempt_number: i64,
    terminal_state: &str,
    error_code: &str,
    error_message: &str,
    uncertain: bool,
    timestamp: i64,
) -> PersistenceResult<()> {
    let changed = connection
        .execute(
            "UPDATE review_stage_attempts
             SET state = ?1, error_code = ?2, error_message = ?3,
                 completed_at_unix_ms = ?4
             WHERE id = ?5 AND state IN ('admitted', 'running')",
            params![
                terminal_state,
                error_code,
                error_message,
                timestamp,
                stage_id
            ],
        )
        .map_err(PersistenceError::database)?;
    if changed != 1 {
        return Err(PersistenceError::new(
            "REVIEW_STAGE_STATE_CONFLICT",
            "The review stage changed before its failure was recorded.",
            true,
        ));
    }
    let awaiting_human = uncertain || attempt_number >= MAX_STAGE_ATTEMPTS;
    connection
        .execute(
            "UPDATE review_flows
             SET state = ?1, last_error_code = ?2, last_error_message = ?3,
                 updated_at_unix_ms = ?4
             WHERE id = ?5",
            params![
                if awaiting_human {
                    "awaiting_human"
                } else {
                    "awaiting_review"
                },
                error_code,
                error_message,
                timestamp,
                flow.id
            ],
        )
        .map_err(PersistenceError::database)?;
    connection
        .execute(
            "UPDATE agent_tasks
             SET status = ?1, phase = ?2, completed_at = NULL,
                 review_status = 'Failed', review_result = ?3
             WHERE owner_agent_id = ?4 AND id = ?5",
            params![
                "Under Review",
                if awaiting_human {
                    "Supervisor Approval"
                } else {
                    level.task_phase()
                },
                error_message,
                flow.task_owner_agent_id,
                flow.task_id
            ],
        )
        .map_err(PersistenceError::database)?;
    advance_review_revision(connection)?;
    advance_task_orchestration_revision(connection)?;
    Ok(())
}

fn project_run_completion_to_queue(
    transaction: &Transaction<'_>,
    attempt: &RunAttemptProjection,
    status: RunAttemptStatus,
    recovery_disposition: Option<&str>,
) -> PersistenceResult<()> {
    if attempt.run_mode != RunAttemptMode::Execute {
        return Ok(());
    }
    let disposition = match status {
        RunAttemptStatus::StartupFailed => "queued",
        RunAttemptStatus::Cancelled if recovery_disposition == Some("safe_to_retry") => "queued",
        RunAttemptStatus::Cancelled => "held",
        RunAttemptStatus::Interrupted if recovery_disposition == Some("safe_to_retry") => "queued",
        RunAttemptStatus::Interrupted => "held",
        RunAttemptStatus::Succeeded | RunAttemptStatus::TimedOut | RunAttemptStatus::Failed => {
            "notQueued"
        }
        _ => {
            return Err(PersistenceError::new(
                "INVALID_RUN_COMPLETION",
                "The execution queue cannot project a nonterminal completion.",
                false,
            ))
        }
    };
    let changed = match disposition {
        "queued" => transaction.execute(
            "UPDATE agent_tasks
             SET queue_state = 'queued', status = 'Pending', phase = 'Assigned',
                 completed_at = NULL
             WHERE owner_agent_id = ?1 AND id = ?2",
            params![attempt.task_owner_agent_id, attempt.task_id],
        ),
        "held" => transaction.execute(
            "UPDATE agent_tasks
             SET queue_state = 'held', status = 'Blocked',
                 phase = 'Supervisor Approval', completed_at = NULL
             WHERE owner_agent_id = ?1 AND id = ?2",
            params![attempt.task_owner_agent_id, attempt.task_id],
        ),
        "notQueued" => transaction.execute(
            "UPDATE agent_tasks
             SET queue_state = 'notQueued', enqueue_sequence = NULL
             WHERE owner_agent_id = ?1 AND id = ?2",
            params![attempt.task_owner_agent_id, attempt.task_id],
        ),
        _ => unreachable!(),
    }
    .map_err(PersistenceError::database)?;
    if changed != 1 {
        return Err(PersistenceError::new(
            "TASK_STATE_CONFLICT",
            "The execution task disappeared before its queue completion was recorded.",
            true,
        ));
    }
    Ok(())
}

fn project_recovered_attempt_to_task(
    transaction: &Transaction<'_>,
    attempt: &RunAttemptProjection,
    safe_to_retry: bool,
) -> PersistenceResult<()> {
    match attempt.run_mode {
        RunAttemptMode::Execute => {
            transaction
                .execute(
                    "UPDATE agent_tasks
                     SET status = ?1, phase = ?2, completed_at = NULL, queue_state = ?3
                     WHERE owner_agent_id = ?4 AND id = ?5",
                    params![
                        if safe_to_retry { "Pending" } else { "Blocked" },
                        if safe_to_retry {
                            "Assigned"
                        } else {
                            "Supervisor Approval"
                        },
                        if safe_to_retry { "queued" } else { "held" },
                        attempt.task_owner_agent_id,
                        attempt.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
        }
        RunAttemptMode::Review => {
            transaction
                .execute(
                    "UPDATE agent_tasks
                     SET status = 'Under Review', phase = 'Senior Review',
                         review_status = ?1
                     WHERE owner_agent_id = ?2 AND id = ?3",
                    params![
                        if safe_to_retry { "Pending" } else { "Failed" },
                        attempt.task_owner_agent_id,
                        attempt.task_id
                    ],
                )
                .map_err(PersistenceError::database)?;
        }
    }
    Ok(())
}

fn run_payload_bytes(
    text_parts: [Option<&str>; 7],
    changed_files_json: &str,
    workspace_evidence_json: &str,
) -> PersistenceResult<i64> {
    let total = text_parts
        .into_iter()
        .flatten()
        .try_fold(
            changed_files_json
                .len()
                .saturating_add(workspace_evidence_json.len()),
            |total, value| total.checked_add(value.len()),
        )
        .ok_or_else(|| {
            PersistenceError::new(
                "RUN_OUTPUT_TOO_LARGE",
                "Run output size could not be represented safely.",
                false,
            )
        })?;
    i64::try_from(total).map_err(|_| {
        PersistenceError::new(
            "RUN_OUTPUT_TOO_LARGE",
            "Run output size exceeds the supported range.",
            false,
        )
    })
}

fn refresh_run_retention_meta(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    let (count, payload): (i64, i64) = transaction
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) FROM run_attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "UPDATE run_coordinator_meta
             SET retained_attempt_count = ?1, retained_payload_bytes = ?2
             WHERE singleton = 1",
            params![count, payload],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn prune_run_history(transaction: &Transaction<'_>, timestamp: i64) -> PersistenceResult<()> {
    let mut pruned = 0_i64;
    loop {
        let (count, payload): (i64, i64) = transaction
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) FROM run_attempts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        if count <= MAX_RETAINED_ATTEMPTS && payload <= MAX_RETAINED_PAYLOAD_BYTES {
            break;
        }
        let oldest_terminal: Option<i64> = transaction
            .query_row(
                "SELECT id FROM run_attempts
                 WHERE status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                  'failed', 'interrupted')
                   AND id NOT IN (
                       SELECT latest_execution_attempt_id FROM review_flows
                       WHERE state NOT IN ('completed', 'failed', 'cancelled')
                         AND latest_execution_attempt_id IS NOT NULL
                   )
                   AND id NOT IN (
                       SELECT stage.run_attempt_id
                       FROM review_stage_attempts AS stage
                       JOIN review_flows AS flow ON flow.id = stage.flow_id
                       WHERE flow.state NOT IN ('completed', 'failed', 'cancelled')
                         AND stage.run_attempt_id IS NOT NULL
                   )
                 ORDER BY completed_at_unix_ms, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(PersistenceError::database)?;
        let Some(attempt_id) = oldest_terminal else {
            return Err(PersistenceError::new(
                "RUN_HISTORY_LIMIT",
                "No unreferenced terminal run history can be pruned within the durable ledger bound.",
                false,
            ));
        };
        transaction
            .execute("DELETE FROM run_attempts WHERE id = ?1", [attempt_id])
            .map_err(PersistenceError::database)?;
        pruned = pruned.saturating_add(1);
    }
    if pruned > 0 {
        transaction
            .execute(
                "UPDATE run_coordinator_meta
                 SET pruned_attempt_count = pruned_attempt_count + ?1,
                     last_pruned_at_unix_ms = ?2
                 WHERE singleton = 1",
                params![pruned, timestamp],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

fn ensure_run_mutation_idle(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    let busy: bool = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM run_coordinator_meta WHERE active_attempt_id IS NOT NULL
                 UNION ALL SELECT 1 FROM run_approval_reservations
                 UNION ALL SELECT 1 FROM run_attempts
                           WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                            'cancel_requested')
             )",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if busy {
        return Err(PersistenceError::new(
            "RUN_ACTIVE",
            "Application state cannot be replaced while an AI run is active or reserved.",
            true,
        ));
    }
    Ok(())
}

fn clear_run_coordination(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    transaction
        .execute_batch(
            "DELETE FROM run_approval_reservations;
             DELETE FROM run_events;
             DELETE FROM run_attempts;
             DELETE FROM review_stage_attempts;
             DELETE FROM review_flows;
             UPDATE review_orchestration_meta SET revision = 0 WHERE singleton = 1;
             UPDATE run_coordinator_meta
             SET revision = 0, active_attempt_id = NULL, retained_attempt_count = 0,
                 retained_payload_bytes = 0, pruned_attempt_count = 0,
                 last_pruned_at_unix_ms = NULL
             WHERE singleton = 1;",
        )
        .map_err(PersistenceError::database)
}

fn retention_cutoff(
    transaction: &Transaction<'_>,
    column: &str,
    timestamp: i64,
) -> PersistenceResult<Option<i64>> {
    let query = match column {
        "task_retention" => "SELECT task_retention FROM retention_settings WHERE singleton = 1",
        "activity_retention" => {
            "SELECT activity_retention FROM retention_settings WHERE singleton = 1"
        }
        _ => {
            return Err(PersistenceError::new(
                "RETENTION_CONFIGURATION_INVALID",
                "Data lifecycle maintenance requested an unsupported retention domain.",
                false,
            ));
        }
    };
    let value: String = transaction
        .query_row(query, [], |row| row.get(0))
        .map_err(PersistenceError::database)?;
    let days = match HistoryRetentionDays::from_storage_value(&value)
        .map_err(PersistenceError::validation)?
    {
        HistoryRetentionDays::Days7 => Some(7_i64),
        HistoryRetentionDays::Days30 => Some(30_i64),
        HistoryRetentionDays::Days90 => Some(90_i64),
        HistoryRetentionDays::Never => None,
    };
    Ok(days.map(|days| {
        days.checked_mul(86_400_000)
            .and_then(|window| timestamp.checked_sub(window))
            .unwrap_or(0)
    }))
}

fn count_retention_protected(
    transaction: &Transaction<'_>,
    task_cutoff: Option<i64>,
    activity_cutoff: Option<i64>,
) -> PersistenceResult<i64> {
    let mut skipped = 0_i64;
    if let Some(cutoff) = task_cutoff {
        let old_tasks: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM agent_tasks
                 WHERE status IN ('Completed', 'Failed')
                   AND completed_at_unix_ms IS NOT NULL
                   AND completed_at_unix_ms < ?1",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let eligible_tasks: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM agent_tasks AS task
                 WHERE task.status IN ('Completed', 'Failed')
                   AND task.completed_at_unix_ms IS NOT NULL
                   AND task.completed_at_unix_ms < ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM run_attempts AS attempt
                       WHERE attempt.task_owner_agent_id = task.owner_agent_id
                         AND attempt.task_id = task.id
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM review_flows AS flow
                       WHERE flow.task_owner_agent_id = task.owner_agent_id
                         AND flow.task_id = task.id
                   )",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        skipped = skipped.saturating_add(old_tasks.saturating_sub(eligible_tasks));

        let old_attempts: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM run_attempts
                 WHERE status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                  'failed', 'interrupted')
                   AND completed_at_unix_ms IS NOT NULL
                   AND completed_at_unix_ms < ?1",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let eligible_attempts = count_eligible_attempts(transaction, cutoff)?;
        skipped = skipped.saturating_add(old_attempts.saturating_sub(eligible_attempts));
    }
    if let Some(cutoff) = activity_cutoff {
        let old_approvals: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM approval_requests
                 WHERE COALESCE(consumed_at_unix_ms, resolved_at_unix_ms,
                                expires_at_unix_ms, created_at_unix_ms) < ?1",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let eligible_approvals: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM approval_requests AS approval
                 WHERE (approval.status IN ('Denied', 'Expired')
                        OR approval.consumed_at_unix_ms IS NOT NULL)
                   AND COALESCE(approval.consumed_at_unix_ms, approval.resolved_at_unix_ms,
                                approval.expires_at_unix_ms, approval.created_at_unix_ms) < ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM run_approval_reservations AS reservation
                       WHERE reservation.approval_id = approval.id
                   )",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        skipped = skipped.saturating_add(old_approvals.saturating_sub(eligible_approvals));
    }
    Ok(skipped)
}

fn count_eligible_attempts(transaction: &Transaction<'_>, cutoff: i64) -> PersistenceResult<i64> {
    transaction
        .query_row(
            "SELECT COUNT(*) FROM run_attempts AS attempt
             WHERE attempt.status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                      'failed', 'interrupted')
               AND attempt.completed_at_unix_ms IS NOT NULL
               AND attempt.completed_at_unix_ms < ?1
               AND NOT EXISTS (
                   SELECT 1 FROM run_coordinator_meta AS meta
                   WHERE meta.active_attempt_id = attempt.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM run_approval_reservations AS reservation
                   WHERE reservation.attempt_id = attempt.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM review_flows AS flow
                   WHERE flow.latest_execution_attempt_id = attempt.id
                      OR flow.id = attempt.review_flow_id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM review_stage_attempts AS stage
                   WHERE stage.run_attempt_id = attempt.id
               )",
            [cutoff],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)
}

fn prune_retention_rows(
    transaction: &Transaction<'_>,
    task_cutoff: Option<i64>,
    activity_cutoff: Option<i64>,
    timestamp: i64,
) -> PersistenceResult<RetentionPruneCounts> {
    let mut counts = RetentionPruneCounts::default();
    let expiring_memory_ids = {
        let mut statement = transaction
            .prepare(
                "SELECT memory.id, memory.revision FROM memory_records AS memory
                 WHERE (memory.expires_at_unix_ms IS NOT NULL
                        AND memory.expires_at_unix_ms <= ?1)
                    OR (memory.retention_policy = 'task_lifetime'
                        AND (
                            NOT EXISTS (
                                SELECT 1 FROM agent_tasks AS task
                                WHERE task.owner_agent_id = memory.task_owner_agent_id
                                  AND task.id = memory.task_id
                            )
                            OR EXISTS (
                                SELECT 1 FROM agent_tasks AS task
                                WHERE task.owner_agent_id = memory.task_owner_agent_id
                                  AND task.id = memory.task_id
                                  AND task.status IN ('Completed', 'Failed')
                            )
                        ))
                 ORDER BY COALESCE(memory.expires_at_unix_ms, memory.updated_at_unix_ms),
                          memory.id
                 LIMIT ?2",
            )
            .map_err(PersistenceError::database)?;
        collect_rows(
            statement.query_map(params![timestamp, MAX_MAINTENANCE_ROWS_PER_DOMAIN], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            }),
        )?
    };
    if !expiring_memory_ids.is_empty() {
        let (memory_revision, mut next_event_id): (i64, i64) = transaction
            .query_row(
                "SELECT revision, next_event_id FROM structured_memory_meta
                 WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(PersistenceError::database)?;
        for (record_id, record_revision) in &expiring_memory_ids {
            transaction
                .execute(
                    "INSERT INTO memory_events
                     (id, record_id, action, actor_kind, record_revision,
                      created_at_unix_ms)
                     VALUES (?1, ?2, 'retention_deleted', 'maintenance', ?3, ?4)",
                    params![next_event_id, record_id, record_revision, timestamp],
                )
                .map_err(PersistenceError::database)?;
            transaction
                .execute("DELETE FROM memory_records WHERE id = ?1", [record_id])
                .map_err(PersistenceError::database)?;
            next_event_id = next_revision(next_event_id)?;
        }
        counts.memory_records = i64::try_from(expiring_memory_ids.len()).map_err(|_| {
            PersistenceError::new(
                "MEMORY_CAPACITY_EXCEEDED",
                "The memory retention count is outside the supported range.",
                false,
            )
        })?;
        transaction
            .execute(
                "UPDATE structured_memory_meta
                 SET revision = ?1, next_event_id = ?2 WHERE singleton = 1",
                params![next_revision(memory_revision)?, next_event_id],
            )
            .map_err(PersistenceError::database)?;
    }
    if let Some(cutoff) = task_cutoff {
        counts.review_flows = transaction
            .execute(
                "DELETE FROM review_flows WHERE id IN (
                     SELECT flow.id FROM review_flows AS flow
                     WHERE flow.state IN ('completed', 'failed', 'cancelled')
                       AND flow.completed_at_unix_ms IS NOT NULL
                       AND flow.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM review_stage_attempts AS stage
                           WHERE stage.flow_id = flow.id
                             AND stage.state IN ('pending', 'admitted', 'running')
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM run_attempts AS attempt
                           WHERE attempt.review_flow_id = flow.id
                             AND attempt.status NOT IN (
                                 'succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                 'failed', 'interrupted'
                             )
                       )
                     ORDER BY flow.completed_at_unix_ms, flow.id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.attempts = transaction
            .execute(
                "DELETE FROM run_attempts WHERE id IN (
                     SELECT attempt.id FROM run_attempts AS attempt
                     WHERE attempt.status IN (
                               'succeeded', 'cancelled', 'timed_out', 'startup_failed',
                               'failed', 'interrupted'
                           )
                       AND attempt.completed_at_unix_ms IS NOT NULL
                       AND attempt.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_coordinator_meta AS meta
                           WHERE meta.active_attempt_id = attempt.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM run_approval_reservations AS reservation
                           WHERE reservation.attempt_id = attempt.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_flows AS flow
                           WHERE flow.latest_execution_attempt_id = attempt.id
                              OR flow.id = attempt.review_flow_id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_stage_attempts AS stage
                           WHERE stage.run_attempt_id = attempt.id
                       )
                     ORDER BY attempt.completed_at_unix_ms, attempt.id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.tasks = transaction
            .execute(
                "DELETE FROM agent_tasks WHERE (owner_agent_id, id) IN (
                     SELECT task.owner_agent_id, task.id FROM agent_tasks AS task
                     WHERE task.status IN ('Completed', 'Failed')
                       AND task.completed_at_unix_ms IS NOT NULL
                       AND task.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_attempts AS attempt
                           WHERE attempt.task_owner_agent_id = task.owner_agent_id
                             AND attempt.task_id = task.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_flows AS flow
                           WHERE flow.task_owner_agent_id = task.owner_agent_id
                             AND flow.task_id = task.id
                       )
                     ORDER BY task.completed_at_unix_ms, task.owner_agent_id, task.id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.management_handoffs = transaction
            .execute(
                "DELETE FROM management_handoffs WHERE id IN (
                     SELECT handoff.id FROM management_handoffs AS handoff
                     WHERE NOT EXISTS (
                         SELECT 1 FROM agent_tasks AS task
                         WHERE task.owner_agent_id = handoff.task_owner_agent_id
                           AND task.id = handoff.task_id
                     )
                     ORDER BY handoff.created_at_unix_ms DESC, handoff.id DESC
                     LIMIT ?1
                 )",
                [MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        if counts.management_handoffs > 0 {
            transaction
                .execute(
                    "UPDATE management_handoff_meta SET revision = revision + 1
                     WHERE singleton = 1",
                    [],
                )
                .map_err(PersistenceError::database)?;
        }
    }
    if let Some(cutoff) = activity_cutoff {
        counts.reminder_occurrences = transaction
            .execute(
                "DELETE FROM reminder_occurrences WHERE id IN (
                     SELECT occurrence.id FROM reminder_occurrences AS occurrence
                     JOIN reminders AS reminder ON reminder.id = occurrence.reminder_id
                     WHERE reminder.status IN ('completed', 'dismissed')
                       AND occurrence.updated_at_unix_ms < ?1
                     ORDER BY occurrence.updated_at_unix_ms, occurrence.id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.activity = transaction
            .execute(
                "DELETE FROM agent_activity WHERE (owner_agent_id, id) IN (
                     SELECT owner_agent_id, id FROM agent_activity
                     WHERE created_at_unix_ms IS NOT NULL AND created_at_unix_ms < ?1
                     ORDER BY created_at_unix_ms, owner_agent_id, id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.approvals = transaction
            .execute(
                "DELETE FROM approval_requests WHERE id IN (
                     SELECT approval.id FROM approval_requests AS approval
                     WHERE (approval.status IN ('Denied', 'Expired')
                            OR approval.consumed_at_unix_ms IS NOT NULL)
                       AND COALESCE(
                               approval.consumed_at_unix_ms,
                               approval.resolved_at_unix_ms,
                               approval.expires_at_unix_ms,
                               approval.created_at_unix_ms
                           ) < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_approval_reservations AS reservation
                           WHERE reservation.approval_id = approval.id
                       )
                     ORDER BY COALESCE(
                                  approval.consumed_at_unix_ms,
                                  approval.resolved_at_unix_ms,
                                  approval.expires_at_unix_ms,
                                  approval.created_at_unix_ms
                              ), approval.id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        counts.reminders = transaction
            .execute(
                "DELETE FROM reminders WHERE id IN (
                     SELECT id FROM reminders
                     WHERE status IN ('completed', 'dismissed')
                       AND resolved_at_unix_ms IS NOT NULL
                       AND resolved_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM reminder_occurrences AS occurrence
                           WHERE occurrence.reminder_id = reminders.id
                       )
                     ORDER BY resolved_at_unix_ms, id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
        if counts.reminders > 0 || counts.reminder_occurrences > 0 {
            transaction
                .execute(
                    "UPDATE reminder_scheduler_meta SET revision = revision + 1
                     WHERE singleton = 1",
                    [],
                )
                .map_err(PersistenceError::database)?;
        }
        counts.system_action_audits = transaction
            .execute(
                "DELETE FROM system_action_audits WHERE id IN (
                     SELECT id FROM system_action_audits
                     WHERE status IN ('taskCreated', 'applied', 'rejected', 'failed', 'uncertain')
                       AND updated_at_unix_ms < ?1
                     ORDER BY updated_at_unix_ms, id
                     LIMIT ?2
                 )",
                params![cutoff, MAX_MAINTENANCE_ROWS_PER_DOMAIN],
            )
            .map_err(PersistenceError::database)? as i64;
    }
    Ok(counts)
}

fn retention_backlog_exists(
    transaction: &Transaction<'_>,
    task_cutoff: Option<i64>,
    activity_cutoff: Option<i64>,
    timestamp: i64,
) -> PersistenceResult<bool> {
    let memory_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM memory_records AS memory
                 WHERE (memory.expires_at_unix_ms IS NOT NULL
                        AND memory.expires_at_unix_ms <= ?1)
                    OR (memory.retention_policy = 'task_lifetime'
                        AND (
                            NOT EXISTS (
                                SELECT 1 FROM agent_tasks AS task
                                WHERE task.owner_agent_id = memory.task_owner_agent_id
                                  AND task.id = memory.task_id
                            )
                            OR EXISTS (
                                SELECT 1 FROM agent_tasks AS task
                                WHERE task.owner_agent_id = memory.task_owner_agent_id
                                  AND task.id = memory.task_id
                                  AND task.status IN ('Completed', 'Failed')
                            )
                        ))
             )",
            [timestamp],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if memory_exists {
        return Ok(true);
    }
    if let Some(cutoff) = task_cutoff {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM review_flows AS flow
                     WHERE flow.state IN ('completed', 'failed', 'cancelled')
                       AND flow.completed_at_unix_ms IS NOT NULL
                       AND flow.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM review_stage_attempts AS stage
                           WHERE stage.flow_id = flow.id
                             AND stage.state IN ('pending', 'admitted', 'running')
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM run_attempts AS attempt
                           WHERE attempt.review_flow_id = flow.id
                             AND attempt.status NOT IN (
                                 'succeeded', 'cancelled', 'timed_out', 'startup_failed',
                                 'failed', 'interrupted'
                             )
                       )
                     UNION ALL
                     SELECT 1 FROM run_attempts AS attempt
                     WHERE attempt.status IN (
                               'succeeded', 'cancelled', 'timed_out', 'startup_failed',
                               'failed', 'interrupted'
                           )
                       AND attempt.completed_at_unix_ms IS NOT NULL
                       AND attempt.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_coordinator_meta AS meta
                           WHERE meta.active_attempt_id = attempt.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_flows AS flow
                           WHERE flow.latest_execution_attempt_id = attempt.id
                              OR flow.id = attempt.review_flow_id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_stage_attempts AS stage
                           WHERE stage.run_attempt_id = attempt.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM run_approval_reservations AS reservation
                           WHERE reservation.attempt_id = attempt.id
                       )
                     UNION ALL
                     SELECT 1 FROM agent_tasks AS task
                     WHERE task.status IN ('Completed', 'Failed')
                       AND task.completed_at_unix_ms IS NOT NULL
                       AND task.completed_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_attempts AS attempt
                           WHERE attempt.task_owner_agent_id = task.owner_agent_id
                             AND attempt.task_id = task.id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM review_flows AS flow
                           WHERE flow.task_owner_agent_id = task.owner_agent_id
                             AND flow.task_id = task.id
                       )
                     UNION ALL
                     SELECT 1 FROM management_handoffs AS handoff
                     WHERE NOT EXISTS (
                         SELECT 1 FROM agent_tasks AS task
                         WHERE task.owner_agent_id = handoff.task_owner_agent_id
                           AND task.id = handoff.task_id
                     )
                 )",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if exists {
            return Ok(true);
        }
    }
    if let Some(cutoff) = activity_cutoff {
        return transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM agent_activity
                     WHERE created_at_unix_ms IS NOT NULL AND created_at_unix_ms < ?1
                     UNION ALL
                     SELECT 1 FROM approval_requests AS approval
                     WHERE (approval.status IN ('Denied', 'Expired')
                            OR approval.consumed_at_unix_ms IS NOT NULL)
                       AND COALESCE(
                               approval.consumed_at_unix_ms,
                               approval.resolved_at_unix_ms,
                               approval.expires_at_unix_ms,
                               approval.created_at_unix_ms
                           ) < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM run_approval_reservations AS reservation
                           WHERE reservation.approval_id = approval.id
                       )
                     UNION ALL
                     SELECT 1 FROM reminders
                     WHERE status IN ('completed', 'dismissed')
                       AND resolved_at_unix_ms IS NOT NULL
                       AND resolved_at_unix_ms < ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM reminder_occurrences AS occurrence
                           WHERE occurrence.reminder_id = reminders.id
                       )
                     UNION ALL
                     SELECT 1 FROM reminder_occurrences AS occurrence
                     JOIN reminders AS reminder ON reminder.id = occurrence.reminder_id
                     WHERE reminder.status IN ('completed', 'dismissed')
                       AND occurrence.updated_at_unix_ms < ?1
                     UNION ALL
                     SELECT 1 FROM system_action_audits
                     WHERE status IN ('taskCreated', 'applied', 'rejected', 'failed', 'uncertain')
                       AND updated_at_unix_ms < ?1
                 )",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database);
    }
    Ok(false)
}

fn update_run_retention_meta(
    transaction: &Transaction<'_>,
    pruned_attempts: i64,
    timestamp: i64,
) -> PersistenceResult<()> {
    let revision: i64 = transaction
        .query_row(
            "SELECT revision FROM run_coordinator_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let revision = next_revision(revision)?;
    transaction
        .execute(
            "UPDATE run_coordinator_meta
             SET revision = ?1,
                 retained_attempt_count = (SELECT COUNT(*) FROM run_attempts),
                 retained_payload_bytes = COALESCE((SELECT SUM(payload_bytes) FROM run_attempts), 0),
                 pruned_attempt_count = pruned_attempt_count + ?2,
                 last_pruned_at_unix_ms = ?3
             WHERE singleton = 1",
            params![revision, pruned_attempts, timestamp],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_data_lifecycle_run(
    transaction: &Transaction<'_>,
    lifecycle_revision: i64,
    application_state_revision: i64,
    trigger_kind: &str,
    status: &str,
    started_at_unix_ms: i64,
    completed_at_unix_ms: i64,
    task_cutoff_unix_ms: Option<i64>,
    activity_cutoff_unix_ms: Option<i64>,
    pruned: &RetentionPruneCounts,
    skipped_protected: i64,
    backlog_remaining: bool,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO data_lifecycle_runs
             (lifecycle_revision, application_state_revision, trigger_kind, status,
              started_at_unix_ms, completed_at_unix_ms,
              task_cutoff_unix_ms, activity_cutoff_unix_ms, pruned_tasks,
              pruned_attempts, pruned_review_flows, pruned_activity, pruned_approvals,
              pruned_reminders, pruned_system_action_audits, skipped_protected,
              backlog_remaining, error_code, error_message, pruned_memory_records,
              pruned_reminder_occurrences, pruned_management_handoffs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                lifecycle_revision,
                application_state_revision,
                trigger_kind,
                status,
                started_at_unix_ms,
                completed_at_unix_ms,
                task_cutoff_unix_ms,
                activity_cutoff_unix_ms,
                pruned.tasks,
                pruned.attempts,
                pruned.review_flows,
                pruned.activity,
                pruned.approvals,
                pruned.reminders,
                pruned.system_action_audits,
                skipped_protected,
                backlog_remaining as i64,
                error_code,
                error_message,
                pruned.memory_records,
                pruned.reminder_occurrences,
                pruned.management_handoffs,
            ],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn trim_data_lifecycle_runs(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    transaction
        .execute(
            "DELETE FROM data_lifecycle_runs WHERE id IN (
                 SELECT id FROM data_lifecycle_runs
                 ORDER BY id DESC LIMIT -1 OFFSET ?1
             )",
            [MAX_MAINTENANCE_EVIDENCE_ROWS],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn bounded_lifecycle_message(message: &str) -> String {
    message.chars().take(1024).collect()
}

fn monitoring_revision(connection: &Connection) -> PersistenceResult<MonitoringRevision> {
    connection
        .query_row(
            "SELECT
                 (SELECT state_revision FROM application_meta WHERE singleton = 1),
                 (SELECT revision FROM task_orchestration_meta WHERE singleton = 1),
                 (SELECT revision FROM run_coordinator_meta WHERE singleton = 1),
                 (SELECT revision FROM review_orchestration_meta WHERE singleton = 1),
                 (SELECT revision FROM data_lifecycle_meta WHERE singleton = 1)",
            [],
            |row| {
                Ok(MonitoringRevision {
                    application_state: row.get(0)?,
                    task_orchestration: row.get(1)?,
                    run_coordinator: row.get(2)?,
                    review_orchestration: row.get(3)?,
                    data_lifecycle: row.get(4)?,
                })
            },
        )
        .map_err(PersistenceError::database)
}

fn ensure_monitoring_revision(
    current: &MonitoringRevision,
    expected: &MonitoringRevision,
) -> PersistenceResult<()> {
    if current != expected {
        return Err(PersistenceError::new(
            "MONITORING_REVISION_CONFLICT",
            "Authoritative monitoring data changed before this query or mutation could complete. Refresh and try again.",
            true,
        ));
    }
    Ok(())
}

fn validate_monitoring_page(offset: i64, limit: i64) -> PersistenceResult<()> {
    if !(0..=MAX_SAFE_INTEGER).contains(&offset) || !(1..=MONITORING_PAGE_LIMIT).contains(&limit) {
        return Err(PersistenceError::new(
            "MONITORING_PAGE_INVALID",
            format!(
                "Monitoring pages require a non-negative offset and a limit from 1 through {MONITORING_PAGE_LIMIT}."
            ),
            true,
        ));
    }
    Ok(())
}

fn monitoring_queue_rank(task: &AgentTask) -> i64 {
    match task.queue_state.as_str() {
        "running" => 0,
        "admitted" => 1,
        "queued" => 2,
        "held" => 3,
        _ => 4,
    }
}

fn read_monitoring_snapshot(
    connection: &Connection,
    timestamp: i64,
) -> PersistenceResult<MonitoringSnapshot> {
    let revision = monitoring_revision(connection)?;
    let counts = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM agents WHERE registry_state = 'active'),
                 (SELECT COUNT(*) FROM agents
                  WHERE registry_state = 'active' AND status = 'Working'),
                 (SELECT COUNT(*) FROM agent_tasks),
                 (SELECT COUNT(*) FROM agent_tasks WHERE status = 'Running'),
                 (SELECT COUNT(*) FROM agent_tasks WHERE status = 'Pending'),
                 (SELECT COUNT(*) FROM agent_tasks WHERE status = 'Blocked'),
                 (SELECT COUNT(*) FROM agent_tasks WHERE status = 'Completed'),
                 (SELECT COUNT(*) FROM agent_tasks WHERE status = 'Failed'),
                 (SELECT COUNT(*) FROM agent_activity),
                 (SELECT COUNT(*) FROM approval_requests
                  WHERE authoritative = 1 AND status = 'Pending'),
                 (SELECT COUNT(*) FROM reminders
                  WHERE status IN ('scheduled', 'due', 'needs_attention')),
                 (SELECT COUNT(*) FROM run_attempts),
                 (SELECT COUNT(*) FROM run_attempts
                  WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                   'cancel_requested'))",
            [],
            |row| {
                Ok(MonitoringCounts {
                    configured_agents: row.get(0)?,
                    active_agents: row.get(1)?,
                    total_tasks: row.get(2)?,
                    running_tasks: row.get(3)?,
                    pending_tasks: row.get(4)?,
                    blocked_tasks: row.get(5)?,
                    completed_tasks: row.get(6)?,
                    failed_tasks: row.get(7)?,
                    activity_entries: row.get(8)?,
                    pending_approvals: row.get(9)?,
                    upcoming_reminders: row.get(10)?,
                    retained_run_attempts: row.get(11)?,
                    active_run_attempts: row.get(12)?,
                })
            },
        )
        .map_err(PersistenceError::database)?;
    let retention = connection
        .query_row(
            "SELECT task_retention, activity_retention
             FROM retention_settings WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(PersistenceError::database)?
        .unwrap_or_else(|| ("30".to_string(), "30".to_string()));
    let lifecycle = connection
        .query_row(
            "SELECT last_observed_at_unix_ms, last_success_at_unix_ms,
                    last_error_code, last_error_message, total_runs,
                    total_pruned_tasks, total_pruned_attempts,
                    total_pruned_review_flows, total_pruned_activity,
                    total_pruned_approvals, total_pruned_reminders,
                    total_pruned_system_action_audits,
                    total_pruned_memory_records, total_pruned_reminder_occurrences,
                    total_pruned_management_handoffs,
                    inferred_timestamp_count
             FROM data_lifecycle_meta WHERE singleton = 1",
            [],
            |row| {
                Ok(DataLifecycleSummary {
                    task_retention: retention.0.clone(),
                    activity_retention: retention.1.clone(),
                    last_observed_at_unix_ms: row.get(0)?,
                    last_success_at_unix_ms: row.get(1)?,
                    last_error_code: row.get(2)?,
                    last_error_message: row.get(3)?,
                    total_runs: row.get(4)?,
                    total_pruned: RetentionPruneCounts {
                        tasks: row.get(5)?,
                        attempts: row.get(6)?,
                        review_flows: row.get(7)?,
                        activity: row.get(8)?,
                        approvals: row.get(9)?,
                        reminders: row.get(10)?,
                        system_action_audits: row.get(11)?,
                        memory_records: row.get(12)?,
                        reminder_occurrences: row.get(13)?,
                        management_handoffs: row.get(14)?,
                    },
                    inferred_timestamp_count: row.get(15)?,
                    latest_run: None,
                })
            },
        )
        .map_err(PersistenceError::database)?;
    let latest_run = connection
        .query_row(
            "SELECT lifecycle_revision, application_state_revision, trigger_kind, status,
                    started_at_unix_ms, completed_at_unix_ms, task_cutoff_unix_ms,
                    activity_cutoff_unix_ms, pruned_tasks, pruned_attempts,
                    pruned_review_flows, pruned_activity, pruned_approvals,
                    pruned_reminders, pruned_system_action_audits,
                    skipped_protected, backlog_remaining,
                    error_code, error_message, pruned_memory_records,
                    pruned_reminder_occurrences, pruned_management_handoffs
             FROM data_lifecycle_runs ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok(RetentionMaintenanceResult {
                    lifecycle_revision: row.get(0)?,
                    application_state_revision: row.get(1)?,
                    trigger_kind: row.get(2)?,
                    status: row.get(3)?,
                    started_at_unix_ms: row.get(4)?,
                    completed_at_unix_ms: row.get(5)?,
                    task_cutoff_unix_ms: row.get(6)?,
                    activity_cutoff_unix_ms: row.get(7)?,
                    pruned: RetentionPruneCounts {
                        tasks: row.get(8)?,
                        attempts: row.get(9)?,
                        review_flows: row.get(10)?,
                        activity: row.get(11)?,
                        approvals: row.get(12)?,
                        reminders: row.get(13)?,
                        system_action_audits: row.get(14)?,
                        memory_records: row.get(19)?,
                        reminder_occurrences: row.get(20)?,
                        management_handoffs: row.get(21)?,
                    },
                    skipped_protected: row.get(15)?,
                    backlog_remaining: row.get::<_, i64>(16)? != 0,
                    error_code: row.get(17)?,
                    error_message: row.get(18)?,
                })
            },
        )
        .optional()
        .map_err(PersistenceError::database)?;
    let mut lifecycle = lifecycle;
    lifecycle.latest_run = latest_run;
    Ok(MonitoringSnapshot {
        authoritative: true,
        generated_at_unix_ms: timestamp,
        revision,
        counts,
        lifecycle,
    })
}

fn advance_monitoring_activity_revision(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    let application_revision: i64 = transaction
        .query_row(
            "SELECT state_revision FROM application_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let lifecycle_revision: i64 = transaction
        .query_row(
            "SELECT revision FROM data_lifecycle_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
            [next_revision(application_revision)?],
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "UPDATE data_lifecycle_meta SET revision = ?1 WHERE singleton = 1",
            [next_revision(lifecycle_revision)?],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn protect_run_owned_state(
    transaction: &Transaction<'_>,
    current: &ApplicationState,
    requested: &ApplicationState,
    timestamp: i64,
) -> PersistenceResult<ApplicationState> {
    if current.reminders != requested.reminders {
        return Err(PersistenceError::new(
            "REMINDER_SCHEDULER_AUTHORITY_REQUIRED",
            "Create and edit reminders through the authoritative reminder scheduler commands.",
            true,
        ));
    }
    let current_memory = current
        .agents
        .iter()
        .map(|agent| (agent.id, agent.memory.as_str()))
        .collect::<HashMap<_, _>>();
    if requested.agents.iter().any(|agent| {
        current_memory
            .get(&agent.id)
            .is_some_and(|memory| **memory != agent.memory)
    }) {
        return Err(PersistenceError::new(
            "STRUCTURED_MEMORY_AUTHORITY_REQUIRED",
            "Create and edit memory through the authoritative structured-memory commands.",
            true,
        ));
    }
    let mut locked_tasks = {
        let mut statement = transaction
            .prepare(
                "SELECT DISTINCT task_owner_agent_id, task_id FROM run_attempts
                 WHERE status IN ('admitted', 'starting', 'dispatching', 'running',
                                  'cancel_requested')",
            )
            .map_err(PersistenceError::database)?;
        collect_rows(
            statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))),
        )?
        .into_iter()
        .collect::<HashSet<_>>()
    };
    let pending_run_intents = {
        let mut statement = transaction
            .prepare(
                "SELECT intent_json FROM approval_requests
                 WHERE authoritative = 1 AND intent_kind = 'runTask'
                   AND status IN ('Pending', 'Approved')
                   AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?1",
            )
            .map_err(PersistenceError::database)?;
        collect_rows(statement.query_map([timestamp], |row| row.get::<_, String>(0)))?
    };
    for intent_json in pending_run_intents {
        let intent: ActionIntent = serde_json::from_str(&intent_json).map_err(|_| {
            PersistenceError::new(
                "MALFORMED_APPROVAL",
                "A stored run approval intent is malformed; task state was not changed.",
                false,
            )
        })?;
        if let ActionIntent::RunTask {
            task_owner_agent_id,
            task_id,
            ..
        } = intent
        {
            locked_tasks.insert((task_owner_agent_id, task_id));
        }
    }

    let current_tasks = current
        .agents
        .iter()
        .flat_map(|owner| {
            owner
                .tasks
                .iter()
                .map(move |task| ((owner.id, task.id), task))
        })
        .collect::<HashMap<_, _>>();
    let requested_tasks = requested
        .agents
        .iter()
        .flat_map(|owner| {
            owner
                .tasks
                .iter()
                .map(move |task| ((owner.id, task.id), task))
        })
        .collect::<HashMap<_, _>>();
    for key in &locked_tasks {
        let Some(current_task) = current_tasks.get(key) else {
            continue;
        };
        let Some(requested_task) = requested_tasks.get(key) else {
            return Err(run_task_locked_error());
        };
        if task_run_inputs_changed(current_task, requested_task) {
            return Err(run_task_locked_error());
        }
    }

    for key in current_tasks.keys() {
        if !requested_tasks.contains_key(key) {
            return Err(PersistenceError::new(
                "TASK_ORCHESTRATION_AUTHORITY_REQUIRED",
                "Create, remove, and relocate tasks through the authoritative task orchestration commands.",
                true,
            ));
        }
    }

    let mut protected = requested.clone();
    for owner in &mut protected.agents {
        for task in &mut owner.tasks {
            let key = (owner.id, task.id);
            if let Some(current_task) = current_tasks.get(&key) {
                if task_orchestration_inputs_changed(current_task, task) {
                    return Err(PersistenceError::new(
                        "TASK_ORCHESTRATION_AUTHORITY_REQUIRED",
                        "Edit routing inputs and executor assignment through the authoritative reroute command.",
                        true,
                    ));
                }
                copy_run_owned_task_fields(task, current_task);
            } else {
                return Err(PersistenceError::new(
                    "TASK_ORCHESTRATION_AUTHORITY_REQUIRED",
                    "Create tasks through the authoritative routed-task command.",
                    true,
                ));
            }
        }
    }
    Ok(protected)
}

fn run_task_locked_error() -> PersistenceError {
    PersistenceError::new(
        "RUN_TASK_LOCKED",
        "The task is locked by a pending approval or active run. Refresh backend state before editing it.",
        true,
    )
}

fn task_run_inputs_changed(current: &AgentTask, requested: &AgentTask) -> bool {
    current.title != requested.title
        || current.category != requested.category
        || current.priority != requested.priority
        || current.assigned_agent_id != requested.assigned_agent_id
        || current.created_at != requested.created_at
        || current.workspace_id != requested.workspace_id
        || current.specialist_request != requested.specialist_request
        || current.routing_mode != requested.routing_mode
        || current.routed_from_agent_id != requested.routed_from_agent_id
        || current.routing_reason != requested.routing_reason
}

fn task_orchestration_inputs_changed(current: &AgentTask, requested: &AgentTask) -> bool {
    current.title != requested.title
        || current.category != requested.category
        || current.priority != requested.priority
        || current.assigned_agent_id != requested.assigned_agent_id
        || current.created_at != requested.created_at
        || current.workspace_id != requested.workspace_id
        || current.specialist_request != requested.specialist_request
        || current.routing_mode != requested.routing_mode
        || current.routed_from_agent_id != requested.routed_from_agent_id
        || current.routing_reason != requested.routing_reason
}

fn copy_run_owned_task_fields(target: &mut AgentTask, source: &AgentTask) {
    target.status.clone_from(&source.status);
    target.phase.clone_from(&source.phase);
    target.completed_at.clone_from(&source.completed_at);
    target.result.clone_from(&source.result);
    target.response_id.clone_from(&source.response_id);
    target.runtime_model.clone_from(&source.runtime_model);
    target.total_tokens = source.total_tokens;
    target.changed_files.clone_from(&source.changed_files);
    target.diff.clone_from(&source.diff);
    target
        .workspace_changes
        .clone_from(&source.workspace_changes);
    target.duration_seconds = source.duration_seconds;
    target.queue_state.clone_from(&source.queue_state);
    target.enqueue_sequence = source.enqueue_sequence;
    target.routing_evidence.clone_from(&source.routing_evidence);
    target.review_agent_id = source.review_agent_id;
    target.review_status.clone_from(&source.review_status);
    target.review_result.clone_from(&source.review_result);
    target.review_model.clone_from(&source.review_model);
    target.review_duration_seconds = source.review_duration_seconds;
    target.reviewed_at.clone_from(&source.reviewed_at);
}

fn prepare_private_database_file(path: &Path) -> PersistenceResult<()> {
    let parent = path.parent().ok_or_else(|| {
        PersistenceError::new(
            "DATA_DIRECTORY_UNAVAILABLE",
            "The application data directory could not be resolved.",
            false,
        )
    })?;
    fs::create_dir_all(parent).map_err(|_| {
        PersistenceError::new(
            "DATA_DIRECTORY_UNAVAILABLE",
            "The application data directory could not be created.",
            false,
        )
    })?;

    #[cfg(unix)]
    {
        use std::io::ErrorKind;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|_| {
            PersistenceError::new(
                "DATA_DIRECTORY_PERMISSIONS",
                "The application data directory could not be made private.",
                false,
            )
        })?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(PersistenceError::new(
                    "DATABASE_FILE_UNSAFE",
                    "The application database path is not a regular file.",
                    false,
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                    .map_err(|_| {
                        PersistenceError::new(
                            "DATABASE_CREATE_FAILED",
                            "The private application database file could not be created.",
                            false,
                        )
                    })?;
            }
            Err(_) => {
                return Err(PersistenceError::new(
                    "DATABASE_METADATA_FAILED",
                    "The application database file could not be inspected safely.",
                    false,
                ));
            }
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| {
            PersistenceError::new(
                "DATABASE_PERMISSIONS",
                "The application database file could not be made private.",
                false,
            )
        })?;
    }

    Ok(())
}

/// The schema-object definitions a freshly migrated database produces with the
/// current migration set. Used to detect an uninitialized on-disk shell left by
/// an older build whose migration DDL has since changed in place.
fn expected_migrated_schema_objects() -> PersistenceResult<Vec<(String, String)>> {
    StateRepository::open_in_memory_internal()?.schema_object_definitions()
}

/// Remove a database file and its SQLite sidecar journals. Missing files are
/// not an error; any other failure is reported so a rebuild never proceeds on a
/// partially removed database.
fn remove_database_artifacts(path: &Path) -> PersistenceResult<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let artifact = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        };
        match fs::remove_file(&artifact) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(PersistenceError::new(
                    "SCHEMA_REBUILD_FAILED",
                    "A superseded application database file could not be removed for rebuild.",
                    false,
                ));
            }
        }
    }
    Ok(())
}

fn now_unix_ms() -> PersistenceResult<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            PersistenceError::new(
                "CLOCK_UNAVAILABLE",
                "The system clock cannot record persistence metadata.",
                false,
            )
        })?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        PersistenceError::new(
            "CLOCK_UNAVAILABLE",
            "The system clock value is outside the supported range.",
            false,
        )
    })
}

fn next_revision(revision: i64) -> PersistenceResult<i64> {
    revision
        .checked_add(1)
        .filter(|next| *next <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            PersistenceError::new(
                "REVISION_EXHAUSTED",
                "Application state revision cannot be advanced safely.",
                false,
            )
        })
}

fn routing_error(error: crate::task_orchestration::RoutingError) -> PersistenceError {
    PersistenceError::new(&error.code, error.message, true)
}

fn find_task_indexes(
    state: &ApplicationState,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<(usize, usize)> {
    let owner_index = state
        .agents
        .iter()
        .position(|agent| agent.id == owner_agent_id)
        .ok_or_else(|| {
            PersistenceError::new(
                "TASK_OWNER_NOT_FOUND",
                "The selected task owner no longer exists.",
                true,
            )
        })?;
    let task_index = state.agents[owner_index]
        .tasks
        .iter()
        .position(|task| task.id == task_id)
        .ok_or_else(|| {
            PersistenceError::new(
                "TASK_NOT_FOUND",
                "The selected task no longer exists.",
                true,
            )
        })?;
    Ok((owner_index, task_index))
}

fn allocate_task_and_enqueue_sequence(
    transaction: &Transaction<'_>,
) -> PersistenceResult<(i64, i64)> {
    let (task_id, enqueue_sequence): (i64, i64) = transaction
        .query_row(
            "SELECT next_task_id, next_enqueue_sequence
             FROM task_orchestration_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(PersistenceError::database)?;
    if task_id >= MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "TASK_ID_EXHAUSTED",
            "No additional JavaScript-safe task identifiers are available.",
            false,
        ));
    }
    if enqueue_sequence >= MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "TASK_QUEUE_SEQUENCE_EXHAUSTED",
            "No additional JavaScript-safe queue sequence values are available.",
            false,
        ));
    }
    transaction
        .execute(
            "UPDATE task_orchestration_meta
             SET next_task_id = ?1, next_enqueue_sequence = ?2
             WHERE singleton = 1",
            params![task_id + 1, enqueue_sequence + 1],
        )
        .map_err(PersistenceError::database)?;
    Ok((task_id, enqueue_sequence))
}

fn allocate_enqueue_sequence(transaction: &Connection) -> PersistenceResult<i64> {
    let sequence: i64 = transaction
        .query_row(
            "SELECT next_enqueue_sequence
             FROM task_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if sequence >= MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "TASK_QUEUE_SEQUENCE_EXHAUSTED",
            "No additional JavaScript-safe queue sequence values are available.",
            false,
        ));
    }
    transaction
        .execute(
            "UPDATE task_orchestration_meta
             SET next_enqueue_sequence = ?1 WHERE singleton = 1",
            [sequence + 1],
        )
        .map_err(PersistenceError::database)?;
    Ok(sequence)
}

fn finish_task_orchestration_mutation(
    transaction: &Transaction<'_>,
    state_revision: i64,
) -> PersistenceResult<()> {
    let next_state_revision = next_revision(state_revision)?;
    let orchestration_revision: i64 = transaction
        .query_row(
            "SELECT revision FROM task_orchestration_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let next_orchestration_revision = next_revision(orchestration_revision)?;
    transaction
        .execute(
            "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
            [next_state_revision],
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "UPDATE task_orchestration_meta SET revision = ?1 WHERE singleton = 1",
            [next_orchestration_revision],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn ensure_task_has_no_active_run(
    transaction: &Transaction<'_>,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<()> {
    let active: bool = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM run_attempts
                 WHERE task_owner_agent_id = ?1 AND task_id = ?2
                   AND status IN ('admitted', 'starting', 'dispatching', 'running',
                                  'cancel_requested')
             )",
            params![owner_agent_id, task_id],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if active {
        return Err(PersistenceError::new(
            "TASK_QUEUE_LOCKED",
            "The task is locked by an active run.",
            true,
        ));
    }
    Ok(())
}

fn ensure_task_has_no_active_review_flow(
    connection: &Connection,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<()> {
    let active: bool = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM review_flows
                 WHERE task_owner_agent_id = ?1 AND task_id = ?2
                   AND state NOT IN ('completed', 'failed', 'cancelled')
             )",
            params![owner_agent_id, task_id],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if active {
        return Err(PersistenceError::new(
            "TASK_REVIEW_LOCKED",
            "The task executor and routing inputs are locked by an active review flow.",
            true,
        ));
    }
    Ok(())
}

fn ensure_review_queue_mutation_allowed(
    connection: &Connection,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<()> {
    let state: Option<String> = connection
        .query_row(
            "SELECT state FROM review_flows
             WHERE task_owner_agent_id = ?1 AND task_id = ?2
               AND state NOT IN ('completed', 'failed', 'cancelled')",
            params![owner_agent_id, task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    if state
        .as_deref()
        .is_some_and(|state| state != "revision_queued")
    {
        return Err(PersistenceError::new(
            "TASK_REVIEW_LOCKED",
            "Only a review-requested revision may be held or resumed through queue controls.",
            true,
        ));
    }
    Ok(())
}

fn expire_task_approvals(
    transaction: &Connection,
    task_owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<()> {
    let timestamp = now_unix_ms()?;
    let resolved_at = format_unix_ms(timestamp);
    transaction
        .execute(
            "UPDATE approval_requests
             SET status = 'Expired', resolved_at = COALESCE(resolved_at, ?1),
                 resolved_at_unix_ms = COALESCE(resolved_at_unix_ms, ?2)
             WHERE authoritative = 1 AND intent_kind = 'runTask' AND task_id = ?3
               AND json_extract(intent_json, '$.taskOwnerAgentId') = ?4
               AND status IN ('Pending', 'Approved')
               AND consumed_at_unix_ms IS NULL",
            params![resolved_at, timestamp, task_id, task_owner_agent_id],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn map_task_queue_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskQueueEntry> {
    Ok(TaskQueueEntry {
        task_owner_agent_id: row.get(0)?,
        task_id: row.get(1)?,
        assigned_agent_id: row.get(2)?,
        title: row.get(3)?,
        priority: row.get(4)?,
        queue_state: row.get(5)?,
        enqueue_sequence: row.get(6)?,
        queue_position: None,
    })
}

fn read_task_queue_entries(
    transaction: &Transaction<'_>,
    queue_state: &str,
) -> PersistenceResult<Vec<TaskQueueEntry>> {
    let mut statement = transaction
        .prepare(
            "SELECT owner_agent_id, id, assigned_agent_id, title, priority,
                    queue_state, enqueue_sequence
             FROM agent_tasks WHERE queue_state = ?1
             ORDER BY CASE priority
                 WHEN 'Critical' THEN 0 WHEN 'High' THEN 1
                 WHEN 'Normal' THEN 2 ELSE 3 END,
                 enqueue_sequence, owner_agent_id, id",
        )
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([queue_state], map_task_queue_entry))
}

fn read_active_task_queue_entries(
    transaction: &Transaction<'_>,
) -> PersistenceResult<Vec<TaskQueueEntry>> {
    let mut statement = transaction
        .prepare(
            "SELECT owner_agent_id, id, assigned_agent_id, title, priority,
                    queue_state, enqueue_sequence
             FROM agent_tasks WHERE queue_state IN ('admitted', 'running')
             ORDER BY enqueue_sequence, owner_agent_id, id",
        )
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([], map_task_queue_entry))
}

fn allocate_agent_id(transaction: &Transaction<'_>) -> PersistenceResult<i64> {
    let next_id: i64 = transaction
        .query_row(
            "SELECT next_agent_id FROM agent_registry_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if next_id >= MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "AGENT_ID_EXHAUSTED",
            "No additional JavaScript-safe agent identifiers are available.",
            false,
        ));
    }
    transaction
        .execute(
            "UPDATE agent_registry_meta SET next_agent_id = ?1 WHERE singleton = 1",
            [next_id + 1],
        )
        .map_err(PersistenceError::database)?;
    Ok(next_id)
}

fn synchronize_agent_id_allocator(
    transaction: &Transaction<'_>,
    state: &ApplicationState,
) -> PersistenceResult<()> {
    let maximum_id = state.agents.iter().map(|agent| agent.id).max().unwrap_or(0);
    let desired = maximum_id.saturating_add(1).min(MAX_SAFE_INTEGER);
    transaction
        .execute(
            "UPDATE agent_registry_meta
             SET next_agent_id = MAX(next_agent_id, ?1)
             WHERE singleton = 1",
            [desired],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn synchronize_task_orchestration_allocators(
    transaction: &Transaction<'_>,
    state: &ApplicationState,
) -> PersistenceResult<()> {
    let maximum_task_id = state
        .agents
        .iter()
        .flat_map(|agent| agent.tasks.iter().map(|task| task.id))
        .max()
        .unwrap_or(0);
    let maximum_enqueue_sequence = state
        .agents
        .iter()
        .flat_map(|agent| agent.tasks.iter().filter_map(|task| task.enqueue_sequence))
        .max()
        .unwrap_or(0);
    let next_task_id = maximum_task_id.saturating_add(1).min(MAX_SAFE_INTEGER);
    let next_enqueue_sequence = maximum_enqueue_sequence
        .saturating_add(1)
        .min(MAX_SAFE_INTEGER);
    transaction
        .execute(
            "UPDATE task_orchestration_meta
             SET next_task_id = MAX(next_task_id, ?1),
                 next_enqueue_sequence = MAX(next_enqueue_sequence, ?2)
             WHERE singleton = 1",
            params![next_task_id, next_enqueue_sequence],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn ensure_expected_revision(
    meta: &ApplicationMeta,
    expected_revision: i64,
) -> PersistenceResult<()> {
    if !meta.initialized {
        return Err(PersistenceError::new(
            "STATE_NOT_INITIALIZED",
            "Application state must be initialized before it can be saved.",
            true,
        ));
    }
    if meta.state_revision != expected_revision {
        return Err(PersistenceError::new(
            "REVISION_CONFLICT",
            "Application state changed before this save could be committed.",
            true,
        ));
    }
    Ok(())
}

fn ensure_agent_registry_structure_unchanged(
    current: &ApplicationState,
    requested: &ApplicationState,
) -> PersistenceResult<()> {
    if current.agents.len() != requested.agents.len() {
        return Err(PersistenceError::new(
            "AGENT_REGISTRY_MUTATION_REQUIRED",
            "Create and delete agents through the authoritative agent registry.",
            true,
        ));
    }
    for (current_agent, requested_agent) in current.agents.iter().zip(&requested.agents) {
        if current_agent.id != requested_agent.id
            || current_agent.template_key != requested_agent.template_key
            || current_agent.registry_state != requested_agent.registry_state
            || current_agent.registry_issue != requested_agent.registry_issue
            || current_agent.deleted_at_unix_ms != requested_agent.deleted_at_unix_ms
            || current_agent.name != requested_agent.name
            || current_agent.description != requested_agent.description
            || current_agent.role != requested_agent.role
            || current_agent.category != requested_agent.category
            || current_agent.reports_to != requested_agent.reports_to
            || current_agent.authority_level != requested_agent.authority_level
        {
            return Err(PersistenceError::new(
                "AGENT_REGISTRY_MUTATION_REQUIRED",
                "Agent identity, lifecycle, and hierarchy can only change through the authoritative agent registry.",
                true,
            ));
        }
    }
    Ok(())
}

fn ensure_state_initialized(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    let meta = application_meta_from(transaction)?;
    if !meta.initialized {
        return Err(PersistenceError::new(
            "STATE_NOT_INITIALIZED",
            "Application state must be initialized before authorization can be evaluated.",
            true,
        ));
    }
    Ok(())
}

fn policy_denial(error: crate::policy::PolicyDenial) -> PersistenceError {
    PersistenceError::new(&error.code, error.message, true)
}

fn protected_security_change_summary(
    current: &ApplicationState,
    requested: &ApplicationState,
) -> Option<String> {
    let mut changes: Vec<String> = Vec::new();
    let current_workspaces = current
        .preferences
        .workspaces
        .iter()
        .map(|workspace| (workspace.id.as_str(), workspace.path.as_str()))
        .collect::<HashMap<_, _>>();
    for workspace in &requested.preferences.workspaces {
        if current_workspaces.get(workspace.id.as_str()).copied() != Some(workspace.path.as_str()) {
            changes.push(format!(
                "workspace {} root -> {}",
                dialog_literal(&workspace.id),
                dialog_literal(&workspace.path)
            ));
        }
    }
    if !requested.preferences.workspace_path.is_empty()
        && requested.preferences.workspace_path != current.preferences.workspace_path
    {
        changes.push(format!(
            "active workspace root {} -> {}",
            dialog_literal(&current.preferences.workspace_path),
            dialog_literal(&requested.preferences.workspace_path)
        ));
    }

    if safety_rank(&requested.preferences.safety_mode)
        > safety_rank(&current.preferences.safety_mode)
    {
        changes.push(format!(
            "global safety mode {} -> {}",
            dialog_literal(&current.preferences.safety_mode),
            dialog_literal(&requested.preferences.safety_mode)
        ));
    }
    if requested.preferences.approval_expiry_minutes > current.preferences.approval_expiry_minutes {
        changes.push(format!(
            "approval lifetime {} -> {} minutes",
            current.preferences.approval_expiry_minutes,
            requested.preferences.approval_expiry_minutes
        ));
    }
    if !current.preferences.voice_control_master_enabled
        && requested.preferences.voice_control_master_enabled
    {
        changes.push("voice-control microphone off -> on".to_string());
    }
    if !current.preferences.background_voice_enabled
        && requested.preferences.background_voice_enabled
    {
        changes.push("background microphone listener off -> on".to_string());
    }

    for requested_agent in &requested.agents {
        let Some(current_agent) = current
            .agents
            .iter()
            .find(|agent| agent.id == requested_agent.id)
        else {
            if requested_agent.status != "Paused" {
                for (scope, value) in [
                    ("files", requested_agent.capabilities.files.as_str()),
                    ("internet", requested_agent.capabilities.internet.as_str()),
                    ("clipboard", requested_agent.capabilities.clipboard.as_str()),
                    ("terminal", requested_agent.capabilities.terminal.as_str()),
                    ("system", requested_agent.capabilities.system.as_str()),
                ] {
                    if capability_rank(scope, value) > 0 {
                        changes.push(format!(
                            "new active agent {} (ID {}) {scope} capability -> {}",
                            dialog_literal(&requested_agent.name),
                            requested_agent.id,
                            dialog_literal(value)
                        ));
                    }
                }
            }
            continue;
        };
        let agent_label = format!(
            "agent {} (ID {})",
            dialog_literal(&requested_agent.name),
            requested_agent.id
        );
        if current_agent.status == "Paused" && requested_agent.status != "Paused" {
            changes.push(format!(
                "{agent_label} status {} -> {}",
                dialog_literal(&current_agent.status),
                dialog_literal(&requested_agent.status)
            ));
        }
        if !is_review_role(&current_agent.role) && is_review_role(&requested_agent.role) {
            changes.push(format!(
                "{agent_label} role {} -> {}",
                dialog_literal(&current_agent.role),
                dialog_literal(&requested_agent.role)
            ));
        }
        for (scope, current_value, requested_value) in [
            (
                "files",
                current_agent.capabilities.files.as_str(),
                requested_agent.capabilities.files.as_str(),
            ),
            (
                "internet",
                current_agent.capabilities.internet.as_str(),
                requested_agent.capabilities.internet.as_str(),
            ),
            (
                "clipboard",
                current_agent.capabilities.clipboard.as_str(),
                requested_agent.capabilities.clipboard.as_str(),
            ),
            (
                "terminal",
                current_agent.capabilities.terminal.as_str(),
                requested_agent.capabilities.terminal.as_str(),
            ),
            (
                "system",
                current_agent.capabilities.system.as_str(),
                requested_agent.capabilities.system.as_str(),
            ),
        ] {
            if capability_rank(scope, requested_value) > capability_rank(scope, current_value) {
                changes.push(format!(
                    "{agent_label} {scope} capability {} -> {}",
                    dialog_literal(current_value),
                    dialog_literal(requested_value)
                ));
            }
        }
        for (scope, current_value, requested_value) in [
            (
                "files",
                current_agent.approvals.files.as_str(),
                requested_agent.approvals.files.as_str(),
            ),
            (
                "internet",
                current_agent.approvals.internet.as_str(),
                requested_agent.approvals.internet.as_str(),
            ),
            (
                "clipboard",
                current_agent.approvals.clipboard.as_str(),
                requested_agent.approvals.clipboard.as_str(),
            ),
            (
                "terminal",
                current_agent.approvals.terminal.as_str(),
                requested_agent.approvals.terminal.as_str(),
            ),
            (
                "system",
                current_agent.approvals.system.as_str(),
                requested_agent.approvals.system.as_str(),
            ),
        ] {
            if approval_rank(requested_value) > approval_rank(current_value) {
                changes.push(format!(
                    "{agent_label} {scope} approval policy {} -> {}",
                    dialog_literal(current_value),
                    dialog_literal(requested_value)
                ));
            }
        }
    }

    (!changes.is_empty()).then(|| changes.join("; "))
}

fn is_review_role(role: &str) -> bool {
    matches!(role, "Senior Agent" | "Team Leader" | "Supervisor")
}

fn safety_rank(value: &str) -> u8 {
    match value {
        "locked" => 0,
        "strict" => 1,
        "balanced" => 2,
        _ => 3,
    }
}

fn approval_rank(value: &str) -> u8 {
    match value {
        "deny" => 0,
        "ask" => 1,
        "allow" => 2,
        _ => 3,
    }
}

fn capability_rank(scope: &str, value: &str) -> u8 {
    match (scope, value) {
        (_, "none") => 0,
        ("files" | "internet" | "clipboard", "read") | ("terminal", "safe") => 1,
        ("files" | "internet" | "clipboard", "write") | ("terminal", "user") => 2,
        ("files" | "internet" | "clipboard", "full") | ("terminal", "admin") => 3,
        ("system", "notifications") => 1,
        ("system", "power") => 2,
        ("system", "full") => 3,
        _ => 4,
    }
}

#[derive(Debug)]
struct StoredApproval {
    agent_id: i64,
    task_id: Option<i64>,
    status: String,
    workspace_id: Option<String>,
    authoritative: bool,
    intent_kind: String,
    intent_json: String,
    intent_fingerprint: String,
    policy_fingerprint: String,
    workspace_fingerprint: String,
    expires_at_unix_ms: Option<i64>,
    consumed_at_unix_ms: Option<i64>,
}

fn read_stored_approval(
    transaction: &Transaction<'_>,
    approval_id: i64,
) -> PersistenceResult<StoredApproval> {
    transaction
        .query_row(
            "SELECT agent_id, task_id, status, workspace_id, authoritative,
                    intent_kind, intent_json, intent_fingerprint, policy_fingerprint,
                    workspace_fingerprint, expires_at_unix_ms, consumed_at_unix_ms
             FROM approval_requests WHERE id = ?1",
            [approval_id],
            |row| {
                Ok(StoredApproval {
                    agent_id: row.get(0)?,
                    task_id: row.get(1)?,
                    status: row.get(2)?,
                    workspace_id: row.get(3)?,
                    authoritative: row.get::<_, i64>(4)? == 1,
                    intent_kind: row.get(5)?,
                    intent_json: row.get(6)?,
                    intent_fingerprint: row.get(7)?,
                    policy_fingerprint: row.get(8)?,
                    workspace_fingerprint: row.get(9)?,
                    expires_at_unix_ms: row.get(10)?,
                    consumed_at_unix_ms: row.get(11)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::new(
                "APPROVAL_NOT_FOUND",
                "The requested approval does not exist.",
                true,
            ),
            other => PersistenceError::database(other),
        })
}

fn ensure_pending_authoritative_approval(
    stored: &StoredApproval,
    timestamp: i64,
) -> PersistenceResult<()> {
    if !stored.authoritative {
        return Err(PersistenceError::new(
            "APPROVAL_NOT_AUTHORITATIVE",
            "Imported and renderer-origin approval records cannot authorize actions.",
            true,
        ));
    }
    if stored.consumed_at_unix_ms.is_some() {
        return Err(PersistenceError::new(
            "APPROVAL_ALREADY_CONSUMED",
            "The approval has already authorized one action and cannot be replayed.",
            true,
        ));
    }
    if match stored.expires_at_unix_ms {
        Some(expires_at) => expires_at <= timestamp,
        None => true,
    } || stored.status == "Expired"
    {
        return Err(PersistenceError::new(
            "APPROVAL_EXPIRED",
            "The approval has expired and cannot authorize an action.",
            true,
        ));
    }
    if stored.status != "Pending" {
        return Err(PersistenceError::new(
            "APPROVAL_ALREADY_RESOLVED",
            "Only a pending approval can be resolved.",
            true,
        ));
    }
    Ok(())
}

fn ensure_evaluation_matches(
    stored: &StoredApproval,
    evaluation: &PolicyEvaluation,
) -> PersistenceResult<()> {
    if evaluation.disposition != PolicyDisposition::ApprovalRequired {
        return Err(PersistenceError::new(
            "STALE_APPROVAL",
            "Current policy no longer matches the stored approval request.",
            true,
        ));
    }
    if stored.agent_id != evaluation.agent_id {
        return Err(PersistenceError::new(
            "WRONG_AGENT_APPROVAL",
            "The approval is bound to a different agent.",
            true,
        ));
    }
    if stored.task_id != evaluation.task_id {
        return Err(PersistenceError::new(
            "WRONG_TASK_APPROVAL",
            "The approval is bound to a different task.",
            true,
        ));
    }
    if stored.workspace_id != evaluation.workspace_id
        || stored.workspace_fingerprint != evaluation.workspace_fingerprint
    {
        return Err(PersistenceError::new(
            "WRONG_WORKSPACE_APPROVAL",
            "The approval is bound to a different workspace state.",
            true,
        ));
    }
    if stored.intent_kind != evaluation.intent_kind
        || stored.intent_fingerprint != evaluation.intent_fingerprint
    {
        return Err(PersistenceError::new(
            "WRONG_INTENT_APPROVAL",
            "The approval is bound to a different action intent.",
            true,
        ));
    }
    if stored.policy_fingerprint != evaluation.policy_fingerprint {
        return Err(PersistenceError::new(
            "STALE_APPROVAL",
            "The approval was issued under different capability or safety policy.",
            true,
        ));
    }
    Ok(())
}

fn expire_authoritative_approvals(
    transaction: &Transaction<'_>,
    timestamp: i64,
) -> PersistenceResult<()> {
    let resolved_at = format_unix_ms(timestamp);
    transaction
        .execute(
            "UPDATE approval_requests
             SET status = 'Expired', resolved_at = COALESCE(resolved_at, ?1),
                 resolved_at_unix_ms = COALESCE(resolved_at_unix_ms, ?2)
             WHERE authoritative = 1 AND status IN ('Pending', 'Approved')
               AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms <= ?2",
            params![resolved_at, timestamp],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn find_matching_active_approval(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    timestamp: i64,
) -> PersistenceResult<Option<i64>> {
    transaction
        .query_row(
            "SELECT id FROM approval_requests
             WHERE authoritative = 1 AND status IN ('Pending', 'Approved')
               AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?1
               AND agent_id = ?2 AND task_id IS ?3 AND workspace_id IS ?4
               AND intent_kind = ?5 AND intent_fingerprint = ?6
               AND policy_fingerprint = ?7 AND workspace_fingerprint = ?8
             ORDER BY CASE status WHEN 'Approved' THEN 0 ELSE 1 END, id DESC
             LIMIT 1",
            params![
                timestamp,
                evaluation.agent_id,
                evaluation.task_id,
                evaluation.workspace_id,
                evaluation.intent_kind,
                evaluation.intent_fingerprint,
                evaluation.policy_fingerprint,
                evaluation.workspace_fingerprint
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)
}

fn find_matching_approved_approval(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    timestamp: i64,
) -> PersistenceResult<Option<i64>> {
    transaction
        .query_row(
            "SELECT id FROM approval_requests
             WHERE authoritative = 1 AND status = 'Approved'
               AND consumed_at_unix_ms IS NULL AND expires_at_unix_ms > ?1
               AND agent_id = ?2 AND task_id IS ?3 AND workspace_id IS ?4
               AND intent_kind = ?5 AND intent_fingerprint = ?6
               AND policy_fingerprint = ?7 AND workspace_fingerprint = ?8
             ORDER BY id DESC LIMIT 1",
            params![
                timestamp,
                evaluation.agent_id,
                evaluation.task_id,
                evaluation.workspace_id,
                evaluation.intent_kind,
                evaluation.intent_fingerprint,
                evaluation.policy_fingerprint,
                evaluation.workspace_fingerprint
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)
}

fn insert_authoritative_approval(
    transaction: &Transaction<'_>,
    intent: &ActionIntent,
    evaluation: &PolicyEvaluation,
    timestamp: i64,
) -> PersistenceResult<ApprovalRequest> {
    let approval_count: i64 = transaction
        .query_row("SELECT COUNT(*) FROM approval_requests", [], |row| {
            row.get(0)
        })
        .map_err(PersistenceError::database)?;
    if approval_count >= MAX_AUTHORIZATION_RECORDS {
        return Err(PersistenceError::new(
            "APPROVAL_HISTORY_LIMIT",
            "Approval history reached its bounded record limit. Reset or future retention controls must clear history before another request can be issued.",
            false,
        ));
    }
    let approval_id: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM approval_requests",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if approval_id <= 0 || approval_id > MAX_SAFE_INTEGER {
        return Err(PersistenceError::new(
            "APPROVAL_ID_EXHAUSTED",
            "No safe approval identifier remains available.",
            false,
        ));
    }
    let position: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM approval_requests",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let expires_at_unix_ms = timestamp
        .checked_add(evaluation.expires_in_ms)
        .ok_or_else(|| {
            PersistenceError::new(
                "CLOCK_UNAVAILABLE",
                "The approval expiration could not be represented safely.",
                false,
            )
        })?;
    let created_at = format_unix_ms(timestamp);
    let expires_at = format_unix_ms(expires_at_unix_ms);
    let intent_json = serde_json::to_string(intent).map_err(|_| {
        PersistenceError::new(
            "INVALID_INTENT",
            "The action intent could not be normalized for authorization.",
            true,
        )
    })?;
    transaction
        .execute(
            "INSERT INTO approval_requests
             (id, position, agent_id, task_id, title, reason, status, created_at,
              resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
              consumed_at, origin, authoritative, intent_kind, intent_json,
              intent_fingerprint, policy_fingerprint, workspace_fingerprint,
              created_at_unix_ms, resolved_at_unix_ms, expires_at_unix_ms,
              consumed_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'Pending', ?7, NULL, ?8, ?9,
                     ?10, ?11, NULL, 'backend_authority', 1, ?12, ?13, ?14,
                     ?15, ?16, ?17, NULL, ?18, NULL)",
            params![
                approval_id,
                position,
                evaluation.agent_id,
                evaluation.task_id,
                evaluation.title,
                evaluation.reason,
                created_at,
                evaluation.risk_level,
                evaluation.workspace_id,
                evaluation.task_snapshot,
                expires_at,
                evaluation.intent_kind,
                intent_json,
                evaluation.intent_fingerprint,
                evaluation.policy_fingerprint,
                evaluation.workspace_fingerprint,
                timestamp,
                expires_at_unix_ms
            ],
        )
        .map_err(PersistenceError::database)?;
    for (position, scope) in evaluation.scopes.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO approval_scopes (approval_id, position, scope)
                 VALUES (?1, ?2, ?3)",
                params![approval_id, position as i64, scope.as_str()],
            )
            .map_err(PersistenceError::database)?;
    }
    read_approval_request(transaction, approval_id)
}

fn missing_approval_error(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    timestamp: i64,
) -> PersistenceResult<PersistenceError> {
    let exact: Option<(String, Option<i64>, i64)> = transaction
        .query_row(
            "SELECT status, consumed_at_unix_ms, authoritative
             FROM approval_requests
             WHERE agent_id = ?1 AND task_id IS ?2 AND workspace_id IS ?3
               AND intent_kind = ?4 AND intent_fingerprint = ?5
               AND policy_fingerprint = ?6 AND workspace_fingerprint = ?7
             ORDER BY id DESC LIMIT 1",
            params![
                evaluation.agent_id,
                evaluation.task_id,
                evaluation.workspace_id,
                evaluation.intent_kind,
                evaluation.intent_fingerprint,
                evaluation.policy_fingerprint,
                evaluation.workspace_fingerprint
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    if let Some((status, consumed_at, authoritative)) = exact {
        if authoritative != 1 {
            return Ok(PersistenceError::new(
                "APPROVAL_NOT_AUTHORITATIVE",
                "Imported and renderer-origin approval records cannot authorize actions.",
                true,
            ));
        }
        if consumed_at.is_some() {
            return Ok(PersistenceError::new(
                "APPROVAL_ALREADY_CONSUMED",
                "The approval has already authorized one action and cannot be replayed.",
                true,
            ));
        }
        if status == "Expired" {
            return Ok(PersistenceError::new(
                "APPROVAL_EXPIRED",
                "The approval has expired and cannot authorize an action.",
                true,
            ));
        }
        if status == "Pending" {
            return Ok(PersistenceError::new(
                "APPROVAL_PENDING",
                "The action is waiting for trusted desktop approval.",
                true,
            ));
        }
        if status == "Denied" {
            return Ok(PersistenceError::new(
                "APPROVAL_DENIED",
                "The action was denied and is not authorized.",
                true,
            ));
        }
    }

    let stale_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM approval_requests
                 WHERE authoritative = 1 AND agent_id = ?1 AND task_id IS ?2
                   AND intent_kind = ?3 AND intent_fingerprint = ?4
                   AND (policy_fingerprint <> ?5 OR workspace_fingerprint <> ?6
                        OR workspace_id IS NOT ?7 OR expires_at_unix_ms <= ?8)
             )",
            params![
                evaluation.agent_id,
                evaluation.task_id,
                evaluation.intent_kind,
                evaluation.intent_fingerprint,
                evaluation.policy_fingerprint,
                evaluation.workspace_fingerprint,
                evaluation.workspace_id,
                timestamp
            ],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if stale_exists {
        return Ok(PersistenceError::new(
            "STALE_APPROVAL",
            "An earlier approval does not match the current workspace or policy state.",
            true,
        ));
    }
    Ok(PersistenceError::new(
        "AUTHORIZATION_REQUIRED",
        "The action requires a current backend-issued approval.",
        true,
    ))
}

fn migration_info(meta: &ApplicationMeta) -> MigrationInfo {
    MigrationInfo {
        source_kind: meta.source_kind.clone(),
        source_version: meta.source_version,
        migrated_at_unix_ms: meta.migrated_at_unix_ms,
        legacy_cleanup_acknowledged: meta.legacy_cleanup_ack_at_unix_ms.is_some(),
    }
}

trait DatabaseConnection {
    fn query_meta(&self) -> rusqlite::Result<ApplicationMeta>;
}

impl DatabaseConnection for Connection {
    fn query_meta(&self) -> rusqlite::Result<ApplicationMeta> {
        self.query_row(APPLICATION_META_QUERY, [], map_application_meta)
    }
}

impl DatabaseConnection for Transaction<'_> {
    fn query_meta(&self) -> rusqlite::Result<ApplicationMeta> {
        self.query_row(APPLICATION_META_QUERY, [], map_application_meta)
    }
}

const APPLICATION_META_QUERY: &str =
    "SELECT initialized, state_revision, source_kind, source_version,
            migrated_at_unix_ms, legacy_cleanup_ack_at_unix_ms
     FROM application_meta WHERE singleton = 1";

fn map_application_meta(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApplicationMeta> {
    Ok(ApplicationMeta {
        initialized: row.get::<_, i64>(0)? != 0,
        state_revision: row.get(1)?,
        source_kind: row.get(2)?,
        source_version: row.get(3)?,
        migrated_at_unix_ms: row.get(4)?,
        legacy_cleanup_ack_at_unix_ms: row.get(5)?,
    })
}

fn application_meta_from(
    connection: &impl DatabaseConnection,
) -> PersistenceResult<ApplicationMeta> {
    connection.query_meta().map_err(PersistenceError::database)
}

fn ensure_application_initialized(
    connection: &impl DatabaseConnection,
    operation: &str,
) -> PersistenceResult<ApplicationMeta> {
    let meta = application_meta_from(connection)?;
    if !meta.initialized {
        return Err(PersistenceError::new(
            "APPLICATION_STATE_UNINITIALIZED",
            format!("Application state must be initialized before {operation}."),
            true,
        ));
    }
    Ok(meta)
}

fn ensure_subsystem_revision(actual: i64, expected: i64, code: &str) -> PersistenceResult<()> {
    if actual != expected {
        return Err(PersistenceError::new(
            code,
            format!(
                "The subsystem changed (expected revision {expected}, current revision {actual}); refresh before retrying."
            ),
            true,
        ));
    }
    Ok(())
}

fn advance_application_revision(transaction: &Transaction<'_>) -> PersistenceResult<i64> {
    let revision: i64 = transaction
        .query_row(
            "SELECT state_revision FROM application_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let revision = next_revision(revision)?;
    transaction
        .execute(
            "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
            [revision],
        )
        .map_err(PersistenceError::database)?;
    Ok(revision)
}

fn persistence_request_fingerprint<T: Serialize>(
    request: &T,
    code: &str,
) -> PersistenceResult<String> {
    let bytes = serde_json::to_vec(request).map_err(|_| {
        PersistenceError::new(
            code,
            "The mutation request could not be canonicalized.",
            false,
        )
    })?;
    Ok(sha256_hex(&bytes))
}

fn reminder_request_is_duplicate(
    transaction: &Transaction<'_>,
    request_id: &str,
    fingerprint: &str,
) -> PersistenceResult<bool> {
    let existing: Option<String> = transaction
        .query_row(
            "SELECT request_fingerprint FROM reminder_mutation_requests
             WHERE request_id = ?1",
            [request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    match existing {
        None => Ok(false),
        Some(existing) if existing == fingerprint => Ok(true),
        Some(_) => Err(PersistenceError::new(
            "REMINDER_IDEMPOTENCY_CONFLICT",
            "The reminder request identifier is already bound to different content.",
            false,
        )),
    }
}

fn record_reminder_request(
    transaction: &Transaction<'_>,
    request_id: &str,
    fingerprint: &str,
    resulting_revision: i64,
    item_id: Option<i64>,
    timestamp: i64,
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO reminder_mutation_requests
             (request_id, request_fingerprint, resulting_revision, item_id,
              created_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request_id,
                fingerprint,
                resulting_revision,
                item_id,
                timestamp,
            ],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn memory_request_is_duplicate(
    transaction: &Transaction<'_>,
    request_id: &str,
    fingerprint: &str,
) -> PersistenceResult<bool> {
    let existing: Option<String> = transaction
        .query_row(
            "SELECT request_fingerprint FROM memory_mutation_requests WHERE request_id = ?1",
            [request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    match existing {
        None => Ok(false),
        Some(existing) if existing == fingerprint => Ok(true),
        Some(_) => Err(PersistenceError::new(
            "MEMORY_IDEMPOTENCY_CONFLICT",
            "The memory request identifier is already bound to different content.",
            false,
        )),
    }
}

fn record_memory_request(
    transaction: &Transaction<'_>,
    request_id: &str,
    fingerprint: &str,
    resulting_revision: i64,
    record_id: Option<i64>,
    timestamp: i64,
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO memory_mutation_requests
             (request_id, request_fingerprint, resulting_revision, record_id,
              created_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request_id,
                fingerprint,
                resulting_revision,
                record_id,
                timestamp,
            ],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn validate_schedule_links(
    transaction: &Transaction<'_>,
    subject_agent_id: Option<i64>,
    workspace_id: Option<&str>,
    task_owner_agent_id: Option<i64>,
    task_id: Option<i64>,
) -> PersistenceResult<()> {
    if let Some(agent_id) = subject_agent_id {
        ensure_active_agent_exists(transaction, agent_id, "REMINDER_AGENT_NOT_FOUND")?;
    }
    if let Some(workspace_id) = workspace_id {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                [workspace_id],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if !exists {
            return Err(PersistenceError::new(
                "REMINDER_WORKSPACE_NOT_FOUND",
                "The selected reminder workspace no longer exists.",
                true,
            ));
        }
    }
    if let (Some(owner), Some(task)) = (task_owner_agent_id, task_id) {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_tasks
                 WHERE owner_agent_id = ?1 AND id = ?2)",
                params![owner, task],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        if !exists {
            return Err(PersistenceError::new(
                "REMINDER_TASK_NOT_FOUND",
                "The selected reminder task no longer exists.",
                true,
            ));
        }
    }
    Ok(())
}

fn validate_memory_scope_references(
    transaction: &Transaction<'_>,
    scope: &MemoryScopeV1,
) -> PersistenceResult<()> {
    match scope.kind {
        MemoryScopeKind::Agent => ensure_active_agent_exists(
            transaction,
            scope.agent_id.expect("validated agent memory scope"),
            "MEMORY_AGENT_NOT_FOUND",
        ),
        MemoryScopeKind::Team => {
            let team_leader_agent_id = scope
                .team_leader_agent_id
                .expect("validated team memory scope");
            let valid_manager: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM agents
                         WHERE id = ?1 AND registry_state = 'active'
                           AND role IN ('Supervisor', 'Team Leader', 'Senior Agent')
                     )",
                    [team_leader_agent_id],
                    |row| row.get(0),
                )
                .map_err(PersistenceError::database)?;
            if valid_manager {
                Ok(())
            } else {
                Err(PersistenceError::new(
                    "MEMORY_TEAM_NOT_FOUND",
                    "Team memory requires an active Supervisor, Team Leader, or Senior Agent.",
                    true,
                ))
            }
        }
        MemoryScopeKind::Project => {
            let workspace_id = scope
                .workspace_id
                .as_deref()
                .expect("validated project memory scope");
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                    [workspace_id],
                    |row| row.get(0),
                )
                .map_err(PersistenceError::database)?;
            if exists {
                Ok(())
            } else {
                Err(PersistenceError::new(
                    "MEMORY_PROJECT_NOT_FOUND",
                    "The selected memory project no longer exists.",
                    true,
                ))
            }
        }
        MemoryScopeKind::Task => {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_tasks
                     WHERE owner_agent_id = ?1 AND id = ?2)",
                    params![
                        scope
                            .task_owner_agent_id
                            .expect("validated task memory scope"),
                        scope.task_id.expect("validated task memory scope"),
                    ],
                    |row| row.get(0),
                )
                .map_err(PersistenceError::database)?;
            if exists {
                Ok(())
            } else {
                Err(PersistenceError::new(
                    "MEMORY_TASK_NOT_FOUND",
                    "The selected memory task no longer exists.",
                    true,
                ))
            }
        }
    }
}

fn ensure_active_agent_exists(
    transaction: &Transaction<'_>,
    agent_id: i64,
    code: &str,
) -> PersistenceResult<()> {
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1 AND registry_state = 'active')",
            [agent_id],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    if exists {
        Ok(())
    } else {
        Err(PersistenceError::new(
            code,
            "The selected active agent no longer exists.",
            true,
        ))
    }
}

fn management_owner_role(role: &str) -> ManagementOwnerRole {
    match role {
        "Supervisor" => ManagementOwnerRole::Supervisor,
        "Team Leader" => ManagementOwnerRole::TeamLeader,
        "Senior Agent" => ManagementOwnerRole::Senior,
        _ => ManagementOwnerRole::Human,
    }
}

fn management_chain_for_agent(
    state: &ApplicationState,
    agent_id: i64,
) -> PersistenceResult<Vec<i64>> {
    let by_id = state
        .agents
        .iter()
        .map(|agent| (agent.id, agent))
        .collect::<HashMap<_, _>>();
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = by_id.get(&agent_id).and_then(|agent| agent.reports_to);
    while let Some(manager_id) = current {
        if !seen.insert(manager_id) {
            return Err(PersistenceError::new(
                "MEMORY_MANAGEMENT_CHAIN_INVALID",
                "The stored agent reporting chain contains a cycle.",
                false,
            ));
        }
        let manager = by_id.get(&manager_id).ok_or_else(|| {
            PersistenceError::new(
                "MEMORY_MANAGEMENT_CHAIN_INVALID",
                "The stored agent reporting chain references a missing manager.",
                false,
            )
        })?;
        chain.push(manager_id);
        current = manager.reports_to;
    }
    Ok(chain)
}

fn insert_management_handoff(
    transaction: &Transaction<'_>,
    handoff: NewManagementHandoff,
    timestamp: i64,
) -> PersistenceResult<i64> {
    handoff
        .validate()
        .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
    let payload_json = serde_json::to_string(&handoff.payload).map_err(|_| {
        PersistenceError::new(
            "HANDOFF_INVALID",
            "The management handoff payload could not be serialized.",
            false,
        )
    })?;
    let existing: Option<(i64, String, String, String)> = transaction
        .query_row(
            "SELECT id, kind, summary, payload_json FROM management_handoffs
             WHERE idempotency_key = ?1",
            [handoff.idempotency_key.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(PersistenceError::database)?;
    if let Some((id, kind, summary, payload)) = existing {
        if kind == handoff.kind.as_storage_value()
            && summary == handoff.summary
            && payload == payload_json
        {
            return Ok(id);
        }
        return Err(PersistenceError::new(
            "HANDOFF_IDEMPOTENCY_CONFLICT",
            "The management handoff identifier is already bound to different evidence.",
            false,
        ));
    }
    let (revision, next_handoff_id): (i64, i64) = transaction
        .query_row(
            "SELECT revision, next_handoff_id FROM management_handoff_meta
             WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "INSERT INTO management_handoffs
             (id, task_owner_agent_id, task_id, kind, from_agent_id, to_agent_id,
              owner_role, revision_round, run_attempt_id, review_flow_id,
              review_stage_attempt_id, source_kind, summary, payload_json,
              idempotency_key, created_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16)",
            params![
                next_handoff_id,
                handoff.task_owner_agent_id,
                handoff.task_id,
                handoff.kind.as_storage_value(),
                handoff.from_agent_id,
                handoff.to_agent_id,
                handoff.owner_role.as_storage_value(),
                handoff.revision_round,
                handoff.run_attempt_id,
                handoff.review_flow_id,
                handoff.review_stage_attempt_id,
                handoff.source.as_storage_value(),
                handoff.summary,
                payload_json,
                handoff.idempotency_key,
                timestamp,
            ],
        )
        .map_err(PersistenceError::database)?;
    transaction
        .execute(
            "UPDATE management_handoff_meta
             SET revision = ?1, next_handoff_id = ?2 WHERE singleton = 1",
            params![next_revision(revision)?, next_revision(next_handoff_id)?],
        )
        .map_err(PersistenceError::database)?;
    Ok(next_handoff_id)
}

fn clear_application_state(
    transaction: &Transaction<'_>,
    replace_approvals: bool,
) -> PersistenceResult<()> {
    if replace_approvals {
        transaction
            .execute("DELETE FROM approval_requests", [])
            .map_err(PersistenceError::database)?;
        transaction
            .execute_batch(
                "DELETE FROM reminder_mutation_requests;
                 DELETE FROM reminder_occurrences;
                 DELETE FROM reminders;
                 DELETE FROM memory_mutation_requests;
                 DELETE FROM memory_events;
                 DELETE FROM memory_records;
                 DELETE FROM management_handoffs;
                 UPDATE reminder_scheduler_meta
                    SET revision = 0, next_reminder_id = 1, next_occurrence_id = 1,
                        last_scan_at_unix_ms = NULL, last_error_code = NULL,
                        last_error_message = NULL
                  WHERE singleton = 1;
                 UPDATE structured_memory_meta
                    SET revision = 0, next_record_id = 1, next_event_id = 1
                  WHERE singleton = 1;
                 UPDATE management_handoff_meta
                    SET revision = 0, next_handoff_id = 1
                  WHERE singleton = 1;",
            )
            .map_err(PersistenceError::database)?;
    }
    transaction
        .execute_batch(
            "DELETE FROM models;
             DELETE FROM agents;
             DELETE FROM workspaces;
             DELETE FROM retention_settings;
             DELETE FROM preferences;",
        )
        .map_err(PersistenceError::database)
}

type ExistingTaskTimestamps = HashMap<(i64, i64), (Option<i64>, Option<i64>)>;
type ExistingActivityTimestamps = HashMap<(i64, i64), Option<i64>>;

fn read_existing_task_timestamps(
    transaction: &Transaction<'_>,
) -> PersistenceResult<ExistingTaskTimestamps> {
    let mut statement = transaction
        .prepare(
            "SELECT owner_agent_id, id, created_at_unix_ms, completed_at_unix_ms
             FROM agent_tasks",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                (row.get::<_, i64>(0)?, row.get::<_, i64>(1)?),
                (row.get::<_, Option<i64>>(2)?, row.get::<_, Option<i64>>(3)?),
            ))
        })
        .map_err(PersistenceError::database)?;
    let mut timestamps = HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(PersistenceError::database)?;
        timestamps.insert(key, value);
    }
    Ok(timestamps)
}

fn read_existing_activity_timestamps(
    transaction: &Transaction<'_>,
) -> PersistenceResult<ExistingActivityTimestamps> {
    let mut statement = transaction
        .prepare("SELECT owner_agent_id, id, created_at_unix_ms FROM agent_activity")
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                (row.get::<_, i64>(0)?, row.get::<_, i64>(1)?),
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(PersistenceError::database)?;
    let mut timestamps = HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(PersistenceError::database)?;
        timestamps.insert(key, value);
    }
    Ok(timestamps)
}

fn write_application_state(
    transaction: &Transaction<'_>,
    state: &ApplicationState,
    default_approval_origin: &str,
    approval_origins: &HashMap<i64, String>,
    replace_approvals: bool,
) -> PersistenceResult<()> {
    validate_application_state(state).map_err(PersistenceError::validation)?;
    let lifecycle_timestamp = now_unix_ms()?;
    let (existing_task_timestamps, existing_activity_timestamps) = if replace_approvals {
        (HashMap::new(), HashMap::new())
    } else {
        (
            read_existing_task_timestamps(transaction)?,
            read_existing_activity_timestamps(transaction)?,
        )
    };
    clear_application_state(transaction, replace_approvals)?;
    write_preferences(transaction, &state.preferences)?;
    transaction
        .execute(
            "INSERT INTO retention_settings (singleton, task_retention, activity_retention)
             VALUES (1, ?1, ?2)",
            params![
                state.task_retention_days.as_storage_value(),
                state.activity_retention_days.as_storage_value()
            ],
        )
        .map_err(PersistenceError::database)?;

    for (position, agent) in state.agents.iter().enumerate() {
        write_agent(
            transaction,
            position,
            agent,
            &existing_task_timestamps,
            &existing_activity_timestamps,
            lifecycle_timestamp,
        )?;
    }
    for (position, model) in state.models.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO models (id, position, name, provider) VALUES (?1, ?2, ?3, ?4)",
                params![model.id, position as i64, model.name, model.provider],
            )
            .map_err(PersistenceError::database)?;
    }
    if replace_approvals {
        for (position, request) in state.approval_requests.iter().enumerate() {
            let origin = approval_origins
                .get(&request.id)
                .map(String::as_str)
                .unwrap_or(default_approval_origin);
            write_approval_request(transaction, position, request, origin, lifecycle_timestamp)?;
        }
    }
    if replace_approvals {
        write_legacy_reminders_v11(transaction, &state.reminders, lifecycle_timestamp)?;
        write_legacy_agent_memory_v11(
            transaction,
            &state.agents,
            default_approval_origin,
            lifecycle_timestamp,
        )?;
    }
    synchronize_agent_id_allocator(transaction, state)?;
    synchronize_task_orchestration_allocators(transaction, state)?;
    Ok(())
}

fn write_legacy_reminders_v11(
    transaction: &Transaction<'_>,
    reminders: &[Reminder],
    lifecycle_timestamp: i64,
) -> PersistenceResult<()> {
    for (position, reminder) in reminders.iter().enumerate() {
        let due_at_unix_ms: Option<i64> = transaction
            .query_row(
                "SELECT CAST(strftime('%s', ?1) AS INTEGER) * 1000",
                [reminder.due_at.as_str()],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let (status, issue_code, issue_message) = match reminder.status.as_str() {
            "Completed" => ("completed", None, None),
            "Dismissed" => ("dismissed", None, None),
            _ if due_at_unix_ms.is_some() => ("scheduled", None, None),
            _ => (
                "needs_attention",
                Some("LEGACY_DUE_AT_INVALID"),
                Some("The imported legacy due time is invalid and must be corrected before scheduling."),
            ),
        };
        let created_at_unix_ms: Option<i64> = transaction
            .query_row(
                "SELECT CAST(strftime('%s', ?1) AS INTEGER) * 1000",
                [reminder.created_at.as_str()],
                |row| row.get(0),
            )
            .map_err(PersistenceError::database)?;
        let resolved_at_unix_ms =
            matches!(status, "completed" | "dismissed").then_some(lifecycle_timestamp);
        transaction
            .execute(
                "INSERT INTO reminders
                 (id, position, revision, kind, title, notes, local_due_at, time_zone,
                  due_at, due_at_unix_ms, dst_resolution, status, recurrence_kind,
                  recurrence_interval, next_occurrence_sequence, missed_occurrence_count,
                  delivery_mode, privacy_mode, subject_agent_id, task_owner_agent_id,
                  task_id, scheduler_agent_id, schedule_issue_code, schedule_issue_message,
                  created_at, created_at_unix_ms, resolved_at_unix_ms, updated_at_unix_ms)
                 VALUES
                 (?1, ?2, 1, 'reminder', ?3, ?4,
                  CASE WHEN ?6 IS NULL THEN ?5
                       ELSE strftime('%Y-%m-%dT%H:%M:%S', ?5) END,
                  'UTC', ?5, ?6,
                  CASE WHEN ?6 IS NULL THEN 'unresolved' ELSE 'exact' END,
                  ?7, 'none', 1, 0, 0, 'in_app', 'generic', ?8,
                  CASE WHEN ?8 IS NOT NULL AND ?9 IS NOT NULL THEN ?8 END,
                  CASE WHEN ?8 IS NOT NULL THEN ?9 END,
                  (SELECT id FROM agents WHERE template_key = 'event-reminder'
                   ORDER BY id LIMIT 1),
                  ?10, ?11, ?12, COALESCE(?13, ?14), ?15, ?14)",
                params![
                    reminder.id,
                    position as i64,
                    reminder.title,
                    reminder.notes,
                    reminder.due_at,
                    due_at_unix_ms,
                    status,
                    reminder.agent_id,
                    reminder.task_id,
                    issue_code,
                    issue_message,
                    reminder.created_at,
                    created_at_unix_ms,
                    lifecycle_timestamp,
                    resolved_at_unix_ms,
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    let next_id = reminders
        .iter()
        .map(|reminder| reminder.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            PersistenceError::new(
                "REMINDER_CAPACITY_EXCEEDED",
                "The reminder identifier allocator is exhausted.",
                false,
            )
        })?;
    transaction
        .execute(
            "UPDATE reminder_scheduler_meta SET next_reminder_id = ?1 WHERE singleton = 1",
            [next_id.max(1)],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn write_legacy_agent_memory_v11(
    transaction: &Transaction<'_>,
    agents: &[Agent],
    source: &str,
    lifecycle_timestamp: i64,
) -> PersistenceResult<()> {
    let provenance = if source.starts_with("backup") {
        "backup_import"
    } else {
        "legacy_agent_memory"
    };
    let mut next_id = 1_i64;
    for agent in agents
        .iter()
        .filter(|agent| !agent.memory.trim().is_empty())
    {
        transaction
            .execute(
                "INSERT INTO memory_records
                 (id, scope_kind, agent_id, record_kind, content, provenance_kind,
                  provenance_ref, revision, retention_policy, created_at_unix_ms,
                  updated_at_unix_ms)
                 VALUES (?1, 'agent', ?2, 'instruction', ?3, ?4, ?5, 1, 'manual', ?6, ?6)",
                params![
                    next_id,
                    agent.id,
                    agent.memory,
                    provenance,
                    source,
                    lifecycle_timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "INSERT INTO memory_events
                 (id, record_id, action, actor_kind, record_revision, created_at_unix_ms)
                 VALUES (?1, ?1, 'created', ?2, 1, ?3)",
                params![
                    next_id,
                    if provenance == "backup_import" {
                        "import"
                    } else {
                        "migration"
                    },
                    lifecycle_timestamp,
                ],
            )
            .map_err(PersistenceError::database)?;
        next_id = next_revision(next_id)?;
    }
    transaction
        .execute(
            "UPDATE structured_memory_meta
             SET next_record_id = ?1, next_event_id = ?1
             WHERE singleton = 1",
            [next_id],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn write_portable_task18_domains(
    transaction: &Transaction<'_>,
    scheduled_items: &[ScheduledItemV1],
    memory_records: &[MemoryRecordV1],
    timestamp: i64,
) -> PersistenceResult<()> {
    for (position, item) in scheduled_items.iter().enumerate() {
        let subject_agent_id = match item.subject_agent_id {
            Some(id) if database_agent_exists(transaction, id)? => Some(id),
            _ => None,
        };
        let workspace_id = match item.workspace_id.as_deref() {
            Some(id)
                if transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                        [id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(PersistenceError::database)? =>
            {
                Some(id)
            }
            _ => None,
        };
        let task_link_valid = match (item.task_owner_agent_id, item.task_id) {
            (Some(owner), Some(task)) => transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_tasks
                     WHERE owner_agent_id = ?1 AND id = ?2)",
                    params![owner, task],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(PersistenceError::database)?,
            _ => false,
        };
        transaction
            .execute(
                "INSERT INTO reminders
                 (id, position, revision, kind, title, notes, local_due_at, time_zone,
                  due_at, due_at_unix_ms, event_end_local, event_end_unix_ms,
                  dst_resolution, status, recurrence_kind, recurrence_interval,
                  recurrence_limit, recurrence_until_unix_ms, next_occurrence_sequence,
                  missed_occurrence_count, delivery_mode, privacy_mode,
                  subject_agent_id, workspace_id, task_owner_agent_id, task_id,
                  scheduler_agent_id, schedule_issue_code, schedule_issue_message,
                  created_at, created_at_unix_ms, resolved_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                         ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, 'in_app', ?21,
                         ?22, ?23, ?24, ?25,
                         (SELECT id FROM agents WHERE template_key = 'event-reminder'
                          AND registry_state = 'active' ORDER BY id LIMIT 1),
                         ?26, ?27, ?28, ?29, ?30, ?31)",
                params![
                    item.id,
                    position as i64,
                    item.revision,
                    item.kind.as_storage_value(),
                    item.title,
                    item.notes,
                    item.local_due_at,
                    item.time_zone,
                    item.due_at,
                    item.due_at_unix_ms,
                    item.event_end_local,
                    item.event_end_unix_ms,
                    item.dst_resolution.as_storage_value(),
                    item.status.as_storage_value(),
                    item.recurrence.kind.as_storage_value(),
                    item.recurrence.interval,
                    item.recurrence.occurrence_limit,
                    item.recurrence.until_unix_ms,
                    item.next_occurrence_sequence,
                    item.missed_occurrence_count,
                    item.privacy_mode.as_storage_value(),
                    subject_agent_id,
                    workspace_id,
                    task_link_valid
                        .then_some(item.task_owner_agent_id)
                        .flatten(),
                    task_link_valid.then_some(item.task_id).flatten(),
                    item.schedule_issue_code,
                    item.schedule_issue_message,
                    item.created_at,
                    item.created_at_unix_ms,
                    item.resolved_at_unix_ms,
                    item.updated_at_unix_ms,
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    let next_reminder_id = scheduled_items
        .iter()
        .map(|item| item.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            PersistenceError::new(
                "REMINDER_CAPACITY_EXCEEDED",
                "The imported reminder identifier allocator is exhausted.",
                false,
            )
        })?
        .max(1);
    transaction
        .execute(
            "UPDATE reminder_scheduler_meta SET next_reminder_id = ?1 WHERE singleton = 1",
            [next_reminder_id],
        )
        .map_err(PersistenceError::database)?;

    let mut next_event_id = 1_i64;
    for record in memory_records {
        // Portable memory retains exact historical scope identifiers even when the
        // referenced agent, project, or task is no longer active. New records still
        // require live references, and prompt selection cannot match an absent scope.
        transaction
            .execute(
                "INSERT INTO memory_records
                 (id, scope_kind, agent_id, workspace_id, task_owner_agent_id, task_id,
                  team_leader_agent_id, record_kind, content, provenance_kind,
                  provenance_ref, revision, retention_policy, expires_at_unix_ms,
                  created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'backup_import',
                         ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    record.id,
                    record.scope.kind.as_storage_value(),
                    record.scope.agent_id,
                    record.scope.workspace_id,
                    record.scope.task_owner_agent_id,
                    record.scope.task_id,
                    record.scope.team_leader_agent_id,
                    record.kind.as_storage_value(),
                    record.content,
                    format!("backup-v4:{}", record.provenance.as_storage_value()),
                    record.revision,
                    record.retention.as_storage_value(),
                    record.expires_at_unix_ms,
                    record.created_at_unix_ms,
                    record.updated_at_unix_ms,
                ],
            )
            .map_err(PersistenceError::database)?;
        transaction
            .execute(
                "INSERT INTO memory_events
                 (id, record_id, action, actor_kind, record_revision, created_at_unix_ms)
                 VALUES (?1, ?2, 'created', 'import', ?3, ?4)",
                params![next_event_id, record.id, record.revision, timestamp],
            )
            .map_err(PersistenceError::database)?;
        next_event_id = next_revision(next_event_id)?;
    }
    let next_record_id = memory_records
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            PersistenceError::new(
                "MEMORY_CAPACITY_EXCEEDED",
                "The imported memory identifier allocator is exhausted.",
                false,
            )
        })?
        .max(1);
    transaction
        .execute(
            "UPDATE structured_memory_meta
             SET next_record_id = ?1, next_event_id = ?2 WHERE singleton = 1",
            params![next_record_id, next_event_id],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn database_agent_exists(connection: &Connection, agent_id: i64) -> PersistenceResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            [agent_id],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)
}

fn write_preferences(
    transaction: &Transaction<'_>,
    preferences: &AppPreferences,
) -> PersistenceResult<()> {
    let default = &preferences.default_performance;
    transaction
        .execute(
            "INSERT INTO preferences
             (singleton, theme, accent_color, density, reduced_motion, default_model,
              active_ai_provider, default_agent_status, default_task_category,
              default_task_priority, default_strength, default_focus, default_cpu_limit,
              default_gpu_limit, default_overflow_action, default_redirect_agent_id,
              workspace_path, active_workspace_id, agent_timeout_minutes, safety_mode,
              approval_expiry_minutes, default_routing_mode, review_mode,
              background_voice_enabled, voice_control_master_enabled, voice_wake_phrase,
              voice_deactivate_phrase, voice_open_phrases, voice_close_phrases,
              voice_command_replacements, voice_state, default_queue_threshold)
             VALUES
             (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
              ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27,
              ?28, ?29, ?30, ?31)",
            params![
                preferences.theme,
                preferences.accent_color,
                preferences.density,
                preferences.reduced_motion as i64,
                preferences.default_model,
                preferences.active_ai_provider,
                preferences.default_agent_status,
                preferences.default_task_category,
                preferences.default_task_priority,
                default.strength,
                default.focus,
                default.cpu_limit,
                default.gpu_limit,
                default.overflow_action,
                default.redirect_agent_id,
                preferences.workspace_path,
                preferences.active_workspace_id,
                preferences.agent_timeout_minutes,
                preferences.safety_mode,
                preferences.approval_expiry_minutes,
                preferences.default_routing_mode,
                preferences.review_mode,
                preferences.background_voice_enabled as i64,
                preferences.voice_control_master_enabled as i64,
                preferences.voice_wake_phrase,
                preferences.voice_deactivate_phrase,
                preferences.voice_open_phrases,
                preferences.voice_close_phrases,
                preferences.voice_command_replacements,
                preferences.voice_state,
                default.queue_threshold
            ],
        )
        .map_err(PersistenceError::database)?;

    for (position, workspace) in preferences.workspaces.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO workspaces (id, position, name, path) VALUES (?1, ?2, ?3, ?4)",
                params![
                    workspace.id,
                    position as i64,
                    workspace.name,
                    workspace.path
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

fn write_agent(
    transaction: &Transaction<'_>,
    position: usize,
    agent: &Agent,
    existing_task_timestamps: &ExistingTaskTimestamps,
    existing_activity_timestamps: &ExistingActivityTimestamps,
    lifecycle_timestamp: i64,
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO agents
             (id, template_key, registry_state, registry_issue, deleted_at_unix_ms,
              position, name, description, status, role, category, reports_to,
              authority_level, model, memory, strength, focus, cpu_limit, gpu_limit,
              overflow_action, redirect_agent_id, capability_files, capability_internet,
              capability_clipboard, capability_terminal, capability_system, approval_files,
              approval_internet, approval_clipboard, approval_terminal, approval_system,
              queue_threshold)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                     ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)",
            params![
                agent.id,
                agent.template_key,
                agent.registry_state,
                agent.registry_issue,
                agent.deleted_at_unix_ms,
                position as i64,
                agent.name,
                agent.description,
                agent.status,
                agent.role,
                agent.category,
                agent.reports_to,
                agent.authority_level,
                agent.model,
                "",
                agent.performance.strength,
                agent.performance.focus,
                agent.performance.cpu_limit,
                agent.performance.gpu_limit,
                agent.performance.overflow_action,
                agent.performance.redirect_agent_id,
                agent.capabilities.files,
                agent.capabilities.internet,
                agent.capabilities.clipboard,
                agent.capabilities.terminal,
                agent.capabilities.system,
                agent.approvals.files,
                agent.approvals.internet,
                agent.approvals.clipboard,
                agent.approvals.terminal,
                agent.approvals.system,
                agent.performance.queue_threshold
            ],
        )
        .map_err(PersistenceError::database)?;

    for (task_position, task) in agent.tasks.iter().enumerate() {
        write_task(
            transaction,
            agent.id,
            task_position,
            task,
            existing_task_timestamps
                .get(&(agent.id, task.id))
                .copied()
                .unwrap_or((None, None)),
            lifecycle_timestamp,
        )?;
    }
    for (activity_position, entry) in agent.activity.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO agent_activity
                 (owner_agent_id, id, position, message, created_at, created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5,
                         COALESCE(?6, CAST(strftime('%s', ?5) AS INTEGER) * 1000, ?7))",
                params![
                    agent.id,
                    entry.id,
                    activity_position as i64,
                    entry.message,
                    entry.created_at,
                    existing_activity_timestamps
                        .get(&(agent.id, entry.id))
                        .copied()
                        .flatten(),
                    lifecycle_timestamp
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

fn write_task(
    transaction: &Transaction<'_>,
    owner_agent_id: i64,
    position: usize,
    task: &AgentTask,
    existing_timestamps: (Option<i64>, Option<i64>),
    lifecycle_timestamp: i64,
) -> PersistenceResult<()> {
    let routing_evidence_json = task
        .routing_evidence
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| {
            PersistenceError::new(
                "ROUTING_EVIDENCE_INVALID",
                "Task routing evidence could not be normalized.",
                false,
            )
        })?;
    let workspace_evidence_json = task
        .workspace_changes
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| {
            PersistenceError::new(
                "INVALID_WORKSPACE_EVIDENCE",
                "Task workspace evidence could not be normalized.",
                false,
            )
        })?;
    let specialist_request_json = task
        .specialist_request
        .as_ref()
        .map(|request| request.canonical_json())
        .transpose()
        .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
    transaction
        .execute(
            "INSERT INTO agent_tasks
             (owner_agent_id, id, position, title, category, priority, assigned_agent_id,
              status, phase, created_at, completed_at, result, response_id, runtime_model,
              total_tokens, workspace_id, diff, duration_seconds, routing_mode,
              routed_from_agent_id, routing_reason, review_agent_id, review_status,
              review_result, review_model, review_duration_seconds, reviewed_at,
              queue_state, enqueue_sequence, routing_evidence_json, workspace_evidence_json,
              specialist_request_json, created_at_unix_ms, completed_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                     ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32,
                     COALESCE(?33, CAST(strftime('%s', ?10) AS INTEGER) * 1000, ?35),
                     CASE WHEN ?11 IS NULL THEN NULL
                          ELSE COALESCE(?34, CAST(strftime('%s', ?11) AS INTEGER) * 1000, ?35)
                     END)",
            params![
                owner_agent_id,
                task.id,
                position as i64,
                task.title,
                task.category,
                task.priority,
                task.assigned_agent_id,
                task.status,
                task.phase,
                task.created_at,
                task.completed_at,
                task.result,
                task.response_id,
                task.runtime_model,
                task.total_tokens,
                task.workspace_id,
                task.diff,
                task.duration_seconds,
                task.routing_mode,
                task.routed_from_agent_id,
                task.routing_reason,
                task.review_agent_id,
                task.review_status,
                task.review_result,
                task.review_model,
                task.review_duration_seconds,
                task.reviewed_at,
                task.queue_state,
                task.enqueue_sequence,
                routing_evidence_json,
                workspace_evidence_json,
                specialist_request_json,
                existing_timestamps.0,
                existing_timestamps.1,
                lifecycle_timestamp
            ],
        )
        .map_err(PersistenceError::database)?;
    for (changed_file_position, changed_file) in task.changed_files.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO task_changed_files (owner_agent_id, task_id, position, path)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    owner_agent_id,
                    task.id,
                    changed_file_position as i64,
                    changed_file
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

fn write_approval_request(
    transaction: &Transaction<'_>,
    position: usize,
    request: &ApprovalRequest,
    origin: &str,
    lifecycle_timestamp: i64,
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO approval_requests
             (id, position, agent_id, task_id, title, reason, status, created_at,
              resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
              consumed_at, origin, authoritative, created_at_unix_ms,
              resolved_at_unix_ms, expires_at_unix_ms, consumed_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, 0,
                     COALESCE(CAST(strftime('%s', ?8) AS INTEGER) * 1000, ?16),
                     CASE WHEN ?7 IN ('Denied', 'Expired') OR ?14 IS NOT NULL
                          THEN COALESCE(
                              CAST(strftime('%s', ?9) AS INTEGER) * 1000,
                              CAST(strftime('%s', ?14) AS INTEGER) * 1000,
                              ?16
                          ) ELSE NULL END,
                     COALESCE(CAST(strftime('%s', ?13) AS INTEGER) * 1000, ?16),
                     CAST(strftime('%s', ?14) AS INTEGER) * 1000)",
            params![
                request.id,
                position as i64,
                request.agent_id,
                request.task_id,
                request.title,
                request.reason,
                request.status,
                request.created_at,
                request.resolved_at,
                request.risk_level,
                request.workspace_id,
                request.task_snapshot,
                request.expires_at,
                request.consumed_at,
                origin,
                lifecycle_timestamp
            ],
        )
        .map_err(PersistenceError::database)?;
    for (scope_position, scope) in request.scopes.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO approval_scopes (approval_id, position, scope)
                 VALUES (?1, ?2, ?3)",
                params![request.id, scope_position as i64, scope],
            )
            .map_err(PersistenceError::database)?;
    }
    Ok(())
}

fn read_application_state(connection: &Connection) -> PersistenceResult<ApplicationState> {
    Ok(ApplicationState {
        agents: read_agents(connection)?,
        models: read_models(connection)?,
        approval_requests: read_approval_requests(connection)?,
        reminders: read_reminders(connection)?,
        task_retention_days: read_retention(connection, "task_retention")?,
        activity_retention_days: read_retention(connection, "activity_retention")?,
        preferences: read_preferences(connection)?,
    })
}

fn read_retention(
    connection: &Connection,
    column: &str,
) -> PersistenceResult<HistoryRetentionDays> {
    let sql = format!("SELECT {column} FROM retention_settings WHERE singleton = 1");
    let value: String = connection
        .query_row(&sql, [], |row| row.get(0))
        .map_err(PersistenceError::database)?;
    HistoryRetentionDays::from_storage_value(&value).map_err(PersistenceError::validation)
}

fn read_preferences(connection: &Connection) -> PersistenceResult<AppPreferences> {
    let workspaces = read_workspaces(connection)?;
    connection
        .query_row(
            "SELECT theme, accent_color, density, reduced_motion, default_model,
                    active_ai_provider, default_agent_status, default_task_category,
                    default_task_priority, default_strength, default_focus, default_cpu_limit,
                    default_gpu_limit, default_overflow_action, default_redirect_agent_id,
                    workspace_path, active_workspace_id, agent_timeout_minutes, safety_mode,
                    approval_expiry_minutes, default_routing_mode, review_mode,
                    background_voice_enabled, voice_control_master_enabled, voice_wake_phrase,
                    voice_deactivate_phrase, voice_open_phrases, voice_close_phrases,
                    voice_command_replacements, voice_state, default_queue_threshold
             FROM preferences WHERE singleton = 1",
            [],
            |row| {
                Ok(AppPreferences {
                    theme: row.get(0)?,
                    accent_color: row.get(1)?,
                    density: row.get(2)?,
                    reduced_motion: row.get::<_, i64>(3)? != 0,
                    default_model: row.get(4)?,
                    active_ai_provider: row.get(5)?,
                    default_agent_status: row.get(6)?,
                    default_task_category: row.get(7)?,
                    default_task_priority: row.get(8)?,
                    default_performance: AgentPerformance {
                        strength: row.get(9)?,
                        focus: row.get(10)?,
                        cpu_limit: row.get(11)?,
                        gpu_limit: row.get(12)?,
                        queue_threshold: row.get(30)?,
                        overflow_action: row.get(13)?,
                        redirect_agent_id: row.get(14)?,
                    },
                    workspace_path: row.get(15)?,
                    workspaces,
                    active_workspace_id: row.get(16)?,
                    agent_timeout_minutes: row.get(17)?,
                    safety_mode: row.get(18)?,
                    approval_expiry_minutes: row.get(19)?,
                    default_routing_mode: row.get(20)?,
                    review_mode: row.get(21)?,
                    background_voice_enabled: row.get::<_, i64>(22)? != 0,
                    voice_control_master_enabled: row.get::<_, i64>(23)? != 0,
                    voice_wake_phrase: row.get(24)?,
                    voice_deactivate_phrase: row.get(25)?,
                    voice_open_phrases: row.get(26)?,
                    voice_close_phrases: row.get(27)?,
                    voice_command_replacements: row.get(28)?,
                    voice_state: row.get(29)?,
                })
            },
        )
        .map_err(PersistenceError::database)
}

fn read_workspaces(connection: &Connection) -> PersistenceResult<Vec<WorkspaceDefinition>> {
    let mut statement = connection
        .prepare("SELECT id, name, path FROM workspaces ORDER BY position")
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([], |row| {
        Ok(WorkspaceDefinition {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
        })
    }))
}

fn read_agents(connection: &Connection) -> PersistenceResult<Vec<Agent>> {
    let mut statement = connection
        .prepare(
            "SELECT id, template_key, registry_state, registry_issue, deleted_at_unix_ms,
                    name, description, status, role, category, reports_to,
                    authority_level, model, memory, strength, focus, cpu_limit, gpu_limit,
                    overflow_action, redirect_agent_id, capability_files, capability_internet,
                    capability_clipboard, capability_terminal, capability_system, approval_files,
                    approval_internet, approval_clipboard, approval_terminal, approval_system,
                    queue_threshold
             FROM agents ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok(Agent {
                id: row.get(0)?,
                template_key: row.get(1)?,
                registry_state: row.get(2)?,
                registry_issue: row.get(3)?,
                deleted_at_unix_ms: row.get(4)?,
                name: row.get(5)?,
                description: row.get(6)?,
                status: row.get(7)?,
                role: row.get(8)?,
                category: row.get(9)?,
                reports_to: row.get(10)?,
                authority_level: row.get(11)?,
                model: row.get(12)?,
                memory: String::new(),
                tasks: Vec::new(),
                activity: Vec::new(),
                performance: AgentPerformance {
                    strength: row.get(14)?,
                    focus: row.get(15)?,
                    cpu_limit: row.get(16)?,
                    gpu_limit: row.get(17)?,
                    queue_threshold: row.get(30)?,
                    overflow_action: row.get(18)?,
                    redirect_agent_id: row.get(19)?,
                },
                capabilities: AgentCapabilities {
                    files: row.get(20)?,
                    internet: row.get(21)?,
                    clipboard: row.get(22)?,
                    terminal: row.get(23)?,
                    system: row.get(24)?,
                },
                approvals: AgentApprovals {
                    files: row.get(25)?,
                    internet: row.get(26)?,
                    clipboard: row.get(27)?,
                    terminal: row.get(28)?,
                    system: row.get(29)?,
                },
            })
        })
        .map_err(PersistenceError::database)?;
    let mut agents = Vec::new();
    for row in rows {
        let mut agent = row.map_err(PersistenceError::database)?;
        agent.tasks = read_tasks(connection, agent.id)?;
        agent.activity = read_activity(connection, agent.id)?;
        agents.push(agent);
    }
    Ok(agents)
}

fn read_tasks(connection: &Connection, owner_agent_id: i64) -> PersistenceResult<Vec<AgentTask>> {
    let mut statement = connection
        .prepare(
            "SELECT id, title, category, priority, assigned_agent_id, status, phase,
                    created_at, completed_at, result, response_id, runtime_model, total_tokens,
                    workspace_id, diff, duration_seconds, routing_mode, routed_from_agent_id,
                    routing_reason, review_agent_id, review_status, review_result, review_model,
                    review_duration_seconds, reviewed_at, queue_state, enqueue_sequence,
                    routing_evidence_json, workspace_evidence_json, specialist_request_json
             FROM agent_tasks WHERE owner_agent_id = ?1 ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([owner_agent_id], |row| {
            let routing_evidence_json: Option<String> = row.get(27)?;
            let routing_evidence = routing_evidence_json
                .map(|json| {
                    serde_json::from_str(&json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            27,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
                })
                .transpose()?;
            let workspace_evidence_json: Option<String> = row.get(28)?;
            let workspace_changes = Some(
                workspace_evidence_json
                    .map(|json| {
                        serde_json::from_str(&json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                28,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })
                    })
                    .transpose()?
                    .unwrap_or_else(|| {
                        WorkspaceChangeEvidenceV1::legacy_unavailable(
                            "This task predates structured workspace evidence persistence.",
                        )
                    }),
            );
            let specialist_request_json: Option<String> = row.get(29)?;
            let specialist_request = specialist_request_json
                .map(|json| {
                    parse_specialist_request_json(&json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            29,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
                })
                .transpose()?;
            Ok(AgentTask {
                id: row.get(0)?,
                title: row.get(1)?,
                category: row.get(2)?,
                priority: row.get(3)?,
                assigned_agent_id: row.get(4)?,
                status: row.get(5)?,
                phase: row.get(6)?,
                created_at: row.get(7)?,
                completed_at: row.get(8)?,
                result: row.get(9)?,
                response_id: row.get(10)?,
                runtime_model: row.get(11)?,
                total_tokens: row.get(12)?,
                workspace_id: row.get(13)?,
                specialist_request,
                changed_files: Vec::new(),
                diff: row.get(14)?,
                workspace_changes,
                duration_seconds: row.get(15)?,
                routing_mode: row.get(16)?,
                routed_from_agent_id: row.get(17)?,
                routing_reason: row.get(18)?,
                queue_state: row.get(25)?,
                enqueue_sequence: row.get(26)?,
                routing_evidence,
                review_agent_id: row.get(19)?,
                review_status: row.get(20)?,
                review_result: row.get(21)?,
                review_model: row.get(22)?,
                review_duration_seconds: row.get(23)?,
                reviewed_at: row.get(24)?,
            })
        })
        .map_err(PersistenceError::database)?;
    let mut tasks = Vec::new();
    for row in rows {
        let mut task = row.map_err(PersistenceError::database)?;
        task.changed_files = read_changed_files(connection, owner_agent_id, task.id)?;
        tasks.push(task);
    }
    Ok(tasks)
}

fn read_changed_files(
    connection: &Connection,
    owner_agent_id: i64,
    task_id: i64,
) -> PersistenceResult<Vec<String>> {
    let mut statement = connection
        .prepare(
            "SELECT path FROM task_changed_files
             WHERE owner_agent_id = ?1 AND task_id = ?2 ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map(params![owner_agent_id, task_id], |row| row.get(0)))
}

fn read_activity(
    connection: &Connection,
    owner_agent_id: i64,
) -> PersistenceResult<Vec<ActivityEntry>> {
    let mut statement = connection
        .prepare(
            "SELECT id, message, created_at FROM agent_activity
             WHERE owner_agent_id = ?1 ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([owner_agent_id], |row| {
        Ok(ActivityEntry {
            id: row.get(0)?,
            message: row.get(1)?,
            created_at: row.get(2)?,
        })
    }))
}

fn read_models(connection: &Connection) -> PersistenceResult<Vec<ModelDefinition>> {
    let mut statement = connection
        .prepare("SELECT id, name, provider FROM models ORDER BY position")
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([], |row| {
        Ok(ModelDefinition {
            id: row.get(0)?,
            name: row.get(1)?,
            provider: row.get(2)?,
        })
    }))
}

fn read_approval_requests(connection: &Connection) -> PersistenceResult<Vec<ApprovalRequest>> {
    let mut statement = connection
        .prepare(
            "SELECT id, agent_id, task_id, title, reason, status, created_at, resolved_at,
                    risk_level, workspace_id, task_snapshot, expires_at, consumed_at
             FROM approval_requests ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok(ApprovalRequest {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                task_id: row.get(2)?,
                title: row.get(3)?,
                reason: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
                resolved_at: row.get(7)?,
                risk_level: row.get(8)?,
                scopes: Vec::new(),
                workspace_id: row.get(9)?,
                task_snapshot: row.get(10)?,
                expires_at: row.get(11)?,
                consumed_at: row.get(12)?,
            })
        })
        .map_err(PersistenceError::database)?;
    let mut requests = Vec::new();
    for row in rows {
        let mut request = row.map_err(PersistenceError::database)?;
        request.scopes = read_approval_scopes(connection, request.id)?;
        requests.push(request);
    }
    Ok(requests)
}

fn read_approval_request(
    connection: &Connection,
    approval_id: i64,
) -> PersistenceResult<ApprovalRequest> {
    let mut request = connection
        .query_row(
            "SELECT id, agent_id, task_id, title, reason, status, created_at, resolved_at,
                    risk_level, workspace_id, task_snapshot, expires_at, consumed_at
             FROM approval_requests WHERE id = ?1",
            [approval_id],
            |row| {
                Ok(ApprovalRequest {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    task_id: row.get(2)?,
                    title: row.get(3)?,
                    reason: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                    resolved_at: row.get(7)?,
                    risk_level: row.get(8)?,
                    scopes: Vec::new(),
                    workspace_id: row.get(9)?,
                    task_snapshot: row.get(10)?,
                    expires_at: row.get(11)?,
                    consumed_at: row.get(12)?,
                })
            },
        )
        .map_err(PersistenceError::database)?;
    request.scopes = read_approval_scopes(connection, approval_id)?;
    Ok(request)
}

fn read_approval_scopes(
    connection: &Connection,
    approval_id: i64,
) -> PersistenceResult<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT scope FROM approval_scopes WHERE approval_id = ?1 ORDER BY position")
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([approval_id], |row| row.get(0)))
}

fn read_reminder_scheduler_snapshot(
    connection: &Connection,
) -> PersistenceResult<ReminderSchedulerSnapshot> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM reminder_scheduler_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let application_state_revision: i64 = connection
        .query_row(
            "SELECT state_revision FROM application_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let mut statement = connection
        .prepare(
            "SELECT id, position, revision, kind, title, notes, local_due_at, time_zone,
                    due_at, due_at_unix_ms, event_end_local, event_end_unix_ms,
                    dst_resolution, status, recurrence_kind, recurrence_interval,
                    recurrence_limit, recurrence_until_unix_ms, next_occurrence_sequence,
                    missed_occurrence_count, delivery_mode, privacy_mode, schedule_fingerprint,
                    subject_agent_id, workspace_id, task_owner_agent_id, task_id,
                    scheduler_agent_id, schedule_issue_code, schedule_issue_message,
                    created_at, created_at_unix_ms, resolved_at_unix_ms, updated_at_unix_ms
             FROM reminders ORDER BY position, id",
        )
        .map_err(PersistenceError::database)?;
    let items = collect_rows(statement.query_map([], |row| {
        let kind: String = row.get(3)?;
        let dst_resolution: String = row.get(12)?;
        let status: String = row.get(13)?;
        let recurrence_kind: String = row.get(14)?;
        let delivery_mode: String = row.get(20)?;
        let privacy_mode: String = row.get(21)?;
        Ok(ScheduledItemV1 {
            id: row.get(0)?,
            position: row.get(1)?,
            revision: row.get(2)?,
            kind: ScheduledItemKind::from_storage_value(&kind)
                .map_err(|error| sql_text_conversion_error(3, error))?,
            title: row.get(4)?,
            notes: row.get(5)?,
            local_due_at: row.get(6)?,
            time_zone: row.get(7)?,
            due_at: row.get(8)?,
            due_at_unix_ms: row.get(9)?,
            event_end_local: row.get(10)?,
            event_end_unix_ms: row.get(11)?,
            dst_resolution: DstResolution::from_storage_value(&dst_resolution)
                .map_err(|error| sql_text_conversion_error(12, error))?,
            status: ScheduleStatus::from_storage_value(&status)
                .map_err(|error| sql_text_conversion_error(13, error))?,
            recurrence: RecurrenceRuleV1 {
                kind: RecurrenceKind::from_storage_value(&recurrence_kind)
                    .map_err(|error| sql_text_conversion_error(14, error))?,
                interval: row.get(15)?,
                occurrence_limit: row.get(16)?,
                until_unix_ms: row.get(17)?,
            },
            next_occurrence_sequence: row.get(18)?,
            missed_occurrence_count: row.get(19)?,
            delivery_mode: DeliveryMode::from_storage_value(&delivery_mode)
                .map_err(|error| sql_text_conversion_error(20, error))?,
            privacy_mode: PrivacyMode::from_storage_value(&privacy_mode)
                .map_err(|error| sql_text_conversion_error(21, error))?,
            schedule_fingerprint: row.get(22)?,
            subject_agent_id: row.get(23)?,
            workspace_id: row.get(24)?,
            task_owner_agent_id: row.get(25)?,
            task_id: row.get(26)?,
            scheduler_agent_id: row.get(27)?,
            schedule_issue_code: row.get(28)?,
            schedule_issue_message: row.get(29)?,
            created_at: row.get(30)?,
            created_at_unix_ms: row.get(31)?,
            resolved_at_unix_ms: row.get(32)?,
            updated_at_unix_ms: row.get(33)?,
        })
    }))?;
    if items.len() > MAX_SCHEDULED_ITEMS {
        return Err(PersistenceError::new(
            "REMINDER_STORAGE_INVALID",
            "The stored reminder count exceeds the supported limit.",
            false,
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT id, reminder_id, schedule_revision, occurrence_sequence, occurrence_key,
                    due_at_unix_ms, status, missed_count, first_missed_at_unix_ms,
                    last_missed_at_unix_ms, detail_code, detail_message,
                    created_at_unix_ms, updated_at_unix_ms
             FROM reminder_occurrences ORDER BY updated_at_unix_ms DESC, id DESC LIMIT 500",
        )
        .map_err(PersistenceError::database)?;
    let recent_occurrences = collect_rows(statement.query_map([], |row| {
        Ok(ReminderOccurrenceV1 {
            id: row.get(0)?,
            reminder_id: row.get(1)?,
            schedule_revision: row.get(2)?,
            occurrence_sequence: row.get(3)?,
            occurrence_key: row.get(4)?,
            due_at_unix_ms: row.get(5)?,
            status: row.get(6)?,
            missed_count: row.get(7)?,
            first_missed_at_unix_ms: row.get(8)?,
            last_missed_at_unix_ms: row.get(9)?,
            detail_code: row.get(10)?,
            detail_message: row.get(11)?,
            created_at_unix_ms: row.get(12)?,
            updated_at_unix_ms: row.get(13)?,
        })
    }))?;
    Ok(ReminderSchedulerSnapshot {
        revision,
        application_state_revision,
        system_time_zone: system_time_zone_name(),
        items,
        recent_occurrences,
    })
}

fn read_structured_memory_snapshot(
    connection: &Connection,
) -> PersistenceResult<StructuredMemorySnapshot> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM structured_memory_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let application_state_revision: i64 = connection
        .query_row(
            "SELECT state_revision FROM application_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let mut statement = connection
        .prepare(
            "SELECT id, scope_kind, agent_id, workspace_id, task_owner_agent_id, task_id,
                    team_leader_agent_id, record_kind, content, provenance_kind,
                    provenance_ref, revision, retention_policy, expires_at_unix_ms,
                    created_at_unix_ms, updated_at_unix_ms
             FROM memory_records ORDER BY updated_at_unix_ms DESC, id DESC",
        )
        .map_err(PersistenceError::database)?;
    let records = collect_rows(statement.query_map([], |row| {
        let scope_kind: String = row.get(1)?;
        let record_kind: String = row.get(7)?;
        let provenance: String = row.get(9)?;
        let retention: String = row.get(12)?;
        let record = MemoryRecordV1 {
            id: row.get(0)?,
            scope: MemoryScopeV1 {
                kind: MemoryScopeKind::from_storage_value(&scope_kind)
                    .map_err(|error| sql_text_conversion_error(1, error))?,
                agent_id: row.get(2)?,
                workspace_id: row.get(3)?,
                task_owner_agent_id: row.get(4)?,
                task_id: row.get(5)?,
                team_leader_agent_id: row.get(6)?,
            },
            kind: MemoryRecordKind::from_storage_value(&record_kind)
                .map_err(|error| sql_text_conversion_error(7, error))?,
            content: row.get(8)?,
            provenance: MemoryProvenanceKind::from_storage_value(&provenance)
                .map_err(|error| sql_text_conversion_error(9, error))?,
            provenance_ref: row.get(10)?,
            revision: row.get(11)?,
            retention: MemoryRetentionPolicy::from_storage_value(&retention)
                .map_err(|error| sql_text_conversion_error(12, error))?,
            expires_at_unix_ms: row.get(13)?,
            created_at_unix_ms: row.get(14)?,
            updated_at_unix_ms: row.get(15)?,
        };
        crate::structured_memory::validate_memory_record(&record)
            .map_err(|error| sql_text_conversion_error(8, error))?;
        Ok(record)
    }))?;
    if records.len() > MAX_MEMORY_RECORDS {
        return Err(PersistenceError::new(
            "MEMORY_STORAGE_INVALID",
            "The stored memory count exceeds the supported limit.",
            false,
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT id, record_id, action, actor_kind, record_revision, created_at_unix_ms
             FROM memory_events ORDER BY created_at_unix_ms DESC, id DESC LIMIT 500",
        )
        .map_err(PersistenceError::database)?;
    let recent_events = collect_rows(statement.query_map([], |row| {
        Ok(MemoryEventV1 {
            id: row.get(0)?,
            record_id: row.get(1)?,
            action: row.get(2)?,
            actor_kind: row.get(3)?,
            record_revision: row.get(4)?,
            created_at_unix_ms: row.get(5)?,
        })
    }))?;
    Ok(StructuredMemorySnapshot {
        revision,
        application_state_revision,
        records,
        recent_events,
    })
}

fn read_management_handoff_snapshot(
    connection: &Connection,
) -> PersistenceResult<ManagementHandoffSnapshot> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM management_handoff_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let application_state_revision: i64 = connection
        .query_row(
            "SELECT state_revision FROM application_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::database)?;
    let mut statement = connection
        .prepare(
            "SELECT id, task_owner_agent_id, task_id, kind, from_agent_id, to_agent_id,
                    owner_role, revision_round, run_attempt_id, review_flow_id,
                    review_stage_attempt_id, source_kind, summary, payload_json,
                    idempotency_key, created_at_unix_ms
             FROM management_handoffs ORDER BY created_at_unix_ms, id",
        )
        .map_err(PersistenceError::database)?;
    let handoffs = collect_rows(statement.query_map([], |row| {
        let kind: String = row.get(3)?;
        let role: String = row.get(6)?;
        let source: String = row.get(11)?;
        let payload_json: String = row.get(13)?;
        let payload = serde_json::from_str(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let handoff = ManagementHandoffV1 {
            id: row.get(0)?,
            task_owner_agent_id: row.get(1)?,
            task_id: row.get(2)?,
            kind: ManagementHandoffKind::from_storage_value(&kind)
                .map_err(|error| sql_text_conversion_error(3, error))?,
            from_agent_id: row.get(4)?,
            to_agent_id: row.get(5)?,
            owner_role: ManagementOwnerRole::from_storage_value(&role)
                .map_err(|error| sql_text_conversion_error(6, error))?,
            revision_round: row.get(7)?,
            run_attempt_id: row.get(8)?,
            review_flow_id: row.get(9)?,
            review_stage_attempt_id: row.get(10)?,
            source: ManagementHandoffSource::from_storage_value(&source)
                .map_err(|error| sql_text_conversion_error(11, error))?,
            summary: row.get(12)?,
            payload,
            idempotency_key: row.get(14)?,
            created_at_unix_ms: row.get(15)?,
        };
        crate::management_handoffs::validate_handoff(&handoff)
            .map_err(|error| sql_text_conversion_error(13, error))?;
        Ok(handoff)
    }))?;
    crate::management_handoffs::validate_sequential_handoffs(&handoffs)
        .map_err(|error| PersistenceError::new(error.code, error.message, false))?;
    Ok(ManagementHandoffSnapshot {
        revision,
        application_state_revision,
        handoffs,
    })
}

fn sql_text_conversion_error(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

fn read_reminders(connection: &Connection) -> PersistenceResult<Vec<Reminder>> {
    let mut statement = connection
        .prepare(
            "SELECT id, title, notes, due_at,
                    CASE status
                        WHEN 'completed' THEN 'Completed'
                        WHEN 'dismissed' THEN 'Dismissed'
                        ELSE 'Upcoming'
                    END,
                    subject_agent_id, task_id, created_at
             FROM reminders ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    collect_rows(statement.query_map([], |row| {
        Ok(Reminder {
            id: row.get(0)?,
            title: row.get(1)?,
            notes: row.get(2)?,
            due_at: row.get(3)?,
            status: row.get(4)?,
            agent_id: row.get(5)?,
            task_id: row.get(6)?,
            created_at: row.get(7)?,
        })
    }))
}

const SYSTEM_ACTION_AUDIT_SELECT_BY_REQUEST: &str =
    "SELECT id, request_id, request_fingerprint, intent_kind, risk_class, target_kind,
            target_id, agent_id, task_owner_agent_id, task_id, approval_id,
            authorization_kind, intent_fingerprint_sha256, policy_fingerprint_sha256,
            status, detail_code, detail_message, content_sha256, content_length,
            created_at_unix_ms, updated_at_unix_ms
     FROM system_action_audits WHERE request_id = ?1";

const SYSTEM_ACTION_AUDIT_SELECT_BY_ID: &str =
    "SELECT id, request_id, request_fingerprint, intent_kind, risk_class, target_kind,
            target_id, agent_id, task_owner_agent_id, task_id, approval_id,
            authorization_kind, intent_fingerprint_sha256, policy_fingerprint_sha256,
            status, detail_code, detail_message, content_sha256, content_length,
            created_at_unix_ms, updated_at_unix_ms
     FROM system_action_audits WHERE id = ?1";

const SYSTEM_ACTION_AUDIT_SELECT_RECENT: &str =
    "SELECT id, request_id, request_fingerprint, intent_kind, risk_class, target_kind,
            target_id, agent_id, task_owner_agent_id, task_id, approval_id,
            authorization_kind, intent_fingerprint_sha256, policy_fingerprint_sha256,
            status, detail_code, detail_message, content_sha256, content_length,
            created_at_unix_ms, updated_at_unix_ms
     FROM system_action_audits ORDER BY id DESC LIMIT ?1";

fn read_system_action_audit_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SystemActionAuditRecord> {
    Ok(SystemActionAuditRecord {
        id: row.get(0)?,
        request_id: row.get(1)?,
        request_fingerprint: row.get(2)?,
        intent_kind: row.get(3)?,
        risk_class: row.get(4)?,
        target_kind: row.get(5)?,
        target_id: row.get(6)?,
        agent_id: row.get(7)?,
        task_owner_agent_id: row.get(8)?,
        task_id: row.get(9)?,
        approval_id: row.get(10)?,
        authorization_kind: row.get(11)?,
        intent_fingerprint_sha256: row.get(12)?,
        policy_fingerprint_sha256: row.get(13)?,
        status: row.get(14)?,
        detail_code: row.get(15)?,
        detail_message: row.get(16)?,
        content_sha256: row.get(17)?,
        content_length: row.get(18)?,
        created_at_unix_ms: row.get(19)?,
        updated_at_unix_ms: row.get(20)?,
    })
}

fn ensure_system_action_audit_binding(
    existing: &SystemActionAuditRecord,
    write: &AuditWrite,
) -> PersistenceResult<()> {
    if existing.request_fingerprint != write.request_fingerprint
        || existing.intent_kind != write.intent_kind
        || existing.risk_class != write.risk_class
        || existing.target_kind != write.target_kind
        || existing.target_id != write.target_id
        || existing.agent_id != write.agent_id
        || existing.content_sha256 != write.content_sha256
        || existing.content_length != write.content_length
    {
        return Err(PersistenceError::new(
            "SYSTEM_ACTION_IDEMPOTENCY_CONFLICT",
            "The voice request identifier is already bound to a different exact action.",
            false,
        ));
    }
    Ok(())
}

fn ensure_system_action_audit_transition(current: &str, next: &str) -> PersistenceResult<()> {
    let allowed = matches!(
        (current, next),
        ("approvalRequired", "dispatched" | "rejected" | "failed")
            | (
                "dispatched",
                "applied" | "taskCreated" | "failed" | "uncertain"
            )
    );
    if !allowed {
        return Err(PersistenceError::new(
            "SYSTEM_ACTION_TERMINAL",
            "The idempotent system-action request is already terminal and was not repeated.",
            false,
        ));
    }
    Ok(())
}

fn enforce_system_action_audit_cap(
    transaction: &Transaction<'_>,
    current_id: i64,
    timestamp: i64,
) -> PersistenceResult<()> {
    let count: i64 = transaction
        .query_row("SELECT COUNT(*) FROM system_action_audits", [], |row| {
            row.get(0)
        })
        .map_err(PersistenceError::database)?;
    let excess = count.saturating_sub(MAX_SYSTEM_ACTION_AUDITS).max(0);
    if excess == 0 {
        return Ok(());
    }
    let pruned = transaction
        .execute(
            "DELETE FROM system_action_audits WHERE id IN (
                 SELECT id FROM system_action_audits
                 WHERE id != ?1
                   AND status IN ('taskCreated', 'applied', 'rejected', 'failed', 'uncertain')
                 ORDER BY updated_at_unix_ms, id
                 LIMIT ?2
             )",
            params![current_id, excess],
        )
        .map_err(PersistenceError::database)? as i64;
    if pruned != excess {
        return Err(PersistenceError::new(
            "SYSTEM_ACTION_AUDIT_CAPACITY",
            "The bounded system-action audit is full of non-terminal requests; no action was dispatched.",
            true,
        ));
    }
    transaction
        .execute(
            "UPDATE data_lifecycle_meta
             SET revision = revision + 1,
                 last_observed_at_unix_ms = CASE
                     WHEN last_observed_at_unix_ms IS NULL OR last_observed_at_unix_ms < ?1
                         THEN ?1 ELSE last_observed_at_unix_ms END,
                 total_pruned_system_action_audits = total_pruned_system_action_audits + ?2
             WHERE singleton = 1",
            params![timestamp, pruned],
        )
        .map_err(PersistenceError::database)?;
    Ok(())
}

fn collect_rows<'statement, T, F>(
    rows: rusqlite::Result<rusqlite::MappedRows<'statement, F>>,
) -> PersistenceResult<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let rows = rows.map_err(PersistenceError::database)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(PersistenceError::database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::RunMode;
    use crate::provider_runtime::{
        catalog_provider_bindings, ollama_descriptor, ProviderAvailability, ProviderRuntimeModel,
        ProviderRuntimeStatus,
    };
    use crate::review_orchestration::{
        ReviewCheckKind, ReviewCheckResultV1, ReviewCheckStatus, ReviewRequestV1,
        REQUIRED_REVIEW_CHECKS,
    };
    use crate::specialist_capabilities::{
        CodingRequestV1, CodingResultV1, SpecialistResultV1, SpecialistTaskRequestV1,
        WorkspaceMutationClass, SPECIALIST_PROFILE_VERSION, SPECIALIST_SCHEMA_VERSION,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ai-agent-control-center-task-0003-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            Self { path }
        }

        fn database_path(&self) -> PathBuf {
            self.path.join("application-state.sqlite3")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn schema_ten_repository(path: &Path) -> StateRepository {
        prepare_private_database_file(path).unwrap();
        let connection = Connection::open(path).unwrap();
        let mut repository = StateRepository { connection };
        repository.configure_connection_preflight().unwrap();
        repository.configure_write_durability(false).unwrap();
        for (version, name, migration) in [
            (1, "initial_application_state", INITIAL_MIGRATION),
            (
                2,
                "authoritative_approval_lifecycle",
                AUTHORIZATION_MIGRATION,
            ),
            (
                3,
                "authoritative_run_coordination",
                RUN_COORDINATION_MIGRATION,
            ),
            (4, "dynamic_agent_registry", AGENT_REGISTRY_MIGRATION),
            (
                5,
                "authoritative_task_orchestration",
                TASK_ORCHESTRATION_MIGRATION,
            ),
            (
                6,
                "structured_review_orchestration",
                REVIEW_ORCHESTRATION_MIGRATION,
            ),
            (
                7,
                "structured_workspace_evidence",
                WORKSPACE_EVIDENCE_MIGRATION,
            ),
            (8, "data_lifecycle_and_monitoring", DATA_LIFECYCLE_MIGRATION),
            (
                9,
                "system_action_policy_gateway",
                SYSTEM_ACTION_GATEWAY_MIGRATION,
            ),
            (
                10,
                "bounded_specialist_capabilities",
                SPECIALIST_CAPABILITIES_MIGRATION,
            ),
        ] {
            repository
                .apply_migration(version, name, migration)
                .unwrap();
        }
        assert_eq!(repository.schema_version().unwrap(), 10);
        repository
    }

    fn legacy_renderer_state(state: &ApplicationState) -> LegacyRendererState {
        LegacyRendererState {
            agents: Some(serde_json::to_string(&state.agents).unwrap()),
            models: Some(serde_json::to_string(&state.models).unwrap()),
            approval_requests: Some(serde_json::to_string(&state.approval_requests).unwrap()),
            reminders: Some(serde_json::to_string(&state.reminders).unwrap()),
            task_retention_days: Some(state.task_retention_days.as_storage_value().to_string()),
            activity_retention_days: Some(
                state.activity_retention_days.as_storage_value().to_string(),
            ),
            preferences: Some(serde_json::to_string(&state.preferences).unwrap()),
        }
    }

    fn task_0010_provider_snapshot() -> ProviderRegistrySnapshot {
        ProviderRegistrySnapshot {
            providers: vec![ProviderRuntimeStatus {
                provider: ollama_descriptor(),
                availability: ProviderAvailability::Ready,
                version: Some("fixture".to_string()),
                models: vec![ProviderRuntimeModel {
                    name: "qwen2.5-coder:7b".to_string(),
                    capabilities: vec!["completion".to_string(), "tools".to_string()],
                    context_length: Some(32_768),
                    availability: ProviderAvailability::Ready,
                    message: "Ready".to_string(),
                }],
                message: "Ready".to_string(),
            }],
            catalog_bindings: catalog_provider_bindings(),
        }
    }

    fn configure_task_0010_repository(repository: &mut StateRepository) -> StateEnvelope {
        let initialized = repository.initialize_fresh().unwrap();
        let mut configured = initialized.state;
        configured.preferences.active_ai_provider = "ollama".to_string();
        configured.preferences.workspaces.push(WorkspaceDefinition {
            id: "workspace-1".to_string(),
            name: "Fixture".to_string(),
            path: "/tmp/task-0010-fixture".to_string(),
        });
        configured.preferences.active_workspace_id = Some("workspace-1".to_string());
        configured.preferences.workspace_path = "/tmp/task-0010-fixture".to_string();
        let receipt = repository
            .save(initialized.revision, &configured, true)
            .unwrap();
        let envelope = repository.load().unwrap().unwrap();
        assert_eq!(envelope.revision, receipt.revision);
        envelope
    }

    fn task_0010_repository() -> (StateRepository, StateEnvelope) {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let envelope = configure_task_0010_repository(&mut repository);
        (repository, envelope)
    }

    fn task_0010_create_request(
        revision: i64,
        title: &str,
        priority: &str,
    ) -> CreateRoutedTaskRequest {
        CreateRoutedTaskRequest {
            expected_revision: revision,
            task_owner_agent_id: 1,
            title: title.to_string(),
            category: "Development".to_string(),
            priority: priority.to_string(),
            workspace_id: "workspace-1".to_string(),
            routing_mode: "automatic".to_string(),
            preferred_agent_id: Some(2),
            selected_agent_id: None,
            specialist_request: Some(coding_request(title)),
        }
    }

    fn coding_request(objective: &str) -> SpecialistTaskRequestV1 {
        SpecialistTaskRequestV1::Coding(CodingRequestV1 {
            schema_version: SPECIALIST_SCHEMA_VERSION,
            profile_version: SPECIALIST_PROFILE_VERSION.to_string(),
            objective: objective.to_string(),
            acceptance_criteria: vec!["The requested bounded change is verified.".to_string()],
            constraints: vec!["Preserve unrelated workspace state.".to_string()],
            mutation_classes: vec![WorkspaceMutationClass::Modify],
            requested_checks: vec![],
            allow_web_research: false,
        })
    }

    fn task_by_title<'a>(state: &'a ApplicationState, title: &str) -> &'a AgentTask {
        state
            .agents
            .iter()
            .flat_map(|agent| &agent.tasks)
            .find(|task| task.title == title)
            .expect("task should exist")
    }

    fn task_by_title_mut<'a>(state: &'a mut ApplicationState, title: &str) -> &'a mut AgentTask {
        state
            .agents
            .iter_mut()
            .flat_map(|agent| &mut agent.tasks)
            .find(|task| task.title == title)
            .expect("task should exist")
    }

    fn task_0015_audit_write(request_id: &str, status: &str) -> AuditWrite {
        AuditWrite {
            request_id: request_id.to_string(),
            request_fingerprint: "voice-intent-v1|fixture".to_string(),
            intent_kind: "launchApplication".to_string(),
            risk_class: "reversible".to_string(),
            target_kind: "desktopEntry".to_string(),
            target_id: "org.example.Editor.desktop".to_string(),
            agent_id: 7,
            task_owner_agent_id: None,
            task_id: None,
            approval_id: None,
            authorization_kind: if status == "approvalRequired" {
                "approvalRequired".to_string()
            } else {
                "policyAllowed".to_string()
            },
            intent_fingerprint_sha256:
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            policy_fingerprint_sha256:
                "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string(),
            status: status.to_string(),
            detail_code: None,
            detail_message: None,
            content_sha256: None,
            content_length: None,
        }
    }

    #[test]
    fn task_0015_private_bounded_system_action_audit_survives_later_schema() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        repository.initialize_fresh().unwrap();
        assert_eq!(repository.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        let table_sql: String = repository
            .connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'system_action_audits'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_sql.contains("request_id TEXT NOT NULL UNIQUE"));
        assert!(table_sql.contains("content_sha256 TEXT"));
        assert!(!table_sql.contains("transcript"));
        assert!(!table_sql.contains("caption"));
        assert!(!table_sql.contains("user_path"));
    }

    #[test]
    fn task_0015_audit_is_idempotent_transitioned_and_content_redacted() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        repository.initialize_fresh().unwrap();
        let first = repository
            .write_system_action_audit(&task_0015_audit_write("voice:audit:1", "approvalRequired"))
            .unwrap();
        let duplicate = repository
            .write_system_action_audit(&task_0015_audit_write("voice:audit:1", "approvalRequired"))
            .unwrap();
        assert_eq!(first.id, duplicate.id);

        let dispatched = repository
            .write_system_action_audit(&task_0015_audit_write("voice:audit:1", "dispatched"))
            .unwrap();
        let applied = repository
            .write_system_action_audit(&task_0015_audit_write("voice:audit:1", "applied"))
            .unwrap();
        assert_eq!(dispatched.id, applied.id);
        assert_eq!(applied.status, "applied");
        assert!(applied.content_sha256.is_none());

        let mut conflict = task_0015_audit_write("voice:audit:1", "applied");
        conflict.target_id = "org.example.Other.desktop".to_string();
        assert_eq!(
            repository
                .write_system_action_audit(&conflict)
                .unwrap_err()
                .code,
            "SYSTEM_ACTION_IDEMPOTENCY_CONFLICT"
        );
        assert_eq!(
            repository
                .write_system_action_audit(&task_0015_audit_write("voice:audit:1", "dispatched",))
                .unwrap_err()
                .code,
            "SYSTEM_ACTION_TERMINAL"
        );
        assert_eq!(
            repository
                .query_system_action_audits(100)
                .unwrap()
                .records
                .len(),
            1
        );
    }

    #[test]
    fn task_0015_restart_marks_dispatched_action_uncertain_without_retry() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        repository.initialize_fresh().unwrap();
        repository
            .write_system_action_audit(&task_0015_audit_write("voice:restart:1", "dispatched"))
            .unwrap();
        repository.reconcile_dispatched_system_actions().unwrap();
        let audit = repository
            .system_action_audit("voice:restart:1")
            .unwrap()
            .unwrap();
        assert_eq!(audit.status, "uncertain");
        assert_eq!(
            audit.detail_code.as_deref(),
            Some("SYSTEM_ACTION_DISPATCH_INTERRUPTED")
        );
    }

    #[test]
    fn fresh_state_has_exact_schema_and_survives_reopen() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let expected = default_application_state().expect("seed should be valid");
        {
            let mut repository = StateRepository::open(&path).expect("database should open");
            assert_eq!(repository.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
            let migrations = repository
                .connection
                .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<(i64, String)>>>()
                .unwrap();
            assert_eq!(
                migrations,
                vec![
                    (1, "initial_application_state".to_string()),
                    (2, "authoritative_approval_lifecycle".to_string()),
                    (3, "authoritative_run_coordination".to_string()),
                    (4, "dynamic_agent_registry".to_string()),
                    (5, "authoritative_task_orchestration".to_string()),
                    (6, "structured_review_orchestration".to_string()),
                    (7, "structured_workspace_evidence".to_string()),
                    (8, "data_lifecycle_and_monitoring".to_string()),
                    (9, "system_action_policy_gateway".to_string()),
                    (10, "bounded_specialist_capabilities".to_string()),
                    (11, "reminders_memory_management_handoffs".to_string())
                ]
            );
            let journal_mode: String = repository
                .connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
            let synchronous: i64 = repository
                .connection
                .pragma_query_value(None, "synchronous", |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 2);
            let foreign_keys: i64 = repository
                .connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                .unwrap();
            assert_eq!(foreign_keys, 1);
            assert!(repository.load().unwrap().is_none());
            let initialized = repository.initialize_fresh().unwrap();
            assert_eq!(initialized.revision, 1);
            assert_eq!(initialized.state, expected);
        }
        let mut repository = StateRepository::open(&path).expect("database should reopen");
        let reopened = repository.load().unwrap().expect("state should exist");
        assert_eq!(reopened.revision, 1);
        assert_eq!(reopened.state, expected);
    }

    fn task_0018_schedule_request(
        request_id: &str,
        delivery_mode: DeliveryMode,
        recurrence: RecurrenceRuleV1,
    ) -> CreateScheduledItemRequest {
        CreateScheduledItemRequest {
            expected_revision: 0,
            request_id: request_id.to_string(),
            kind: ScheduledItemKind::Reminder,
            title: "Inspect the deterministic reminder".to_string(),
            notes: "No model run is attached to this schedule.".to_string(),
            local_due_at: "2026-08-28T12:00:00".to_string(),
            time_zone: "UTC".to_string(),
            event_end_local: None,
            recurrence,
            delivery_mode,
            privacy_mode: PrivacyMode::Title,
            subject_agent_id: Some(8),
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
        }
    }

    #[test]
    fn task_0018_schema_ten_migrates_legacy_reminders_memory_and_task_plans_once() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let legacy_memory = "Preserve this exact legacy instruction.\nSecond line.";
        {
            let mut repository = schema_ten_repository(&path);
            let mut state = authorization_state();
            state.agents[1].memory = legacy_memory.to_string();
            let transaction = repository
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            write_application_state(&transaction, &state, "fresh", &HashMap::new(), false).unwrap();
            transaction
                .execute(
                    "UPDATE agents SET memory = ?1 WHERE id = 2",
                    [legacy_memory],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO reminders
                     (id, position, title, notes, due_at, status, agent_id, task_id,
                      created_at, created_at_unix_ms, resolved_at_unix_ms)
                     VALUES (91, 0, 'Legacy local follow-up', 'Preserve legacy notes',
                             '2026-09-03T14:30:00.000Z', 'Upcoming', 2, 41,
                             '2026-08-28T10:00:00.000Z', 1787911200000, NULL)",
                    [],
                )
                .unwrap();
            transaction
                .execute(
                    "UPDATE application_meta
                     SET initialized = 1, state_revision = 7, source_kind = 'fresh'
                     WHERE singleton = 1",
                    [],
                )
                .unwrap();
            transaction.commit().unwrap();
        }

        {
            let mut migrated = StateRepository::open(&path).unwrap();
            assert_eq!(migrated.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
            let state = migrated.load().unwrap().unwrap().state;
            assert!(state.agents.iter().all(|agent| agent.memory.is_empty()));

            let memory = migrated.structured_memory_snapshot().unwrap();
            assert_eq!(memory.records.len(), 1);
            assert_eq!(memory.records[0].content, legacy_memory);
            assert_eq!(memory.records[0].scope, MemoryScopeV1::agent(2));
            assert_eq!(
                memory.records[0].provenance,
                MemoryProvenanceKind::LegacyAgentMemory
            );

            let reminders = migrated.reminder_scheduler_snapshot().unwrap();
            assert_eq!(reminders.items.len(), 1);
            assert_eq!(reminders.items[0].title, "Legacy local follow-up");
            assert_eq!(reminders.items[0].notes, "Preserve legacy notes");
            assert_eq!(reminders.items[0].local_due_at, "2026-09-03T14:30:00");
            assert_eq!(reminders.items[0].time_zone, "UTC");
            assert_eq!(reminders.items[0].delivery_mode, DeliveryMode::InApp);
            assert_eq!(reminders.items[0].task_owner_agent_id, Some(2));
            assert_eq!(reminders.items[0].task_id, Some(41));

            let handoffs = migrated.management_handoff_snapshot().unwrap();
            assert_eq!(handoffs.handoffs.len(), 2);
            assert_eq!(handoffs.handoffs[0].kind, ManagementHandoffKind::TaskPlan);
            assert_eq!(
                handoffs.handoffs[0].source,
                ManagementHandoffSource::MigrationV11
            );
            assert_eq!(handoffs.handoffs[0].payload["historical"], true);
            assert_eq!(handoffs.handoffs[1].kind, ManagementHandoffKind::Assignment);
            assert_eq!(handoffs.handoffs[1].payload["assignedAgentId"], 2);
            assert_eq!(migrated.export_backup().unwrap().counts.reminders, 1);
        }

        let mut reopened = StateRepository::open(&path).unwrap();
        assert_eq!(
            reopened.structured_memory_snapshot().unwrap().records.len(),
            1
        );
        assert_eq!(
            reopened.reminder_scheduler_snapshot().unwrap().items.len(),
            1
        );
        assert_eq!(
            reopened
                .management_handoff_snapshot()
                .unwrap()
                .handoffs
                .len(),
            2
        );
    }

    #[test]
    fn task_0018_reminder_scheduler_is_authoritative_restart_safe_and_model_passive() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        repository.initialize_fresh().unwrap();
        let request = task_0018_schedule_request(
            "task-0018-reminder-create",
            DeliveryMode::Portal,
            RecurrenceRuleV1 {
                kind: RecurrenceKind::Daily,
                interval: 1,
                occurrence_limit: Some(3),
                until_unix_ms: None,
            },
        );
        let created = repository.create_scheduled_item(request.clone()).unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.items.len(), 1);
        let duplicate = repository.create_scheduled_item(request).unwrap();
        assert_eq!(duplicate.revision, 1);
        assert_eq!(duplicate.items.len(), 1);
        let (authorization_kind, approval_id, policy_fingerprint): (String, Option<i64>, String) =
            repository
                .connection
                .query_row(
                    "SELECT authorization_kind, approval_id, authorization_policy_fingerprint
                 FROM reminders WHERE id = ?1",
                    [created.items[0].id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
        assert_eq!(authorization_kind, "policy_allow");
        assert_eq!(approval_id, None);
        assert_eq!(policy_fingerprint.len(), 64);

        let due = crate::reminder_scheduler::resolve_local_due_at("2026-08-28T12:00:00", "UTC")
            .unwrap()
            .due_at_unix_ms;
        let jobs = repository
            .scan_due_reminders(due + 2 * 24 * 60 * 60 * 1000)
            .unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM run_attempts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        repository
            .finish_reminder_delivery(jobs[0].occurrence_id, true, None)
            .unwrap();
        repository.reconcile_reserved_reminder_deliveries().unwrap();
        let snapshot = repository.reminder_scheduler_snapshot().unwrap();
        assert_eq!(snapshot.items[0].status, ScheduleStatus::Due);
        assert_eq!(snapshot.items[0].missed_occurrence_count, 2);
        assert_eq!(
            snapshot
                .recent_occurrences
                .iter()
                .filter(|occurrence| occurrence.status == "portal_accepted")
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .recent_occurrences
                .iter()
                .filter(|occurrence| occurrence.status == "uncertain")
                .count(),
            2
        );
        assert!(repository
            .scan_due_reminders(due + 3 * 24 * 60 * 60 * 1000)
            .unwrap()
            .is_empty());

        let envelope = repository.load().unwrap().unwrap();
        let mut forged = envelope.state;
        forged.reminders[0].title = "Renderer-forged title".to_string();
        assert_eq!(
            repository
                .save(envelope.revision, &forged, true)
                .unwrap_err()
                .code,
            "REMINDER_SCHEDULER_AUTHORITY_REQUIRED"
        );
    }

    #[test]
    fn task_0018_unrepresentable_next_recurrence_is_held_with_evidence() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        repository.initialize_fresh().unwrap();
        let mut request = task_0018_schedule_request(
            "task-0018-recurrence-boundary",
            DeliveryMode::InApp,
            RecurrenceRuleV1 {
                kind: RecurrenceKind::Daily,
                interval: 366,
                occurrence_limit: None,
                until_unix_ms: None,
            },
        );
        request.local_due_at = "9999-01-01T12:00:00".to_string();
        let created = repository.create_scheduled_item(request).unwrap();
        let due = created.items[0].due_at_unix_ms.unwrap();

        assert!(repository.scan_due_reminders(due).unwrap().is_empty());
        let snapshot = repository.reminder_scheduler_snapshot().unwrap();
        assert_eq!(snapshot.items[0].status, ScheduleStatus::NeedsAttention);
        assert_eq!(snapshot.items[0].next_occurrence_sequence, 1);
        assert_eq!(
            snapshot.items[0].schedule_issue_code.as_deref(),
            Some("REMINDER_RECURRENCE_INVALID")
        );
        assert_eq!(snapshot.recent_occurrences.len(), 1);
        assert_eq!(snapshot.recent_occurrences[0].status, "in_app_due");
        let backup = repository.export_backup().unwrap();
        assert_eq!(backup.counts.reminders, 1);
    }

    #[test]
    fn task_0018_memory_mutations_are_scoped_revised_deletable_and_run_exact() {
        let (mut repository, task_id) = task_0011_repository();
        let records = [
            (MemoryScopeV1::agent(2), "agent-two-only"),
            (MemoryScopeV1::agent(3), "wrong-agent-secret"),
            (MemoryScopeV1::project("workspace-1"), "project-one-only"),
            (MemoryScopeV1::task(2, task_id), "task-only"),
            (MemoryScopeV1::team(6), "team-chain-only"),
        ];
        let mut snapshot = repository.structured_memory_snapshot().unwrap();
        for (index, (scope, content)) in records.into_iter().enumerate() {
            snapshot = repository
                .create_memory_record(CreateMemoryRecordRequest {
                    expected_revision: snapshot.revision,
                    request_id: format!("task-0018-memory-create-{index}"),
                    scope,
                    kind: MemoryRecordKind::Instruction,
                    content: content.to_string(),
                    retention: MemoryRetentionPolicy::Manual,
                })
                .unwrap();
        }
        assert_eq!(snapshot.records.len(), 5);
        let editable = snapshot
            .records
            .iter()
            .find(|record| record.content == "agent-two-only")
            .unwrap()
            .clone();
        let updated = repository
            .update_memory_record(UpdateMemoryRecordRequest {
                expected_revision: snapshot.revision,
                expected_record_revision: editable.revision,
                request_id: "task-0018-memory-update".to_string(),
                record_id: editable.id,
                kind: MemoryRecordKind::Decision,
                content: "agent-two-updated".to_string(),
                retention: MemoryRetentionPolicy::Days7,
            })
            .unwrap();
        let edited = updated
            .records
            .iter()
            .find(|record| record.id == editable.id)
            .unwrap();
        assert_eq!(edited.revision, 2);
        assert!(edited.expires_at_unix_ms.is_some());

        let intent = ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        approve_authorization(&mut repository, &intent);
        let admitted = repository
            .admit_run("task-0018-exact-memory-bundle", &intent)
            .unwrap();
        assert!(admitted.memory_bundle_json.contains("agent-two-updated"));
        assert!(admitted.memory_bundle_json.contains("project-one-only"));
        assert!(admitted.memory_bundle_json.contains("task-only"));
        assert!(admitted.memory_bundle_json.contains("team-chain-only"));
        assert!(!admitted.memory_bundle_json.contains("wrong-agent-secret"));
        let (stored_json, stored_sha): (String, String) = repository
            .connection
            .query_row(
                "SELECT memory_bundle_json, memory_bundle_sha256
                 FROM run_attempts WHERE id = ?1",
                [admitted.attempt.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_json, admitted.memory_bundle_json);
        assert_eq!(stored_sha, sha256_hex(stored_json.as_bytes()));

        let deletable = updated
            .records
            .iter()
            .find(|record| record.content == "wrong-agent-secret")
            .unwrap();
        let after_delete = repository
            .delete_memory_record(DeleteMemoryRecordRequest {
                expected_revision: updated.revision,
                expected_record_revision: deletable.revision,
                request_id: "task-0018-memory-delete".to_string(),
                record_id: deletable.id,
            })
            .unwrap();
        assert!(!after_delete
            .records
            .iter()
            .any(|record| record.content == "wrong-agent-secret"));

        let envelope = repository.load().unwrap().unwrap();
        let mut forged = envelope.state;
        forged.agents[1].memory = "renderer free text".to_string();
        assert_eq!(
            repository
                .save(envelope.revision, &forged, true)
                .unwrap_err()
                .code,
            "STRUCTURED_MEMORY_AUTHORITY_REQUIRED"
        );
    }

    #[test]
    fn task_0018_backup_v4_restores_portable_schedules_and_memory_without_portal_grants() {
        let mut source = StateRepository::open_in_memory().unwrap();
        source.initialize_fresh().unwrap();
        source
            .create_scheduled_item(task_0018_schedule_request(
                "task-0018-backup-reminder",
                DeliveryMode::Portal,
                RecurrenceRuleV1::default(),
            ))
            .unwrap();
        source
            .create_memory_record(CreateMemoryRecordRequest {
                expected_revision: 0,
                request_id: "task-0018-backup-memory".to_string(),
                scope: MemoryScopeV1::agent(2),
                kind: MemoryRecordKind::Fact,
                content: "portable structured memory".to_string(),
                retention: MemoryRetentionPolicy::Manual,
            })
            .unwrap();
        let export = source.export_backup().unwrap();
        let value: serde_json::Value = serde_json::from_str(&export.backup_json).unwrap();
        assert_eq!(value["version"], 4);
        assert_eq!(value["scheduledItems"][0]["deliveryMode"], "in_app");
        assert!(value["scheduledItems"][0]["scheduleFingerprint"].is_null());
        assert_eq!(value["memoryRecords"].as_array().unwrap().len(), 1);
        assert_eq!(export.sanitizations.portal_deliveries_disabled, 1);
        assert!(export
            .omitted_domains
            .iter()
            .any(|domain| domain == "notificationDeliveryEvidence"));
        let mut tampered = value.clone();
        tampered["scheduledItems"][0]["dueAtUnixMs"] = serde_json::json!(1);
        assert_eq!(
            crate::data_lifecycle::parse_backup_candidate(
                &serde_json::to_string(&tampered).unwrap(),
                &source.load().unwrap().unwrap().state,
            )
            .unwrap_err()
            .path,
            "backup.scheduledItems"
        );

        let mut target = StateRepository::open_in_memory().unwrap();
        let initialized = target.initialize_fresh().unwrap();
        target
            .apply_backup_import(initialized.revision, &export.backup_json)
            .unwrap();
        let reminders = target.reminder_scheduler_snapshot().unwrap();
        assert_eq!(reminders.items.len(), 1);
        assert_eq!(reminders.items[0].delivery_mode, DeliveryMode::InApp);
        let memory = target.structured_memory_snapshot().unwrap();
        assert_eq!(memory.records.len(), 1);
        assert_eq!(
            memory.records[0].provenance,
            MemoryProvenanceKind::BackupImport
        );
        assert_eq!(memory.records[0].content, "portable structured memory");
        assert_eq!(
            target
                .connection
                .query_row("SELECT COUNT(*) FROM management_handoffs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn task_0018_retention_prunes_expired_memory_occurrences_and_orphaned_handoffs_truthfully() {
        let (mut repository, task_id) = task_0011_repository();
        let memory = repository.structured_memory_snapshot().unwrap();
        let memory = repository
            .create_memory_record(CreateMemoryRecordRequest {
                expected_revision: memory.revision,
                request_id: "task-0018-retention-agent-memory".to_string(),
                scope: MemoryScopeV1::agent(2),
                kind: MemoryRecordKind::Fact,
                content: "Expires after seven days.".to_string(),
                retention: MemoryRetentionPolicy::Days7,
            })
            .unwrap();
        repository
            .create_memory_record(CreateMemoryRecordRequest {
                expected_revision: memory.revision,
                request_id: "task-0018-retention-task-memory".to_string(),
                scope: MemoryScopeV1::task(2, task_id),
                kind: MemoryRecordKind::Decision,
                content: "Expires when the task becomes terminal.".to_string(),
                retention: MemoryRetentionPolicy::TaskLifetime,
            })
            .unwrap();

        let created = repository
            .create_scheduled_item(task_0018_schedule_request(
                "task-0018-retention-reminder",
                DeliveryMode::InApp,
                RecurrenceRuleV1::default(),
            ))
            .unwrap();
        let due = created.items[0].due_at_unix_ms.unwrap();
        assert!(repository.scan_due_reminders(due).unwrap().is_empty());
        let due_snapshot = repository.reminder_scheduler_snapshot().unwrap();
        assert_eq!(due_snapshot.recent_occurrences.len(), 1);
        repository
            .set_scheduled_item_status(SetScheduledItemStatusRequest {
                expected_revision: due_snapshot.revision,
                expected_item_revision: due_snapshot.items[0].revision,
                request_id: "task-0018-retention-reminder-complete".to_string(),
                item_id: due_snapshot.items[0].id,
                status: ScheduleStatus::Completed,
            })
            .unwrap();

        let terminal_at = now_unix_ms().unwrap();
        repository
            .connection
            .execute(
                "UPDATE agent_tasks
                 SET status = 'Completed', phase = 'Finished', queue_state = 'notQueued',
                     enqueue_sequence = NULL, completed_at = ?1, completed_at_unix_ms = ?2
                 WHERE owner_agent_id = 2 AND id = ?3",
                params![format_unix_ms(terminal_at), terminal_at, task_id],
            )
            .unwrap();

        let maintenance_at = terminal_at + 40 * 24 * 60 * 60 * 1000;
        let result = repository
            .run_data_lifecycle_maintenance("test", maintenance_at)
            .unwrap();
        assert_eq!(result.pruned.memory_records, 2);
        assert_eq!(result.pruned.reminder_occurrences, 1);
        assert_eq!(result.pruned.reminders, 1);
        assert_eq!(result.pruned.tasks, 1);
        assert_eq!(result.pruned.management_handoffs, 2);
        assert!(!result.backlog_remaining);
        assert!(repository
            .structured_memory_snapshot()
            .unwrap()
            .records
            .is_empty());
        assert!(repository
            .reminder_scheduler_snapshot()
            .unwrap()
            .items
            .is_empty());
        assert!(repository
            .management_handoff_snapshot()
            .unwrap()
            .handoffs
            .is_empty());
    }

    #[test]
    fn task_0018_bounded_handoff_retention_preserves_a_valid_prefix() {
        let (mut repository, task_id) = task_0011_repository();
        repository
            .connection
            .execute(
                "DELETE FROM agent_tasks WHERE owner_agent_id = 2 AND id = ?1",
                [task_id],
            )
            .unwrap();
        let first_extra_id: i64 = repository
            .connection
            .query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM management_handoffs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let timestamp = now_unix_ms().unwrap();
        let transaction = repository.connection.transaction().unwrap();
        for offset in 0..MAX_MAINTENANCE_ROWS_PER_DOMAIN {
            transaction
                .execute(
                    "INSERT INTO management_handoffs
                     (id, task_owner_agent_id, task_id, kind, from_agent_id, to_agent_id,
                      owner_role, revision_round, source_kind, summary, payload_json,
                      idempotency_key, created_at_unix_ms)
                     VALUES (?1, 2, ?2, 'assignment', 2, 2, 'human', 0,
                             'task_orchestration', 'Retained assignment evidence.',
                             '{\"bulk\":true}', ?3, ?4)",
                    params![
                        first_extra_id + offset,
                        task_id,
                        format!("task-0018-retention-prefix-{offset}"),
                        timestamp + offset,
                    ],
                )
                .unwrap();
        }
        transaction
            .execute(
                "UPDATE management_handoff_meta
                 SET revision = revision + ?1, next_handoff_id = ?2
                 WHERE singleton = 1",
                params![
                    MAX_MAINTENANCE_ROWS_PER_DOMAIN,
                    first_extra_id + MAX_MAINTENANCE_ROWS_PER_DOMAIN,
                ],
            )
            .unwrap();
        transaction.commit().unwrap();

        let first = repository
            .run_data_lifecycle_maintenance("test", timestamp + 40 * 24 * 60 * 60 * 1000)
            .unwrap();
        assert_eq!(
            first.pruned.management_handoffs,
            MAX_MAINTENANCE_ROWS_PER_DOMAIN
        );
        assert!(first.backlog_remaining);
        let retained = repository.management_handoff_snapshot().unwrap();
        assert_eq!(
            retained
                .handoffs
                .iter()
                .map(|handoff| handoff.kind)
                .collect::<Vec<_>>(),
            vec![
                ManagementHandoffKind::TaskPlan,
                ManagementHandoffKind::Assignment,
            ]
        );

        let second = repository
            .run_data_lifecycle_maintenance("test", timestamp + 40 * 24 * 60 * 60 * 1000 + 1)
            .unwrap();
        assert_eq!(second.pruned.management_handoffs, 2);
        assert!(repository
            .management_handoff_snapshot()
            .unwrap()
            .handoffs
            .is_empty());
    }

    #[test]
    fn task_0017_schema_ten_persists_strict_requests_and_protects_renderer_mutation() {
        let (mut repository, envelope) = task_0010_repository();
        let request = SpecialistTaskRequestV1::Coding(CodingRequestV1 {
            schema_version: SPECIALIST_SCHEMA_VERSION,
            profile_version: SPECIALIST_PROFILE_VERSION.to_string(),
            objective: "Implement a strict persisted request".to_string(),
            acceptance_criteria: vec!["The canonical request survives restart".to_string()],
            constraints: vec![],
            mutation_classes: vec![WorkspaceMutationClass::Modify],
            requested_checks: vec![],
            allow_web_research: false,
        });
        let mut create = task_0010_create_request(envelope.revision, "Specialist task", "Normal");
        create.specialist_request = Some(request.clone());
        let created = repository
            .create_routed_task(create, &task_0010_provider_snapshot())
            .unwrap();
        let task = task_by_title(&created.state, "Specialist task");
        assert_eq!(task.specialist_request.as_ref(), Some(&request));

        let stored: String = repository
            .connection
            .query_row(
                "SELECT specialist_request_json FROM agent_tasks WHERE id = ?1",
                [task.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, request.canonical_json().unwrap());
        for (table, column) in [
            ("run_attempts", "specialist_contract_json"),
            ("run_attempts", "specialist_result_json"),
        ] {
            let count: i64 = repository
                .connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }

        let mut forged = created.state.clone();
        task_by_title_mut(&mut forged, "Specialist task").specialist_request = None;
        assert_eq!(
            repository
                .save(created.revision, &forged, true)
                .unwrap_err()
                .code,
            "TASK_ORCHESTRATION_AUTHORITY_REQUIRED"
        );
        assert_eq!(
            task_by_title(
                &repository.load().unwrap().unwrap().state,
                "Specialist task"
            )
            .specialist_request
            .as_ref(),
            Some(&request)
        );

        let task_id = task.id;
        let intent = ActionIntent::RunTask {
            agent_id: task.assigned_agent_id,
            task_owner_agent_id: 1,
            task_id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        let approval = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(approval.id, ApprovalResolution::Approve, true)
            .unwrap();
        let admitted = repository
            .admit_run("task-0017-contract-ledger", &intent)
            .unwrap();
        let contract = admitted
            .attempt
            .specialist_contract
            .as_ref()
            .expect("specialist contract should be immutable at admission");
        assert_eq!(contract.approval_id, Some(approval.id));
        assert_eq!(contract.request_sha256, request.fingerprint().unwrap());
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                &contract.provider,
                &contract.model,
                admitted.attempt.workspace_id.as_deref(),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();
        let specialist_result = SpecialistResultV1::Coding(CodingResultV1 {
            summary: "The bounded change was completed.".to_string(),
            changes: vec![],
            verification: vec![],
            evidence_refs: vec![],
            limitations: vec![],
        });
        let mut completion =
            successful_completion(&serde_json::to_string(&specialist_result).unwrap());
        completion.specialist_result = Some(specialist_result.clone());
        let completed = repository
            .complete_run(admitted.attempt.id, &completion)
            .unwrap();
        assert_eq!(completed.specialist_result, Some(specialist_result));
        assert_eq!(
            repository
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM run_attempts WHERE id = ?1 AND specialist_contract_json IS NOT NULL AND specialist_result_json IS NOT NULL",
                    [admitted.attempt.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn task_0017_reroute_persists_the_exact_revalidated_specialist_request() {
        let (mut repository, envelope) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let created = repository
            .create_routed_task(
                task_0010_create_request(
                    envelope.revision,
                    "Original specialist request",
                    "Normal",
                ),
                &providers,
            )
            .unwrap();
        let original = task_by_title(&created.state, "Original specialist request").clone();
        let updated_request = coding_request("Updated specialist request");

        let rerouted = repository
            .reroute_task(
                RerouteTaskRequest {
                    expected_revision: created.revision,
                    task_owner_agent_id: 1,
                    task_id: original.id,
                    title: "Updated specialist request".to_string(),
                    category: original.category,
                    priority: original.priority,
                    workspace_id: original.workspace_id.unwrap(),
                    routing_mode: "automatic".to_string(),
                    preferred_agent_id: Some(2),
                    selected_agent_id: None,
                    specialist_request: Some(updated_request.clone()),
                },
                &providers,
            )
            .unwrap();

        assert_eq!(
            task_by_title(&rerouted.state, "Updated specialist request")
                .specialist_request
                .as_ref(),
            Some(&updated_request)
        );
        let stored: String = repository
            .connection
            .query_row(
                "SELECT specialist_request_json FROM agent_tasks WHERE id = ?1",
                [original.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, updated_request.canonical_json().unwrap());
    }

    #[test]
    fn task_0011_schema_five_inflight_review_migrates_conservatively_to_human() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        for (version, name, sql) in [
            (1, "initial_application_state", INITIAL_MIGRATION),
            (
                2,
                "authoritative_approval_lifecycle",
                AUTHORIZATION_MIGRATION,
            ),
            (
                3,
                "authoritative_run_coordination",
                RUN_COORDINATION_MIGRATION,
            ),
            (4, "dynamic_agent_registry", AGENT_REGISTRY_MIGRATION),
            (
                5,
                "authoritative_task_orchestration",
                TASK_ORCHESTRATION_MIGRATION,
            ),
        ] {
            connection.execute_batch(sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at_unix_ms)
                     VALUES (?1, ?2, ?3)",
                    params![version, name, version],
                )
                .unwrap();
        }
        connection.pragma_update(None, "user_version", 5).unwrap();

        let mut state = authorization_state();
        state.preferences.review_mode = "manual".to_string();
        let task = &mut state.agents[1].tasks[0];
        task.status = "Under Review".to_string();
        task.phase = "Senior Review".to_string();
        task.queue_state = "notQueued".to_string();
        task.enqueue_sequence = None;
        task.review_agent_id = Some(3);
        task.review_status = "Running".to_string();
        task.review_result = Some("Legacy unbound review output".to_string());
        let mut pending_review = task.clone();
        pending_review.id = 42;
        pending_review.title = "Legacy pending review".to_string();
        pending_review.review_status = "Pending".to_string();
        state.agents[1].tasks.push(pending_review);
        connection
            .execute(
                "ALTER TABLE agent_tasks ADD COLUMN workspace_evidence_json TEXT",
                [],
            )
            .unwrap();
        connection
            .execute(
                "ALTER TABLE agent_tasks ADD COLUMN specialist_request_json TEXT",
                [],
            )
            .unwrap();
        for statement in [
            "ALTER TABLE agent_tasks ADD COLUMN created_at_unix_ms INTEGER",
            "ALTER TABLE agent_tasks ADD COLUMN completed_at_unix_ms INTEGER",
            "ALTER TABLE agent_activity ADD COLUMN created_at_unix_ms INTEGER",
            "ALTER TABLE reminders ADD COLUMN created_at_unix_ms INTEGER",
            "ALTER TABLE reminders ADD COLUMN resolved_at_unix_ms INTEGER",
        ] {
            connection.execute(statement, []).unwrap();
        }
        let transaction = connection.transaction().unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )
        .unwrap();
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = 1,
                     source_kind = 'fresh', source_version = NULL
                 WHERE singleton = 1",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
        connection
            .execute(
                "ALTER TABLE agent_tasks DROP COLUMN workspace_evidence_json",
                [],
            )
            .unwrap();
        connection
            .execute(
                "ALTER TABLE agent_tasks DROP COLUMN specialist_request_json",
                [],
            )
            .unwrap();
        for statement in [
            "ALTER TABLE agent_tasks DROP COLUMN created_at_unix_ms",
            "ALTER TABLE agent_tasks DROP COLUMN completed_at_unix_ms",
            "ALTER TABLE agent_activity DROP COLUMN created_at_unix_ms",
            "ALTER TABLE reminders DROP COLUMN created_at_unix_ms",
            "ALTER TABLE reminders DROP COLUMN resolved_at_unix_ms",
        ] {
            connection.execute(statement, []).unwrap();
        }
        drop(connection);

        let mut repository = StateRepository::open(&path).unwrap();
        assert_eq!(repository.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.flows.len(), 2);
        assert!(snapshot
            .flows
            .iter()
            .all(|flow| flow.latest_execution_attempt_id.is_some()));
        let flow = snapshot
            .flows
            .iter()
            .find(|flow| flow.task_id == 41)
            .unwrap();
        assert_eq!(flow.state, "awaiting_human");
        assert_eq!(flow.revision_round, 0);
        assert_eq!(
            flow.required_levels,
            vec![
                ReviewLevel::Senior,
                ReviewLevel::TeamLeader,
                ReviewLevel::Supervisor
            ]
        );
        assert_eq!(flow.current_level, Some(ReviewLevel::Senior));
        assert_eq!(
            flow.last_error_code.as_deref(),
            Some("LEGACY_REVIEW_UNBOUND")
        );
        assert!(flow.latest_execution_attempt_id.is_some());
        assert!(flow.stages.is_empty());

        let blocked = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: snapshot.revision,
                    task_owner_agent_id: 2,
                    task_id: 41,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        assert_eq!(
            blocked.blocked_code.as_deref(),
            Some("LEGACY_REVIEW_UNBOUND")
        );
        assert!(blocked.stage.is_none());
        assert!(blocked.context.is_none());
        let recovered = repository
            .record_human_review_decision(HumanReviewDecisionRequest {
                expected_revision: blocked.snapshot.revision,
                task_owner_agent_id: 2,
                task_id: 41,
                flow_id: flow.id,
                verdict: ReviewVerdict::ChangesRequested,
                feedback: "Run a fresh execution because legacy evidence is incomplete."
                    .to_string(),
            })
            .unwrap();
        let recovered_flow = recovered
            .flows
            .iter()
            .find(|flow| flow.task_id == 41)
            .unwrap();
        assert_eq!(recovered_flow.state, "revision_queued");
        assert_eq!(recovered_flow.revision_round, 1);
        let task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_eq!(task.status, "Pending");
        assert_eq!(task.queue_state, "queued");
        assert_eq!(
            repository
                .connection
                .query_row("PRAGMA foreign_key_check", [], |_| Ok(false))
                .optional()
                .unwrap(),
            None
        );
    }

    #[test]
    fn task_0010_persistence_allocates_monotonic_ids_and_orders_one_global_queue() {
        let (mut repository, configured) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let normal = repository
            .create_routed_task(
                task_0010_create_request(configured.revision, "Normal parser work", "Normal"),
                &providers,
            )
            .unwrap();
        let critical = repository
            .create_routed_task(
                task_0010_create_request(normal.revision, "Critical parser fix", "Critical"),
                &providers,
            )
            .unwrap();

        let normal_task = task_by_title(&critical.state, "Normal parser work");
        let critical_task = task_by_title(&critical.state, "Critical parser fix");
        assert_eq!((normal_task.id, normal_task.enqueue_sequence), (1, Some(1)));
        assert_eq!(
            (critical_task.id, critical_task.enqueue_sequence),
            (2, Some(2))
        );
        assert_eq!(normal_task.assigned_agent_id, 2);
        assert_eq!(critical_task.assigned_agent_id, 2);
        assert!(normal_task.routing_evidence.is_some());

        let snapshot = repository.task_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.execute_queue.len(), 2);
        assert_eq!(snapshot.execute_queue[0].task_id, critical_task.id);
        assert_eq!(snapshot.execute_queue[0].queue_position, Some(1));
        assert_eq!(snapshot.execute_queue[1].task_id, normal_task.id);
        assert_eq!(snapshot.execute_queue[1].queue_position, Some(2));
        assert!(snapshot.held_tasks.is_empty());
        assert!(snapshot.active_execute.is_none());
    }

    #[test]
    fn task_0010_persistence_enforces_head_admission_and_hold_preserves_age() {
        let (mut repository, configured) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let normal = repository
            .create_routed_task(
                task_0010_create_request(configured.revision, "Normal queued task", "Normal"),
                &providers,
            )
            .unwrap();
        let critical = repository
            .create_routed_task(
                task_0010_create_request(normal.revision, "Critical queued task", "Critical"),
                &providers,
            )
            .unwrap();
        let normal_task = task_by_title(&critical.state, "Normal queued task").clone();
        let critical_task = task_by_title(&critical.state, "Critical queued task").clone();
        let normal_intent = ActionIntent::RunTask {
            agent_id: normal_task.assigned_agent_id,
            task_owner_agent_id: 1,
            task_id: normal_task.id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        assert_eq!(
            repository
                .admit_run("task-0010-non-head", &normal_intent)
                .unwrap_err()
                .code,
            "QUEUE_HEAD_REQUIRED"
        );

        let held = repository
            .set_task_queue_disposition(SetTaskQueueDispositionRequest {
                expected_revision: critical.revision,
                task_owner_agent_id: 1,
                task_id: critical_task.id,
                disposition: QueueDisposition::Hold,
            })
            .unwrap();
        let held_task = task_by_title(&held.state, "Critical queued task");
        assert_eq!(held_task.queue_state, "held");
        assert_eq!(held_task.enqueue_sequence, critical_task.enqueue_sequence);
        let snapshot = repository.task_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.execute_queue[0].task_id, normal_task.id);
        assert_eq!(snapshot.held_tasks[0].task_id, critical_task.id);

        approve_authorization(&mut repository, &normal_intent);
        let admitted = repository
            .admit_run("task-0010-head", &normal_intent)
            .unwrap();
        assert_eq!(
            task_by_title(&admitted.application_state, "Normal queued task").queue_state,
            "admitted"
        );
    }

    #[test]
    fn task_0010_persistence_failure_and_cancellation_preserve_safe_queue_recovery() {
        let (mut repository, configured) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let high = repository
            .create_routed_task(
                task_0010_create_request(configured.revision, "Head task", "High"),
                &providers,
            )
            .unwrap();
        let second = repository
            .create_routed_task(
                task_0010_create_request(high.revision, "Later task", "Normal"),
                &providers,
            )
            .unwrap();
        let head_task = task_by_title(&second.state, "Head task").clone();
        let later_task = task_by_title(&second.state, "Later task").clone();
        let head_intent = ActionIntent::RunTask {
            agent_id: head_task.assigned_agent_id,
            task_owner_agent_id: 1,
            task_id: head_task.id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        let later_intent = ActionIntent::RunTask {
            agent_id: later_task.assigned_agent_id,
            task_owner_agent_id: 1,
            task_id: later_task.id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        approve_authorization(&mut repository, &head_intent);
        let admitted = repository
            .admit_run("task-0010-head-failure", &head_intent)
            .unwrap();
        assert_eq!(
            repository
                .admit_run("task-0010-parallel", &later_intent)
                .unwrap_err()
                .code,
            "RUN_BUSY"
        );
        repository
            .complete_run(
                admitted.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::StartupFailed,
                    "PROVIDER_START_FAILED",
                    "Provider startup failed before dispatch.",
                    0,
                ),
            )
            .unwrap();

        let snapshot = repository.task_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.execute_queue[0].task_id, head_task.id);
        assert_eq!(
            snapshot.execute_queue[0].enqueue_sequence,
            head_task.enqueue_sequence.unwrap()
        );
        assert_eq!(
            repository
                .admit_run("task-0010-skip-failed-head", &later_intent)
                .unwrap_err()
                .code,
            "QUEUE_HEAD_REQUIRED"
        );

        let cancelled_before_dispatch = repository
            .admit_run("task-0010-cancel-before-dispatch", &head_intent)
            .unwrap();
        repository
            .request_run_cancellation(cancelled_before_dispatch.attempt.id)
            .unwrap();
        repository
            .complete_run(
                cancelled_before_dispatch.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Cancelled,
                    "RUN_CANCELLED",
                    "Cancelled before provider dispatch.",
                    0,
                ),
            )
            .unwrap();
        let safely_requeued = repository.task_orchestration_snapshot().unwrap();
        assert_eq!(safely_requeued.execute_queue[0].task_id, head_task.id);
        assert_eq!(
            safely_requeued.execute_queue[0].enqueue_sequence,
            head_task.enqueue_sequence.unwrap()
        );

        let cancelled_during_dispatch = repository
            .admit_run("task-0010-cancel-during-dispatch", &head_intent)
            .unwrap();
        repository
            .prepare_run_attempt(
                cancelled_during_dispatch.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                None,
            )
            .unwrap();
        repository
            .mark_run_dispatching(cancelled_during_dispatch.attempt.id)
            .unwrap();
        repository
            .request_run_cancellation(cancelled_during_dispatch.attempt.id)
            .unwrap();
        repository
            .complete_run(
                cancelled_during_dispatch.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Cancelled,
                    "RUN_CANCELLED",
                    "Cancelled after provider dispatch became uncertain.",
                    0,
                ),
            )
            .unwrap();
        let held_after_uncertain_dispatch = repository.task_orchestration_snapshot().unwrap();
        assert_eq!(
            held_after_uncertain_dispatch.held_tasks[0].task_id,
            head_task.id
        );
        assert_eq!(
            held_after_uncertain_dispatch.held_tasks[0].enqueue_sequence,
            head_task.enqueue_sequence.unwrap()
        );
        approve_authorization(&mut repository, &later_intent);
        assert!(repository
            .admit_run("task-0010-next-after-hold", &later_intent)
            .is_ok());
    }

    #[test]
    fn task_0010_persistence_stale_create_rolls_back_allocator_and_renderer_cannot_forge_tasks() {
        let (mut repository, configured) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let first = repository
            .create_routed_task(
                task_0010_create_request(configured.revision, "First task", "Normal"),
                &providers,
            )
            .unwrap();
        assert_eq!(
            repository
                .create_routed_task(
                    task_0010_create_request(configured.revision, "Stale task", "Normal"),
                    &providers,
                )
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        let second = repository
            .create_routed_task(
                task_0010_create_request(first.revision, "Second task", "Normal"),
                &providers,
            )
            .unwrap();
        assert_eq!(task_by_title(&second.state, "Second task").id, 2);

        let mut forged = second.state.clone();
        let task = forged
            .agents
            .iter_mut()
            .flat_map(|agent| &mut agent.tasks)
            .find(|task| task.title == "First task")
            .unwrap();
        task.assigned_agent_id = 1;
        assert_eq!(
            repository
                .save(second.revision, &forged, false)
                .unwrap_err()
                .code,
            "TASK_ORCHESTRATION_AUTHORITY_REQUIRED"
        );
    }

    #[test]
    fn task_0010_persistence_reopens_queue_order_override_evidence_and_allocators() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let providers = task_0010_provider_snapshot();
        let expected_snapshot;
        {
            let mut repository = StateRepository::open(&path).unwrap();
            let configured = configure_task_0010_repository(&mut repository);
            let mut selected =
                task_0010_create_request(configured.revision, "Selected task", "Normal");
            selected.routing_mode = "selected".to_string();
            selected.selected_agent_id = Some(2);
            let selected_state = repository.create_routed_task(selected, &providers).unwrap();
            let critical = repository
                .create_routed_task(
                    task_0010_create_request(selected_state.revision, "Critical task", "Critical"),
                    &providers,
                )
                .unwrap();
            let selected_task = task_by_title(&critical.state, "Selected task");
            assert_eq!(
                selected_task
                    .routing_evidence
                    .as_ref()
                    .map(|evidence| evidence.outcome_code.as_str()),
                Some("MANUAL_SELECTION")
            );
            assert!(selected_task
                .routing_evidence
                .as_ref()
                .is_some_and(|evidence| evidence.manual_override));
            expected_snapshot = repository.task_orchestration_snapshot().unwrap();
        }

        let mut reopened = StateRepository::open(&path).unwrap();
        let reopened_snapshot = reopened.task_orchestration_snapshot().unwrap();
        assert_eq!(reopened_snapshot, expected_snapshot);
        let envelope = reopened.load().unwrap().unwrap();
        assert!(task_by_title(&envelope.state, "Selected task")
            .routing_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.manual_override));
        let third = reopened
            .create_routed_task(
                task_0010_create_request(envelope.revision, "Third task", "Low"),
                &providers,
            )
            .unwrap();
        let task = task_by_title(&third.state, "Third task");
        assert_eq!((task.id, task.enqueue_sequence), (3, Some(3)));
    }

    #[test]
    fn task_0010_persistence_reroute_preserves_age_and_terminal_reset_allocates_new_age() {
        let (mut repository, configured) = task_0010_repository();
        let providers = task_0010_provider_snapshot();
        let created = repository
            .create_routed_task(
                task_0010_create_request(configured.revision, "Lifecycle task", "Normal"),
                &providers,
            )
            .unwrap();
        let original = task_by_title(&created.state, "Lifecycle task").clone();
        let rerouted = repository
            .reroute_task(
                RerouteTaskRequest {
                    expected_revision: created.revision,
                    task_owner_agent_id: 1,
                    task_id: original.id,
                    title: original.title.clone(),
                    category: original.category.clone(),
                    priority: original.priority.clone(),
                    workspace_id: original.workspace_id.clone().unwrap(),
                    routing_mode: "selected".to_string(),
                    preferred_agent_id: Some(2),
                    selected_agent_id: Some(2),
                    specialist_request: original.specialist_request.clone(),
                },
                &providers,
            )
            .unwrap();
        let rerouted_task = task_by_title(&rerouted.state, "Lifecycle task");
        assert_eq!(rerouted_task.enqueue_sequence, original.enqueue_sequence);
        assert!(rerouted_task
            .routing_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.manual_override));

        let intent = ActionIntent::RunTask {
            agent_id: rerouted_task.assigned_agent_id,
            task_owner_agent_id: 1,
            task_id: rerouted_task.id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        approve_authorization(&mut repository, &intent);
        let attempt = repository.admit_run("task-0010-reset", &intent).unwrap();
        repository
            .prepare_run_attempt(attempt.attempt.id, "Ollama", "qwen2.5-coder:7b", None)
            .unwrap();
        repository.mark_run_dispatching(attempt.attempt.id).unwrap();
        repository.mark_run_started(attempt.attempt.id).unwrap();
        repository
            .complete_run(
                attempt.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Failed,
                    "TASK_FAILED",
                    "The task failed after dispatch.",
                    3,
                ),
            )
            .unwrap();
        let completed = repository.load().unwrap().unwrap();
        let completed_task = task_by_title(&completed.state, "Lifecycle task");
        assert_eq!(completed_task.queue_state, "notQueued");
        assert_eq!(completed_task.enqueue_sequence, None);

        let reset = repository
            .set_task_queue_disposition(SetTaskQueueDispositionRequest {
                expected_revision: completed.revision,
                task_owner_agent_id: 1,
                task_id: completed_task.id,
                disposition: QueueDisposition::ResetTerminal,
            })
            .unwrap();
        let reset_task = task_by_title(&reset.state, "Lifecycle task");
        assert_eq!(reset_task.queue_state, "queued");
        assert_eq!(reset_task.status, "Pending");
        assert!(reset_task.enqueue_sequence > original.enqueue_sequence);
    }

    #[test]
    fn task_0009_registry_crud_is_monotonic_persistent_and_template_restorable() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let created = repository
            .create_agent(CreateAgentRequest {
                expected_revision: initialized.revision,
                name: "Custom Builder".to_string(),
                description: "Builds custom workspace features".to_string(),
                role: "Specialist".to_string(),
                category: "Development".to_string(),
                reports_to: Some(3),
            })
            .unwrap();
        let custom = created
            .state
            .agents
            .iter()
            .find(|agent| agent.name == "Custom Builder")
            .unwrap();
        assert_eq!(custom.id, 12);
        assert_eq!(custom.template_key, None);
        assert_eq!(custom.registry_state, "active");
        let custom_id = custom.id;

        let updated = repository
            .update_agent(UpdateAgentRequest {
                expected_revision: created.revision,
                agent_id: custom_id,
                name: "Custom Builder Renamed".to_string(),
                description: "Builds custom workspace features safely".to_string(),
                role: "Specialist".to_string(),
                category: "General".to_string(),
                reports_to: Some(3),
            })
            .unwrap();
        assert_eq!(
            updated
                .state
                .agents
                .iter()
                .find(|agent| agent.id == custom_id)
                .unwrap()
                .name,
            "Custom Builder Renamed"
        );

        let deleted_custom = repository
            .delete_agent(DeleteAgentRequest {
                expected_revision: updated.revision,
                agent_id: custom_id,
                replacement_manager_id: None,
            })
            .unwrap();
        assert_eq!(
            deleted_custom
                .state
                .agents
                .iter()
                .find(|agent| agent.id == custom_id)
                .unwrap()
                .registry_state,
            "deleted"
        );

        let deleted_browser = repository
            .delete_agent(DeleteAgentRequest {
                expected_revision: deleted_custom.revision,
                agent_id: 4,
                replacement_manager_id: None,
            })
            .unwrap();
        assert_eq!(
            repository
                .load()
                .unwrap()
                .unwrap()
                .state
                .agents
                .iter()
                .find(|agent| agent.id == 4)
                .unwrap()
                .registry_state,
            "deleted"
        );

        let restored = repository
            .restore_agent_template(RestoreAgentTemplateRequest {
                expected_revision: deleted_browser.revision,
                template_key: "browser".to_string(),
                reports_to: None,
            })
            .unwrap();
        let browser = restored
            .state
            .agents
            .iter()
            .find(|agent| agent.template_key.as_deref() == Some("browser"))
            .unwrap();
        assert_eq!(browser.id, 4);
        assert_eq!(browser.registry_state, "active");
        assert_eq!(browser.reports_to, Some(9));

        let next = repository
            .create_agent(CreateAgentRequest {
                expected_revision: restored.revision,
                name: "Second Builder".to_string(),
                description: "Proves deleted identifiers are not reused".to_string(),
                role: "Specialist".to_string(),
                category: "Development".to_string(),
                reports_to: Some(3),
            })
            .unwrap();
        assert_eq!(
            next.state
                .agents
                .iter()
                .find(|agent| agent.name == "Second Builder")
                .unwrap()
                .id,
            13
        );
    }

    #[test]
    fn task_0009_created_and_deleted_agents_survive_database_reopen() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        {
            let mut repository = StateRepository::open(&path).unwrap();
            let initialized = repository.initialize_fresh().unwrap();
            let created = repository
                .create_agent(CreateAgentRequest {
                    expected_revision: initialized.revision,
                    name: "Persistent Custom Agent".to_string(),
                    description: "Must remain present after a database reopen".to_string(),
                    role: "Specialist".to_string(),
                    category: "Development".to_string(),
                    reports_to: Some(3),
                })
                .unwrap();
            repository
                .delete_agent(DeleteAgentRequest {
                    expected_revision: created.revision,
                    agent_id: 4,
                    replacement_manager_id: None,
                })
                .unwrap();
        }

        let mut reopened = StateRepository::open(&path).unwrap();
        let state = reopened.load().unwrap().unwrap().state;
        assert!(state.agents.iter().any(|agent| {
            agent.name == "Persistent Custom Agent" && agent.registry_state == "active"
        }));
        assert_eq!(
            state
                .agents
                .iter()
                .find(|agent| agent.id == 4)
                .unwrap()
                .registry_state,
            "deleted"
        );
        let browser_template = reopened
            .agent_registry_snapshot()
            .unwrap()
            .templates
            .into_iter()
            .find(|template| template.template_key == "browser")
            .unwrap();
        assert!(browser_template.restorable);
        assert_eq!(browser_template.active_agent_id, None);
    }

    #[test]
    fn task_0009_registry_rejects_invalid_hierarchy_and_renderer_bypass() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let error = repository
            .update_agent(UpdateAgentRequest {
                expected_revision: initialized.revision,
                agent_id: 2,
                name: "Coding Agent".to_string(),
                description: "Builds and edits project files".to_string(),
                role: "Specialist".to_string(),
                category: "Development".to_string(),
                reports_to: Some(2),
            })
            .unwrap_err();
        assert_eq!(error.code, "STATE_VALIDATION_FAILED");
        assert!(error.message.contains("cannot report to themselves"));
        assert_eq!(
            repository.load().unwrap().unwrap().revision,
            initialized.revision
        );

        let mut bypass = initialized.state;
        bypass.agents[1].name = "Renderer Renamed Agent".to_string();
        assert_eq!(
            repository
                .save(initialized.revision, &bypass, false)
                .unwrap_err()
                .code,
            "AGENT_REGISTRY_MUTATION_REQUIRED"
        );

        let receipt = install_authorization_fixture(&mut repository, initialized.revision);
        let deleted = repository
            .delete_agent(DeleteAgentRequest {
                expected_revision: receipt.revision,
                agent_id: 2,
                replacement_manager_id: None,
            })
            .unwrap();
        let retained_task = &deleted
            .state
            .agents
            .iter()
            .find(|agent| agent.id == 2)
            .unwrap()
            .tasks[0];
        assert_eq!(retained_task.status, "Failed");
        assert_eq!(retained_task.phase, "Failed");
        assert!(retained_task.completed_at.is_some());
        assert_eq!(
            retained_task.result.as_deref(),
            Some("Task closed because its owning agent was deleted.")
        );
        assert_eq!(
            repository
                .request_authorization(&authorization_intent())
                .unwrap_err()
                .code,
            "AGENT_REGISTRY_INACTIVE"
        );
    }

    #[test]
    fn task_0009_legacy_import_preserves_absence_and_quarantines_invalid_agents() {
        let mut legacy = default_application_state().unwrap();
        legacy.agents.retain(|agent| agent.id != 4);
        let coding = legacy
            .agents
            .iter_mut()
            .find(|agent| agent.id == 2)
            .unwrap();
        coding.reports_to = Some(999_999);
        let legacy = legacy_renderer_state(&legacy);

        let mut repository = StateRepository::open_in_memory().unwrap();
        let migrated = repository.migrate_legacy(&legacy).unwrap();
        assert_eq!(migrated.state.agents.len(), 10);
        assert!(migrated.state.agents.iter().all(|agent| agent.id != 4));
        let coding = migrated
            .state
            .agents
            .iter()
            .find(|agent| agent.id == 2)
            .unwrap();
        assert_eq!(coding.registry_state, "unassigned");
        assert_eq!(coding.registry_issue.as_deref(), Some("missing-manager"));
        assert_eq!(coding.reports_to, None);
        assert_eq!(coding.status, "Paused");
    }

    #[test]
    fn task_0021_migrate_legacy_repairs_duplicate_agent_identities() {
        // Reproduce the real TASK-0020 S3 blocker: 12 legacy agents, ids 1..11,
        // with two `id = 5` rows (`Finance Agent` -> Supervisor, `Financial
        // Agent` -> Finance Senior) and no template keys.
        let mut legacy_state = default_application_state().unwrap();
        for agent in &mut legacy_state.agents {
            agent.template_key = None;
        }
        let position = legacy_state
            .agents
            .iter()
            .position(|agent| agent.id == 5)
            .unwrap();
        legacy_state.agents[position].name = "Finance Agent".to_string();
        legacy_state.agents[position].reports_to = Some(1);
        let mut clone = legacy_state.agents[position].clone();
        clone.name = "Financial Agent".to_string();
        clone.reports_to = Some(10);
        legacy_state.agents.insert(position + 1, clone);
        let legacy = legacy_renderer_state(&legacy_state);

        let mut repository = StateRepository::open_in_memory().unwrap();
        let migrated = repository.migrate_legacy(&legacy).unwrap();

        assert_eq!(migrated.revision, 1);
        assert_eq!(
            migrated.migration.source_kind.as_deref(),
            Some("legacy_local_storage")
        );
        assert_eq!(migrated.state.agents.len(), 12);
        let mut ids: Vec<i64> = migrated.state.agents.iter().map(|agent| agent.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12, "every migrated agent id is unique");

        let canonical = migrated
            .state
            .agents
            .iter()
            .find(|agent| agent.name == "Finance Agent")
            .unwrap();
        assert_eq!(canonical.id, 5);
        assert_eq!(canonical.registry_state, "active");
        assert!(canonical.registry_issue.is_none());

        let requarantined = migrated
            .state
            .agents
            .iter()
            .find(|agent| agent.name == "Financial Agent")
            .unwrap();
        assert_ne!(requarantined.id, 5);
        assert_eq!(requarantined.registry_state, "unassigned");
        assert_eq!(
            requarantined.registry_issue.as_deref(),
            Some("duplicate-id")
        );
        assert_eq!(requarantined.status, "Paused");
        assert_eq!(requarantined.reports_to, None);

        // The migration is committed once; a retry returns the same state.
        let again = repository.migrate_legacy(&legacy).unwrap();
        assert_eq!(again, migrated);
    }

    #[test]
    fn task_0022_first_then_second_launch_recover_duplicate_identity_on_disk() {
        // TASK-0022: replay the real TASK-0020 S3 first launch against an
        // on-disk database, then prove the second launch is stable without
        // re-importing. The legacy store holds 12 agents (ids 1..11) with two
        // `id = 5` rows (`Finance Agent` -> Supervisor, `Financial Agent` ->
        // Finance Senior) and no template keys.
        let legacy = twelve_agent_finance_financial_legacy();

        let directory = TestDirectory::new();
        let path = directory.database_path();

        // First launch: fresh database, schema migrates to 11, the legacy
        // migration commits the repaired 12-agent registry.
        let first_revision;
        {
            let mut repository = StateRepository::open(&path).unwrap();
            assert_eq!(repository.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
            let user_version: i64 = repository
                .connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(user_version, 11);
            assert!(repository.load().unwrap().is_none());

            let migrated = repository.migrate_legacy(&legacy).unwrap();
            first_revision = migrated.revision;
            assert_eq!(migrated.revision, 1);
            assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
            assert_eq!(
                migrated.migration.source_kind.as_deref(),
                Some("legacy_local_storage")
            );
            assert!(!migrated.migration.legacy_cleanup_acknowledged);
            assert_eq!(migrated.state.agents.len(), 12);

            let requarantined = migrated
                .state
                .agents
                .iter()
                .find(|agent| agent.name == "Financial Agent")
                .unwrap();
            assert_ne!(requarantined.id, 5);
            assert_eq!(requarantined.registry_state, "unassigned");
            assert_eq!(
                requarantined.registry_issue.as_deref(),
                Some("duplicate-id")
            );
            assert_eq!(requarantined.status, "Paused");
            assert_eq!(requarantined.reports_to, None);

            // The renderer performs its one-time legacy cleanup acknowledgement
            // after clearing browser storage; the revision does not advance.
            let acknowledged = repository
                .acknowledge_legacy_cleanup(migrated.revision)
                .unwrap();
            assert!(acknowledged.migration.legacy_cleanup_acknowledged);
            assert_eq!(acknowledged.revision, first_revision);
        }

        // Second launch: the existing database loads directly. The
        // `duplicate-id` quarantine survived the round trip through the on-disk
        // CHECK constraint, and no re-import occurs even if the renderer still
        // offers the legacy keys.
        {
            let mut repository = StateRepository::open(&path).unwrap();
            let reopened = repository.load().unwrap().unwrap();
            assert_eq!(reopened.revision, first_revision);
            assert_eq!(reopened.schema_version, CURRENT_SCHEMA_VERSION);
            assert!(reopened.migration.legacy_cleanup_acknowledged);
            assert_eq!(reopened.state.agents.len(), 12);

            let mut ids: Vec<i64> = reopened.state.agents.iter().map(|agent| agent.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), 12, "every persisted agent id stays unique");

            let canonical = reopened
                .state
                .agents
                .iter()
                .find(|agent| agent.name == "Finance Agent")
                .unwrap();
            assert_eq!(canonical.id, 5);
            assert_eq!(canonical.registry_state, "active");
            assert!(canonical.registry_issue.is_none());

            let requarantined = reopened
                .state
                .agents
                .iter()
                .find(|agent| agent.name == "Financial Agent")
                .unwrap();
            assert_eq!(requarantined.registry_state, "unassigned");
            assert_eq!(
                requarantined.registry_issue.as_deref(),
                Some("duplicate-id")
            );

            // A renderer that still holds the legacy keys cannot trigger a
            // second migration or a second quarantined row.
            let replay = repository.migrate_legacy(&legacy).unwrap();
            assert_eq!(replay.revision, first_revision);
            assert_eq!(replay.state.agents.len(), 12);
            let quarantined_rows: i64 = repository
                .connection
                .query_row(
                    "SELECT count(*) FROM agents WHERE registry_issue = 'duplicate-id'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(quarantined_rows, 1);
        }

        // Third launch: still stable, still revision 1.
        {
            let mut repository = StateRepository::open(&path).unwrap();
            let reopened = repository.load().unwrap().unwrap();
            assert_eq!(reopened.revision, first_revision);
            assert_eq!(reopened.state.agents.len(), 12);
        }
    }

    #[test]
    fn task_0022_migrated_agents_check_constraint_permits_duplicate_id() {
        // TASK-0021 synced the migration-0004 `registry_issue` CHECK with the
        // recognised registry-issue contract. A database created before that
        // fix keeps the old five-value constraint and rejects a quarantined
        // `duplicate-id` row, which is why the disposable TASK-0020 S3
        // acceptance database must be removed rather than reused for the
        // migration replay.
        let repository = StateRepository::open_in_memory().unwrap();
        let agents_ddl: String = repository
            .connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            agents_ddl.contains("'duplicate-id'"),
            "migrated agents.registry_issue CHECK must permit 'duplicate-id': {agents_ddl}"
        );
    }

    /// Rewrites the persisted `agents` definition so its `registry_issue` CHECK
    /// predates the TASK-0021 fix, reproducing the schema of the real stale
    /// database that a pre-fix binary left on the acceptance machine.
    fn downgrade_persisted_agents_check(path: &Path) {
        let connection = Connection::open(path).unwrap();
        let current: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let pre_fix = current.replace(",\n            'duplicate-id'", "");
        assert_ne!(
            pre_fix, current,
            "fixture must remove the duplicate-id value"
        );
        connection
            .execute_batch("PRAGMA writable_schema = ON;")
            .unwrap();
        connection
            .execute(
                "UPDATE sqlite_master SET sql = ?1
                 WHERE type = 'table' AND name = 'agents'",
                [pre_fix],
            )
            .unwrap();
        connection
            .execute_batch("PRAGMA writable_schema = OFF;")
            .unwrap();
    }

    fn twelve_agent_finance_financial_legacy() -> LegacyRendererState {
        let mut legacy_state = default_application_state().unwrap();
        for agent in &mut legacy_state.agents {
            agent.template_key = None;
        }
        let position = legacy_state
            .agents
            .iter()
            .position(|agent| agent.id == 5)
            .unwrap();
        let mut clone = legacy_state.agents[position].clone();
        clone.name = "Financial Agent".to_string();
        clone.reports_to = Some(10);
        legacy_state.agents[position].name = "Finance Agent".to_string();
        legacy_state.agents[position].reports_to = Some(1);
        legacy_state.agents.insert(position + 1, clone);
        legacy_renderer_state(&legacy_state)
    }

    #[test]
    fn task_0022_open_rebuilds_an_uninitialized_database_with_a_superseded_schema() {
        let directory = TestDirectory::new();
        let path = directory.database_path();

        drop(StateRepository::open(&path).unwrap());
        downgrade_persisted_agents_check(&path);

        // The fixture is the real stuck state: a fully migrated but never
        // initialized database whose schema can no longer accept a quarantined
        // duplicate.
        {
            let stale = StateRepository::open_and_migrate(&path).unwrap();
            assert!(stale.is_superseded_uninitialized_shell().unwrap());
        }

        // `open` rebuilds the empty shell; the schema is current again and the
        // database is still uninitialized, ready for the legacy migration.
        let mut repository = StateRepository::open(&path).unwrap();
        assert!(!repository.is_superseded_uninitialized_shell().unwrap());
        assert!(repository.load().unwrap().is_none());
        let healed: String = repository
            .connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            healed.contains("'duplicate-id'"),
            "open must rebuild the superseded schema"
        );

        // The real duplicate-identity legacy migration now completes end to end.
        let migrated = repository
            .migrate_legacy(&twelve_agent_finance_financial_legacy())
            .unwrap();
        assert_eq!(migrated.state.agents.len(), 12);
        assert_eq!(
            migrated
                .state
                .agents
                .iter()
                .filter(|agent| agent.registry_issue.as_deref() == Some("duplicate-id"))
                .count(),
            1
        );
    }

    #[test]
    fn task_0022_open_preserves_an_initialized_database_with_a_drifted_schema() {
        let directory = TestDirectory::new();
        let path = directory.database_path();

        let baseline = {
            let mut repository = StateRepository::open(&path).unwrap();
            repository
                .migrate_legacy(&legacy_renderer_state(
                    &default_application_state().unwrap(),
                ))
                .unwrap()
        };
        downgrade_persisted_agents_check(&path);

        // The database carries committed user state, so `open` must not rebuild
        // it even though its schema no longer matches a fresh migration.
        let mut repository = StateRepository::open(&path).unwrap();
        assert!(!repository.is_superseded_uninitialized_shell().unwrap());
        let reopened = repository.load().unwrap().unwrap();
        assert_eq!(reopened.revision, baseline.revision);
        assert_eq!(reopened.state.agents.len(), baseline.state.agents.len());
        let agents_ddl: String = repository
            .connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'agents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !agents_ddl.contains("'duplicate-id'"),
            "an initialized database must never be rebuilt"
        );
    }

    #[test]
    fn stale_revision_is_rejected_without_changing_state() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut changed = initialized.state.clone();
        changed.preferences.theme = "light".to_string();
        let receipt = repository
            .save(initialized.revision, &changed, false)
            .unwrap();
        assert_eq!(receipt.revision, 2);

        let mut stale = changed.clone();
        stale.preferences.theme = "system".to_string();
        assert_eq!(
            repository
                .save(initialized.revision, &stale, false)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(
            repository.load().unwrap().unwrap().state.preferences.theme,
            "light"
        );
    }

    #[test]
    fn concurrent_connections_cannot_overwrite_a_newer_revision() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let mut first = StateRepository::open(&path).unwrap();
        let initialized = first.initialize_fresh().unwrap();
        let mut second = StateRepository::open(&path).unwrap();
        let second_snapshot = second.load().unwrap().unwrap();

        let mut first_change = initialized.state;
        first_change.preferences.theme = "light".to_string();
        assert_eq!(
            first
                .save(initialized.revision, &first_change, false)
                .unwrap()
                .revision,
            2
        );

        let mut stale_change = second_snapshot.state;
        stale_change.preferences.theme = "system".to_string();
        assert_eq!(
            second
                .save(second_snapshot.revision, &stale_change, false)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        let current = second.load().unwrap().unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.state.preferences.theme, "light");
    }

    #[test]
    fn interrupted_write_rolls_back_all_rows_and_revision() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut changed = initialized.state.clone();
        changed.preferences.theme = "light".to_string();
        assert_eq!(
            repository
                .simulate_interrupted_save(initialized.revision, &changed)
                .unwrap_err()
                .code,
            "INJECTED_INTERRUPTION"
        );
        let after = repository.load().unwrap().unwrap();
        assert_eq!(after.revision, initialized.revision);
        assert_eq!(after.state, initialized.state);
    }

    #[test]
    fn legacy_migration_is_atomic_idempotent_and_survives_restart() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let mut legacy_state = default_application_state().unwrap();
        legacy_state.preferences.theme = "light".to_string();
        legacy_state.approval_requests.push(ApprovalRequest {
            id: 100,
            agent_id: 1,
            task_id: None,
            title: "Legacy approval".to_string(),
            reason: "Preserve as non-authoritative history".to_string(),
            status: "Approved".to_string(),
            created_at: "2026-08-20T10:00:00.000Z".to_string(),
            resolved_at: Some("2026-08-20T10:01:00.000Z".to_string()),
            risk_level: "High".to_string(),
            scopes: vec!["files".to_string()],
            workspace_id: None,
            task_snapshot: "Legacy task".to_string(),
            expires_at: "2026-08-20T10:10:00.000Z".to_string(),
            consumed_at: None,
        });
        let legacy = legacy_renderer_state(&legacy_state);

        {
            let mut repository = StateRepository::open(&path).unwrap();
            let migrated = repository.migrate_legacy(&legacy).unwrap();
            assert_eq!(migrated.revision, 1);
            assert_eq!(migrated.state.preferences.theme, "light");
            assert_eq!(migrated.state.approval_requests[0].status, "Expired");
            assert_eq!(
                migrated.migration.source_kind.as_deref(),
                Some("legacy_local_storage")
            );
            assert_eq!(migrated.migration.source_version, Some(0));
            assert!(!migrated.migration.legacy_cleanup_acknowledged);
            let origin: (String, i64) = repository
                .connection
                .query_row(
                    "SELECT origin, authoritative FROM approval_requests WHERE id = 100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(origin, ("legacy_local_storage".to_string(), 0));

            let already_committed = repository
                .migrate_legacy(&LegacyRendererState {
                    agents: Some("{".to_string()),
                    ..LegacyRendererState::default()
                })
                .unwrap();
            assert_eq!(already_committed, migrated);

            let acknowledged = repository
                .acknowledge_legacy_cleanup(migrated.revision)
                .unwrap();
            assert!(acknowledged.migration.legacy_cleanup_acknowledged);
        }

        let mut repository = StateRepository::open(&path).unwrap();
        let reopened = repository.load().unwrap().unwrap();
        assert_eq!(reopened.state.preferences.theme, "light");
        assert_eq!(reopened.state.approval_requests[0].status, "Expired");
        assert!(reopened.migration.legacy_cleanup_acknowledged);
    }

    #[test]
    fn malformed_legacy_data_is_refused_without_partial_state() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let malformed = LegacyRendererState {
            agents: Some("not-json".to_string()),
            preferences: Some("{}".to_string()),
            ..LegacyRendererState::default()
        };
        assert_eq!(
            repository.migrate_legacy(&malformed).unwrap_err().code,
            "STATE_VALIDATION_FAILED"
        );
        assert!(repository.load().unwrap().is_none());

        let valid = legacy_renderer_state(&default_application_state().unwrap());
        assert_eq!(repository.migrate_legacy(&valid).unwrap().revision, 1);
    }

    #[test]
    fn interrupted_legacy_migration_rolls_back_and_can_retry() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let legacy = legacy_renderer_state(&default_application_state().unwrap());
        assert_eq!(
            repository
                .simulate_interrupted_migration(&legacy)
                .unwrap_err()
                .code,
            "INJECTED_MIGRATION_INTERRUPTION"
        );
        assert!(repository.load().unwrap().is_none());
        assert_eq!(repository.migrate_legacy(&legacy).unwrap().revision, 1);
    }

    #[test]
    fn legacy_backup_import_is_validated_downgraded_and_atomic() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut imported = initialized.state.clone();
        imported.preferences.theme = "light".to_string();
        let deleted_browser = imported
            .agents
            .iter_mut()
            .find(|agent| agent.id == 4)
            .unwrap();
        deleted_browser.registry_state = "deleted".to_string();
        deleted_browser.status = "Paused".to_string();
        deleted_browser.reports_to = None;
        deleted_browser.deleted_at_unix_ms = Some(1_777_000_000_000);
        imported.approval_requests.push(ApprovalRequest {
            id: 200,
            agent_id: 1,
            task_id: None,
            title: "Imported approval".to_string(),
            reason: "Legacy backup data".to_string(),
            status: "Pending".to_string(),
            created_at: "2026-08-21T10:00:00.000Z".to_string(),
            resolved_at: None,
            risk_level: "Medium".to_string(),
            scopes: vec!["terminal".to_string()],
            workspace_id: None,
            task_snapshot: "Imported task".to_string(),
            expires_at: "2026-08-21T10:30:00.000Z".to_string(),
            consumed_at: None,
        });
        let backup = serde_json::json!({
            "version": 2,
            "exportedAt": "2026-08-21T10:00:00.000Z",
            "agents": imported.agents,
            "models": imported.models,
            "approvalRequests": imported.approval_requests,
            "reminders": imported.reminders,
            "taskRetentionDays": imported.task_retention_days,
            "activityRetentionDays": imported.activity_retention_days,
            "preferences": imported.preferences,
        })
        .to_string();

        let envelope = repository
            .apply_backup_import(initialized.revision, &backup)
            .unwrap();
        assert_eq!(envelope.revision, 2);
        assert_eq!(envelope.state.preferences.theme, "light");
        assert_eq!(envelope.state.approval_requests[0].status, "Expired");
        assert_eq!(
            envelope
                .state
                .agents
                .iter()
                .find(|agent| agent.id == 4)
                .unwrap()
                .registry_state,
            "deleted"
        );
        assert_eq!(
            envelope.migration.source_kind.as_deref(),
            Some("legacy_backup")
        );
        assert_eq!(envelope.migration.source_version, Some(2));
        let before_invalid = envelope.state;

        assert_eq!(
            repository
                .apply_backup_import(envelope.revision, "{\"version\":2}")
                .unwrap_err()
                .code,
            "STATE_VALIDATION_FAILED"
        );
        let after_invalid = repository.load().unwrap().unwrap();
        assert_eq!(after_invalid.revision, envelope.revision);
        assert_eq!(after_invalid.state, before_invalid);
    }

    #[test]
    fn task_0014_backup_repository_preview_apply_is_atomic_and_revision_guarded() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let exported = repository.export_backup().unwrap();
        let mut backup: serde_json::Value = serde_json::from_str(&exported.backup_json).unwrap();
        backup["data"]["preferences"]["theme"] = serde_json::json!("light");
        let backup_json = serde_json::to_string(&backup).unwrap();

        let preview = repository
            .preview_backup_import(initialized.revision, &backup_json)
            .unwrap();
        assert_eq!(preview.format_version, 4);
        assert_eq!(preview.source_schema_version, Some(CURRENT_SCHEMA_VERSION));
        assert!(preview.replaces_current_state);
        assert!(preview.clears_run_and_review_history);

        let imported = repository
            .apply_backup_import(initialized.revision, &backup_json)
            .unwrap();
        assert_eq!(imported.revision, initialized.revision + 1);
        assert_eq!(imported.state.preferences.theme, "light");
        assert_eq!(imported.migration.source_kind.as_deref(), Some("backup_v4"));
        assert_eq!(imported.migration.source_version, Some(4));

        assert_eq!(
            repository
                .apply_backup_import(initialized.revision, &backup_json)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(
            repository
                .apply_backup_import(imported.revision, "{\"version\":3}")
                .unwrap_err()
                .code,
            "STATE_VALIDATION_FAILED"
        );
        let after_failure = repository.load().unwrap().unwrap();
        assert_eq!(after_failure.revision, imported.revision);
        assert_eq!(after_failure.state, imported.state);
    }

    #[test]
    fn task_0014_normalized_timestamps_survive_unrelated_state_saves() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut state = authorization_state();
        state.agents[1].tasks[0].created_at = "legacy-task-time".to_string();
        state.agents[1].activity = vec![ActivityEntry {
            id: 7,
            message: "Legacy activity".to_string(),
            created_at: "legacy-activity-time".to_string(),
        }];
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )
        .unwrap();
        let revision = next_revision(initialized.revision).unwrap();
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [revision],
            )
            .unwrap();
        advance_task_orchestration_revision(&transaction).unwrap();
        transaction.commit().unwrap();
        let scheduled = repository
            .create_scheduled_item(task_0018_schedule_request(
                "task-0014-timestamp-stability",
                DeliveryMode::InApp,
                RecurrenceRuleV1::default(),
            ))
            .unwrap();
        let reminder_id = scheduled.items[0].id;
        let revision = scheduled.application_state_revision;
        repository
            .connection
            .execute(
                "UPDATE agent_tasks SET created_at_unix_ms = 101 WHERE owner_agent_id = 2 AND id = 41",
                [],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "UPDATE agent_activity SET created_at_unix_ms = 102 WHERE owner_agent_id = 2 AND id = 7",
                [],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "UPDATE reminders SET status = 'completed', created_at_unix_ms = 103,
                         resolved_at_unix_ms = 104, updated_at_unix_ms = 104
                 WHERE id = ?1",
                [reminder_id],
            )
            .unwrap();

        let mut unchanged = repository.load().unwrap().unwrap().state;
        unchanged.preferences.theme = "light".to_string();
        repository.save(revision, &unchanged, false).unwrap();

        let task_created: i64 = repository
            .connection
            .query_row(
                "SELECT created_at_unix_ms FROM agent_tasks WHERE owner_agent_id = 2 AND id = 41",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let activity_created: i64 = repository
            .connection
            .query_row(
                "SELECT created_at_unix_ms FROM agent_activity WHERE owner_agent_id = 2 AND id = 7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let reminder_timestamps: (i64, i64) = repository
            .connection
            .query_row(
                "SELECT created_at_unix_ms, resolved_at_unix_ms FROM reminders WHERE id = ?1",
                [reminder_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(task_created, 101);
        assert_eq!(activity_created, 102);
        assert_eq!(reminder_timestamps, (103, 104));
    }

    #[test]
    fn task_0014_retention_prunes_terminal_history_and_protects_active_work() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let maintenance_time = 2_000_000_000_000_i64;
        let old_time = maintenance_time - 10 * 86_400_000;
        let old_timestamp = format_unix_ms(old_time);
        let mut state = initialized.state;
        let owner_id = state.agents[0].id;
        let terminal_task = AgentTask {
            id: 101,
            title: "Old terminal task".to_string(),
            category: "Development".to_string(),
            priority: "Normal".to_string(),
            assigned_agent_id: owner_id,
            status: "Completed".to_string(),
            phase: "Finished".to_string(),
            created_at: old_timestamp.clone(),
            completed_at: Some(old_timestamp.clone()),
            result: Some("done".to_string()),
            response_id: None,
            runtime_model: None,
            total_tokens: None,
            workspace_id: None,
            specialist_request: None,
            changed_files: Vec::new(),
            diff: None,
            workspace_changes: None,
            duration_seconds: None,
            routing_mode: "selected".to_string(),
            routed_from_agent_id: None,
            routing_reason: None,
            queue_state: "notQueued".to_string(),
            enqueue_sequence: None,
            routing_evidence: None,
            review_agent_id: None,
            review_status: "Not Requested".to_string(),
            review_result: None,
            review_model: None,
            review_duration_seconds: None,
            reviewed_at: None,
        };
        let mut protected_terminal_task = terminal_task.clone();
        protected_terminal_task.id = 102;
        protected_terminal_task.title = "Protected terminal task".to_string();
        let mut active_task = terminal_task.clone();
        active_task.id = 103;
        active_task.title = "Active queued task".to_string();
        active_task.status = "Pending".to_string();
        active_task.phase = "Assigned".to_string();
        active_task.completed_at = None;
        active_task.result = None;
        active_task.queue_state = "queued".to_string();
        active_task.enqueue_sequence = Some(1);
        state.agents[0].tasks = vec![terminal_task, protected_terminal_task, active_task];
        state.agents[0].activity.push(ActivityEntry {
            id: 201,
            message: "Old activity".to_string(),
            created_at: old_timestamp.clone(),
        });
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let meta = application_meta_from(&transaction).unwrap();
        ensure_expected_revision(&meta, initialized.revision).unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )
        .unwrap();
        let saved_revision = next_revision(meta.state_revision).unwrap();
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [saved_revision],
            )
            .unwrap();
        advance_task_orchestration_revision(&transaction).unwrap();
        transaction.commit().unwrap();
        let scheduled = repository
            .create_scheduled_item(task_0018_schedule_request(
                "task-0014-retention-reminder",
                DeliveryMode::InApp,
                RecurrenceRuleV1::default(),
            ))
            .unwrap();
        let reminder_id = scheduled.items[0].id;
        let saved_revision = scheduled.application_state_revision;
        repository
            .connection
            .execute(
                "UPDATE reminders
                 SET status = 'completed', created_at = ?1,
                     created_at_unix_ms = ?2, resolved_at_unix_ms = ?2,
                     updated_at_unix_ms = ?2
                 WHERE id = ?3",
                params![old_timestamp, old_time, reminder_id],
            )
            .unwrap();

        let expired_approval = ApprovalRequest {
            id: 401,
            agent_id: owner_id,
            task_id: None,
            title: "Expired history".to_string(),
            reason: "Retention fixture".to_string(),
            status: "Expired".to_string(),
            created_at: old_timestamp.clone(),
            resolved_at: Some(old_timestamp.clone()),
            risk_level: "Low".to_string(),
            scopes: vec!["files".to_string()],
            workspace_id: None,
            task_snapshot: "fixture".to_string(),
            expires_at: old_timestamp.clone(),
            consumed_at: None,
        };
        let mut active_approval = expired_approval.clone();
        active_approval.id = 402;
        active_approval.title = "Protected pending approval".to_string();
        active_approval.status = "Pending".to_string();
        active_approval.resolved_at = None;
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        write_approval_request(
            &transaction,
            0,
            &expired_approval,
            "legacy_backup",
            old_time,
        )
        .unwrap();
        write_approval_request(&transaction, 1, &active_approval, "legacy_backup", old_time)
            .unwrap();
        transaction
            .execute(
                "INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                  workspace_fingerprint, agent_id, task_owner_agent_id, task_id,
                  task_title, run_mode, status, task_status_before, task_phase_before,
                  review_status_before, admitted_at_unix_ms)
                 VALUES ('task-0014-active', '{}', 'active', 'active', 'active',
                         ?1, ?1, 102, 'Protected terminal task', 'execute', 'running',
                         'Pending', 'Assigned', 'Not Requested', ?2)",
                params![owner_id, old_time],
            )
            .unwrap();
        let active_attempt_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "UPDATE run_coordinator_meta
                 SET active_attempt_id = ?1, retained_attempt_count = 1
                 WHERE singleton = 1",
                [active_attempt_id],
            )
            .unwrap();
        transaction
            .execute(
                "UPDATE retention_settings
                 SET task_retention = '7', activity_retention = '7'
                 WHERE singleton = 1",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();

        let result = repository
            .run_data_lifecycle_maintenance("test", maintenance_time)
            .unwrap();
        assert_eq!(result.status, "succeeded");
        assert_eq!(result.pruned.tasks, 1);
        assert_eq!(result.pruned.activity, 1);
        assert_eq!(result.pruned.approvals, 1);
        assert_eq!(result.pruned.reminders, 1);
        assert!(result.skipped_protected >= 2);
        assert_eq!(result.application_state_revision, saved_revision + 1);
        let after = repository.load().unwrap().unwrap();
        let task_ids = after.state.agents[0]
            .tasks
            .iter()
            .map(|task| task.id)
            .collect::<HashSet<_>>();
        assert_eq!(task_ids, HashSet::from([102, 103]));
        assert!(after.state.agents[0].activity.is_empty());
        assert_eq!(after.state.approval_requests.len(), 1);
        assert_eq!(after.state.approval_requests[0].id, 402);
        assert!(after.state.reminders.is_empty());

        let rollback = repository
            .run_data_lifecycle_maintenance("test", maintenance_time - 1)
            .unwrap();
        assert_eq!(rollback.status, "clock_rollback");
        assert_eq!(rollback.pruned, RetentionPruneCounts::default());
        assert_eq!(rollback.error_code.as_deref(), Some("CLOCK_ROLLBACK"));
    }

    #[test]
    fn task_0014_retention_is_bounded_and_reports_backlog() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let maintenance_time = 2_000_000_000_000_i64;
        let old_timestamp = format_unix_ms(maintenance_time - 10 * 86_400_000);
        let mut state = initialized.state;
        state.agents[0].activity = (1..=501)
            .map(|id| ActivityEntry {
                id,
                message: format!("Old activity {id}"),
                created_at: old_timestamp.clone(),
            })
            .collect();
        repository
            .save(initialized.revision, &state, false)
            .unwrap();
        repository
            .connection
            .execute(
                "UPDATE retention_settings SET activity_retention = '7' WHERE singleton = 1",
                [],
            )
            .unwrap();

        let first = repository
            .run_data_lifecycle_maintenance("test", maintenance_time)
            .unwrap();
        assert_eq!(first.pruned.activity, MAX_MAINTENANCE_ROWS_PER_DOMAIN);
        assert!(first.backlog_remaining);
        let second = repository
            .run_data_lifecycle_maintenance("test", maintenance_time)
            .unwrap();
        assert_eq!(second.pruned.activity, 1);
        assert!(!second.backlog_remaining);
        let remaining: i64 = repository
            .connection
            .query_row("SELECT COUNT(*) FROM agent_activity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn task_0014_monitoring_is_transactional_paged_and_activity_scoped() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut state = authorization_state();
        state.agents[1].activity = vec![
            ActivityEntry {
                id: 1,
                message: "Older local activity".to_string(),
                created_at: "2026-08-25T10:00:00.000Z".to_string(),
            },
            ActivityEntry {
                id: 2,
                message: "Newer local activity".to_string(),
                created_at: "2026-08-26T10:00:00.000Z".to_string(),
            },
        ];
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let meta = application_meta_from(&transaction).unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )
        .unwrap();
        let revision = next_revision(meta.state_revision).unwrap();
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [revision],
            )
            .unwrap();
        advance_task_orchestration_revision(&transaction).unwrap();
        transaction
            .execute(
                "INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                  workspace_fingerprint, agent_id, task_owner_agent_id, task_id,
                  task_title, run_mode, status, task_status_before, task_phase_before,
                  review_status_before, admitted_at_unix_ms, completed_at_unix_ms)
                 VALUES ('task-0014-monitoring-run', '{}', 'history', 'history', 'history',
                         2, 2, 41, 'Monitoring task', 'execute', 'failed', 'Pending',
                         'Assigned', 'Not Requested', 1800000000000, 1800000000001)",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();

        let snapshot = repository.monitoring_snapshot().unwrap();
        assert!(snapshot.authoritative);
        assert_eq!(
            snapshot.revision.application_state,
            initialized.revision + 1
        );
        assert_eq!(snapshot.counts.total_tasks, 1);
        assert_eq!(snapshot.counts.pending_tasks, 1);
        assert_eq!(snapshot.counts.activity_entries, 2);
        assert_eq!(snapshot.counts.retained_run_attempts, 1);

        let tasks = repository
            .query_monitoring_tasks(
                &snapshot.revision,
                Some("Pending"),
                Some("Development"),
                0,
                MONITORING_PAGE_LIMIT,
            )
            .unwrap();
        assert_eq!(tasks.total, 1);
        assert_eq!(tasks.records[0].task.id, 41);
        assert_eq!(tasks.records[0].owner_name, "Coding Agent");
        assert_eq!(
            repository
                .query_monitoring_tasks(
                    &snapshot.revision,
                    None,
                    None,
                    0,
                    MONITORING_PAGE_LIMIT + 1,
                )
                .unwrap_err()
                .code,
            "MONITORING_PAGE_INVALID"
        );

        let activity = repository
            .query_monitoring_activity(&snapshot.revision, 0, 1)
            .unwrap();
        assert_eq!(activity.total, 2);
        assert_eq!(activity.records.len(), 1);
        assert_eq!(activity.records[0].entry_id, 2);

        let observed_time: i64 = repository
            .connection
            .query_row(
                "SELECT MAX(created_at_unix_ms) FROM agent_activity",
                [],
                |row| row.get(0),
            )
            .unwrap();
        repository
            .run_data_lifecycle_maintenance("test", observed_time + 1)
            .unwrap();
        assert_eq!(
            repository
                .query_monitoring_activity(&snapshot.revision, 0, 1)
                .unwrap_err()
                .code,
            "MONITORING_REVISION_CONFLICT"
        );
        let refreshed = repository.monitoring_snapshot().unwrap();
        let deleted = repository
            .delete_monitoring_activity(&refreshed.revision, 2, 2)
            .unwrap();
        assert_eq!(deleted.deleted_count, 1);
        assert_eq!(deleted.snapshot.counts.activity_entries, 1);
        assert_eq!(deleted.snapshot.counts.retained_run_attempts, 1);
        let cleared = repository
            .clear_monitoring_activity(&deleted.snapshot.revision)
            .unwrap();
        assert_eq!(cleared.deleted_count, 1);
        assert_eq!(cleared.snapshot.counts.activity_entries, 0);
        assert_eq!(cleared.snapshot.counts.retained_run_attempts, 1);
    }

    #[test]
    fn future_schema_and_corrupt_database_are_refused_without_overwrite() {
        let future_directory = TestDirectory::new();
        let future_path = future_directory.database_path();
        let connection = Connection::open(&future_path).unwrap();
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        assert_eq!(
            StateRepository::open(&future_path).unwrap_err().code,
            "UNSUPPORTED_NEWER_SCHEMA"
        );
        let connection = Connection::open(&future_path).unwrap();
        let journal_mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(connection);

        let corrupt_directory = TestDirectory::new();
        let corrupt_path = corrupt_directory.database_path();
        fs::write(&corrupt_path, b"not a sqlite database").unwrap();
        let original = fs::read(&corrupt_path).unwrap();
        assert_eq!(
            StateRepository::open(&corrupt_path).unwrap_err().code,
            "DATABASE_CORRUPT"
        );
        assert_eq!(fs::read(&corrupt_path).unwrap(), original);
    }

    fn authorization_state() -> ApplicationState {
        let mut state = default_application_state().unwrap();
        state.preferences.workspaces.push(WorkspaceDefinition {
            id: "workspace-authorization".to_string(),
            name: "Authorization fixture".to_string(),
            path: "/tmp/authorization-fixture".to_string(),
        });
        state.preferences.active_workspace_id = Some("workspace-authorization".to_string());
        state.preferences.workspace_path = "/tmp/authorization-fixture".to_string();
        state.preferences.active_ai_provider = "ollama".to_string();
        state.agents[1].tasks.push(AgentTask {
            id: 41,
            title: "Run cargo test and edit the parser".to_string(),
            category: "Development".to_string(),
            priority: "Normal".to_string(),
            assigned_agent_id: 2,
            status: "Pending".to_string(),
            phase: "Assigned".to_string(),
            created_at: "2026-08-23T10:00:00.000Z".to_string(),
            completed_at: None,
            result: None,
            response_id: None,
            runtime_model: None,
            total_tokens: None,
            workspace_id: Some("workspace-authorization".to_string()),
            specialist_request: Some(coding_request("Run cargo test and edit the parser")),
            changed_files: Vec::new(),
            diff: None,
            workspace_changes: None,
            duration_seconds: None,
            routing_mode: "selected".to_string(),
            routed_from_agent_id: None,
            routing_reason: None,
            queue_state: "queued".to_string(),
            enqueue_sequence: Some(1),
            routing_evidence: None,
            review_agent_id: None,
            review_status: "Not Requested".to_string(),
            review_result: None,
            review_model: None,
            review_duration_seconds: None,
            reviewed_at: None,
        });
        state
    }

    fn authorization_intent() -> ActionIntent {
        ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id: 41,
            run_mode: RunMode::Execute,
            review_context: None,
        }
    }

    fn install_authorization_fixture(
        repository: &mut StateRepository,
        expected_revision: i64,
    ) -> StateEnvelope {
        let state = authorization_state();
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let meta = application_meta_from(&transaction).unwrap();
        ensure_expected_revision(&meta, expected_revision).unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            false,
        )
        .unwrap();
        let revision = next_revision(meta.state_revision).unwrap();
        transaction
            .execute(
                "UPDATE application_meta SET state_revision = ?1 WHERE singleton = 1",
                [revision],
            )
            .unwrap();
        advance_task_orchestration_revision(&transaction).unwrap();
        transaction.commit().unwrap();
        repository.load().unwrap().unwrap()
    }

    fn initialized_authorization_repository() -> StateRepository {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        install_authorization_fixture(&mut repository, initialized.revision);
        repository
    }

    fn approve_authorization(repository: &mut StateRepository, intent: &ActionIntent) -> i64 {
        let pending = repository
            .request_authorization(intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        pending.id
    }

    fn successful_completion(output: &str) -> RunCompletion {
        RunCompletion {
            status: RunAttemptStatus::Succeeded,
            output_summary: Some(output.to_string()),
            stderr_excerpt: None,
            response_id: Some("response-1".to_string()),
            runtime_model: Some("runtime-model".to_string()),
            usage: RunUsage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
            },
            changed_files: Vec::new(),
            diff: None,
            workspace_changes: WorkspaceChangeEvidenceV1::complete_without_changes(
                crate::workspace_evidence::WorkspaceEvidenceMode::Filesystem,
            ),
            specialist_result: Some(SpecialistResultV1::Coding(CodingResultV1 {
                summary: "The bounded fixture change completed.".to_string(),
                changes: vec![],
                verification: vec![],
                evidence_refs: vec![],
                limitations: vec![],
            })),
            duration_seconds: 3,
            error_code: None,
            error_message: None,
            truncation: RunTruncationEvidence::default(),
            recovery_disposition: None,
        }
    }

    #[test]
    fn task_0012_structured_evidence_is_persisted_projected_protected_and_immutable() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        approve_authorization(&mut repository, &intent);
        let admitted = repository.admit_run("task-0012-evidence", &intent).unwrap();
        repository
            .prepare_run_attempt(admitted.attempt.id, "Ollama", "fixture-model", None)
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();
        let completion = successful_completion("Structured evidence persisted.");
        let completed = repository
            .complete_run(admitted.attempt.id, &completion)
            .unwrap();
        assert_eq!(completed.workspace_changes, completion.workspace_changes);

        let (stored_json, payload_bytes): (String, i64) = repository
            .connection
            .query_row(
                "SELECT workspace_evidence_json, payload_bytes
                 FROM run_attempts WHERE id = ?1",
                [admitted.attempt.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<WorkspaceChangeEvidenceV1>(&stored_json).unwrap(),
            completion.workspace_changes
        );
        assert!(payload_bytes >= stored_json.len() as i64);

        let envelope = repository.load().unwrap().unwrap();
        assert_eq!(
            envelope.state.agents[1].tasks[0].workspace_changes.as_ref(),
            Some(&completion.workspace_changes)
        );
        let mut forged = envelope.state;
        forged.agents[1].tasks[0].workspace_changes = None;
        repository
            .save(envelope.revision, &forged, true)
            .expect("generic renderer save should preserve backend-owned evidence");
        assert_eq!(
            repository.load().unwrap().unwrap().state.agents[1].tasks[0]
                .workspace_changes
                .as_ref(),
            Some(&completion.workspace_changes)
        );
        assert!(repository
            .connection
            .execute(
                "UPDATE run_attempts SET workspace_evidence_json = NULL WHERE id = ?1",
                [admitted.attempt.id],
            )
            .is_err());
    }

    fn task_0011_repository() -> (StateRepository, i64) {
        let (mut repository, configured) = task_0010_repository();
        let mut state = configured.state;
        state.preferences.review_mode = "manual".to_string();
        for agent_id in [1, 3, 6] {
            let reviewer = state
                .agents
                .iter_mut()
                .find(|agent| agent.id == agent_id)
                .unwrap();
            reviewer.model = "qwen2.5-coder:7b".to_string();
            reviewer.approvals.files = "allow".to_string();
        }
        let saved = repository.save(configured.revision, &state, true).unwrap();
        let created = repository
            .create_routed_task(
                CreateRoutedTaskRequest {
                    expected_revision: saved.revision,
                    task_owner_agent_id: 2,
                    title: "Implement parser boundary".to_string(),
                    category: "Development".to_string(),
                    priority: "Normal".to_string(),
                    workspace_id: "workspace-1".to_string(),
                    routing_mode: "selected".to_string(),
                    preferred_agent_id: Some(2),
                    selected_agent_id: Some(2),
                    specialist_request: Some(coding_request("Implement parser boundary")),
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let task_id = created.state.agents[1].tasks[0].id;
        (repository, task_id)
    }

    fn task_0011_execute(
        repository: &mut StateRepository,
        task_id: i64,
        request_id: &str,
    ) -> RunAttemptProjection {
        let intent = ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        approve_authorization(repository, &intent);
        let admitted = repository.admit_run(request_id, &intent).unwrap();
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                Some("workspace-1"),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();
        repository
            .complete_run(
                admitted.attempt.id,
                &successful_completion("Implemented and verified the parser boundary."),
            )
            .unwrap()
    }

    fn task_0011_result_json(request_json: &str, verdict: ReviewVerdict) -> String {
        let request: ReviewRequestV1 = serde_json::from_str(request_json).unwrap();
        serde_json::to_string(&ReviewResultV1 {
            schema_version: 1,
            flow_id: request.flow_id,
            task_id: request.subject.task_id,
            revision_round: request.revision_round,
            level: request.level,
            stage_attempt_id: request.stage_attempt_id,
            request_fingerprint: request.request_fingerprint,
            verdict,
            checks: REQUIRED_REVIEW_CHECKS
                .iter()
                .copied()
                .map(|check| ReviewCheckResultV1 {
                    check,
                    status: if verdict == ReviewVerdict::Approved {
                        ReviewCheckStatus::Pass
                    } else if check == ReviewCheckKind::Correctness {
                        ReviewCheckStatus::Fail
                    } else {
                        ReviewCheckStatus::Pass
                    },
                    evidence_ids: vec!["execution.summary".to_string()],
                    finding: "Bound evidence inspected.".to_string(),
                })
                .collect(),
            blocking_issues: if verdict == ReviewVerdict::Approved {
                Vec::new()
            } else {
                vec!["The boundary behavior needs another revision.".to_string()]
            },
            feedback: if verdict == ReviewVerdict::Approved {
                String::new()
            } else {
                "Correct the boundary behavior and rerun verification.".to_string()
            },
        })
        .unwrap()
    }

    fn task_0011_review_stage(
        repository: &mut StateRepository,
        task_id: i64,
        request_id: &str,
        verdict: ReviewVerdict,
    ) -> (ReviewStageStart, RunAttemptProjection) {
        let snapshot = repository.review_orchestration_snapshot().unwrap();
        let start = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: snapshot.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let context = start.context.clone().unwrap();
        let reviewer_agent_id = start.stage.as_ref().unwrap().reviewer_agent_id.unwrap();
        let intent = ActionIntent::RunTask {
            agent_id: reviewer_agent_id,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: Some(context),
        };
        assert_eq!(
            repository.request_authorization(&intent).unwrap().decision,
            crate::authorization::AuthorizationDecision::Allowed
        );
        let admitted = repository.admit_run(request_id, &intent).unwrap();
        let output =
            task_0011_result_json(admitted.review_request_json.as_deref().unwrap(), verdict);
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                Some("workspace-1"),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();
        let completion = RunCompletion {
            changed_files: Vec::new(),
            diff: None,
            specialist_result: None,
            ..successful_completion(&output)
        };
        let completed = repository
            .complete_run(admitted.attempt.id, &completion)
            .unwrap();
        (start, completed)
    }

    #[test]
    fn task_0011_backend_runs_exact_reporting_chain_and_structured_completion() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-execute-0");
        for (index, (level, reviewer)) in [
            (ReviewLevel::Senior, 3),
            (ReviewLevel::TeamLeader, 6),
            (ReviewLevel::Supervisor, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let snapshot = repository.review_orchestration_snapshot().unwrap();
            let flow = &snapshot.flows[0];
            assert_eq!(flow.state, "awaiting_review");
            assert_eq!(flow.current_level, Some(level));
            let (start, _) = task_0011_review_stage(
                &mut repository,
                task_id,
                &format!("task-0011-review-{index}"),
                ReviewVerdict::Approved,
            );
            assert_eq!(start.stage.unwrap().reviewer_agent_id, Some(reviewer));
        }
        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.flows[0].state, "completed");
        let task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_eq!(task.status, "Completed");
        assert_eq!(task.phase, "Finished");
        assert_eq!(task.review_status, "Approved");
    }

    #[test]
    fn task_0018_management_handoffs_follow_task_execution_and_review_sequentially() {
        let (mut repository, task_id) = task_0011_repository();
        let initial = repository.management_handoff_snapshot().unwrap();
        assert_eq!(
            initial
                .handoffs
                .iter()
                .map(|handoff| handoff.kind)
                .collect::<Vec<_>>(),
            vec![
                ManagementHandoffKind::TaskPlan,
                ManagementHandoffKind::Assignment,
            ]
        );

        let execution = task_0011_execute(&mut repository, task_id, "task-0018-handoff-execution");
        task_0011_review_stage(
            &mut repository,
            task_id,
            "task-0018-handoff-review",
            ReviewVerdict::ChangesRequested,
        );
        let snapshot = repository.management_handoff_snapshot().unwrap();
        crate::management_handoffs::validate_sequential_handoffs(&snapshot.handoffs).unwrap();
        assert_eq!(
            snapshot
                .handoffs
                .iter()
                .map(|handoff| handoff.kind)
                .collect::<Vec<_>>(),
            vec![
                ManagementHandoffKind::TaskPlan,
                ManagementHandoffKind::Assignment,
                ManagementHandoffKind::ExecutionEvidence,
                ManagementHandoffKind::RevisionRequest,
            ]
        );
        let execution_handoff = snapshot
            .handoffs
            .iter()
            .find(|handoff| handoff.kind == ManagementHandoffKind::ExecutionEvidence)
            .unwrap();
        assert_eq!(execution_handoff.run_attempt_id, Some(execution.id));
        assert!(execution_handoff.payload["runEvidence"].is_object());
        assert_eq!(
            execution_handoff.payload["runEvidence"]["workspaceEvidenceSha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            execution_handoff.payload["memoryBundleSha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        let revision = snapshot.handoffs.last().unwrap();
        assert_eq!(
            revision.source,
            ManagementHandoffSource::ReviewOrchestration
        );
        assert_eq!(revision.revision_round, 0);
        assert!(revision.review_flow_id.is_some());
        assert!(revision.review_stage_attempt_id.is_some());
    }

    #[test]
    fn task_0018_large_run_evidence_cannot_block_terminal_handoff_projection() {
        let (mut repository, task_id) = task_0011_repository();
        let intent = ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        approve_authorization(&mut repository, &intent);
        let admitted = repository
            .admit_run("task-0018-large-handoff", &intent)
            .unwrap();
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                Some("workspace-1"),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();

        let oversized_summary = "x".repeat(MAX_HANDOFF_SUMMARY_BYTES + 1_024);
        let completed = repository
            .complete_run(
                admitted.attempt.id,
                &successful_completion(&oversized_summary),
            )
            .unwrap();
        assert_eq!(completed.status, RunAttemptStatus::Succeeded);

        let snapshot = repository.management_handoff_snapshot().unwrap();
        let handoff = snapshot.handoffs.last().unwrap();
        assert_eq!(handoff.kind, ManagementHandoffKind::ExecutionEvidence);
        assert_eq!(handoff.summary.len(), MAX_HANDOFF_SUMMARY_BYTES);
        assert_eq!(handoff.payload["summaryTruncatedForHandoff"], true);
        assert!(handoff.payload.get("workspaceEvidence").is_none());
        assert_eq!(
            handoff.payload["runEvidence"]["fullEvidenceLocation"],
            "run_attempt"
        );
    }

    #[test]
    fn task_0011_changes_requeue_fresh_execution_and_stale_stage_cannot_replay() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-revision-execute-0");
        let (start, _) = task_0011_review_stage(
            &mut repository,
            task_id,
            "task-0011-revision-review-0",
            ReviewVerdict::ChangesRequested,
        );
        let stale_context = start.context.unwrap();
        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.flows[0].state, "revision_queued");
        assert_eq!(snapshot.flows[0].revision_round, 1);
        let task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_eq!(task.queue_state, "queued");
        assert_eq!(task.review_status, "Changes Requested");
        assert!(task.enqueue_sequence.unwrap() > 1);

        task_0011_execute(&mut repository, task_id, "task-0011-revision-execute-1");
        let fresh = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(fresh.flows[0].state, "awaiting_review");
        assert_eq!(fresh.flows[0].revision_round, 1);
        let stale_intent = ActionIntent::RunTask {
            agent_id: 3,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: Some(stale_context),
        };
        assert_eq!(
            repository
                .request_authorization(&stale_intent)
                .unwrap_err()
                .code,
            "REVIEW_INTENT_STALE"
        );
    }

    #[test]
    fn task_0011_invalid_text_never_approves_and_three_attempts_require_human() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-invalid-execute");
        for attempt_number in 1..=3 {
            let snapshot = repository.review_orchestration_snapshot().unwrap();
            let start = repository
                .start_review_stage(
                    StartReviewStageRequest {
                        expected_revision: snapshot.revision,
                        task_owner_agent_id: 2,
                        task_id,
                    },
                    &task_0010_provider_snapshot(),
                )
                .unwrap();
            let context = start.context.unwrap();
            let intent = ActionIntent::RunTask {
                agent_id: 3,
                task_owner_agent_id: 2,
                task_id,
                run_mode: RunMode::Review,
                review_context: Some(context),
            };
            let admitted = repository
                .admit_run(
                    &format!("task-0011-invalid-review-{attempt_number}"),
                    &intent,
                )
                .unwrap();
            repository
                .prepare_run_attempt(
                    admitted.attempt.id,
                    "Ollama",
                    "qwen2.5-coder:7b",
                    Some("workspace-1"),
                )
                .unwrap();
            repository
                .mark_run_dispatching(admitted.attempt.id)
                .unwrap();
            repository.mark_run_started(admitted.attempt.id).unwrap();
            repository
                .complete_run(
                    admitted.attempt.id,
                    &RunCompletion {
                        changed_files: Vec::new(),
                        diff: None,
                        specialist_result: None,
                        ..successful_completion("VERDICT: APPROVED")
                    },
                )
                .unwrap();
        }
        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.flows[0].state, "awaiting_human");
        assert_eq!(snapshot.flows[0].stages.len(), 3);
        assert!(snapshot.flows[0]
            .stages
            .iter()
            .all(|stage| stage.state == "invalid"));
        assert_ne!(
            repository.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Completed"
        );
    }

    #[test]
    fn task_0011_review_cancellation_retries_only_before_dispatch() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-cancel-execute");

        let snapshot = repository.review_orchestration_snapshot().unwrap();
        let first = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: snapshot.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let first_intent = ActionIntent::RunTask {
            agent_id: 3,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: first.context,
        };
        let admitted = repository
            .admit_run("task-0011-cancel-safe", &first_intent)
            .unwrap();
        repository
            .request_run_cancellation(admitted.attempt.id)
            .unwrap();
        let safe_cancel = RunCompletion::terminal_error(
            RunAttemptStatus::Cancelled,
            "RUN_CANCELLED",
            "Review cancelled before dispatch.",
            0,
        );
        repository
            .complete_run(admitted.attempt.id, &safe_cancel)
            .unwrap();
        let retryable = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(retryable.flows[0].state, "awaiting_review");
        assert_eq!(retryable.flows[0].stages[0].state, "cancelled");

        let second = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: retryable.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let second_intent = ActionIntent::RunTask {
            agent_id: 3,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: second.context,
        };
        let dispatched = repository
            .admit_run("task-0011-cancel-uncertain", &second_intent)
            .unwrap();
        repository
            .prepare_run_attempt(
                dispatched.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                Some("workspace-1"),
            )
            .unwrap();
        repository
            .mark_run_dispatching(dispatched.attempt.id)
            .unwrap();
        repository.mark_run_started(dispatched.attempt.id).unwrap();
        repository
            .request_run_cancellation(dispatched.attempt.id)
            .unwrap();
        repository
            .complete_run(
                dispatched.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Cancelled,
                    "RUN_CANCELLED",
                    "Review cancellation occurred after dispatch.",
                    1,
                ),
            )
            .unwrap();
        let uncertain = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(uncertain.flows[0].state, "awaiting_human");
        assert_eq!(uncertain.flows[0].stages[1].state, "cancelled");
        assert_ne!(
            repository.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Completed"
        );
    }

    #[test]
    fn task_0011_active_review_evidence_is_not_pruned_at_the_run_history_bound() {
        let (mut repository, task_id) = task_0011_repository();
        let execution = task_0011_execute(&mut repository, task_id, "task-0011-retention-execute");
        let execution_completed_at = execution.completed_at_unix_ms.unwrap();
        repository
            .connection
            .execute(
                "WITH RECURSIVE ids(id) AS (
                     VALUES(1)
                     UNION ALL SELECT id + 1 FROM ids WHERE id < ?1
                 )
                 INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                  workspace_fingerprint, agent_id, task_owner_agent_id, task_id,
                  task_title, run_mode, status, task_status_before, task_phase_before,
                  review_status_before, admitted_at_unix_ms, completed_at_unix_ms)
                 SELECT 'review-retention-history-' || id, '{}', 'history', 'history',
                        'history', 2, 2, ?2, 'History', 'execute', 'failed', 'Pending',
                        'Assigned', 'Not Requested', ?3 + id, ?3 + id
                 FROM ids",
                params![MAX_RETAINED_ATTEMPTS, task_id, execution_completed_at],
            )
            .unwrap();

        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        prune_run_history(&transaction, execution_completed_at + 10_000).unwrap();
        refresh_run_retention_meta(&transaction).unwrap();
        transaction.commit().unwrap();

        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(
            snapshot.flows[0].latest_execution_attempt_id,
            Some(execution.id)
        );
        assert!(repository
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM run_attempts WHERE id = ?1)",
                [execution.id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        let run_snapshot = repository.run_snapshot().unwrap();
        assert_eq!(
            run_snapshot.retained_attempt_count,
            MAX_RETAINED_ATTEMPTS as u64
        );
        assert_eq!(run_snapshot.pruned_attempt_count, 1);
    }

    #[test]
    fn task_0011_three_revision_cap_requires_human_and_cannot_queue_a_fourth() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-cap-execute-0");
        for round in 0..=MAX_REVISION_ROUNDS {
            task_0011_review_stage(
                &mut repository,
                task_id,
                &format!("task-0011-cap-review-{round}"),
                ReviewVerdict::ChangesRequested,
            );
            let snapshot = repository.review_orchestration_snapshot().unwrap();
            if round < MAX_REVISION_ROUNDS {
                assert_eq!(snapshot.flows[0].state, "revision_queued");
                assert_eq!(snapshot.flows[0].revision_round, round + 1);
                task_0011_execute(
                    &mut repository,
                    task_id,
                    &format!("task-0011-cap-execute-{}", round + 1),
                );
            } else {
                assert_eq!(snapshot.flows[0].state, "awaiting_human");
                assert_eq!(snapshot.flows[0].revision_round, MAX_REVISION_ROUNDS);
                let updated = repository
                    .record_human_review_decision(HumanReviewDecisionRequest {
                        expected_revision: snapshot.revision,
                        task_owner_agent_id: 2,
                        task_id,
                        flow_id: snapshot.flows[0].id,
                        verdict: ReviewVerdict::ChangesRequested,
                        feedback: "Do not accept this task without another revision.".to_string(),
                    })
                    .unwrap();
                assert_eq!(updated.flows[0].state, "failed");
                let task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
                assert_eq!(task.status, "Failed");
                assert_eq!(task.queue_state, "notQueued");
            }
        }
    }

    #[test]
    fn task_0011_unavailable_exact_reviewer_never_substitutes_and_survives_renderer_save() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-unavailable-execute");
        let envelope = repository.load().unwrap().unwrap();
        let mut renderer_state = envelope.state;
        renderer_state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 3)
            .unwrap()
            .status = "Paused".to_string();
        renderer_state.agents[1].tasks[0].status = "Completed".to_string();
        repository
            .save(envelope.revision, &renderer_state, false)
            .unwrap();

        let snapshot = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(snapshot.flows[0].state, "awaiting_review");
        let start = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: snapshot.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        assert_eq!(start.blocked_code.as_deref(), Some("REVIEWER_INACTIVE"));
        assert!(start.stage.is_none());
        assert!(start.context.is_none());
        assert_eq!(start.snapshot.flows[0].state, "awaiting_human");
        assert!(start.snapshot.flows[0].stages.is_empty());
        let task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_eq!(task.status, "Under Review");
        assert_eq!(task.phase, "Supervisor Approval");
    }

    #[test]
    fn task_0011_restart_recovery_never_dispatches_and_escalates_uncertain_review() {
        let (mut repository, task_id) = task_0011_repository();
        task_0011_execute(&mut repository, task_id, "task-0011-recovery-execute");

        let snapshot = repository.review_orchestration_snapshot().unwrap();
        let first = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: snapshot.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let first_intent = ActionIntent::RunTask {
            agent_id: 3,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: first.context,
        };
        repository
            .admit_run("task-0011-recovery-safe", &first_intent)
            .unwrap();
        repository.reconcile_interrupted_runs().unwrap();
        let safe = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(safe.flows[0].state, "awaiting_review");
        assert_eq!(safe.flows[0].stages[0].state, "interrupted");

        let second = repository
            .start_review_stage(
                StartReviewStageRequest {
                    expected_revision: safe.revision,
                    task_owner_agent_id: 2,
                    task_id,
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        let second_intent = ActionIntent::RunTask {
            agent_id: 3,
            task_owner_agent_id: 2,
            task_id,
            run_mode: RunMode::Review,
            review_context: second.context,
        };
        let admitted = repository
            .admit_run("task-0011-recovery-uncertain", &second_intent)
            .unwrap();
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                "Ollama",
                "qwen2.5-coder:7b",
                Some("workspace-1"),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        repository.mark_run_started(admitted.attempt.id).unwrap();
        repository.reconcile_interrupted_runs().unwrap();
        let uncertain = repository.review_orchestration_snapshot().unwrap();
        assert_eq!(uncertain.flows[0].state, "awaiting_human");
        assert_eq!(uncertain.flows[0].stages[1].state, "interrupted");
        assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
    }

    #[test]
    fn task_0005_admission_is_single_idempotent_and_completion_is_immutable() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let approval_id = approve_authorization(&mut repository, &intent);
        let admitted = repository.admit_run("request-1", &intent).unwrap();
        assert!(!admitted.duplicate);
        assert_eq!(admitted.attempt.status, RunAttemptStatus::Admitted);
        assert_eq!(admitted.attempt.approval_id, Some(approval_id));
        let consumed_before_start: Option<i64> = repository
            .connection
            .query_row(
                "SELECT consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [approval_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed_before_start.is_none());

        let duplicate = repository.admit_run("request-1", &intent).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.attempt.id, admitted.attempt.id);
        assert_eq!(
            repository.admit_run("request-2", &intent).unwrap_err().code,
            "RUN_BUSY"
        );

        let attempt_id = admitted.attempt.id;
        repository
            .prepare_run_attempt(
                attempt_id,
                "OpenAI",
                "codex-test",
                Some("workspace-authorization"),
            )
            .unwrap();
        repository.mark_run_dispatching(attempt_id).unwrap();
        repository.mark_run_started(attempt_id).unwrap();
        let consumed_after_start: Option<i64> = repository
            .connection
            .query_row(
                "SELECT consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [approval_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed_after_start.is_some());
        assert_eq!(
            repository.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Running"
        );

        let complete = repository
            .complete_run(attempt_id, &successful_completion("Completed safely."))
            .unwrap();
        assert_eq!(complete.status, RunAttemptStatus::Succeeded);
        assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
        let duplicate_completion = repository
            .complete_run(
                attempt_id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Failed,
                    "LATE_FAILURE",
                    "late result",
                    4,
                ),
            )
            .unwrap();
        assert_eq!(duplicate_completion.status, RunAttemptStatus::Succeeded);
        let envelope = repository.load().unwrap().unwrap();
        let mut forged_lifecycle = envelope.state;
        forged_lifecycle.agents[1].tasks[0].status = "Pending".to_string();
        forged_lifecycle.agents[1].tasks[0].phase = "Assigned".to_string();
        forged_lifecycle.agents[1].tasks[0].result = Some("forged result".to_string());
        repository
            .save(envelope.revision, &forged_lifecycle, false)
            .unwrap();
        let protected_task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_ne!(protected_task.status, "Pending");
        assert_eq!(protected_task.result.as_deref(), Some("Completed safely."));
        assert_eq!(
            repository
                .record_run_event(attempt_id, "progress", "late")
                .unwrap_err()
                .code,
            "RUN_EVENT_STALE"
        );
        assert!(repository
            .connection
            .execute(
                "UPDATE run_attempts SET status = 'failed' WHERE id = ?1",
                [attempt_id]
            )
            .is_err());
    }

    #[test]
    fn task_0005_concurrent_connections_admit_exactly_one_global_run() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let mut first = StateRepository::open(&path).unwrap();
        let initialized = first.initialize_fresh().unwrap();
        install_authorization_fixture(&mut first, initialized.revision);
        let intent = authorization_intent();
        approve_authorization(&mut first, &intent);
        let second = StateRepository::open(&path).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let first_barrier = barrier.clone();
        let first_intent = intent.clone();
        let first_thread = std::thread::spawn(move || {
            first_barrier.wait();
            first.admit_run("concurrent-first", &first_intent)
        });
        let second_barrier = barrier.clone();
        let second_intent = intent.clone();
        let second_thread = std::thread::spawn(move || {
            let mut second = second;
            second_barrier.wait();
            second.admit_run("concurrent-second", &second_intent)
        });
        let results = [first_thread.join().unwrap(), second_thread.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            vec!["RUN_BUSY"]
        );
    }

    #[test]
    fn task_0005_startup_failure_preserves_approval_and_cancellation_is_deterministic() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let approval_id = approve_authorization(&mut repository, &intent);
        let envelope = repository.load().unwrap().unwrap();
        let mut forged_pending_lifecycle = envelope.state;
        forged_pending_lifecycle.agents[1].tasks[0].status = "Running".to_string();
        forged_pending_lifecycle.agents[1].tasks[0].phase = "Specialist Work".to_string();
        forged_pending_lifecycle.agents[1].tasks[0].result = Some("forged result".to_string());
        repository
            .save(envelope.revision, &forged_pending_lifecycle, false)
            .unwrap();
        let protected_task = &repository.load().unwrap().unwrap().state.agents[1].tasks[0];
        assert_eq!(protected_task.status, "Pending");
        assert_eq!(protected_task.phase, "Assigned");
        assert!(protected_task.result.is_none());

        let attempt = repository.admit_run("startup-failure", &intent).unwrap();
        repository
            .prepare_run_attempt(attempt.attempt.id, "OpenAI", "codex-test", None)
            .unwrap();
        let failed = repository
            .complete_run(
                attempt.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::StartupFailed,
                    "START_FAILED",
                    "Provider startup failed before dispatch.",
                    0,
                ),
            )
            .unwrap();
        assert_eq!(failed.status, RunAttemptStatus::StartupFailed);
        let (status, consumed): (String, Option<i64>) = repository
            .connection
            .query_row(
                "SELECT status, consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [approval_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "Approved");
        assert!(consumed.is_none());

        let retry = repository
            .admit_run("cancel-before-spawn", &intent)
            .unwrap();
        let cancelling = repository
            .request_run_cancellation(retry.attempt.id)
            .unwrap();
        assert_eq!(cancelling.status, RunAttemptStatus::CancelRequested);
        assert_eq!(
            repository
                .request_run_cancellation(retry.attempt.id)
                .unwrap()
                .status,
            RunAttemptStatus::CancelRequested
        );
        let cancelled = repository
            .complete_run(
                retry.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Cancelled,
                    "RUN_CANCELLED",
                    "Cancelled before provider startup.",
                    0,
                ),
            )
            .unwrap();
        assert_eq!(cancelled.status, RunAttemptStatus::Cancelled);
        let consumed: Option<i64> = repository
            .connection
            .query_row(
                "SELECT consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [approval_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed.is_none());

        let interrupted = repository.admit_run("worker-stopped", &intent).unwrap();
        repository
            .prepare_run_attempt(interrupted.attempt.id, "OpenAI", "codex-test", None)
            .unwrap();
        let mut completion = RunCompletion::terminal_error(
            RunAttemptStatus::Interrupted,
            "RUN_WORKER_STOPPED",
            "The worker stopped before dispatch.",
            0,
        );
        completion.recovery_disposition = Some("safe_to_retry".to_string());
        let interrupted = repository
            .complete_run(interrupted.attempt.id, &completion)
            .unwrap();
        assert_eq!(
            interrupted.recovery_disposition.as_deref(),
            Some("safe_to_retry")
        );
        assert_eq!(
            repository.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Pending"
        );
    }

    #[test]
    fn task_0005_dispatch_cancellation_consumes_once_and_timeout_is_terminal() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let approval_id = approve_authorization(&mut repository, &intent);
        let attempt = repository
            .admit_run("cancel-during-spawn", &intent)
            .unwrap();
        repository
            .prepare_run_attempt(attempt.attempt.id, "OpenAI", "codex-test", None)
            .unwrap();
        repository.mark_run_dispatching(attempt.attempt.id).unwrap();
        repository
            .request_run_cancellation(attempt.attempt.id)
            .unwrap();
        repository.mark_run_started(attempt.attempt.id).unwrap();
        let cancelled = repository
            .complete_run(
                attempt.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::Cancelled,
                    "RUN_CANCELLED",
                    "Cancelled during provider startup.",
                    1,
                ),
            )
            .unwrap();
        assert_eq!(cancelled.status, RunAttemptStatus::Cancelled);
        let consumed: Option<i64> = repository
            .connection
            .query_row(
                "SELECT consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [approval_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed.is_some());

        let queue_revision = repository.load().unwrap().unwrap().revision;
        repository
            .set_task_queue_disposition(SetTaskQueueDispositionRequest {
                expected_revision: queue_revision,
                task_owner_agent_id: 2,
                task_id: 41,
                disposition: QueueDisposition::Resume,
            })
            .unwrap();
        let approval_id = approve_authorization(&mut repository, &intent);
        let timeout = repository.admit_run("timeout", &intent).unwrap();
        repository
            .prepare_run_attempt(timeout.attempt.id, "Ollama", "local-test", None)
            .unwrap();
        repository.mark_run_dispatching(timeout.attempt.id).unwrap();
        repository.mark_run_started(timeout.attempt.id).unwrap();
        let timed_out = repository
            .complete_run(
                timeout.attempt.id,
                &RunCompletion::terminal_error(
                    RunAttemptStatus::TimedOut,
                    "RUN_TIMED_OUT",
                    "The bounded run timeout elapsed.",
                    60,
                ),
            )
            .unwrap();
        assert_eq!(timed_out.status, RunAttemptStatus::TimedOut);
        assert_eq!(timed_out.approval_id, Some(approval_id));
    }

    #[test]
    fn task_0005_output_progress_and_history_bounds_are_visible() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        approve_authorization(&mut repository, &intent);
        let attempt = repository.admit_run("bounded-output", &intent).unwrap();
        repository
            .prepare_run_attempt(attempt.attempt.id, "OpenAI", "codex-test", None)
            .unwrap();
        repository.mark_run_dispatching(attempt.attempt.id).unwrap();
        repository.mark_run_started(attempt.attempt.id).unwrap();
        for index in 0..=MAX_PROGRESS_EVENTS {
            repository
                .record_run_event(attempt.attempt.id, "progress", &format!("event-{index}"))
                .unwrap();
        }
        let mut completion = successful_completion(&"s".repeat(MAX_SUMMARY_BYTES + 1));
        completion.changed_files = (0..=crate::run_coordinator::MAX_CHANGED_FILES)
            .map(|index| format!("src/{index}.rs"))
            .collect();
        completion.diff = Some("d".repeat(crate::run_coordinator::MAX_DIFF_CHARS + 1));
        let complete = repository
            .complete_run(attempt.attempt.id, &completion)
            .unwrap();
        assert!(complete.truncation.summary_truncated);
        assert!(complete.truncation.changed_files_truncated);
        assert!(complete.truncation.diff_truncated);
        assert!(complete.truncation.progress_truncated);
        assert_eq!(complete.progress_event_count, MAX_PROGRESS_EVENTS as u64);
        assert_eq!(
            complete.changed_files.len(),
            crate::run_coordinator::MAX_CHANGED_FILES
        );
    }

    #[test]
    fn task_0005_ledger_prunes_oldest_terminal_attempts_at_the_history_bound() {
        let mut repository = initialized_authorization_repository();
        repository
            .connection
            .execute(
                "WITH RECURSIVE ids(id) AS (
                     VALUES(1)
                     UNION ALL SELECT id + 1 FROM ids WHERE id <= ?1
                 )
                 INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                  workspace_fingerprint, agent_id, task_owner_agent_id, task_id,
                  task_title, run_mode, status, task_status_before, task_phase_before,
                  review_status_before, admitted_at_unix_ms, completed_at_unix_ms)
                 SELECT 'history-' || id, '{}', 'history', 'history', 'history',
                        2, 2, 41, 'History', 'execute', 'failed', 'Pending',
                        'Assigned', 'Not Requested', id, id
                 FROM ids",
                [MAX_RETAINED_ATTEMPTS],
            )
            .unwrap();
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        prune_run_history(&transaction, 10_000).unwrap();
        refresh_run_retention_meta(&transaction).unwrap();
        transaction.commit().unwrap();
        let snapshot = repository.run_snapshot().unwrap();
        assert_eq!(
            snapshot.retained_attempt_count,
            MAX_RETAINED_ATTEMPTS as u64
        );
        assert_eq!(snapshot.pruned_attempt_count, 1);
        assert_eq!(snapshot.last_pruned_at_unix_ms, Some(10_000));
        assert!(snapshot.recent_attempts.iter().all(|attempt| {
            attempt.status == RunAttemptStatus::Failed && attempt.request_id != "history-1"
        }));
    }

    #[test]
    fn task_0005_restart_reconciliation_distinguishes_safe_and_uncertain_dispatch() {
        let safe_directory = TestDirectory::new();
        let safe_path = safe_directory.database_path();
        let safe_approval;
        {
            let mut repository = StateRepository::open(&safe_path).unwrap();
            let initialized = repository.initialize_fresh().unwrap();
            install_authorization_fixture(&mut repository, initialized.revision);
            let intent = authorization_intent();
            safe_approval = approve_authorization(&mut repository, &intent);
            let attempt = repository.admit_run("safe-restart", &intent).unwrap();
            repository
                .prepare_run_attempt(attempt.attempt.id, "OpenAI", "codex-test", None)
                .unwrap();
        }
        let mut reopened = StateRepository::open(&safe_path).unwrap();
        let safe = reopened.run_snapshot().unwrap().recent_attempts.remove(0);
        assert_eq!(safe.status, RunAttemptStatus::Interrupted);
        assert_eq!(safe.recovery_disposition.as_deref(), Some("safe_to_retry"));
        let safe_consumed: Option<i64> = reopened
            .connection
            .query_row(
                "SELECT consumed_at_unix_ms FROM approval_requests WHERE id = ?1",
                [safe_approval],
                |row| row.get(0),
            )
            .unwrap();
        assert!(safe_consumed.is_none());

        let uncertain_directory = TestDirectory::new();
        let uncertain_path = uncertain_directory.database_path();
        let uncertain_approval;
        {
            let mut repository = StateRepository::open(&uncertain_path).unwrap();
            let initialized = repository.initialize_fresh().unwrap();
            install_authorization_fixture(&mut repository, initialized.revision);
            let intent = authorization_intent();
            uncertain_approval = approve_authorization(&mut repository, &intent);
            let attempt = repository.admit_run("uncertain-restart", &intent).unwrap();
            repository
                .prepare_run_attempt(attempt.attempt.id, "OpenAI", "codex-test", None)
                .unwrap();
            repository.mark_run_dispatching(attempt.attempt.id).unwrap();
        }
        let mut reopened = StateRepository::open(&uncertain_path).unwrap();
        let uncertain = reopened.run_snapshot().unwrap().recent_attempts.remove(0);
        assert_eq!(uncertain.status, RunAttemptStatus::Interrupted);
        assert_eq!(
            uncertain.recovery_disposition.as_deref(),
            Some("manual_review_required")
        );
        let status: String = reopened
            .connection
            .query_row(
                "SELECT status FROM approval_requests WHERE id = ?1",
                [uncertain_approval],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "Expired");
        assert_eq!(
            reopened.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Blocked"
        );

        reopened
            .connection
            .execute(
                "UPDATE agent_tasks
                 SET status = 'Running', phase = 'Specialist Work'
                 WHERE owner_agent_id = 2 AND id = 41",
                [],
            )
            .unwrap();
        drop(reopened);
        let mut reopened = StateRepository::open(&uncertain_path).unwrap();
        let snapshot = reopened.run_snapshot().unwrap();
        assert!(snapshot.recent_attempts.iter().any(|attempt| {
            attempt.request_id.starts_with("recovery:2:41:execute:")
                && attempt.recovery_disposition.as_deref() == Some("manual_review_required")
        }));
        assert_eq!(
            reopened.load().unwrap().unwrap().state.agents[1].tasks[0].status,
            "Blocked"
        );
    }

    #[test]
    fn schema_one_approvals_upgrade_as_non_authoritative_expired_history() {
        let directory = TestDirectory::new();
        let path = directory.database_path();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(INITIAL_MIGRATION).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at_unix_ms)
                 VALUES (1, 'initial_application_state', 1)",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO approval_requests
                 (id, position, agent_id, task_id, title, reason, status, created_at,
                  resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
                  consumed_at, origin, authoritative)
                 VALUES (1, 0, 2, 41, 'Old approval', 'Old renderer record',
                         'Approved', '2026-08-20T10:00:00.000Z', NULL, 'High', NULL,
                         'Old task', '2026-08-20T10:10:00.000Z', NULL,
                         'renderer_prototype', 0)",
                [],
            )
            .unwrap();
        drop(connection);

        let repository = StateRepository::open(&path).unwrap();
        assert_eq!(repository.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        let upgraded: (String, i64, String) = repository
            .connection
            .query_row(
                "SELECT status, authoritative, intent_fingerprint
                 FROM approval_requests WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(upgraded, ("Expired".to_string(), 0, String::new()));
    }

    #[test]
    fn approval_is_backend_issued_native_confirmed_exact_and_single_use() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let first = repository.request_authorization(&intent).unwrap();
        assert_eq!(
            first.decision,
            crate::authorization::AuthorizationDecision::ApprovalRequired
        );
        let pending = first.approval.unwrap();
        assert_eq!(pending.status, "Pending");
        let authority: (String, i64) = repository
            .connection
            .query_row(
                "SELECT origin, authoritative FROM approval_requests WHERE id = ?1",
                [pending.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(authority, ("backend_authority".to_string(), 1));

        let duplicate = repository.request_authorization(&intent).unwrap();
        assert_eq!(duplicate.approval.unwrap().id, pending.id);
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "APPROVAL_PENDING"
        );
        assert_eq!(
            repository
                .resolve_approval(pending.id, ApprovalResolution::Approve, false)
                .unwrap_err()
                .code,
            "NATIVE_CONFIRMATION_REQUIRED"
        );
        let approved = repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        assert_eq!(approved.status, "Approved");

        let grant = repository.authorize_intent(&intent).unwrap();
        assert!(grant.approval.unwrap().consumed_at.is_some());
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "APPROVAL_ALREADY_CONSUMED"
        );
    }

    #[test]
    fn renderer_save_cannot_mint_or_overwrite_authoritative_approval_state() {
        let mut repository = initialized_authorization_repository();
        let envelope = repository.load().unwrap().unwrap();
        let mut forged = envelope.state;
        forged.approval_requests.push(ApprovalRequest {
            id: 999,
            agent_id: 2,
            task_id: Some(41),
            title: "Forged".to_string(),
            reason: "Renderer asserted approval".to_string(),
            status: "Approved".to_string(),
            created_at: "2026-08-23T10:00:00.000Z".to_string(),
            resolved_at: Some("2026-08-23T10:00:01.000Z".to_string()),
            risk_level: "High".to_string(),
            scopes: vec!["files".to_string(), "terminal".to_string()],
            workspace_id: Some("workspace-authorization".to_string()),
            task_snapshot: "Forged task".to_string(),
            expires_at: "2099-08-23T10:30:00.000Z".to_string(),
            consumed_at: None,
        });
        repository.save(envelope.revision, &forged, false).unwrap();
        assert!(repository
            .load()
            .unwrap()
            .unwrap()
            .state
            .approval_requests
            .is_empty());
        assert_eq!(
            repository
                .authorize_intent(&authorization_intent())
                .unwrap_err()
                .code,
            "AUTHORIZATION_REQUIRED"
        );
    }

    #[test]
    fn task_and_policy_changes_invalidate_approved_authority() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let pending = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        let envelope = repository.load().unwrap().unwrap();
        let mut changed_task = envelope.state;
        changed_task.agents[1].tasks[0].title =
            "Run cargo test and edit a different parser".to_string();
        assert_eq!(
            repository
                .save(envelope.revision, &changed_task, false)
                .unwrap_err()
                .code,
            "RUN_TASK_LOCKED"
        );
        repository
            .connection
            .execute(
                "UPDATE approval_requests SET status = 'Expired' WHERE id = ?1",
                [pending.id],
            )
            .unwrap();
        repository
            .reroute_task(
                RerouteTaskRequest {
                    expected_revision: envelope.revision,
                    task_owner_agent_id: 2,
                    task_id: 41,
                    title: changed_task.agents[1].tasks[0].title.clone(),
                    category: "Development".to_string(),
                    priority: "Normal".to_string(),
                    workspace_id: "workspace-authorization".to_string(),
                    routing_mode: "selected".to_string(),
                    preferred_agent_id: Some(2),
                    selected_agent_id: Some(2),
                    specialist_request: Some(coding_request(
                        &changed_task.agents[1].tasks[0].title,
                    )),
                },
                &task_0010_provider_snapshot(),
            )
            .unwrap();
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "STALE_APPROVAL"
        );

        let mut repository = initialized_authorization_repository();
        let pending = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        let envelope = repository.load().unwrap().unwrap();
        let mut changed_policy = envelope.state;
        changed_policy.agents[1].capabilities.internet = "none".to_string();
        repository
            .save(envelope.revision, &changed_policy, false)
            .unwrap();
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "STALE_APPROVAL"
        );
    }

    #[test]
    fn authoritative_issuance_respects_the_bounded_history_limit() {
        let mut repository = initialized_authorization_repository();
        repository
            .connection
            .execute(
                "WITH RECURSIVE ids(id) AS (
                     VALUES(1)
                     UNION ALL SELECT id + 1 FROM ids WHERE id < ?1
                 )
                 INSERT INTO approval_requests
                 (id, position, agent_id, task_id, title, reason, status, created_at,
                  resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
                  consumed_at, origin, authoritative)
                 SELECT id, id - 1, 2, NULL, 'History', 'Non-authoritative history',
                        'Expired', '2026-08-23T00:00:00.000Z', NULL, 'Low', NULL, '',
                        '2026-08-23T00:00:00.000Z', NULL, 'renderer_prototype', 0
                 FROM ids",
                [MAX_AUTHORIZATION_RECORDS],
            )
            .unwrap();
        assert_eq!(
            repository
                .request_authorization(&authorization_intent())
                .unwrap_err()
                .code,
            "APPROVAL_HISTORY_LIMIT"
        );
    }

    #[test]
    fn privilege_increases_require_backend_recorded_native_confirmation() {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        let mut elevated = initialized.state;
        let pc_agent = elevated
            .agents
            .iter_mut()
            .find(|agent| agent.name == "PC Control Agent")
            .unwrap();
        pc_agent.status = "Working".to_string();
        pc_agent.capabilities.system = "full".to_string();
        pc_agent.approvals.system = "allow".to_string();
        elevated.preferences.workspaces.push(WorkspaceDefinition {
            id: "trusted-workspace".to_string(),
            name: "Trusted workspace".to_string(),
            path: "/tmp/trusted-workspace".to_string(),
        });
        elevated.preferences.active_workspace_id = Some("trusted-workspace".to_string());
        elevated.preferences.workspace_path = "/tmp/trusted-workspace".to_string();

        let summary = repository
            .security_change_summary(initialized.revision, &elevated)
            .unwrap()
            .unwrap();
        assert!(summary.contains("agent \"PC Control Agent\" (ID 7)"));
        assert!(summary.contains("system capability \"notifications\" -> \"full\""));
        assert!(summary.contains("system approval policy \"ask\" -> \"allow\""));
        assert!(
            summary.contains("workspace \"trusted-workspace\" root -> \"/tmp/trusted-workspace\"")
        );
        assert_eq!(
            repository
                .save(initialized.revision, &elevated, false)
                .unwrap_err()
                .code,
            "NATIVE_CONFIRMATION_REQUIRED"
        );
        assert_eq!(
            repository.load().unwrap().unwrap().revision,
            initialized.revision
        );
        let elevated_receipt = repository
            .save(initialized.revision, &elevated, true)
            .unwrap();

        let mut reduced = elevated;
        let pc_agent = reduced
            .agents
            .iter_mut()
            .find(|agent| agent.name == "PC Control Agent")
            .unwrap();
        pc_agent.status = "Paused".to_string();
        pc_agent.capabilities.system = "none".to_string();
        pc_agent.approvals.system = "deny".to_string();
        assert!(repository
            .security_change_summary(elevated_receipt.revision, &reduced)
            .unwrap()
            .is_none());
        repository
            .save(elevated_receipt.revision, &reduced, false)
            .unwrap();
    }

    #[test]
    fn stale_expired_malformed_and_wrong_subject_approvals_fail_closed() {
        let mut repository = initialized_authorization_repository();
        let intent = authorization_intent();
        let pending = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .connection
            .execute(
                "UPDATE approval_requests SET intent_json = '{' WHERE id = ?1",
                [pending.id],
            )
            .unwrap();
        assert_eq!(
            repository
                .resolve_approval(pending.id, ApprovalResolution::Approve, true)
                .unwrap_err()
                .code,
            "MALFORMED_APPROVAL"
        );

        repository
            .connection
            .execute("DELETE FROM approval_requests", [])
            .unwrap();
        let pending = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        repository
            .connection
            .execute(
                "UPDATE approval_requests
                 SET created_at_unix_ms = -2, expires_at_unix_ms = -1
                 WHERE id = ?1",
                [pending.id],
            )
            .unwrap();
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "APPROVAL_EXPIRED"
        );

        repository
            .connection
            .execute("DELETE FROM approval_requests", [])
            .unwrap();
        let pending = repository
            .request_authorization(&intent)
            .unwrap()
            .approval
            .unwrap();
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
        let envelope = repository.load().unwrap().unwrap();
        let mut moved_workspace = envelope.state;
        moved_workspace.preferences.workspaces[0].path = "/tmp/moved-workspace".to_string();
        moved_workspace.preferences.workspace_path = "/tmp/moved-workspace".to_string();
        repository
            .save(envelope.revision, &moved_workspace, true)
            .unwrap();
        assert_eq!(
            repository.authorize_intent(&intent).unwrap_err().code,
            "STALE_APPROVAL"
        );

        let wrong_task = ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id: 999,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        assert_eq!(
            repository.authorize_intent(&wrong_task).unwrap_err().code,
            "TASK_NOT_FOUND"
        );
        let wrong_agent = ActionIntent::RunTask {
            agent_id: 1,
            task_owner_agent_id: 2,
            task_id: 41,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        assert_eq!(
            repository.authorize_intent(&wrong_agent).unwrap_err().code,
            "WRONG_TASK_AGENT"
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_and_directory_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new();
        let path = directory.database_path();
        StateRepository::open(&path).unwrap();
        assert_eq!(
            fs::metadata(&directory.path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_path_symlinks_are_refused_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let target = directory.path.join("unrelated-file");
        let path = directory.database_path();
        fs::write(&target, b"unrelated content").unwrap();
        symlink(&target, &path).unwrap();

        assert_eq!(
            StateRepository::open(&path).unwrap_err().code,
            "DATABASE_FILE_UNSAFE"
        );
        assert_eq!(fs::read(target).unwrap(), b"unrelated content");
    }
}
