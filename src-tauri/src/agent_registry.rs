use crate::app_state::{Agent, StateValidationError, MAX_SAFE_INTEGER};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const TEMPLATE_KEYS: [&str; 11] = [
    "supervisor",
    "coding",
    "debugging",
    "browser",
    "financial",
    "development-team-leader",
    "pc-control",
    "event-reminder",
    "research-web-senior",
    "finance-senior",
    "operations-senior",
];

const REGISTRY_ISSUES: [&str; 6] = [
    "self-parent",
    "missing-manager",
    "manager-not-active",
    "manager-authority",
    "cycle",
    "duplicate-id",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAgentRequest {
    pub expected_revision: i64,
    pub name: String,
    pub description: String,
    pub role: String,
    pub category: String,
    pub reports_to: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAgentRequest {
    pub expected_revision: i64,
    pub agent_id: i64,
    pub name: String,
    pub description: String,
    pub role: String,
    pub category: String,
    pub reports_to: Option<i64>,
    /// Optional so older callers keep working; `None` leaves the model unchanged.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAgentRequest {
    pub expected_revision: i64,
    pub agent_id: i64,
    pub replacement_manager_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreAgentTemplateRequest {
    pub expected_revision: i64,
    pub template_key: String,
    pub reports_to: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplateSummary {
    pub template_key: String,
    pub name: String,
    pub description: String,
    pub role: String,
    pub category: String,
    pub authority_level: i64,
    pub active_agent_id: Option<i64>,
    pub restorable: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRegistrySnapshot {
    pub revision: i64,
    pub templates: Vec<AgentTemplateSummary>,
}

pub fn authority_for_role(role: &str) -> Option<i64> {
    match role {
        "Supervisor" => Some(4),
        "Team Leader" => Some(3),
        "Senior Agent" => Some(2),
        "Specialist" => Some(1),
        _ => None,
    }
}

pub fn template_summaries(defaults: &[Agent], agents: &[Agent]) -> Vec<AgentTemplateSummary> {
    defaults
        .iter()
        .filter_map(|template| {
            let template_key = template.template_key.clone()?;
            let existing = agents
                .iter()
                .find(|agent| agent.template_key.as_deref() == Some(template_key.as_str()));
            Some(AgentTemplateSummary {
                template_key,
                name: template.name.clone(),
                description: template.description.clone(),
                role: template.role.clone(),
                category: template.category.clone(),
                authority_level: template.authority_level,
                active_agent_id: existing
                    .filter(|agent| agent.registry_state == "active")
                    .map(|agent| agent.id),
                restorable: existing
                    .map(|agent| agent.registry_state != "active")
                    .unwrap_or(true),
            })
        })
        .collect()
}

fn legacy_template_key(agent: &Agent) -> Option<&'static str> {
    match (agent.id, agent.role.as_str(), agent.category.as_str()) {
        (1, "Supervisor", "Management") => Some("supervisor"),
        (2, "Specialist", "Development") => Some("coding"),
        (3, "Senior Agent", "Development") => Some("debugging"),
        (4, "Specialist", "Browsing") => Some("browser"),
        (5, "Specialist", "Finance") => Some("financial"),
        (6, "Team Leader", "Management") => Some("development-team-leader"),
        (7, "Specialist", "System Control") => Some("pc-control"),
        (8, "Specialist", "Business") => Some("event-reminder"),
        (9, "Senior Agent", "Browsing") => Some("research-web-senior"),
        (10, "Senior Agent", "Finance") => Some("finance-senior"),
        (11, "Senior Agent", "Business") => Some("operations-senior"),
        _ => None,
    }
}

/// Repairs a parsed legacy agent list that contains more than one agent sharing
/// the same `id`. Legacy prototype data (for example the real `Finance Agent` /
/// `Financial Agent` pair that both carry `id = 5`) predates the authoritative
/// unique-identity contract, so the raw import must be made self-consistent
/// before [`normalize_legacy_agents`] and authoritative validation run.
///
/// The first occurrence of every id keeps that id and stays canonical. Each
/// later duplicate is re-keyed to a fresh JavaScript-safe id and quarantined as
/// `unassigned` with the `duplicate-id` registry issue (paused and detached)
/// rather than deleted, so no prototype agent is lost. Only references that are
/// provably owned by the re-keyed instance itself - its own nested tasks whose
/// `assigned_agent_id` equals the old shared id - follow the new id. Every other
/// agent-id-bearing reference is left untouched and therefore keeps resolving to
/// the canonical first occurrence, so a duplicate can never inherit approvals,
/// reminders, reviews, routing decisions, or reporting edges by accident.
///
/// Returns an error only when no JavaScript-safe id remains for a re-key, in
/// which case the caller aborts the migration with the legacy source intact.
/// The transform is idempotent: an already-unique list is returned unchanged.
pub fn repair_duplicate_agent_ids(agents: &mut [Agent]) -> Result<(), StateValidationError> {
    let mut next_id = agents.iter().map(|agent| agent.id).max().unwrap_or(0);
    let mut claimed: HashSet<i64> = HashSet::with_capacity(agents.len());

    for agent in agents.iter_mut() {
        let old_id = agent.id;
        if claimed.insert(old_id) {
            continue;
        }

        // Allocate above the current maximum so a fresh id can never collide
        // with an id that already exists earlier or later in the list.
        next_id = next_id
            .checked_add(1)
            .filter(|candidate| *candidate <= MAX_SAFE_INTEGER)
            .ok_or_else(|| {
                StateValidationError::new(
                    "agents",
                    "no JavaScript-safe identifier remains to re-key a duplicate agent id",
                )
            })?;
        let new_id = next_id;
        claimed.insert(new_id);

        agent.id = new_id;
        agent.registry_state = "unassigned".to_string();
        agent.registry_issue = Some("duplicate-id".to_string());
        agent.status = "Paused".to_string();
        agent.reports_to = None;
        agent.deleted_at_unix_ms = None;

        // Self-owned references only: a task nested in this agent that is also
        // assigned to the shared old id follows the re-key. Reviewer, routed-from,
        // redirect, approval, and reminder references stay pointed at the
        // canonical first occurrence (fail closed).
        for task in &mut agent.tasks {
            if task.assigned_agent_id == old_id {
                task.assigned_agent_id = new_id;
            }
        }
    }

    Ok(())
}

pub fn normalize_legacy_agents(agents: &mut [Agent]) {
    for agent in agents.iter_mut() {
        if agent.template_key.is_none() {
            agent.template_key = legacy_template_key(agent).map(str::to_string);
        }
        if let Some(authority) = authority_for_role(&agent.role) {
            agent.authority_level = authority;
        }
        if agent.registry_state == "active" && agent.role == "Supervisor" {
            agent.reports_to = None;
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        let snapshot = agents
            .iter()
            .map(|agent| {
                (
                    agent.id,
                    (agent.registry_state.clone(), agent.authority_level),
                )
            })
            .collect::<HashMap<_, _>>();
        for agent in agents.iter_mut() {
            if agent.registry_state != "active" || agent.role == "Supervisor" {
                continue;
            }
            let issue = match agent.reports_to {
                Some(manager_id) if manager_id == agent.id => Some("self-parent"),
                None => Some("missing-manager"),
                Some(manager_id) => match snapshot.get(&manager_id) {
                    None => Some("missing-manager"),
                    Some((state, _)) if state != "active" => Some("manager-not-active"),
                    Some((_, authority)) if *authority <= agent.authority_level => {
                        Some("manager-authority")
                    }
                    Some(_) => None,
                },
            };
            if let Some(issue) = issue {
                agent.registry_state = "unassigned".to_string();
                agent.registry_issue = Some(issue.to_string());
                agent.status = "Paused".to_string();
                agent.reports_to = None;
                changed = true;
            }
        }
    }
}

pub fn validate_agent_registry(agents: &[Agent]) -> Result<(), StateValidationError> {
    let mut template_keys = HashSet::new();
    let by_id = agents
        .iter()
        .enumerate()
        .map(|(index, agent)| (agent.id, (index, agent)))
        .collect::<HashMap<_, _>>();

    for (index, agent) in agents.iter().enumerate() {
        if agent.registry_state != "active" || agent.reports_to == Some(agent.id) {
            continue;
        }
        let mut current = Some(agent.id);
        let mut visited = HashSet::new();
        while let Some(agent_id) = current {
            if !visited.insert(agent_id) {
                return Err(StateValidationError::new(
                    format!("agents[{index}].reportsTo"),
                    "reporting hierarchy contains a cycle",
                ));
            }
            current = by_id.get(&agent_id).and_then(|(_, current_agent)| {
                (current_agent.registry_state == "active")
                    .then_some(current_agent.reports_to)
                    .flatten()
            });
        }
    }

    for (index, agent) in agents.iter().enumerate() {
        let path = format!("agents[{index}]");
        if let Some(template_key) = agent.template_key.as_deref() {
            if !TEMPLATE_KEYS.contains(&template_key) {
                return Err(StateValidationError::new(
                    format!("{path}.templateKey"),
                    "template key is not recognized",
                ));
            }
            if !template_keys.insert(template_key) {
                return Err(StateValidationError::new(
                    format!("{path}.templateKey"),
                    "template key must be unique",
                ));
            }
        }

        let expected_authority = authority_for_role(&agent.role).ok_or_else(|| {
            StateValidationError::new(format!("{path}.role"), "agent role is not recognized")
        })?;
        if agent.authority_level != expected_authority {
            return Err(StateValidationError::new(
                format!("{path}.authorityLevel"),
                "authority level must be derived from role",
            ));
        }

        match agent.registry_state.as_str() {
            "active" => {
                if agent.registry_issue.is_some() || agent.deleted_at_unix_ms.is_some() {
                    return Err(StateValidationError::new(
                        format!("{path}.registryState"),
                        "active agents cannot carry a registry issue or deletion timestamp",
                    ));
                }
                if agent.role == "Supervisor" {
                    if agent.reports_to.is_some() {
                        return Err(StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "supervisors cannot report to another agent",
                        ));
                    }
                } else {
                    let manager_id = agent.reports_to.ok_or_else(|| {
                        StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "active non-supervisors require a manager",
                        )
                    })?;
                    if manager_id == agent.id {
                        return Err(StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "agents cannot report to themselves",
                        ));
                    }
                    let (_, manager) = by_id.get(&manager_id).ok_or_else(|| {
                        StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "manager does not exist",
                        )
                    })?;
                    if manager.registry_state != "active" {
                        return Err(StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "manager must be active",
                        ));
                    }
                    if manager.authority_level <= agent.authority_level {
                        return Err(StateValidationError::new(
                            format!("{path}.reportsTo"),
                            "manager must have greater authority",
                        ));
                    }
                }
            }
            "unassigned" => {
                if agent
                    .registry_issue
                    .as_deref()
                    .is_some_and(|issue| !REGISTRY_ISSUES.contains(&issue))
                {
                    return Err(StateValidationError::new(
                        format!("{path}.registryIssue"),
                        "registry issue is not recognized",
                    ));
                }
                if agent.registry_issue.is_none()
                    || agent.deleted_at_unix_ms.is_some()
                    || agent.reports_to.is_some()
                    || agent.status != "Paused"
                {
                    return Err(StateValidationError::new(
                        format!("{path}.registryState"),
                        "unassigned agents must be paused, detached, and explain the issue",
                    ));
                }
            }
            "deleted" => {
                if agent
                    .deleted_at_unix_ms
                    .is_some_and(|timestamp| !(0..=MAX_SAFE_INTEGER).contains(&timestamp))
                {
                    return Err(StateValidationError::new(
                        format!("{path}.deletedAtUnixMs"),
                        "deletion timestamp must be a safe non-negative integer",
                    ));
                }
                if agent.registry_issue.is_some()
                    || agent.deleted_at_unix_ms.is_none()
                    || agent.reports_to.is_some()
                    || agent.status != "Paused"
                {
                    return Err(StateValidationError::new(
                        format!("{path}.registryState"),
                        "deleted agents must be paused, detached, and timestamped",
                    ));
                }
            }
            _ => {
                return Err(StateValidationError::new(
                    format!("{path}.registryState"),
                    "registry state is not recognized",
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::{default_application_state, AgentTask};

    /// Seed agents stripped of their template keys, matching the shape of real
    /// legacy prototype data (which never carried a `templateKey`).
    fn legacy_like_agents() -> Vec<Agent> {
        let mut agents = default_application_state()
            .expect("seed application state is valid")
            .agents;
        for agent in &mut agents {
            agent.template_key = None;
        }
        agents
    }

    /// Reproduces the real `Finance Agent` (id 5, reports to the Supervisor) /
    /// `Financial Agent` (id 5, reports to Finance Senior) duplicate-identity
    /// case observed in the TASK-0020 S3 blocker.
    fn with_duplicate_finance_identity() -> Vec<Agent> {
        let mut agents = legacy_like_agents();
        let position = agents
            .iter()
            .position(|agent| agent.id == 5)
            .expect("seed has an agent with id 5");
        agents[position].name = "Finance Agent".to_string();
        agents[position].reports_to = Some(1);
        let mut clone = agents[position].clone();
        clone.name = "Financial Agent".to_string();
        clone.reports_to = Some(10);
        agents.insert(position + 1, clone);
        agents
    }

    fn legacy_task(assigned_agent_id: i64, review_agent_id: Option<i64>) -> AgentTask {
        AgentTask {
            id: 9_100 + assigned_agent_id,
            title: "Legacy finance task".to_string(),
            category: "Finance".to_string(),
            priority: "Normal".to_string(),
            assigned_agent_id,
            status: "Completed".to_string(),
            phase: "Finished".to_string(),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            completed_at: Some("2026-01-01T00:05:00.000Z".to_string()),
            result: None,
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
            review_agent_id,
            review_status: "Not Requested".to_string(),
            review_result: None,
            review_model: None,
            review_duration_seconds: None,
            reviewed_at: None,
        }
    }

    #[test]
    fn repair_is_noop_for_unique_ids() {
        let mut agents = legacy_like_agents();
        let before = agents.clone();
        repair_duplicate_agent_ids(&mut agents).expect("unique ids need no repair");
        assert_eq!(agents, before);
    }

    #[test]
    fn first_occurrence_stays_canonical_and_later_duplicate_is_requarantined() {
        let mut agents = with_duplicate_finance_identity();
        let position = agents
            .iter()
            .position(|agent| agent.name == "Finance Agent")
            .unwrap();

        repair_duplicate_agent_ids(&mut agents).expect("duplicate id is repairable");

        // No prototype agent is lost.
        assert_eq!(agents.len(), legacy_like_agents().len() + 1);
        // Every id is now unique.
        let mut ids = HashSet::new();
        assert!(agents.iter().all(|agent| ids.insert(agent.id)));

        let canonical = &agents[position];
        assert_eq!(canonical.id, 5);
        assert_eq!(canonical.name, "Finance Agent");
        assert_eq!(canonical.registry_state, "active");
        assert!(canonical.registry_issue.is_none());
        assert_eq!(canonical.reports_to, Some(1));

        let requarantined = &agents[position + 1];
        assert_eq!(requarantined.name, "Financial Agent");
        assert_eq!(requarantined.id, 12);
        assert_eq!(requarantined.registry_state, "unassigned");
        assert_eq!(
            requarantined.registry_issue.as_deref(),
            Some("duplicate-id")
        );
        assert_eq!(requarantined.status, "Paused");
        assert_eq!(requarantined.reports_to, None);
        assert_eq!(requarantined.deleted_at_unix_ms, None);
    }

    #[test]
    fn multiple_duplicates_receive_distinct_fresh_ids() {
        let mut agents = legacy_like_agents();
        let finance = agents.iter().find(|agent| agent.id == 5).unwrap().clone();
        let debugging = agents.iter().find(|agent| agent.id == 3).unwrap().clone();
        agents.push(finance.clone());
        agents.push(finance);
        agents.push(debugging);

        repair_duplicate_agent_ids(&mut agents).expect("duplicates are repairable");

        let mut ids = HashSet::new();
        assert!(agents.iter().all(|agent| ids.insert(agent.id)));
        let requarantined: Vec<i64> = agents
            .iter()
            .filter(|agent| agent.registry_issue.as_deref() == Some("duplicate-id"))
            .map(|agent| agent.id)
            .collect();
        assert_eq!(requarantined, vec![12, 13, 14]);
    }

    #[test]
    fn repair_is_idempotent() {
        let mut agents = with_duplicate_finance_identity();
        repair_duplicate_agent_ids(&mut agents).expect("first repair succeeds");
        let once = agents.clone();
        repair_duplicate_agent_ids(&mut agents).expect("second repair is a no-op");
        assert_eq!(agents, once);
    }

    #[test]
    fn exhausted_identifier_space_fails_closed() {
        let mut agents = legacy_like_agents();
        agents[0].id = MAX_SAFE_INTEGER;
        agents[1].id = MAX_SAFE_INTEGER;

        let error =
            repair_duplicate_agent_ids(&mut agents).expect_err("no safe id remains for the re-key");
        assert_eq!(error.path, "agents");
        assert!(error
            .message
            .contains("no JavaScript-safe identifier remains"));
    }

    #[test]
    fn self_owned_task_reference_follows_rekey_but_external_references_do_not() {
        let mut agents = with_duplicate_finance_identity();
        // An unrelated agent that legitimately reports to the shared id.
        let browser = agents.iter_mut().find(|agent| agent.id == 4).unwrap();
        browser.reports_to = Some(5);
        // The later duplicate owns a task assigned to the shared id, and also
        // carries a reviewer / redirect reference to another agent at id 5.
        let clone = agents
            .iter_mut()
            .find(|agent| agent.name == "Financial Agent")
            .unwrap();
        clone.tasks.push(legacy_task(5, Some(5)));
        clone.tasks.push(legacy_task(99, Some(5)));
        clone.performance.redirect_agent_id = Some(5);

        repair_duplicate_agent_ids(&mut agents).expect("duplicate id is repairable");

        let requarantined = agents
            .iter()
            .find(|agent| agent.name == "Financial Agent")
            .unwrap();
        assert_eq!(requarantined.id, 12);
        // Self-owned assignment follows the re-key.
        assert_eq!(requarantined.tasks[0].assigned_agent_id, 12);
        // A task assigned elsewhere is untouched.
        assert_eq!(requarantined.tasks[1].assigned_agent_id, 99);
        // External authority references stay pointed at the canonical id 5.
        assert_eq!(requarantined.tasks[0].review_agent_id, Some(5));
        assert_eq!(requarantined.tasks[1].review_agent_id, Some(5));
        assert_eq!(requarantined.performance.redirect_agent_id, Some(5));

        let browser = agents.iter().find(|agent| agent.id == 4).unwrap();
        assert_eq!(browser.reports_to, Some(5));
        let canonical = agents
            .iter()
            .find(|agent| agent.name == "Finance Agent")
            .unwrap();
        assert_eq!(canonical.id, 5);
        assert_eq!(canonical.tasks.len(), 0);
    }

    #[test]
    fn repaired_duplicate_passes_registry_validation_after_normalization() {
        let mut agents = with_duplicate_finance_identity();
        repair_duplicate_agent_ids(&mut agents).expect("duplicate id is repairable");
        normalize_legacy_agents(&mut agents);

        validate_agent_registry(&agents).expect("repaired legacy registry is valid");

        let canonical = agents.iter().find(|agent| agent.id == 5).unwrap();
        assert_eq!(canonical.template_key.as_deref(), Some("financial"));
        let requarantined = agents.iter().find(|agent| agent.id == 12).unwrap();
        assert!(requarantined.template_key.is_none());
        assert_eq!(
            requarantined.registry_issue.as_deref(),
            Some("duplicate-id")
        );
    }
}
