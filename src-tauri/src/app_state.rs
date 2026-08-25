use crate::agent_registry::{normalize_legacy_agents, validate_agent_registry};
use crate::task_orchestration::{RoutingEvidence, ROUTING_ALGORITHM_VERSION};
use crate::workspace_evidence::{
    WorkspaceChangeEvidenceV1, MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

pub const CURRENT_SCHEMA_VERSION: i64 = 7;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;

const MAX_AGENTS: usize = 1_000;
const MAX_MODELS: usize = 10_000;
const MAX_APPROVALS: usize = 10_000;
const MAX_REMINDERS: usize = 10_000;
const MAX_WORKSPACES: usize = 10_000;
const MAX_TASKS: usize = 50_000;
const MAX_ACTIVITY: usize = 50_000;
const MAX_SHORT_TEXT: usize = 4 * 1024;
const MAX_PATH_TEXT: usize = 32 * 1024;
const MAX_LARGE_TEXT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRetentionDays {
    Days7,
    Days30,
    Days90,
    Never,
}

impl Serialize for HistoryRetentionDays {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Days7 => serializer.serialize_u16(7),
            Self::Days30 => serializer.serialize_u16(30),
            Self::Days90 => serializer.serialize_u16(90),
            Self::Never => serializer.serialize_str("never"),
        }
    }
}

impl<'de> Deserialize<'de> for HistoryRetentionDays {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(number) if number.as_u64() == Some(7) => Ok(Self::Days7),
            serde_json::Value::Number(number) if number.as_u64() == Some(30) => Ok(Self::Days30),
            serde_json::Value::Number(number) if number.as_u64() == Some(90) => Ok(Self::Days90),
            serde_json::Value::String(value) if value == "never" => Ok(Self::Never),
            _ => Err(serde::de::Error::custom(
                "retention must be 7, 30, 90, or never",
            )),
        }
    }
}

