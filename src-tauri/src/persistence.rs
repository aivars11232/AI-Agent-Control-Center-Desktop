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
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_application_state.sql");
const AUTHORIZATION_MIGRATION: &str =
    include_str!("../migrations/0002_authoritative_approvals.sql");
const MAX_AUTHORIZATION_RECORDS: i64 = 10_000;

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
            format!("Stored application state is invalid at {}.", error.path),
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
        let current = read_application_state(&transaction)?;
        let state = application_state_from_legacy_backup(backup_json, &current)
            .map_err(PersistenceError::validation)?;
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
        validate_application_state(state).map_err(PersistenceError::validation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(PersistenceError::database)?;
        let meta = application_meta_from(&transaction)?;
        ensure_expected_revision(&meta, expected_revision)?;
        let current = read_application_state(&transaction)?;
        if let Some(summary) = protected_security_change_summary(&current, state) {
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
            state,
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
              voice_command_replacements, voice_state)
             VALUES
             (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
              ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27,
              ?28, ?29, ?30)",
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
                preferences.voice_state
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
             (id, position, name, description, status, role, category, reports_to,
              authority_level, model, memory, strength, focus, cpu_limit, gpu_limit,
              overflow_action, redirect_agent_id, capability_files, capability_internet,
              capability_clipboard, capability_terminal, capability_system, approval_files,
              approval_internet, approval_clipboard, approval_terminal, approval_system)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                     ?25, ?26, ?27)",
            params![
                agent.id,
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
                agent.approvals.system
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
    transaction
        .execute(
            "INSERT INTO agent_tasks
             (owner_agent_id, id, position, title, category, priority, assigned_agent_id,
              status, phase, created_at, completed_at, result, response_id, runtime_model,
              total_tokens, workspace_id, diff, duration_seconds, routing_mode,
              routed_from_agent_id, routing_reason, review_agent_id, review_status,
              review_result, review_model, review_duration_seconds, reviewed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                     ?25, ?26, ?27)",
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
                task.reviewed_at
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
                    voice_command_replacements, voice_state
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
            "SELECT id, name, description, status, role, category, reports_to,
                    authority_level, model, memory, strength, focus, cpu_limit, gpu_limit,
                    overflow_action, redirect_agent_id, capability_files, capability_internet,
                    capability_clipboard, capability_terminal, capability_system, approval_files,
                    approval_internet, approval_clipboard, approval_terminal, approval_system
             FROM agents ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok(Agent {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                status: row.get(3)?,
                role: row.get(4)?,
                category: row.get(5)?,
                reports_to: row.get(6)?,
                authority_level: row.get(7)?,
                model: row.get(8)?,
                memory: row.get(9)?,
                tasks: Vec::new(),
                activity: Vec::new(),
                performance: AgentPerformance {
                    strength: row.get(10)?,
                    focus: row.get(11)?,
                    cpu_limit: row.get(12)?,
                    gpu_limit: row.get(13)?,
                    overflow_action: row.get(14)?,
                    redirect_agent_id: row.get(15)?,
                },
                capabilities: AgentCapabilities {
                    files: row.get(16)?,
                    internet: row.get(17)?,
                    clipboard: row.get(18)?,
                    terminal: row.get(19)?,
                    system: row.get(20)?,
                },
                approvals: AgentApprovals {
                    files: row.get(21)?,
                    internet: row.get(22)?,
                    clipboard: row.get(23)?,
                    terminal: row.get(24)?,
                    system: row.get(25)?,
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
                    review_duration_seconds, reviewed_at
             FROM agent_tasks WHERE owner_agent_id = ?1 ORDER BY position",
        )
        .map_err(PersistenceError::database)?;
    let rows = statement
        .query_map([owner_agent_id], |row| {
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
                duration_seconds: row.get(15)?,
                routing_mode: row.get(16)?,
                routed_from_agent_id: row.get(17)?,
                routing_reason: row.get(18)?,
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
                    (2, "authoritative_approval_lifecycle".to_string())
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
            duration_seconds: None,
            routing_mode: "selected".to_string(),
            routed_from_agent_id: None,
            routing_reason: None,
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
        }
    }

    fn initialized_authorization_repository() -> StateRepository {
        let mut repository = StateRepository::open_in_memory().unwrap();
        let initialized = repository.initialize_fresh().unwrap();
        repository
            .save(initialized.revision, &authorization_state(), true)
            .unwrap();
        repository
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
        assert_eq!(repository.schema_version().unwrap(), 2);
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
        repository
            .save(envelope.revision, &changed_task, false)
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
        pc_agent.role = "Senior Agent".to_string();
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
        assert!(summary.contains("role \"Specialist\" -> \"Senior Agent\""));
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
