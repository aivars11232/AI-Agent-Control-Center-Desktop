use crate::{
    app_state::{Agent, AgentTask, ApplicationState},
    policy::{evaluate_policy, ActionIntent, RunMode},
    provider_runtime::{
        resolve_model_identity, ProviderAvailability, ProviderRegistrySnapshot, RuntimeProviderId,
    },
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fmt};

pub(crate) const ROUTING_ALGORITHM_VERSION: &str = "task-routing-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateRoutedTaskRequest {
    pub(crate) expected_revision: i64,
    pub(crate) task_owner_agent_id: i64,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) priority: String,
    pub(crate) workspace_id: String,
    pub(crate) routing_mode: String,
    pub(crate) preferred_agent_id: Option<i64>,
    pub(crate) selected_agent_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RerouteTaskRequest {
    pub(crate) expected_revision: i64,
    pub(crate) task_owner_agent_id: i64,
    pub(crate) task_id: i64,
    pub(crate) title: String,
    pub(crate) category: String,
    pub(crate) priority: String,
    pub(crate) workspace_id: String,
    pub(crate) routing_mode: String,
    pub(crate) preferred_agent_id: Option<i64>,
    pub(crate) selected_agent_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QueueDisposition {
    Hold,
    Resume,
    ResetTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetTaskQueueDispositionRequest {
    pub(crate) expected_revision: i64,
    pub(crate) task_owner_agent_id: i64,
    pub(crate) task_id: i64,
    pub(crate) disposition: QueueDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskQueueEntry {
    pub(crate) task_owner_agent_id: i64,
    pub(crate) task_id: i64,
    pub(crate) assigned_agent_id: i64,
    pub(crate) title: String,
    pub(crate) priority: String,
    pub(crate) queue_state: String,
    pub(crate) enqueue_sequence: i64,
    pub(crate) queue_position: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskOrchestrationSnapshot {
    pub(crate) revision: i64,
    pub(crate) execute_queue: Vec<TaskQueueEntry>,
    pub(crate) held_tasks: Vec<TaskQueueEntry>,
    pub(crate) active_execute: Option<TaskQueueEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoutingTaskInput {
    pub(crate) task_owner_agent_id: i64,
    pub(crate) task: AgentTask,
    pub(crate) preferred_agent_id: Option<i64>,
    pub(crate) selected_agent_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RouteScoreComponent {
    pub(crate) code: String,
    pub(crate) points: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RouteDisqualification {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RouteCandidateEvidence {
    pub(crate) agent_id: i64,
    pub(crate) agent_name: String,
    pub(crate) category: String,
    pub(crate) role: String,
    pub(crate) model: String,
    pub(crate) eligible: bool,
    pub(crate) disqualifications: Vec<RouteDisqualification>,
    pub(crate) score: i64,
    pub(crate) score_components: Vec<RouteScoreComponent>,
    pub(crate) workload: i64,
    pub(crate) queue_threshold: i64,
    pub(crate) overloaded: bool,
    pub(crate) overflow_action: String,
    pub(crate) redirect_agent_id: Option<i64>,
    pub(crate) selection_excluded_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoutingEvidence {
    pub(crate) algorithm_version: String,
    pub(crate) routing_mode: String,
    pub(crate) preferred_agent_id: Option<i64>,
    pub(crate) selected_agent_id: Option<i64>,
    pub(crate) winning_agent_id: i64,
    pub(crate) outcome_code: String,
    pub(crate) reason: String,
    pub(crate) manual_override: bool,
    pub(crate) candidates: Vec<RouteCandidateEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoutingError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) candidates: Vec<RouteCandidateEvidence>,
}

impl RoutingError {
    fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        candidates: Vec<RouteCandidateEvidence>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            candidates,
        }
    }
}

impl fmt::Display for RoutingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RoutingError {}

fn add_disqualification(
    disqualifications: &mut Vec<RouteDisqualification>,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    let code = code.into();
    if disqualifications
        .iter()
        .any(|disqualification| disqualification.code == code)
    {
        return;
    }
    disqualifications.push(RouteDisqualification {
        code,
        message: message.into(),
    });
}

fn workload_for_agent(state: &ApplicationState, input: &RoutingTaskInput, agent_id: i64) -> i64 {
    state
        .agents
        .iter()
        .flat_map(|owner| owner.tasks.iter().map(move |task| (owner.id, task)))
        .filter(|(owner_id, task)| {
            !(*owner_id == input.task_owner_agent_id && task.id == input.task.id)
                && task.assigned_agent_id == agent_id
                && matches!(task.status.as_str(), "Pending" | "Running")
        })
        .count() as i64
}

fn score_candidate(
    agent: &Agent,
    input: &RoutingTaskInput,
    workload: i64,
) -> (i64, Vec<RouteScoreComponent>) {
    let category_match = agent.category == input.task.category
        || (input.task.category == "General" && agent.category == "Management");
    let status_points = match agent.status.as_str() {
        "Waiting" => 18,
        "Working" => 10,
        _ => 0,
    };
    let category_points = if category_match { 50 } else { 0 };
    let preferred_points = if input.preferred_agent_id == Some(agent.id) {
        if category_match {
            12
        } else {
            3
        }
    } else {
        0
    };
    let specialist_points = if agent.role == "Specialist" { 10 } else { 0 };
    let file_points = if input.task.category == "Development"
        && matches!(agent.capabilities.files.as_str(), "write" | "full")
    {
        18
    } else {
        0
    };
    let terminal_points = if input.task.category == "Development"
        && matches!(agent.capabilities.terminal.as_str(), "safe" | "user")
    {
        8
    } else {
        0
    };
    let internet_points = if matches!(input.task.category.as_str(), "Browsing" | "Research")
        && matches!(
            agent.capabilities.internet.as_str(),
            "read" | "write" | "full"
        ) {
        16
    } else {
        0
    };
    let workload_points = workload.saturating_mul(-4);
    let components = vec![
        RouteScoreComponent {
            code: "STATUS".to_string(),
            points: status_points,
        },
        RouteScoreComponent {
            code: "CATEGORY".to_string(),
            points: category_points,
        },
        RouteScoreComponent {
            code: "PREFERRED".to_string(),
            points: preferred_points,
        },
        RouteScoreComponent {
            code: "SPECIALIST_ROLE".to_string(),
            points: specialist_points,
        },
        RouteScoreComponent {
            code: "STRENGTH".to_string(),
            points: agent.performance.strength,
        },
        RouteScoreComponent {
            code: "DEVELOPMENT_FILES".to_string(),
            points: file_points,
        },
        RouteScoreComponent {
            code: "DEVELOPMENT_TERMINAL".to_string(),
            points: terminal_points,
        },
        RouteScoreComponent {
            code: "RESEARCH_INTERNET".to_string(),
            points: internet_points,
        },
        RouteScoreComponent {
            code: "WORKLOAD".to_string(),
            points: workload_points,
        },
    ];
    let score = components.iter().map(|component| component.points).sum();
    (score, components)
}

fn validate_provider_runtime(
    state: &ApplicationState,
    providers: &ProviderRegistrySnapshot,
    agent: &Agent,
) -> Result<(), RouteDisqualification> {
    let identity = resolve_model_identity(
        &state.models,
        &agent.model,
        &state.preferences.active_ai_provider,
    )
    .map_err(|error| RouteDisqualification {
        code: error.code.as_str().to_string(),
        message: error.message,
    })?;
    let status = providers
        .providers
        .iter()
        .find(|status| status.provider.id == identity.provider_id)
        .ok_or_else(|| RouteDisqualification {
            code: "PROVIDER_UNAVAILABLE".to_string(),
            message: "The active runtime provider is absent from the inspected registry."
                .to_string(),
        })?;
    if status.availability != ProviderAvailability::Ready {
        return Err(RouteDisqualification {
            code: "PROVIDER_UNAVAILABLE".to_string(),
            message: status.message.clone(),
        });
    }
    if identity.provider_id == RuntimeProviderId::Ollama {
        let matching_models = status
            .models
            .iter()
            .filter(|model| model.name == identity.runtime_model)
            .collect::<Vec<_>>();
        let model = match matching_models.as_slice() {
            [] => {
                return Err(RouteDisqualification {
                    code: "MODEL_UNAVAILABLE".to_string(),
                    message: "The exact Ollama model is not installed in the inspected runtime."
                        .to_string(),
                })
            }
            [model] => *model,
            _ => {
                return Err(RouteDisqualification {
                    code: "MODEL_AMBIGUOUS".to_string(),
                    message: "The inspected Ollama runtime reports the model more than once."
                        .to_string(),
                })
            }
        };
        if model.availability != ProviderAvailability::Ready {
            return Err(RouteDisqualification {
                code: "MODEL_UNAVAILABLE".to_string(),
                message: model.message.clone(),
            });
        }
        if !model
            .capabilities
            .iter()
            .any(|capability| capability == "tools")
        {
            return Err(RouteDisqualification {
                code: "CAPABILITY_UNSUPPORTED".to_string(),
                message: "The exact Ollama model does not report workspace tool support."
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn policy_fixture(
    state: &ApplicationState,
    input: &RoutingTaskInput,
) -> Result<ApplicationState, RoutingError> {
    let mut policy_state = state.clone();
    for agent in &mut policy_state.agents {
        agent.tasks.clear();
        agent.activity.clear();
    }
    let owner = policy_state
        .agents
        .iter_mut()
        .find(|agent| agent.id == input.task_owner_agent_id)
        .ok_or_else(|| {
            RoutingError::new(
                "TASK_OWNER_NOT_FOUND",
                "The task owner does not exist.",
                Vec::new(),
            )
        })?;
    owner.tasks.push(input.task.clone());
    Ok(policy_state)
}

fn compare_candidates(left: &RouteCandidateEvidence, right: &RouteCandidateEvidence) -> Ordering {
    right
        .eligible
        .cmp(&left.eligible)
        .then_with(|| right.score.cmp(&left.score))
        .then_with(|| left.workload.cmp(&right.workload))
        .then_with(|| left.agent_id.cmp(&right.agent_id))
}

pub(crate) fn route_task(
    state: &ApplicationState,
    providers: &ProviderRegistrySnapshot,
    input: &RoutingTaskInput,
) -> Result<RoutingEvidence, RoutingError> {
    if !matches!(input.task.routing_mode.as_str(), "automatic" | "selected") {
        return Err(RoutingError::new(
            "INVALID_ROUTING_MODE",
            "Routing mode must be selected or automatic.",
            Vec::new(),
        ));
    }
    if input.task.id <= 0 || input.task_owner_agent_id <= 0 {
        return Err(RoutingError::new(
            "INVALID_TASK",
            "The task and owner identifiers must be positive.",
            Vec::new(),
        ));
    }

    let mut policy_state = policy_fixture(state, input)?;
    let mut candidates = Vec::with_capacity(state.agents.len());
    for agent in &state.agents {
        let workload = workload_for_agent(state, input, agent.id);
        let queue_threshold = agent.performance.queue_threshold;
        let overloaded = workload >= queue_threshold;
        let (score, score_components) = score_candidate(agent, input, workload);
        let mut disqualifications = Vec::new();

        if agent.registry_state != "active" {
            add_disqualification(
                &mut disqualifications,
                "AGENT_REGISTRY_INACTIVE",
                "Deleted or unassigned agents cannot execute tasks.",
            );
        }
        if agent.status == "Paused" {
            add_disqualification(
                &mut disqualifications,
                "AGENT_PAUSED",
                "Paused agents cannot execute tasks.",
            );
        }

        if let Some(owner) = policy_state
            .agents
            .iter_mut()
            .find(|candidate| candidate.id == input.task_owner_agent_id)
        {
            if let Some(task) = owner.tasks.first_mut() {
                task.assigned_agent_id = agent.id;
            }
        }
        let intent = ActionIntent::RunTask {
            agent_id: agent.id,
            task_owner_agent_id: input.task_owner_agent_id,
            task_id: input.task.id,
            run_mode: RunMode::Execute,
            review_context: None,
        };
        if let Err(error) = evaluate_policy(&policy_state, &intent) {
            add_disqualification(&mut disqualifications, error.code, error.message);
        }
        if let Err(disqualification) = validate_provider_runtime(state, providers, agent) {
            add_disqualification(
                &mut disqualifications,
                disqualification.code,
                disqualification.message,
            );
        }

        candidates.push(RouteCandidateEvidence {
            agent_id: agent.id,
            agent_name: agent.name.clone(),
            category: agent.category.clone(),
            role: agent.role.clone(),
            model: agent.model.clone(),
            eligible: disqualifications.is_empty(),
            disqualifications,
            score,
            score_components,
            workload,
            queue_threshold,
            overloaded,
            overflow_action: agent.performance.overflow_action.clone(),
            redirect_agent_id: agent.performance.redirect_agent_id,
            selection_excluded_code: None,
        });
    }
    candidates.sort_by(compare_candidates);

    let manual_agent_id = input
        .selected_agent_id
        .or(input.preferred_agent_id)
        .filter(|_| input.task.routing_mode == "selected");
    if input.task.routing_mode == "selected" && manual_agent_id.is_none() {
        return Err(RoutingError::new(
            "SELECTED_AGENT_REQUIRED",
            "Selected routing requires an explicit agent.",
            candidates,
        ));
    }

    let (winning_agent_id, outcome_code, reason, manual_override) = if let Some(selected_agent_id) =
        manual_agent_id
    {
        let selected = candidates
            .iter()
            .find(|candidate| candidate.agent_id == selected_agent_id)
            .ok_or_else(|| {
                RoutingError::new(
                    "SELECTED_AGENT_NOT_FOUND",
                    "The explicitly selected agent does not exist.",
                    candidates.clone(),
                )
            })?;
        if !selected.eligible {
            return Err(RoutingError::new(
                "SELECTED_AGENT_INELIGIBLE",
                "The explicitly selected agent failed a hard eligibility requirement.",
                candidates,
            ));
        }
        let overloaded = selected.overloaded;
        (
                selected_agent_id,
                if overloaded {
                    "MANUAL_WORKLOAD_OVERRIDE"
                } else {
                    "MANUAL_SELECTION"
                }
                .to_string(),
                if overloaded {
                    "The explicitly selected agent is hard-eligible and was retained despite its configured queue threshold."
                } else {
                    "The explicitly selected agent passed all hard eligibility requirements."
                }
                .to_string(),
                true,
            )
    } else {
        let ranked = candidates
            .iter()
            .filter(|candidate| candidate.eligible)
            .cloned()
            .collect::<Vec<_>>();
        let initial = ranked.first().cloned().ok_or_else(|| {
            RoutingError::new(
                "NO_ELIGIBLE_AGENT",
                "No agent passed the task's workspace, policy, model, and runtime requirements.",
                candidates.clone(),
            )
        })?;
        if initial.overloaded && initial.overflow_action == "redirect" {
            let initial_id = initial.agent_id;
            let initial_redirect_id = initial.redirect_agent_id;
            if let Some(excluded) = candidates
                .iter_mut()
                .find(|candidate| candidate.agent_id == initial_id)
            {
                excluded.selection_excluded_code = Some("OVERFLOW_REDIRECT".to_string());
            }
            let redirect = initial_redirect_id.and_then(|redirect_agent_id| {
                ranked
                    .iter()
                    .find(|candidate| {
                        candidate.agent_id == redirect_agent_id
                            && candidate.agent_id != initial_id
                            && candidate.eligible
                    })
                    .cloned()
            });
            let fallback = ranked
                .iter()
                .find(|candidate| candidate.agent_id != initial_id)
                .cloned();
            let redirected = redirect.is_some();
            let winner = redirect.or(fallback).ok_or_else(|| {
                    RoutingError::new(
                        "OVERFLOW_REDIRECT_UNAVAILABLE",
                        "The highest-ranked agent requires overflow redirection, but no other agent is eligible.",
                        candidates.clone(),
                    )
                })?;
            (
                winner.agent_id,
                if redirected {
                    "OVERFLOW_REDIRECTED"
                } else {
                    "OVERFLOW_REDIRECT_FALLBACK"
                }
                .to_string(),
                format!(
                    "Agent {} exceeded its queue threshold; task routed to hard-eligible agent {}.",
                    initial_id, winner.agent_id
                ),
                false,
            )
        } else {
            (
                initial.agent_id,
                if initial.overloaded {
                    "OVERFLOW_QUEUED"
                } else {
                    "AUTOMATIC_SELECTION"
                }
                .to_string(),
                if initial.overloaded {
                    format!(
                            "Agent {} ranked first and remains eligible under its queue overflow policy.",
                            initial.agent_id
                        )
                } else {
                    format!(
                            "Agent {} ranked first by score, workload, and stable identifier tie-breaks.",
                            initial.agent_id
                        )
                },
                false,
            )
        }
    };

    Ok(RoutingEvidence {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_string(),
        routing_mode: input.task.routing_mode.clone(),
        preferred_agent_id: input.preferred_agent_id,
        selected_agent_id: input.selected_agent_id,
        winning_agent_id,
        outcome_code,
        reason,
        manual_override,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_state::{default_application_state, WorkspaceDefinition},
        provider_runtime::{
            catalog_provider_bindings, ollama_descriptor, ProviderAvailability,
            ProviderRuntimeModel, ProviderRuntimeStatus,
        },
    };

    fn routing_fixture() -> (ApplicationState, ProviderRegistrySnapshot, RoutingTaskInput) {
        let mut state = default_application_state().expect("seed state should be valid");
        state.preferences.active_ai_provider = "ollama".to_string();
        state.preferences.workspaces.push(WorkspaceDefinition {
            id: "workspace-1".to_string(),
            name: "Fixture".to_string(),
            path: "/tmp/fixture".to_string(),
        });
        state.preferences.active_workspace_id = Some("workspace-1".to_string());

        let task = AgentTask {
            id: 41,
            title: "Implement the parser".to_string(),
            category: "Development".to_string(),
            priority: "Normal".to_string(),
            assigned_agent_id: 1,
            status: "Pending".to_string(),
            phase: "Assigned".to_string(),
            created_at: "2026-08-24T10:00:00.000Z".to_string(),
            completed_at: None,
            result: None,
            response_id: None,
            runtime_model: None,
            total_tokens: None,
            workspace_id: Some("workspace-1".to_string()),
            changed_files: Vec::new(),
            diff: None,
            duration_seconds: None,
            routing_mode: "automatic".to_string(),
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
        };
        let providers = ProviderRegistrySnapshot {
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
        };
        let input = RoutingTaskInput {
            task_owner_agent_id: 1,
            task,
            preferred_agent_id: Some(2),
            selected_agent_id: None,
        };
        (state, providers, input)
    }

    #[test]
    fn task_0010_routing_is_deterministic_and_records_eligibility() {
        let (state, providers, input) = routing_fixture();
        let first = route_task(&state, &providers, &input).expect("route should resolve");
        let second = route_task(&state, &providers, &input).expect("route should resolve");

        assert_eq!(first, second);
        assert_eq!(first.algorithm_version, ROUTING_ALGORITHM_VERSION);
        assert_eq!(first.winning_agent_id, 2);
        assert!(first
            .candidates
            .iter()
            .find(|candidate| candidate.agent_id == 2)
            .is_some_and(|candidate| candidate.eligible));
        assert!(first
            .candidates
            .iter()
            .filter(|candidate| candidate.agent_id != 2)
            .all(|candidate| !candidate.eligible));
    }

    #[test]
    fn task_0010_routing_rejects_a_model_without_ollama_tools() {
        let (state, mut providers, input) = routing_fixture();
        providers.providers[0].models[0].capabilities = vec!["completion".to_string()];
        let error = route_task(&state, &providers, &input).expect_err("route must fail closed");

        assert_eq!(error.code, "NO_ELIGIBLE_AGENT");
        assert!(error.candidates.iter().any(|candidate| {
            candidate.agent_id == 2
                && candidate
                    .disqualifications
                    .iter()
                    .any(|reason| reason.code == "CAPABILITY_UNSUPPORTED")
        }));
    }

    #[test]
    fn task_0010_routing_selected_mode_never_overrides_hard_filters() {
        let (state, providers, mut input) = routing_fixture();
        input.task.routing_mode = "selected".to_string();
        input.selected_agent_id = Some(4);
        let error = route_task(&state, &providers, &input).expect_err("paused agent must fail");

        assert_eq!(error.code, "SELECTED_AGENT_INELIGIBLE");
        assert!(error.candidates.iter().any(|candidate| {
            candidate.agent_id == 4
                && candidate
                    .disqualifications
                    .iter()
                    .any(|reason| reason.code == "AGENT_PAUSED")
        }));
    }

    #[test]
    fn task_0010_routing_redirects_overflow_and_records_the_excluded_winner() {
        let (mut state, providers, input) = routing_fixture();
        let candidate = state
            .agents
            .iter()
            .find(|agent| agent.id == 2)
            .expect("coding agent should exist")
            .clone();
        let redirect = state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 3)
            .expect("debugging agent should exist");
        redirect.status = candidate.status.clone();
        redirect.role = candidate.role.clone();
        redirect.category = candidate.category.clone();
        redirect.model = candidate.model.clone();
        redirect.capabilities = candidate.capabilities.clone();
        redirect.approvals = candidate.approvals.clone();
        redirect.performance = candidate.performance.clone();

        let mut queued = input.task.clone();
        queued.id = 42;
        queued.assigned_agent_id = 2;
        queued.enqueue_sequence = Some(2);
        state
            .agents
            .iter_mut()
            .find(|agent| agent.id == input.task_owner_agent_id)
            .expect("owner should exist")
            .tasks
            .push(queued);

        let coding = state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 2)
            .expect("coding agent should exist");
        coding.performance.queue_threshold = 1;
        coding.performance.overflow_action = "redirect".to_string();
        coding.performance.redirect_agent_id = Some(3);

        let evidence = route_task(&state, &providers, &input).expect("route should redirect");

        assert_eq!(evidence.winning_agent_id, 3);
        assert_eq!(evidence.outcome_code, "OVERFLOW_REDIRECTED");
        assert!(!evidence.manual_override);
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.agent_id == 2
                && candidate.selection_excluded_code.as_deref() == Some("OVERFLOW_REDIRECT")
        }));

        state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 2)
            .expect("coding agent should exist")
            .performance
            .redirect_agent_id = Some(2);
        let fallback =
            route_task(&state, &providers, &input).expect("self redirect should fall back");
        assert_eq!(fallback.winning_agent_id, 3);
        assert_eq!(fallback.outcome_code, "OVERFLOW_REDIRECT_FALLBACK");
    }

    #[test]
    fn task_0010_routing_records_selected_agent_workload_override() {
        let (mut state, providers, mut input) = routing_fixture();
        let mut queued = input.task.clone();
        queued.id = 42;
        queued.assigned_agent_id = 2;
        queued.enqueue_sequence = Some(2);
        state
            .agents
            .iter_mut()
            .find(|agent| agent.id == input.task_owner_agent_id)
            .expect("owner should exist")
            .tasks
            .push(queued);
        let coding = state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 2)
            .expect("coding agent should exist");
        coding.performance.queue_threshold = 1;
        input.task.routing_mode = "selected".to_string();
        input.selected_agent_id = Some(2);

        let evidence = route_task(&state, &providers, &input).expect("manual route should resolve");

        assert_eq!(evidence.winning_agent_id, 2);
        assert_eq!(evidence.outcome_code, "MANUAL_WORKLOAD_OVERRIDE");
        assert!(evidence.manual_override);
    }

    #[test]
    fn task_0010_routing_breaks_equal_scores_by_stable_agent_id() {
        let (mut state, providers, mut input) = routing_fixture();
        let earlier = state
            .agents
            .iter()
            .find(|agent| agent.id == 2)
            .expect("coding agent should exist")
            .clone();
        let later = state
            .agents
            .iter_mut()
            .find(|agent| agent.id == 3)
            .expect("debugging agent should exist");
        later.status = earlier.status.clone();
        later.role = earlier.role.clone();
        later.category = earlier.category.clone();
        later.model = earlier.model.clone();
        later.capabilities = earlier.capabilities.clone();
        later.approvals = earlier.approvals.clone();
        later.performance = earlier.performance.clone();
        input.preferred_agent_id = None;

        let evidence = route_task(&state, &providers, &input).expect("tie should resolve");

        assert_eq!(evidence.winning_agent_id, 2);
        assert_eq!(evidence.outcome_code, "AUTOMATIC_SELECTION");
    }
}
