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

const REGISTRY_ISSUES: [&str; 5] = [
    "self-parent",
    "missing-manager",
    "manager-not-active",
    "manager-authority",
    "cycle",
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