impl HistoryRetentionDays {
    pub fn as_storage_value(self) -> &'static str {
        match self {
            Self::Days7 => "7",
            Self::Days30 => "30",
            Self::Days90 => "90",
            Self::Never => "never",
        }
    }

    pub fn from_storage_value(value: &str) -> Result<Self, StateValidationError> {
        match value {
            "7" => Ok(Self::Days7),
            "30" => Ok(Self::Days30),
            "90" => Ok(Self::Days90),
            "never" => Ok(Self::Never),
            _ => Err(StateValidationError::new(
                "retention",
                "retention must be 7, 30, 90, or never",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationState {
    pub agents: Vec<Agent>,
    pub models: Vec<ModelDefinition>,
    pub approval_requests: Vec<ApprovalRequest>,
    pub reminders: Vec<Reminder>,
    pub task_retention_days: HistoryRetentionDays,
    pub activity_retention_days: HistoryRetentionDays,
    pub preferences: AppPreferences,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyRendererState {
    pub agents: Option<String>,
    pub models: Option<String>,
    pub approval_requests: Option<String>,
    pub reminders: Option<String>,
    pub task_retention_days: Option<String>,
    pub activity_retention_days: Option<String>,
    pub preferences: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyBackupV2 {
    version: Option<i64>,
    exported_at: Option<String>,
    agents: Vec<Agent>,
    models: Vec<ModelDefinition>,
    approval_requests: Vec<ApprovalRequest>,
    reminders: Option<Vec<Reminder>>,
    task_retention_days: Option<HistoryRetentionDays>,
    activity_retention_days: Option<HistoryRetentionDays>,
    preferences: Option<AppPreferences>,
}

impl LegacyRendererState {
    pub fn is_empty(&self) -> bool {
        self.agents.is_none()
            && self.models.is_none()
            && self.approval_requests.is_none()
            && self.reminders.is_none()
            && self.task_retention_days.is_none()
            && self.activity_retention_days.is_none()
            && self.preferences.is_none()
    }

    fn encoded_size(&self) -> Option<usize> {
        [
            self.agents.as_ref(),
            self.models.as_ref(),
            self.approval_requests.as_ref(),
            self.reminders.as_ref(),
            self.task_retention_days.as_ref(),
            self.activity_retention_days.as_ref(),
            self.preferences.as_ref(),
        ]
        .into_iter()
        .flatten()
        .try_fold(0usize, |total, value| total.checked_add(value.len()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceDefinition {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPerformance {
    pub strength: i64,
    pub focus: String,
    pub cpu_limit: i64,
    pub gpu_limit: i64,
    #[serde(default = "default_queue_threshold")]
    pub queue_threshold: i64,
    pub overflow_action: String,
    pub redirect_agent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPreferences {
    pub theme: String,
    pub accent_color: String,
    pub density: String,
    pub reduced_motion: bool,
    pub default_model: String,
    pub active_ai_provider: String,
    pub default_agent_status: String,
    pub default_task_category: String,
    pub default_task_priority: String,
    pub default_performance: AgentPerformance,
    pub workspace_path: String,
    pub workspaces: Vec<WorkspaceDefinition>,
    pub active_workspace_id: Option<String>,
    pub agent_timeout_minutes: i64,
    pub safety_mode: String,
    pub approval_expiry_minutes: i64,
    pub default_routing_mode: String,
    pub review_mode: String,
    pub background_voice_enabled: bool,
    pub voice_control_master_enabled: bool,
    pub voice_wake_phrase: String,
    pub voice_deactivate_phrase: String,
    pub voice_open_phrases: String,
    pub voice_close_phrases: String,
    pub voice_command_replacements: String,
    pub voice_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalRequest {
    pub id: i64,
    pub agent_id: i64,
    pub task_id: Option<i64>,
    pub title: String,
    pub reason: String,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub risk_level: String,
    pub scopes: Vec<String>,
    pub workspace_id: Option<String>,
    pub task_snapshot: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTask {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub priority: String,
    pub assigned_agent_id: i64,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub result: Option<String>,
    pub response_id: Option<String>,
    pub runtime_model: Option<String>,
    pub total_tokens: Option<i64>,
    pub workspace_id: Option<String>,
    pub changed_files: Vec<String>,
    pub diff: Option<String>,
    #[serde(default)]
    pub workspace_changes: Option<WorkspaceChangeEvidenceV1>,
    pub duration_seconds: Option<f64>,
    pub routing_mode: String,
    pub routed_from_agent_id: Option<i64>,
    pub routing_reason: Option<String>,
    #[serde(default = "default_task_queue_state")]
    pub queue_state: String,
    #[serde(default)]
    pub enqueue_sequence: Option<i64>,
    #[serde(default)]
    pub routing_evidence: Option<RoutingEvidence>,
    pub review_agent_id: Option<i64>,
    pub review_status: String,
    pub review_result: Option<String>,
    pub review_model: Option<String>,
    pub review_duration_seconds: Option<f64>,
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDefinition {
    pub id: i64,
    pub name: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityEntry {
    pub id: i64,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub due_at: String,
    pub status: String,
    pub agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCapabilities {
    pub files: String,
    pub internet: String,
    pub clipboard: String,
    pub terminal: String,
    pub system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentApprovals {
    pub files: String,
    pub internet: String,
    pub clipboard: String,
    pub terminal: String,
    pub system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Agent {
    pub id: i64,
    #[serde(default)]
    pub template_key: Option<String>,
    #[serde(default = "default_registry_state")]
    pub registry_state: String,
    #[serde(default)]
    pub registry_issue: Option<String>,
    #[serde(default)]
    pub deleted_at_unix_ms: Option<i64>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub role: String,
    pub category: String,
    pub reports_to: Option<i64>,
    pub authority_level: i64,
    pub model: String,
    pub memory: String,
    pub tasks: Vec<AgentTask>,
    pub activity: Vec<ActivityEntry>,
    pub performance: AgentPerformance,
    pub capabilities: AgentCapabilities,
    pub approvals: AgentApprovals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateValidationError {
    pub path: String,
    pub message: String,
}

impl StateValidationError {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

fn default_registry_state() -> String {
    "active".to_string()
}

pub(crate) const fn default_queue_threshold() -> i64 {
    10
}

fn default_task_queue_state() -> String {
    "notQueued".to_string()
}

impl fmt::Display for StateValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for StateValidationError {}

pub fn default_application_state() -> Result<ApplicationState, StateValidationError> {
    let state: ApplicationState =
        serde_json::from_str(include_str!("../../src/application-state-seed.json"))
            .map_err(|error| StateValidationError::new("seed", error.to_string()))?;
    validate_application_state(&state)?;
    Ok(state)
}

pub fn application_state_from_legacy(
    legacy: &LegacyRendererState,
) -> Result<ApplicationState, StateValidationError> {
    if match legacy.encoded_size() {
        Some(size) => size > MAX_STATE_BYTES,
        None => true,
    } {
        return Err(StateValidationError::new(
            "legacy",
            format!("legacy renderer state exceeds {MAX_STATE_BYTES} bytes"),
        ));
    }

    let mut state = default_application_state()?;
    if let Some(value) = &legacy.preferences {
        state.preferences = parse_legacy_json("preferences", value)?;
    }
    if let Some(value) = &legacy.agents {
        state.agents = parse_legacy_json("agents", value)?;
        normalize_legacy_agents(&mut state.agents);
    }
    if let Some(value) = &legacy.models {
        state.models = parse_legacy_json("models", value)?;
        append_missing_ollama_model(&mut state)?;
    }
    if let Some(value) = &legacy.approval_requests {
        state.approval_requests = parse_legacy_json("approvalRequests", value)?;
    }
    if let Some(value) = &legacy.reminders {
        state.reminders = parse_legacy_json("reminders", value)?;
    }
    if let Some(value) = &legacy.task_retention_days {
        state.task_retention_days = HistoryRetentionDays::from_storage_value(value)?;
    }
    if let Some(value) = &legacy.activity_retention_days {
        state.activity_retention_days = HistoryRetentionDays::from_storage_value(value)?;
    }

    downgrade_legacy_approvals(&mut state);
    normalize_legacy_task_orchestration(&mut state);
    validate_application_state(&state)?;
    Ok(state)
}

pub fn application_state_from_legacy_backup(
    backup_json: &str,
    current: &ApplicationState,
) -> Result<ApplicationState, StateValidationError> {
    if backup_json.len() > MAX_STATE_BYTES {
        return Err(StateValidationError::new(
            "backup",
            format!("backup exceeds {MAX_STATE_BYTES} bytes"),
        ));
    }
    let backup: LegacyBackupV2 = serde_json::from_str(backup_json).map_err(|_| {
        StateValidationError::new("backup", "backup JSON is malformed or unsupported")
    })?;
    if backup.version.unwrap_or(2) != 2 {
        return Err(StateValidationError::new(
            "backup.version",
            "only legacy backup version 2 is supported",
        ));
    }
    if let Some(exported_at) = backup.exported_at.as_deref() {
        validate_text("backup.exportedAt", exported_at, MAX_SHORT_TEXT, false)?;
    }

    let mut state = ApplicationState {
        agents: backup.agents,
        models: backup.models,
        approval_requests: backup.approval_requests,
        reminders: backup.reminders.unwrap_or_default(),
        task_retention_days: backup
            .task_retention_days
            .unwrap_or(current.task_retention_days),
        activity_retention_days: backup
            .activity_retention_days
            .unwrap_or(current.activity_retention_days),
        preferences: backup
            .preferences
            .unwrap_or_else(|| current.preferences.clone()),
    };
    normalize_legacy_agents(&mut state.agents);
    append_missing_ollama_model(&mut state)?;
    downgrade_legacy_approvals(&mut state);
    normalize_legacy_task_orchestration(&mut state);
    validate_application_state(&state)?;
    Ok(state)
}

fn parse_legacy_json<T>(path: &str, value: &str) -> Result<T, StateValidationError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(value).map_err(|_| {
        StateValidationError::new(path, "legacy renderer JSON is malformed or unsupported")
    })
}

fn append_missing_ollama_model(state: &mut ApplicationState) -> Result<(), StateValidationError> {
    if state
        .models
        .iter()
        .any(|model| model.name.eq_ignore_ascii_case("qwen2.5-coder:7b"))
    {
        return Ok(());
    }
    let next_id = state
        .models
        .iter()
        .map(|model| model.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            StateValidationError::new("models", "no safe identifier remains for the default model")
        })?;
    state.models.push(ModelDefinition {
        id: next_id,
        name: "qwen2.5-coder:7b".to_string(),
        provider: "Ollama".to_string(),
    });
    Ok(())
}

fn normalize_legacy_task_orchestration(state: &mut ApplicationState) {
    let mut queued = state
        .agents
        .iter()
        .enumerate()
        .flat_map(|(agent_index, agent)| {
            agent
                .tasks
                .iter()
                .enumerate()
                .filter(|(_, task)| {
                    matches!(task.status.as_str(), "Pending" | "Blocked" | "Running")
                })
                .map(move |(task_index, task)| {
                    (
                        task.created_at.clone(),
                        agent.id,
                        task.id,
                        agent_index,
                        task_index,
                    )
                })
        })
        .collect::<Vec<_>>();
    queued.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    for agent in &mut state.agents {
        for task in &mut agent.tasks {
            task.queue_state = "notQueued".to_string();
            task.enqueue_sequence = None;
            task.routing_evidence = None;
        }
    }
    for (position, (_, _, _, agent_index, task_index)) in queued.into_iter().enumerate() {
        let task = &mut state.agents[agent_index].tasks[task_index];
        task.queue_state = match task.status.as_str() {
            "Pending" => "queued",
            "Blocked" => "held",
            "Running" => "running",
            _ => "notQueued",
        }
        .to_string();
        task.enqueue_sequence = Some(position as i64 + 1);
    }
}

fn downgrade_legacy_approvals(state: &mut ApplicationState) {
    let mut blocked_tasks = HashSet::new();
    for request in &mut state.approval_requests {
        if matches!(request.status.as_str(), "Pending" | "Approved") {
            request.status = "Expired".to_string();
            if let Some(task_id) = request.task_id {
                blocked_tasks.insert((request.agent_id, task_id));
            }
        }
    }
    for agent in &mut state.agents {
        for task in &mut agent.tasks {
            if blocked_tasks.contains(&(agent.id, task.id)) && task.status == "Blocked" {
                task.status = "Pending".to_string();
                task.phase = "Assigned".to_string();
            }
        }
    }
}

pub fn validate_application_state(state: &ApplicationState) -> Result<(), StateValidationError> {
    let encoded = serde_json::to_vec(state)
        .map_err(|error| StateValidationError::new("state", error.to_string()))?;
    if encoded.len() > MAX_STATE_BYTES {
        return Err(StateValidationError::new(
            "state",
            format!("serialized state exceeds {MAX_STATE_BYTES} bytes"),
        ));
    }

    validate_count("agents", state.agents.len(), MAX_AGENTS)?;
    validate_count("models", state.models.len(), MAX_MODELS)?;
    validate_count(
        "approvalRequests",
        state.approval_requests.len(),
        MAX_APPROVALS,
    )?;
    validate_count("reminders", state.reminders.len(), MAX_REMINDERS)?;
    validate_count(
        "preferences.workspaces",
        state.preferences.workspaces.len(),
        MAX_WORKSPACES,
    )?;

    let mut agent_ids = HashSet::new();
    let mut enqueue_sequences = HashSet::new();
    let mut total_tasks = 0;
    let mut total_activity = 0;
    for (agent_index, agent) in state.agents.iter().enumerate() {
        let path = format!("agents[{agent_index}]");
        validate_id(&format!("{path}.id"), agent.id)?;
        if !agent_ids.insert(agent.id) {
            return Err(StateValidationError::new(
                format!("{path}.id"),
                "agent id must be unique",
            ));
        }
        validate_text(&format!("{path}.name"), &agent.name, MAX_SHORT_TEXT, false)?;
        validate_text(
            &format!("{path}.description"),
            &agent.description,
            MAX_SHORT_TEXT,
            true,
        )?;
        validate_text(
            &format!("{path}.model"),
            &agent.model,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.memory"),
            &agent.memory,
            MAX_LARGE_TEXT,
            true,
        )?;
        validate_enum(
            &format!("{path}.status"),
            &agent.status,
            &["Working", "Waiting", "Paused"],
        )?;
        validate_enum(
            &format!("{path}.role"),
            &agent.role,
            &["Supervisor", "Team Leader", "Senior Agent", "Specialist"],
        )?;
        validate_enum(
            &format!("{path}.category"),
            &agent.category,
            &agent_categories(),
        )?;
        validate_range(
            &format!("{path}.authorityLevel"),
            agent.authority_level,
            1,
            4,
        )?;
        validate_optional_id(&format!("{path}.reportsTo"), agent.reports_to)?;
        validate_performance(&format!("{path}.performance"), &agent.performance)?;
        validate_capabilities(&path, &agent.capabilities)?;
        validate_approvals(&path, &agent.approvals)?;

        total_tasks += agent.tasks.len();
        total_activity += agent.activity.len();
        let mut task_ids = HashSet::new();
        for (task_index, task) in agent.tasks.iter().enumerate() {
            let task_path = format!("{path}.tasks[{task_index}]");
            validate_task(&task_path, task)?;
            if !task_ids.insert(task.id) {
                return Err(StateValidationError::new(
                    format!("{task_path}.id"),
                    "task id must be unique within its agent",
                ));
            }
            if let Some(sequence) = task.enqueue_sequence {
                if !enqueue_sequences.insert(sequence) {
                    return Err(StateValidationError::new(
                        format!("{task_path}.enqueueSequence"),
                        "enqueue sequence must be globally unique",
                    ));
                }
            }
        }

        let mut activity_ids = HashSet::new();
        for (activity_index, entry) in agent.activity.iter().enumerate() {
            let activity_path = format!("{path}.activity[{activity_index}]");
            validate_id(&format!("{activity_path}.id"), entry.id)?;
            if !activity_ids.insert(entry.id) {
                return Err(StateValidationError::new(
                    format!("{activity_path}.id"),
                    "activity id must be unique within its agent",
                ));
            }
            validate_text(
                &format!("{activity_path}.message"),
                &entry.message,
                MAX_LARGE_TEXT,
                false,
            )?;
            validate_text(
                &format!("{activity_path}.createdAt"),
                &entry.created_at,
                MAX_SHORT_TEXT,
                false,
            )?;
        }
    }
    validate_count("tasks", total_tasks, MAX_TASKS)?;
    validate_count("activity", total_activity, MAX_ACTIVITY)?;
    validate_agent_registry(&state.agents)?;

    validate_models(&state.models)?;
    validate_approval_requests(&state.approval_requests)?;
    validate_reminders(&state.reminders)?;
    validate_preferences(&state.preferences)?;
    Ok(())
}

fn validate_models(models: &[ModelDefinition]) -> Result<(), StateValidationError> {
    let mut ids = HashSet::new();
    for (index, model) in models.iter().enumerate() {
        let path = format!("models[{index}]");
        validate_id(&format!("{path}.id"), model.id)?;
        if !ids.insert(model.id) {
            return Err(StateValidationError::new(
                format!("{path}.id"),
                "model id must be unique",
            ));
        }
        validate_text(&format!("{path}.name"), &model.name, MAX_SHORT_TEXT, false)?;
        validate_enum(
            &format!("{path}.provider"),
            &model.provider,
            &["OpenAI", "Anthropic", "Google", "Ollama", "Custom"],
        )?;
    }
    Ok(())
}

fn validate_approval_requests(requests: &[ApprovalRequest]) -> Result<(), StateValidationError> {
    let mut ids = HashSet::new();
    for (index, request) in requests.iter().enumerate() {
        let path = format!("approvalRequests[{index}]");
        validate_id(&format!("{path}.id"), request.id)?;
        validate_id(&format!("{path}.agentId"), request.agent_id)?;
        validate_optional_id(&format!("{path}.taskId"), request.task_id)?;
        if !ids.insert(request.id) {
            return Err(StateValidationError::new(
                format!("{path}.id"),
                "approval request id must be unique",
            ));
        }
        validate_text(
            &format!("{path}.title"),
            &request.title,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.reason"),
            &request.reason,
            MAX_LARGE_TEXT,
            false,
        )?;
        validate_enum(
            &format!("{path}.status"),
            &request.status,
            &["Pending", "Approved", "Denied", "Expired"],
        )?;
        validate_enum(
            &format!("{path}.riskLevel"),
            &request.risk_level,
            &["Low", "Medium", "High", "Critical"],
        )?;
        validate_text(
            &format!("{path}.createdAt"),
            &request.created_at,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.expiresAt"),
            &request.expires_at,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_optional_text(
            &format!("{path}.resolvedAt"),
            request.resolved_at.as_deref(),
            MAX_SHORT_TEXT,
        )?;
        validate_optional_text(
            &format!("{path}.consumedAt"),
            request.consumed_at.as_deref(),
            MAX_SHORT_TEXT,
        )?;
        validate_optional_text(
            &format!("{path}.workspaceId"),
            request.workspace_id.as_deref(),
            MAX_SHORT_TEXT,
        )?;
        validate_text(
            &format!("{path}.taskSnapshot"),
            &request.task_snapshot,
            MAX_LARGE_TEXT,
            true,
        )?;
        if request.scopes.len() > 5 {
            return Err(StateValidationError::new(
                format!("{path}.scopes"),
                "approval request has too many scopes",
            ));
        }
        let mut scopes = HashSet::new();
        for (scope_index, scope) in request.scopes.iter().enumerate() {
            validate_enum(
                &format!("{path}.scopes[{scope_index}]"),
                scope,
                &["files", "internet", "clipboard", "terminal", "system"],
            )?;
            if !scopes.insert(scope.as_str()) {
                return Err(StateValidationError::new(
                    format!("{path}.scopes[{scope_index}]"),
                    "approval scope must be unique",
                ));
            }
        }
    }
    Ok(())
}

fn validate_reminders(reminders: &[Reminder]) -> Result<(), StateValidationError> {
    let mut ids = HashSet::new();
    for (index, reminder) in reminders.iter().enumerate() {
        let path = format!("reminders[{index}]");
        validate_id(&format!("{path}.id"), reminder.id)?;
        if !ids.insert(reminder.id) {
            return Err(StateValidationError::new(
                format!("{path}.id"),
                "reminder id must be unique",
            ));
        }
        validate_text(
            &format!("{path}.title"),
            &reminder.title,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.notes"),
            &reminder.notes,
            MAX_LARGE_TEXT,
            true,
        )?;
        validate_text(
            &format!("{path}.dueAt"),
            &reminder.due_at,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.createdAt"),
            &reminder.created_at,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_enum(
            &format!("{path}.status"),
            &reminder.status,
            &["Upcoming", "Completed", "Dismissed"],
        )?;
        validate_optional_id(&format!("{path}.agentId"), reminder.agent_id)?;
        validate_optional_id(&format!("{path}.taskId"), reminder.task_id)?;
    }
    Ok(())
}

fn validate_preferences(preferences: &AppPreferences) -> Result<(), StateValidationError> {
    validate_enum(
        "preferences.theme",
        &preferences.theme,
        &["dark", "light", "system"],
    )?;
    validate_enum(
        "preferences.accentColor",
        &preferences.accent_color,
        &["violet", "blue", "cyan", "green"],
    )?;
    validate_enum(
        "preferences.density",
        &preferences.density,
        &["comfortable", "compact"],
    )?;
    validate_text(
        "preferences.defaultModel",
        &preferences.default_model,
        MAX_SHORT_TEXT,
        false,
    )?;
    validate_enum(
        "preferences.activeAiProvider",
        &preferences.active_ai_provider,
        &["codex", "ollama"],
    )?;
    validate_enum(
        "preferences.defaultAgentStatus",
        &preferences.default_agent_status,
        &["Working", "Waiting", "Paused"],
    )?;
    validate_enum(
        "preferences.defaultTaskCategory",
        &preferences.default_task_category,
        &task_categories(),
    )?;
    validate_enum(
        "preferences.defaultTaskPriority",
        &preferences.default_task_priority,
        &["Low", "Normal", "High", "Critical"],
    )?;
    validate_performance(
        "preferences.defaultPerformance",
        &preferences.default_performance,
    )?;
    validate_text(
        "preferences.workspacePath",
        &preferences.workspace_path,
        MAX_PATH_TEXT,
        true,
    )?;
    validate_range(
        "preferences.agentTimeoutMinutes",
        preferences.agent_timeout_minutes,
        1,
        120,
    )?;
    validate_range(
        "preferences.approvalExpiryMinutes",
        preferences.approval_expiry_minutes,
        5,
        120,
    )?;
    validate_enum(
        "preferences.safetyMode",
        &preferences.safety_mode,
        &["balanced", "strict", "locked"],
    )?;
    validate_enum(
        "preferences.defaultRoutingMode",
        &preferences.default_routing_mode,
        &["selected", "automatic"],
    )?;
    validate_enum(
        "preferences.reviewMode",
        &preferences.review_mode,
        &["off", "manual", "automatic"],
    )?;
    validate_enum(
        "preferences.voiceState",
        &preferences.voice_state,
        &["VOICE_OFF", "VOICE_PASSIVE", "VOICE_ACTIVE"],
    )?;
    for (name, value, allow_empty) in [
        (
            "voiceWakePhrase",
            preferences.voice_wake_phrase.as_str(),
            false,
        ),
        (
            "voiceDeactivatePhrase",
            preferences.voice_deactivate_phrase.as_str(),
            false,
        ),
        (
            "voiceOpenPhrases",
            preferences.voice_open_phrases.as_str(),
            false,
        ),
        (
            "voiceClosePhrases",
            preferences.voice_close_phrases.as_str(),
            false,
        ),
        (
            "voiceCommandReplacements",
            preferences.voice_command_replacements.as_str(),
            true,
        ),
    ] {
        validate_text(
            &format!("preferences.{name}"),
            value,
            MAX_SHORT_TEXT,
            allow_empty,
        )?;
    }

    let mut workspace_ids = HashSet::new();
    for (index, workspace) in preferences.workspaces.iter().enumerate() {
        let path = format!("preferences.workspaces[{index}]");
        validate_text(&format!("{path}.id"), &workspace.id, MAX_SHORT_TEXT, false)?;
        validate_text(
            &format!("{path}.name"),
            &workspace.name,
            MAX_SHORT_TEXT,
            false,
        )?;
        validate_text(
            &format!("{path}.path"),
            &workspace.path,
            MAX_PATH_TEXT,
            false,
        )?;
        if !workspace_ids.insert(workspace.id.as_str()) {
            return Err(StateValidationError::new(
                format!("{path}.id"),
                "workspace id must be unique",
            ));
        }
    }
    if let Some(active_workspace_id) = &preferences.active_workspace_id {
        if !workspace_ids.contains(active_workspace_id.as_str()) {
            return Err(StateValidationError::new(
                "preferences.activeWorkspaceId",
                "active workspace must reference an existing workspace",
            ));
        }
    }
    Ok(())
}

fn validate_task(path: &str, task: &AgentTask) -> Result<(), StateValidationError> {
    validate_id(&format!("{path}.id"), task.id)?;
    validate_id(&format!("{path}.assignedAgentId"), task.assigned_agent_id)?;
    validate_text(&format!("{path}.title"), &task.title, MAX_SHORT_TEXT, false)?;
    validate_enum(
        &format!("{path}.category"),
        &task.category,
        &task_categories(),
    )?;
    validate_enum(
        &format!("{path}.priority"),
        &task.priority,
        &["Low", "Normal", "High", "Critical"],
    )?;
    validate_enum(
        &format!("{path}.status"),
        &task.status,
        &[
            "Pending",
            "Running",
            "Blocked",
            "Under Review",
            "Completed",
            "Failed",
        ],
    )?;
    validate_enum(
        &format!("{path}.phase"),
        &task.phase,
        &[
            "Assigned",
            "Specialist Work",
            "Senior Review",
            "Team Leader Review",
            "Supervisor Approval",
            "Finished",
            "Failed",
        ],
    )?;
    validate_enum(
        &format!("{path}.routingMode"),
        &task.routing_mode,
        &["selected", "automatic"],
    )?;
    validate_enum(
        &format!("{path}.queueState"),
        &task.queue_state,
        &["queued", "held", "admitted", "running", "notQueued"],
    )?;
    validate_optional_id(&format!("{path}.enqueueSequence"), task.enqueue_sequence)?;
    let requires_sequence = matches!(
        task.queue_state.as_str(),
        "queued" | "held" | "admitted" | "running"
    );
    if requires_sequence != task.enqueue_sequence.is_some() {
        return Err(StateValidationError::new(
            format!("{path}.enqueueSequence"),
            "queued lifecycle states require a sequence and notQueued forbids one",
        ));
    }
    let lifecycle_matches = match task.queue_state.as_str() {
        "queued" | "admitted" => task.status == "Pending",
        "held" => matches!(task.status.as_str(), "Pending" | "Blocked"),
        "running" => task.status == "Running",
        "notQueued" => !matches!(task.status.as_str(), "Pending" | "Running" | "Blocked"),
        _ => false,
    };
    if !lifecycle_matches {
        return Err(StateValidationError::new(
            format!("{path}.queueState"),
            "queue state does not match the task lifecycle status",
        ));
    }
    if let Some(evidence) = &task.routing_evidence {
        validate_routing_evidence(path, task, evidence)?;
    }
    validate_enum(
        &format!("{path}.reviewStatus"),
        &task.review_status,
        &[
            "Not Requested",
            "Pending",
            "Running",
            "Approved",
            "Changes Requested",
            "Failed",
        ],
    )?;
    validate_text(
        &format!("{path}.createdAt"),
        &task.created_at,
        MAX_SHORT_TEXT,
        false,
    )?;
    for (name, value, maximum) in [
        ("completedAt", task.completed_at.as_deref(), MAX_SHORT_TEXT),
        ("result", task.result.as_deref(), MAX_LARGE_TEXT),
        ("responseId", task.response_id.as_deref(), MAX_SHORT_TEXT),
        (
            "runtimeModel",
            task.runtime_model.as_deref(),
            MAX_SHORT_TEXT,
        ),
        ("workspaceId", task.workspace_id.as_deref(), MAX_SHORT_TEXT),
        ("diff", task.diff.as_deref(), MAX_LARGE_TEXT),
        (
            "routingReason",
            task.routing_reason.as_deref(),
            MAX_LARGE_TEXT,
        ),
        (
            "reviewResult",
            task.review_result.as_deref(),
            MAX_LARGE_TEXT,
        ),
        ("reviewModel", task.review_model.as_deref(), MAX_SHORT_TEXT),
        ("reviewedAt", task.reviewed_at.as_deref(), MAX_SHORT_TEXT),
    ] {
        validate_optional_text(&format!("{path}.{name}"), value, maximum)?;
    }
    validate_optional_id(
        &format!("{path}.routedFromAgentId"),
        task.routed_from_agent_id,
    )?;
    validate_optional_id(&format!("{path}.reviewAgentId"), task.review_agent_id)?;
    if task.total_tokens.is_some_and(|value| value < 0) {
        return Err(StateValidationError::new(
            format!("{path}.totalTokens"),
            "token count cannot be negative",
        ));
    }
    for (name, value) in [
        ("durationSeconds", task.duration_seconds),
        ("reviewDurationSeconds", task.review_duration_seconds),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(StateValidationError::new(
                format!("{path}.{name}"),
                "duration must be finite and non-negative",
            ));
        }
    }
    if task.changed_files.len() > 10_000 {
        return Err(StateValidationError::new(
            format!("{path}.changedFiles"),
            "task has too many changed files",
        ));
    }
    for (index, changed_file) in task.changed_files.iter().enumerate() {
        validate_text(
            &format!("{path}.changedFiles[{index}]"),
            changed_file,
            MAX_PATH_TEXT,
            false,
        )?;
    }
    if let Some(evidence) = &task.workspace_changes {
        evidence.validate().map_err(|message| {
            StateValidationError::new(format!("{path}.workspaceChanges"), message)
        })?;
        let bytes = serde_json::to_vec(evidence).map_err(|_| {
            StateValidationError::new(
                format!("{path}.workspaceChanges"),
                "workspace evidence could not be normalized",
            )
        })?;
        if bytes.len() > MAX_PERSISTED_WORKSPACE_EVIDENCE_BYTES {
            return Err(StateValidationError::new(
                format!("{path}.workspaceChanges"),
                "workspace evidence exceeds the persisted payload bound",
            ));
        }
    }
    Ok(())
}

fn validate_routing_evidence(
    path: &str,
    task: &AgentTask,
    evidence: &RoutingEvidence,
) -> Result<(), StateValidationError> {
    let evidence_path = format!("{path}.routingEvidence");
    if evidence.algorithm_version != ROUTING_ALGORITHM_VERSION {
        return Err(StateValidationError::new(
            format!("{evidence_path}.algorithmVersion"),
            "routing evidence uses an unsupported algorithm version",
        ));
    }
    if evidence.routing_mode != task.routing_mode {
        return Err(StateValidationError::new(
            format!("{evidence_path}.routingMode"),
            "routing evidence mode must match the task",
        ));
    }
    validate_optional_id(
        &format!("{evidence_path}.preferredAgentId"),
        evidence.preferred_agent_id,
    )?;
    validate_optional_id(
        &format!("{evidence_path}.selectedAgentId"),
        evidence.selected_agent_id,
    )?;
    validate_id(
        &format!("{evidence_path}.winningAgentId"),
        evidence.winning_agent_id,
    )?;
    if evidence.winning_agent_id != task.assigned_agent_id {
        return Err(StateValidationError::new(
            format!("{evidence_path}.winningAgentId"),
            "routing winner must match the assigned executor",
        ));
    }
    validate_text(
        &format!("{evidence_path}.outcomeCode"),
        &evidence.outcome_code,
        MAX_SHORT_TEXT,
        false,
    )?;
    validate_text(
        &format!("{evidence_path}.reason"),
        &evidence.reason,
        MAX_LARGE_TEXT,
        false,
    )?;
    validate_count(
        &format!("{evidence_path}.candidates"),
        evidence.candidates.len(),
        MAX_AGENTS,
    )?;
    if evidence.candidates.is_empty() {
        return Err(StateValidationError::new(
            format!("{evidence_path}.candidates"),
            "routing evidence must include evaluated candidates",
        ));
    }

    let mut candidate_ids = HashSet::new();
    let mut winner_is_eligible = false;
    for (candidate_index, candidate) in evidence.candidates.iter().enumerate() {
        let candidate_path = format!("{evidence_path}.candidates[{candidate_index}]");
        validate_id(&format!("{candidate_path}.agentId"), candidate.agent_id)?;
        if !candidate_ids.insert(candidate.agent_id) {
            return Err(StateValidationError::new(
                format!("{candidate_path}.agentId"),
                "routing candidate agent ids must be unique",
            ));
        }
        for (name, value) in [
            ("agentName", candidate.agent_name.as_str()),
            ("category", candidate.category.as_str()),
            ("role", candidate.role.as_str()),
            ("model", candidate.model.as_str()),
            ("overflowAction", candidate.overflow_action.as_str()),
        ] {
            validate_text(
                &format!("{candidate_path}.{name}"),
                value,
                MAX_SHORT_TEXT,
                false,
            )?;
        }
        validate_range(
            &format!("{candidate_path}.queueThreshold"),
            candidate.queue_threshold,
            1,
            100,
        )?;
        if candidate.workload < 0 {
            return Err(StateValidationError::new(
                format!("{candidate_path}.workload"),
                "routing workload cannot be negative",
            ));
        }
        validate_optional_id(
            &format!("{candidate_path}.redirectAgentId"),
            candidate.redirect_agent_id,
        )?;
        validate_optional_text(
            &format!("{candidate_path}.selectionExcludedCode"),
            candidate.selection_excluded_code.as_deref(),
            MAX_SHORT_TEXT,
        )?;
        if candidate.disqualifications.len() > 32 || candidate.score_components.len() > 32 {
            return Err(StateValidationError::new(
                candidate_path,
                "routing candidate evidence exceeds the bounded reason count",
            ));
        }
        for (reason_index, reason) in candidate.disqualifications.iter().enumerate() {
            validate_text(
                &format!("{candidate_path}.disqualifications[{reason_index}].code"),
                &reason.code,
                MAX_SHORT_TEXT,
                false,
            )?;
            validate_text(
                &format!("{candidate_path}.disqualifications[{reason_index}].message"),
                &reason.message,
                MAX_LARGE_TEXT,
                false,
            )?;
        }
        let mut computed_score = 0i64;
        for (component_index, component) in candidate.score_components.iter().enumerate() {
            validate_text(
                &format!("{candidate_path}.scoreComponents[{component_index}].code"),
                &component.code,
                MAX_SHORT_TEXT,
                false,
            )?;
            computed_score = computed_score
                .checked_add(component.points)
                .ok_or_else(|| {
                    StateValidationError::new(
                        format!("{candidate_path}.score"),
                        "routing score exceeds the supported range",
                    )
                })?;
        }
        if computed_score != candidate.score {
            return Err(StateValidationError::new(
                format!("{candidate_path}.score"),
                "routing score does not equal its recorded components",
            ));
        }
        if candidate.eligible != candidate.disqualifications.is_empty() {
            return Err(StateValidationError::new(
                format!("{candidate_path}.eligible"),
                "routing eligibility conflicts with disqualification evidence",
            ));
        }
        if candidate.agent_id == evidence.winning_agent_id && candidate.eligible {
            winner_is_eligible = true;
        }
    }
    if !winner_is_eligible {
        return Err(StateValidationError::new(
            format!("{evidence_path}.winningAgentId"),
            "routing winner must be present and hard-eligible",
        ));
    }
    Ok(())
}

fn validate_performance(
    path: &str,
    performance: &AgentPerformance,
) -> Result<(), StateValidationError> {
    validate_range(&format!("{path}.strength"), performance.strength, 1, 10)?;
    validate_range(&format!("{path}.cpuLimit"), performance.cpu_limit, 10, 100)?;
    validate_range(&format!("{path}.gpuLimit"), performance.gpu_limit, 0, 100)?;
    validate_range(
        &format!("{path}.queueThreshold"),
        performance.queue_threshold,
        1,
        100,
    )?;
    validate_enum(
        &format!("{path}.focus"),
        &performance.focus,
        &["speed", "balanced", "strength"],
    )?;
    validate_enum(
        &format!("{path}.overflowAction"),
        &performance.overflow_action,
        &["queue", "redirect"],
    )?;
    validate_optional_id(
        &format!("{path}.redirectAgentId"),
        performance.redirect_agent_id,
    )
}

fn validate_capabilities(
    path: &str,
    value: &AgentCapabilities,
) -> Result<(), StateValidationError> {
    for (name, capability) in [
        ("files", value.files.as_str()),
        ("internet", value.internet.as_str()),
        ("clipboard", value.clipboard.as_str()),
    ] {
        validate_enum(
            &format!("{path}.capabilities.{name}"),
            capability,
            &["none", "read", "write", "full"],
        )?;
    }
    validate_enum(
        &format!("{path}.capabilities.terminal"),
        &value.terminal,
        &["none", "safe", "user", "admin"],
    )?;
    validate_enum(
        &format!("{path}.capabilities.system"),
        &value.system,
        &["none", "notifications", "power", "full"],
    )
}

fn validate_approvals(path: &str, value: &AgentApprovals) -> Result<(), StateValidationError> {
    for (name, mode) in [
        ("files", value.files.as_str()),
        ("internet", value.internet.as_str()),
        ("clipboard", value.clipboard.as_str()),
        ("terminal", value.terminal.as_str()),
        ("system", value.system.as_str()),
    ] {
        validate_enum(
            &format!("{path}.approvals.{name}"),
            mode,
            &["allow", "ask", "deny"],
        )?;
    }
    Ok(())
}

fn validate_count(path: &str, actual: usize, maximum: usize) -> Result<(), StateValidationError> {
    if actual > maximum {
        return Err(StateValidationError::new(
            path,
            format!("record count {actual} exceeds limit {maximum}"),
        ));
    }
    Ok(())
}

fn validate_id(path: &str, value: i64) -> Result<(), StateValidationError> {
    if !(1..=MAX_SAFE_INTEGER).contains(&value) {
        return Err(StateValidationError::new(
            path,
            "id must be a positive JavaScript-safe integer",
        ));
    }
    Ok(())
}

fn validate_optional_id(path: &str, value: Option<i64>) -> Result<(), StateValidationError> {
    if let Some(value) = value {
        validate_id(path, value)?;
    }
    Ok(())
}

fn validate_range(
    path: &str,
    value: i64,
    minimum: i64,
    maximum: i64,
) -> Result<(), StateValidationError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(StateValidationError::new(
            path,
            format!("value must be between {minimum} and {maximum}"),
        ));
    }
    Ok(())
}

fn validate_text(
    path: &str,
    value: &str,
    maximum: usize,
    allow_empty: bool,
) -> Result<(), StateValidationError> {
    if !allow_empty && value.trim().is_empty() {
        return Err(StateValidationError::new(path, "value cannot be empty"));
    }
    if value.len() > maximum {
        return Err(StateValidationError::new(
            path,
            format!("text exceeds {maximum} bytes"),
        ));
    }
    Ok(())
}

fn validate_optional_text(
    path: &str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), StateValidationError> {
    if let Some(value) = value {
        validate_text(path, value, maximum, true)?;
    }
    Ok(())
}

fn validate_enum(path: &str, value: &str, allowed: &[&str]) -> Result<(), StateValidationError> {
    if !allowed.contains(&value) {
        return Err(StateValidationError::new(
            path,
            format!("unsupported value {value:?}"),
        ));
    }
    Ok(())
}

fn task_categories() -> [&'static str; 8] {
    [
        "Development",
        "Research",
        "Browsing",
        "Finance",
        "Business",
        "Communication",
        "System Control",
        "General",
    ]
}

fn agent_categories() -> [&'static str; 9] {
    [
        "Management",
        "Development",
        "Research",
        "Browsing",
        "Finance",
        "Business",
        "Communication",
        "System Control",
        "General",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_valid_and_deterministic() {
        let first = default_application_state().expect("seed should be valid");
        let second = default_application_state().expect("seed should be valid");
        assert_eq!(first, second);
        assert_eq!(first.agents.len(), 11);
        assert_eq!(first.models.len(), 6);
        assert!(first.approval_requests.is_empty());
        assert_eq!(first.task_retention_days, HistoryRetentionDays::Days30);
    }

    #[test]
    fn rejects_duplicate_and_unsafe_identifiers() {
        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].id = state.agents[0].id;
        assert!(validate_application_state(&state)
            .unwrap_err()
            .message
            .contains("unique"));

        let mut state = default_application_state().expect("seed should be valid");
        state.models[0].id = MAX_SAFE_INTEGER + 1;
        assert!(validate_application_state(&state)
            .unwrap_err()
            .message
            .contains("JavaScript-safe"));
    }

    #[test]
    fn rejects_unknown_enums_and_dangling_active_workspace() {
        let mut state = default_application_state().expect("seed should be valid");
        state.preferences.theme = "ultraviolet".to_string();
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "preferences.theme"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.preferences.active_workspace_id = Some("missing".to_string());
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "preferences.activeWorkspaceId"
        );
    }

    #[test]
    fn task_0009_rejects_invalid_reporting_relationships() {
        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].reports_to = Some(state.agents[1].id);
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[1].reportsTo"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].reports_to = Some(999_999);
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[1].reportsTo"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.agents[0].reports_to = Some(state.agents[1].id);
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[0].reportsTo"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.agents[2].reports_to = Some(state.agents[1].id);
        let error = validate_application_state(&state).unwrap_err();
        assert_eq!(error.path, "agents[1].reportsTo");
        assert!(error.message.contains("cycle"));
    }

    #[test]
    fn task_0009_rejects_role_authority_mismatch() {
        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].authority_level = 4;
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[1].authorityLevel"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].registry_state = "unassigned".to_string();
        state.agents[1].registry_issue = Some("unexpected".to_string());
        state.agents[1].status = "Paused".to_string();
        state.agents[1].reports_to = None;
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[1].registryIssue"
        );

        let mut state = default_application_state().expect("seed should be valid");
        state.agents[1].registry_state = "deleted".to_string();
        state.agents[1].deleted_at_unix_ms = Some(-1);
        state.agents[1].status = "Paused".to_string();
        state.agents[1].reports_to = None;
        assert_eq!(
            validate_application_state(&state).unwrap_err().path,
            "agents[1].deletedAtUnixMs"
        );
    }
}
