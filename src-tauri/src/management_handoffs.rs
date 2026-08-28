use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt};

pub const MAX_HANDOFF_SUMMARY_BYTES: usize = 32 * 1024;
pub const MAX_HANDOFF_PAYLOAD_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffValidationError {
    pub code: &'static str,
    pub message: String,
}

impl HandoffValidationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for HandoffValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HandoffValidationError {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagementHandoffKind {
    TaskPlan,
    Assignment,
    ExecutionEvidence,
    ReviewDecision,
    RevisionRequest,
    HumanOverride,
    Failure,
    Recovery,
}

impl ManagementHandoffKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::TaskPlan => "task_plan",
            Self::Assignment => "assignment",
            Self::ExecutionEvidence => "execution_evidence",
            Self::ReviewDecision => "review_decision",
            Self::RevisionRequest => "revision_request",
            Self::HumanOverride => "human_override",
            Self::Failure => "failure",
            Self::Recovery => "recovery",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, HandoffValidationError> {
        match value {
            "task_plan" => Ok(Self::TaskPlan),
            "assignment" => Ok(Self::Assignment),
            "execution_evidence" => Ok(Self::ExecutionEvidence),
            "review_decision" => Ok(Self::ReviewDecision),
            "revision_request" => Ok(Self::RevisionRequest),
            "human_override" => Ok(Self::HumanOverride),
            "failure" => Ok(Self::Failure),
            "recovery" => Ok(Self::Recovery),
            _ => Err(HandoffValidationError::new(
                "HANDOFF_STORAGE_INVALID",
                "The stored management handoff kind is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagementOwnerRole {
    Senior,
    TeamLeader,
    Supervisor,
    Human,
}

impl ManagementOwnerRole {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Senior => "senior",
            Self::TeamLeader => "team_leader",
            Self::Supervisor => "supervisor",
            Self::Human => "human",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, HandoffValidationError> {
        match value {
            "senior" => Ok(Self::Senior),
            "team_leader" => Ok(Self::TeamLeader),
            "supervisor" => Ok(Self::Supervisor),
            "human" => Ok(Self::Human),
            _ => Err(HandoffValidationError::new(
                "HANDOFF_STORAGE_INVALID",
                "The stored management owner role is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagementHandoffSource {
    TaskOrchestration,
    RunCoordinator,
    ReviewOrchestration,
    HumanDecision,
    MigrationV11,
}

impl ManagementHandoffSource {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::TaskOrchestration => "task_orchestration",
            Self::RunCoordinator => "run_coordinator",
            Self::ReviewOrchestration => "review_orchestration",
            Self::HumanDecision => "human_decision",
            Self::MigrationV11 => "migration_v11",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, HandoffValidationError> {
        match value {
            "task_orchestration" => Ok(Self::TaskOrchestration),
            "run_coordinator" => Ok(Self::RunCoordinator),
            "review_orchestration" => Ok(Self::ReviewOrchestration),
            "human_decision" => Ok(Self::HumanDecision),
            "migration_v11" => Ok(Self::MigrationV11),
            _ => Err(HandoffValidationError::new(
                "HANDOFF_STORAGE_INVALID",
                "The stored management handoff source is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementHandoffV1 {
    pub id: i64,
    pub task_owner_agent_id: i64,
    pub task_id: i64,
    pub kind: ManagementHandoffKind,
    pub from_agent_id: Option<i64>,
    pub to_agent_id: Option<i64>,
    pub owner_role: ManagementOwnerRole,
    pub revision_round: i64,
    pub run_attempt_id: Option<i64>,
    pub review_flow_id: Option<i64>,
    pub review_stage_attempt_id: Option<i64>,
    pub source: ManagementHandoffSource,
    pub summary: String,
    pub payload: Value,
    pub idempotency_key: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementHandoffSnapshot {
    pub revision: i64,
    pub application_state_revision: i64,
    pub handoffs: Vec<ManagementHandoffV1>,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagementVisibilityContext {
    pub viewer_agent_id: Option<i64>,
    pub viewer_role: ManagementOwnerRole,
    pub managed_agent_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewManagementHandoff {
    pub task_owner_agent_id: i64,
    pub task_id: i64,
    pub kind: ManagementHandoffKind,
    pub from_agent_id: Option<i64>,
    pub to_agent_id: Option<i64>,
    pub owner_role: ManagementOwnerRole,
    pub revision_round: i64,
    pub run_attempt_id: Option<i64>,
    pub review_flow_id: Option<i64>,
    pub review_stage_attempt_id: Option<i64>,
    pub source: ManagementHandoffSource,
    pub summary: String,
    pub payload: Value,
    pub idempotency_key: String,
}

impl NewManagementHandoff {
    pub(crate) fn validate(&self) -> Result<(), HandoffValidationError> {
        validate_fields(
            self.task_owner_agent_id,
            self.task_id,
            self.from_agent_id,
            self.to_agent_id,
            self.revision_round,
            self.run_attempt_id,
            self.review_flow_id,
            self.review_stage_attempt_id,
            &self.summary,
            &self.payload,
            &self.idempotency_key,
        )
    }
}

pub fn validate_handoff(record: &ManagementHandoffV1) -> Result<(), HandoffValidationError> {
    if record.id <= 0 || record.created_at_unix_ms < 0 {
        return Err(HandoffValidationError::new(
            "HANDOFF_STORAGE_INVALID",
            "The stored handoff identifier or timestamp is invalid.",
        ));
    }
    validate_fields(
        record.task_owner_agent_id,
        record.task_id,
        record.from_agent_id,
        record.to_agent_id,
        record.revision_round,
        record.run_attempt_id,
        record.review_flow_id,
        record.review_stage_attempt_id,
        &record.summary,
        &record.payload,
        &record.idempotency_key,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_fields(
    task_owner_agent_id: i64,
    task_id: i64,
    from_agent_id: Option<i64>,
    to_agent_id: Option<i64>,
    revision_round: i64,
    run_attempt_id: Option<i64>,
    review_flow_id: Option<i64>,
    review_stage_attempt_id: Option<i64>,
    summary: &str,
    payload: &Value,
    idempotency_key: &str,
) -> Result<(), HandoffValidationError> {
    if task_owner_agent_id <= 0
        || task_id <= 0
        || from_agent_id.is_some_and(|value| value <= 0)
        || to_agent_id.is_some_and(|value| value <= 0)
        || !(0..=10_000).contains(&revision_round)
        || run_attempt_id.is_some_and(|value| value <= 0)
        || review_flow_id.is_some_and(|value| value <= 0)
        || review_stage_attempt_id.is_some_and(|value| value <= 0)
        || summary.trim().is_empty()
        || summary.len() > MAX_HANDOFF_SUMMARY_BYTES
        || idempotency_key.trim().is_empty()
        || idempotency_key.len() > 512
    {
        return Err(HandoffValidationError::new(
            "HANDOFF_INVALID",
            "The management handoff contains invalid or unbounded metadata.",
        ));
    }
    let payload_json = serde_json::to_string(payload).map_err(|_| {
        HandoffValidationError::new(
            "HANDOFF_INVALID",
            "The management handoff payload cannot be serialized.",
        )
    })?;
    if payload_json.len() < 2 || payload_json.len() > MAX_HANDOFF_PAYLOAD_BYTES {
        return Err(HandoffValidationError::new(
            "HANDOFF_INVALID",
            "The management handoff payload is outside the 128 KiB limit.",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub fn handoff_visible_to(
    handoff: &ManagementHandoffV1,
    context: &ManagementVisibilityContext,
) -> bool {
    if context.viewer_role == ManagementOwnerRole::Human {
        return true;
    }
    let Some(viewer_id) = context.viewer_agent_id else {
        return false;
    };
    handoff.task_owner_agent_id == viewer_id
        || handoff.from_agent_id == Some(viewer_id)
        || handoff.to_agent_id == Some(viewer_id)
        || context
            .managed_agent_ids
            .contains(&handoff.task_owner_agent_id)
        || handoff
            .from_agent_id
            .is_some_and(|agent| context.managed_agent_ids.contains(&agent))
        || handoff
            .to_agent_id
            .is_some_and(|agent| context.managed_agent_ids.contains(&agent))
}

pub fn validate_sequential_handoffs(
    handoffs: &[ManagementHandoffV1],
) -> Result<(), HandoffValidationError> {
    let mut task_state: HashMap<(i64, i64), Vec<ManagementHandoffKind>> = HashMap::new();
    let mut ordered: Vec<&ManagementHandoffV1> = handoffs.iter().collect();
    ordered.sort_by_key(|handoff| (handoff.created_at_unix_ms, handoff.id));
    for handoff in ordered {
        validate_handoff(handoff)?;
        let history = task_state
            .entry((handoff.task_owner_agent_id, handoff.task_id))
            .or_default();
        let permitted = match handoff.kind {
            ManagementHandoffKind::TaskPlan => history.is_empty(),
            ManagementHandoffKind::Assignment => history.contains(&ManagementHandoffKind::TaskPlan),
            ManagementHandoffKind::ExecutionEvidence | ManagementHandoffKind::Failure => {
                history.contains(&ManagementHandoffKind::Assignment)
            }
            ManagementHandoffKind::ReviewDecision | ManagementHandoffKind::RevisionRequest => {
                history.iter().any(|kind| {
                    matches!(
                        kind,
                        ManagementHandoffKind::ExecutionEvidence | ManagementHandoffKind::Failure
                    )
                })
            }
            ManagementHandoffKind::HumanOverride => !history.is_empty(),
            ManagementHandoffKind::Recovery => history.iter().any(|kind| {
                matches!(
                    kind,
                    ManagementHandoffKind::Failure
                        | ManagementHandoffKind::RevisionRequest
                        | ManagementHandoffKind::HumanOverride
                )
            }),
        };
        if !permitted {
            return Err(HandoffValidationError::new(
                "HANDOFF_SEQUENCE_INVALID",
                format!(
                    "Handoff {} for task {}/{} does not have its required predecessor.",
                    handoff.id, handoff.task_owner_agent_id, handoff.task_id
                ),
            ));
        }
        history.push(handoff.kind);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn handoff(id: i64, kind: ManagementHandoffKind) -> ManagementHandoffV1 {
        ManagementHandoffV1 {
            id,
            task_owner_agent_id: 10,
            task_id: 5,
            kind,
            from_agent_id: Some(10),
            to_agent_id: Some(11),
            owner_role: ManagementOwnerRole::TeamLeader,
            revision_round: 0,
            run_attempt_id: None,
            review_flow_id: None,
            review_stage_attempt_id: None,
            source: ManagementHandoffSource::TaskOrchestration,
            summary: "Bounded handoff".to_string(),
            payload: json!({"evidence": "retained"}),
            idempotency_key: format!("test-{id}"),
            created_at_unix_ms: id,
        }
    }

    #[test]
    fn task_0018_handoff_sequence_requires_explicit_predecessors() {
        let valid = vec![
            handoff(1, ManagementHandoffKind::TaskPlan),
            handoff(2, ManagementHandoffKind::Assignment),
            handoff(3, ManagementHandoffKind::ExecutionEvidence),
            handoff(4, ManagementHandoffKind::ReviewDecision),
        ];
        assert!(validate_sequential_handoffs(&valid).is_ok());
        assert_eq!(
            validate_sequential_handoffs(&[handoff(1, ManagementHandoffKind::ReviewDecision)])
                .unwrap_err()
                .code,
            "HANDOFF_SEQUENCE_INVALID"
        );
    }

    #[test]
    fn task_0018_handoff_visibility_is_bounded_to_management_chain() {
        let record = handoff(1, ManagementHandoffKind::TaskPlan);
        assert!(handoff_visible_to(
            &record,
            &ManagementVisibilityContext {
                viewer_agent_id: Some(20),
                viewer_role: ManagementOwnerRole::Supervisor,
                managed_agent_ids: vec![10],
            }
        ));
        assert!(!handoff_visible_to(
            &record,
            &ManagementVisibilityContext {
                viewer_agent_id: Some(30),
                viewer_role: ManagementOwnerRole::Senior,
                managed_agent_ids: vec![],
            }
        ));
        assert!(handoff_visible_to(
            &record,
            &ManagementVisibilityContext {
                viewer_agent_id: None,
                viewer_role: ManagementOwnerRole::Human,
                managed_agent_ids: vec![],
            }
        ));
    }

    #[test]
    fn task_0018_handoff_payload_is_bounded() {
        let mut record = handoff(1, ManagementHandoffKind::TaskPlan);
        record.payload = json!({"value": "x".repeat(MAX_HANDOFF_PAYLOAD_BYTES)});
        assert_eq!(
            validate_handoff(&record).unwrap_err().code,
            "HANDOFF_INVALID"
        );
    }
}
