use crate::agent_registry::{
    authority_for_role, template_summaries, AgentRegistrySnapshot, CreateAgentRequest,
    DeleteAgentRequest, RestoreAgentTemplateRequest, UpdateAgentRequest,
};
use crate::app_state::{
    application_state_from_legacy, application_state_from_legacy_backup, default_application_state,
    validate_application_state, ActivityEntry, Agent, AgentApprovals, AgentCapabilities,
    AgentPerformance, AgentTask, AppPreferences, ApplicationState, ApprovalRequest,
    HistoryRetentionDays, LegacyRendererState, ModelDefinition, Reminder, StateValidationError,
    WorkspaceDefinition, CURRENT_SCHEMA_VERSION, MAX_SAFE_INTEGER,
};
use crate::authorization::{
    build_approval_confirmation, dialog_literal, format_unix_ms, ApprovalConfirmation,
    ApprovalResolution, AuthorizationGrant, AuthorizationOutcome,
};
use crate::policy::{evaluate_policy, ActionIntent, PolicyDisposition, PolicyEvaluation};
use crate::provider_runtime::ProviderRegistrySnapshot;
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
    path::Path,
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
const MAX_AUTHORIZATION_RECORDS: i64 = 10_000;

#[derive(Debug, Clone)]
pub struct RunAdmission {
    pub attempt: RunAttemptProjection,
    pub authorization: Option<AuthorizationGrant>,
    pub application_state: ApplicationState,
    pub review_request_json: Option<String>,
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

    pub async fn import_legacy_backup(
        &self,
        expected_revision: i64,
        backup_json: String,
    ) -> PersistenceResult<StateEnvelope> {
        self.run(move |repository| repository.import_legacy_backup(expected_revision, &backup_json))
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
        prepare_private_database_file(path)?;
        let connection = Connection::open(path).map_err(PersistenceError::database)?;
        let mut repository = Self { connection };
        repository.configure_connection_preflight()?;
        repository.verify_integrity()?;
        repository.verify_supported_schema_version()?;
        repository.configure_write_durability(false)?;
        repository.apply_migrations()?;
        repository.reconcile_interrupted_runs()?;
        Ok(repository)
    }

    #[cfg(test)]
    fn open_in_memory() -> PersistenceResult<Self> {
        let connection = Connection::open_in_memory().map_err(PersistenceError::database)?;
        let mut repository = Self { connection };
        repository.configure_connection_preflight()?;
        repository.verify_integrity()?;
        repository.verify_supported_schema_version()?;
        repository.configure_write_durability(true)?;
        repository.apply_migrations()?;
        repository.reconcile_interrupted_runs()?;
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
            for reminder in &mut state.reminders {
                if reminder.agent_id == Some(request.agent_id) {
                    reminder.agent_id = None;
                    reminder.task_id = None;
                }
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
                restored.memory = std::mem::take(&mut state.agents[index].memory);
                state.agents[index] = restored;
            } else {
                restored.id = allocate_agent_id(transaction)?;
                state.agents.push(restored);
            }
            Ok(())
        })
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
        transaction
            .execute(
                "UPDATE agent_tasks
                 SET title = ?1, category = ?2, priority = ?3, workspace_id = ?4,
                     routing_mode = ?5, assigned_agent_id = ?6,
                     routed_from_agent_id = ?7, routing_reason = ?8,
                     routing_evidence_json = ?9
                 WHERE owner_agent_id = ?10 AND id = ?11",
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
                    request.task_owner_agent_id,
                    request.task_id
                ],
            )
            .map_err(PersistenceError::database)?;
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
        let reviewer = match select_reviewer(
            &state,
            providers,
            flow.executor_agent_id,
            level,
            &prior_reviewer_ids,
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

    pub fn import_legacy_backup(
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
        let state = application_state_from_legacy_backup(backup_json, &current)
            .map_err(PersistenceError::validation)?;
        clear_review_orchestration(&transaction)?;
        let approval_origins = state
            .approval_requests
            .iter()
            .map(|request| (request.id, "legacy_backup".to_string()))
            .collect::<HashMap<_, _>>();
        write_application_state(
            &transaction,
            &state,
            "legacy_backup",
            &approval_origins,
            true,
        )?;
        let revision = next_revision(meta.state_revision)?;
        transaction
            .execute(
                "UPDATE application_meta
                 SET initialized = 1, state_revision = ?1, source_kind = 'legacy_backup',
                     source_version = 2, migrated_at_unix_ms = ?2,
                     legacy_cleanup_ack_at_unix_ms = NULL
                 WHERE singleton = 1",
                params![revision, timestamp],
            )
            .map_err(PersistenceError::database)?;
        transaction.commit().map_err(PersistenceError::database)?;
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
            return Ok(AuthorizationOutcome::allowed());
        }

        if let Some(approval_id) =
            find_matching_active_approval(&transaction, &evaluation, timestamp)?
        {
            let approval = read_approval_request(&transaction, approval_id)?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(AuthorizationOutcome::approval_required(approval));
        }

        let approval = insert_authoritative_approval(&transaction, intent, &evaluation, timestamp)?;
        transaction.commit().map_err(PersistenceError::database)?;
        Ok(AuthorizationOutcome::approval_required(approval))
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
            return Ok((AuthorizationGrant::policy_allowed(), state));
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
        Ok((AuthorizationGrant::consumed(approval), state))
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
                });
            let review_request_json = attempt
                .review_stage_attempt_id
                .map(|stage_id| read_review_request_json(&transaction, stage_id))
                .transpose()?;
            transaction.commit().map_err(PersistenceError::database)?;
            return Ok(RunAdmission {
                attempt,
                authorization,
                application_state: state,
                review_request_json,
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
        transaction
            .execute(
                "INSERT INTO run_attempts
                 (request_id, intent_json, intent_fingerprint, policy_fingerprint,
                 workspace_fingerprint, agent_id, task_owner_agent_id, task_id, task_title,
                  run_mode, review_flow_id, review_stage_attempt_id, review_revision_round,
                  status, workspace_id, approval_id, task_status_before,
                  task_phase_before, review_status_before, admitted_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                         ?13, 'admitted', ?14, ?15, ?16, ?17, ?18, ?19)",
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
                    timestamp
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
                     workspace_evidence_json = ?31
                 WHERE id = ?32 AND status = ?33",
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
            omitted_progress_event_count, workspace_evidence_json
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
    text_parts: [Option<&str>; 6],
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

fn clear_review_orchestration(transaction: &Transaction<'_>) -> PersistenceResult<()> {
    transaction
        .execute_batch(
            "DELETE FROM review_stage_attempts;
             DELETE FROM review_flows;
             UPDATE review_orchestration_meta SET revision = 0 WHERE singleton = 1;",
        )
        .map_err(PersistenceError::database)
}

fn protect_run_owned_state(
    transaction: &Transaction<'_>,
    current: &ApplicationState,
    requested: &ApplicationState,
    timestamp: i64,
) -> PersistenceResult<ApplicationState> {
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

fn clear_application_state(
    transaction: &Transaction<'_>,
    replace_approvals: bool,
) -> PersistenceResult<()> {
    if replace_approvals {
        transaction
            .execute("DELETE FROM approval_requests", [])
            .map_err(PersistenceError::database)?;
    }
    transaction
        .execute_batch(
            "DELETE FROM reminders;
             DELETE FROM models;
             DELETE FROM agents;
             DELETE FROM workspaces;
             DELETE FROM retention_settings;
             DELETE FROM preferences;",
        )
        .map_err(PersistenceError::database)
}

fn write_application_state(
    transaction: &Transaction<'_>,
    state: &ApplicationState,
    default_approval_origin: &str,
    approval_origins: &HashMap<i64, String>,
    replace_approvals: bool,
) -> PersistenceResult<()> {
    validate_application_state(state).map_err(PersistenceError::validation)?;
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
        write_agent(transaction, position, agent)?;
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
            write_approval_request(transaction, position, request, origin)?;
        }
    }
    for (position, reminder) in state.reminders.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO reminders
                 (id, position, title, notes, due_at, status, agent_id, task_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    reminder.id,
                    position as i64,
                    reminder.title,
                    reminder.notes,
                    reminder.due_at,
                    reminder.status,
                    reminder.agent_id,
                    reminder.task_id,
                    reminder.created_at
                ],
            )
            .map_err(PersistenceError::database)?;
    }
    synchronize_agent_id_allocator(transaction, state)?;
    synchronize_task_orchestration_allocators(transaction, state)?;
    Ok(())
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
                agent.memory,
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
        write_task(transaction, agent.id, task_position, task)?;
    }
    for (activity_position, entry) in agent.activity.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO agent_activity
                 (owner_agent_id, id, position, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    agent.id,
                    entry.id,
                    activity_position as i64,
                    entry.message,
                    entry.created_at
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
    transaction
        .execute(
            "INSERT INTO agent_tasks
             (owner_agent_id, id, position, title, category, priority, assigned_agent_id,
              status, phase, created_at, completed_at, result, response_id, runtime_model,
              total_tokens, workspace_id, diff, duration_seconds, routing_mode,
              routed_from_agent_id, routing_reason, review_agent_id, review_status,
              review_result, review_model, review_duration_seconds, reviewed_at,
              queue_state, enqueue_sequence, routing_evidence_json, workspace_evidence_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                     ?25, ?26, ?27, ?28, ?29, ?30, ?31)",
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
                workspace_evidence_json
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
) -> PersistenceResult<()> {
    transaction
        .execute(
            "INSERT INTO approval_requests
             (id, position, agent_id, task_id, title, reason, status, created_at,
              resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
              consumed_at, origin, authoritative)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, 0)",
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
                origin
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
                memory: row.get(13)?,
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
                    routing_evidence_json, workspace_evidence_json
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

fn read_reminders(connection: &Connection) -> PersistenceResult<Vec<Reminder>> {
    let mut statement = connection
        .prepare(
            "SELECT id, title, notes, due_at, status, agent_id, task_id, created_at
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
        }
    }

    fn task_by_title<'a>(state: &'a ApplicationState, title: &str) -> &'a AgentTask {
        state
            .agents
            .iter()
            .flat_map(|agent| &agent.tasks)
            .find(|task| task.title == title)
            .expect("task should exist")
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
                    (7, "structured_workspace_evidence".to_string())
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
        let transaction = connection.transaction().unwrap();
        write_application_state(
            &transaction,
            &state,
            "renderer_prototype",
            &HashMap::new(),
            true,
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
            .import_legacy_backup(initialized.revision, &backup)
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
                .import_legacy_backup(envelope.revision, "{\"version\":2}")
                .unwrap_err()
                .code,
            "STATE_VALIDATION_FAILED"
        );
        let after_invalid = repository.load().unwrap().unwrap();
        assert_eq!(after_invalid.revision, envelope.revision);
        assert_eq!(after_invalid.state, before_invalid);
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
        assert_eq!(
            repository.request_authorization(&intent).unwrap().decision,
            crate::authorization::AuthorizationDecision::Allowed
        );
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
        changed_policy.agents[1].capabilities.clipboard = "read".to_string();
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
