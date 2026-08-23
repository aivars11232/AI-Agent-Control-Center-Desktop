use crate::{
    app_state::{Agent, AgentTask, ApplicationState, WorkspaceDefinition},
    provider_runtime::resolve_model_identity,
};
use serde::{Deserialize, Serialize};

const MAX_INTENT_TEXT: usize = 4 * 1024;
const POLICY_FINGERPRINT_VERSION: &str = "policy-v3";
const INTENT_FINGERPRINT_VERSION: &str = "intent-v1";
const WORKSPACE_FINGERPRINT_VERSION: &str = "workspace-v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    Execute,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ActionIntent {
    RunTask {
        agent_id: i64,
        task_owner_agent_id: i64,
        task_id: i64,
        run_mode: RunMode,
    },
    OpenWorkspaceItem {
        agent_id: i64,
        workspace_id: String,
        item_path: String,
    },
    LaunchAllowedApplication {
        agent_id: i64,
        application: String,
    },
    LaunchDesktopApplication {
        agent_id: i64,
        application: String,
    },
    OpenStandardFolder {
        agent_id: i64,
        folder: String,
    },
    CloseAllowedApplication {
        agent_id: i64,
        application: String,
    },
    CloseActiveApplication {
        agent_id: i64,
    },
    DesktopKeyboard {
        agent_id: i64,
        action: String,
    },
    DesktopWindow {
        agent_id: i64,
        application: String,
        action: String,
    },
    TypeDesktopText {
        agent_id: i64,
        text: String,
    },
    EnableDesktopControl {
        agent_id: i64,
    },
    DesktopPointer {
        agent_id: i64,
        action: String,
    },
    InstallVoiceRuntime {
        agent_id: i64,
    },
    InstallHighAccuracyVoiceRuntime {
        agent_id: i64,
    },
    StartVoiceListener {
        agent_id: i64,
    },
}

impl ActionIntent {
    pub fn agent_id(&self) -> i64 {
        match self {
            Self::RunTask { agent_id, .. }
            | Self::OpenWorkspaceItem { agent_id, .. }
            | Self::LaunchAllowedApplication { agent_id, .. }
            | Self::LaunchDesktopApplication { agent_id, .. }
            | Self::OpenStandardFolder { agent_id, .. }
            | Self::CloseAllowedApplication { agent_id, .. }
            | Self::CloseActiveApplication { agent_id }
            | Self::DesktopKeyboard { agent_id, .. }
            | Self::DesktopWindow { agent_id, .. }
            | Self::TypeDesktopText { agent_id, .. }
            | Self::EnableDesktopControl { agent_id }
            | Self::DesktopPointer { agent_id, .. }
            | Self::InstallVoiceRuntime { agent_id }
            | Self::InstallHighAccuracyVoiceRuntime { agent_id }
            | Self::StartVoiceListener { agent_id } => *agent_id,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::RunTask { .. } => "runTask",
            Self::OpenWorkspaceItem { .. } => "openWorkspaceItem",
            Self::LaunchAllowedApplication { .. } => "launchAllowedApplication",
            Self::LaunchDesktopApplication { .. } => "launchDesktopApplication",
            Self::OpenStandardFolder { .. } => "openStandardFolder",
            Self::CloseAllowedApplication { .. } => "closeAllowedApplication",
            Self::CloseActiveApplication { .. } => "closeActiveApplication",
            Self::DesktopKeyboard { .. } => "desktopKeyboard",
            Self::DesktopWindow { .. } => "desktopWindow",
            Self::TypeDesktopText { .. } => "typeDesktopText",
            Self::EnableDesktopControl { .. } => "enableDesktopControl",
            Self::DesktopPointer { .. } => "desktopPointer",
            Self::InstallVoiceRuntime { .. } => "installVoiceRuntime",
            Self::InstallHighAccuracyVoiceRuntime { .. } => "installHighAccuracyVoiceRuntime",
            Self::StartVoiceListener { .. } => "startVoiceListener",
        }
    }

    pub fn validate(&self) -> Result<(), PolicyDenial> {
        if self.agent_id() <= 0 {
            return Err(PolicyDenial::new(
                "INVALID_AGENT",
                "The authorization intent has an invalid agent identifier.",
            ));
        }
        match self {
            Self::RunTask {
                task_owner_agent_id,
                task_id,
                ..
            } if *task_owner_agent_id <= 0 || *task_id <= 0 => Err(PolicyDenial::new(
                "INVALID_TASK",
                "The authorization intent has an invalid task identifier.",
            )),
            Self::OpenWorkspaceItem {
                workspace_id,
                item_path,
                ..
            } => {
                validate_text("workspace identifier", workspace_id, 256)?;
                validate_text("workspace item path", item_path, MAX_INTENT_TEXT)
            }
            Self::LaunchAllowedApplication { application, .. }
            | Self::LaunchDesktopApplication { application, .. }
            | Self::CloseAllowedApplication { application, .. } => {
                validate_text("application", application, 256)
            }
            Self::OpenStandardFolder { folder, .. } => {
                validate_text("standard folder", folder, 128)
            }
            Self::DesktopKeyboard { action, .. } | Self::DesktopPointer { action, .. } => {
                validate_text("desktop action", action, 128)
            }
            Self::DesktopWindow {
                application,
                action,
                ..
            } => {
                validate_text("application", application, 256)?;
                validate_text("window action", action, 128)
            }
            Self::TypeDesktopText { text, .. } => {
                if text.trim().is_empty()
                    || text.len() > MAX_INTENT_TEXT
                    || text
                        .chars()
                        .any(|character| character.is_control() && character != '\n')
                {
                    Err(PolicyDenial::new(
                        "INVALID_INTENT",
                        "The desktop text is empty, too long, or contains unsupported control characters.",
                    ))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}

fn validate_text(label: &str, value: &str, maximum: usize) -> Result<(), PolicyDenial> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(PolicyDenial::new(
            "INVALID_INTENT",
            format!("The {label} is empty, too long, or contains control characters."),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDisposition {
    Allow,
    ApprovalRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Files,
    Internet,
    Clipboard,
    Terminal,
    System,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Internet => "internet",
            Self::Clipboard => "clipboard",
            Self::Terminal => "terminal",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub disposition: PolicyDisposition,
    pub agent_id: i64,
    pub task_id: Option<i64>,
    pub workspace_id: Option<String>,
    pub task_snapshot: String,
    pub title: String,
    pub reason: String,
    pub risk_level: String,
    pub scopes: Vec<Scope>,
    pub intent_kind: String,
    pub intent_fingerprint: String,
    pub policy_fingerprint: String,
    pub workspace_fingerprint: String,
    pub expires_in_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDenial {
    pub code: String,
    pub message: String,
}

impl PolicyDenial {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub fn evaluate_policy(
    state: &ApplicationState,
    intent: &ActionIntent,
) -> Result<PolicyEvaluation, PolicyDenial> {
    intent.validate()?;
    let agent = state
        .agents
        .iter()
        .find(|candidate| candidate.id == intent.agent_id())
        .ok_or_else(|| {
            PolicyDenial::new("AGENT_NOT_FOUND", "The selected agent does not exist.")
        })?;
    if agent.status == "Paused" {
        return Err(PolicyDenial::new(
            "AGENT_PAUSED",
            "Paused agents cannot execute privileged actions.",
        ));
    }

    let mut context = EvaluationContext::new(state, agent, intent)?;
    context.evaluate()?;
    context.finish()
}

struct EvaluationContext<'a> {
    state: &'a ApplicationState,
    agent: &'a Agent,
    intent: &'a ActionIntent,
    task: Option<&'a AgentTask>,
    workspace: Option<&'a WorkspaceDefinition>,
    scopes: Vec<Scope>,
    approval_required: bool,
    force_approval: bool,
    risk_level: &'static str,
    title: String,
    reason: String,
}

impl<'a> EvaluationContext<'a> {
    fn new(
        state: &'a ApplicationState,
        agent: &'a Agent,
        intent: &'a ActionIntent,
    ) -> Result<Self, PolicyDenial> {
        let (task, workspace) = match intent {
            ActionIntent::RunTask {
                task_owner_agent_id,
                task_id,
                ..
            } => {
                let owner = state
                    .agents
                    .iter()
                    .find(|candidate| candidate.id == *task_owner_agent_id)
                    .ok_or_else(|| {
                        PolicyDenial::new("TASK_OWNER_NOT_FOUND", "The task owner does not exist.")
                    })?;
                let task = owner
                    .tasks
                    .iter()
                    .find(|candidate| candidate.id == *task_id)
                    .ok_or_else(|| {
                        PolicyDenial::new("TASK_NOT_FOUND", "The selected task does not exist.")
                    })?;
                let workspace_id = task
                    .workspace_id
                    .as_ref()
                    .or(state.preferences.active_workspace_id.as_ref())
                    .ok_or_else(|| {
                        PolicyDenial::new(
                            "WORKSPACE_REQUIRED",
                            "The task must be bound to a selected workspace.",
                        )
                    })?;
                let workspace = find_workspace(state, workspace_id)?;
                (Some(task), Some(workspace))
            }
            ActionIntent::OpenWorkspaceItem { workspace_id, .. } => {
                (None, Some(find_workspace(state, workspace_id)?))
            }
            _ => (None, None),
        };

        Ok(Self {
            state,
            agent,
            intent,
            task,
            workspace,
            scopes: Vec::new(),
            approval_required: false,
            force_approval: false,
            risk_level: "Low",
            title: String::new(),
            reason: String::new(),
        })
    }

    fn evaluate(&mut self) -> Result<(), PolicyDenial> {
        match self.intent {
            ActionIntent::RunTask { run_mode, .. } => self.evaluate_run(*run_mode),
            ActionIntent::OpenWorkspaceItem { .. } => {
                self.title = "Open workspace item".to_string();
                self.risk_level = "Low";
                self.require_scope(Scope::Files, 1)
            }
            ActionIntent::DesktopKeyboard { action, .. } => {
                self.evaluate_system_action("Send desktop keyboard input", true)?;
                if matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "copy" | "cut" | "paste"
                ) {
                    let level = if action.eq_ignore_ascii_case("copy") {
                        1
                    } else {
                        2
                    };
                    self.require_scope(Scope::Clipboard, level)?;
                }
                Ok(())
            }
            ActionIntent::LaunchAllowedApplication { .. }
            | ActionIntent::LaunchDesktopApplication { .. } => {
                self.evaluate_system_action("Launch desktop application", true)
            }
            ActionIntent::OpenStandardFolder { .. } => {
                self.evaluate_system_action("Open standard folder", true)
            }
            ActionIntent::CloseAllowedApplication { .. }
            | ActionIntent::CloseActiveApplication { .. } => {
                self.evaluate_system_action("Close desktop application", true)
            }
            ActionIntent::DesktopWindow { .. } => {
                self.evaluate_system_action("Control desktop window", true)
            }
            ActionIntent::TypeDesktopText { .. } => {
                self.evaluate_system_action("Type desktop text", true)
            }
            ActionIntent::EnableDesktopControl { .. } => {
                self.evaluate_system_action("Enable desktop input control", true)
            }
            ActionIntent::DesktopPointer { .. } => {
                self.evaluate_system_action("Send desktop pointer input", true)
            }
            ActionIntent::InstallVoiceRuntime { .. }
            | ActionIntent::InstallHighAccuracyVoiceRuntime { .. } => {
                self.evaluate_system_action("Install offline voice runtime", true)?;
                self.require_scope(Scope::Internet, 1)?;
                self.require_scope(Scope::Terminal, 1)
            }
            ActionIntent::StartVoiceListener { .. } => {
                self.evaluate_system_action("Start microphone listener", true)
            }
        }
    }

    fn evaluate_run(&mut self, run_mode: RunMode) -> Result<(), PolicyDenial> {
        let task = self.task.expect("run intents always resolve a task");
        if run_mode == RunMode::Execute && task.assigned_agent_id != self.agent.id {
            return Err(PolicyDenial::new(
                "WRONG_TASK_AGENT",
                "The task is not assigned to the selected executing agent.",
            ));
        }
        if run_mode == RunMode::Review
            && (task
                .review_agent_id
                .is_some_and(|review_agent_id| review_agent_id != self.agent.id)
                || (task.review_agent_id.is_none()
                    && !matches!(
                        self.agent.role.as_str(),
                        "Senior Agent" | "Team Leader" | "Supervisor"
                    )))
        {
            return Err(PolicyDenial::new(
                "WRONG_REVIEW_AGENT",
                "The selected agent is not eligible to review this task.",
            ));
        }

        self.title = if run_mode == RunMode::Review {
            format!("Review task: {}", task.title)
        } else {
            format!("Run task: {}", task.title)
        };
        self.require_scope(Scope::Files, 1)?;
        if run_mode == RunMode::Review {
            self.risk_level = "Low";
            self.reason = "Review runs are confined to read-only workspace access.".to_string();
            return Ok(());
        }

        let text = format!("{} {}", task.title, task.category).to_ascii_lowercase();
        let privileged = contains_any(
            &text,
            &[
                "sudo",
                "doas",
                "mkfs",
                "chown",
                "chmod",
                "mount",
                "umount",
                "pacman",
                "apt ",
                "dnf ",
                "account management",
                "package removal",
            ],
        );
        if privileged {
            return Err(PolicyDenial::new(
                "PRIVILEGED_ACTION_BLOCKED",
                "Privileged, package-management, and account commands are blocked.",
            ));
        }
        let destructive = contains_any(
            &text,
            &[
                "delete",
                "remove",
                "erase",
                "wipe",
                "truncate",
                "overwrite",
                "reset --hard",
                "clean -",
                "rm ",
                "rmdir",
                "unlink",
            ],
        );
        let writes = destructive
            || task.category == "Development"
            || contains_any(
                &text,
                &[
                    "create",
                    "write",
                    "edit",
                    "modify",
                    "change",
                    "update",
                    "refactor",
                    "fix",
                    "move",
                    "rename",
                    "replace",
                    "generate",
                    "add",
                    "implement",
                    "build",
                    "compile",
                    "install",
                    "format",
                ],
            );
        if writes {
            self.replace_scope_requirement(Scope::Files, if destructive { 3 } else { 2 })?;
        }
        if contains_any(
            &text,
            &[
                "command", "terminal", "shell", "bash", "execute", "npm", "pnpm", "yarn", "cargo",
                "rustc", "git", "python", "pytest", "compile", "install",
            ],
        ) {
            self.require_scope(Scope::Terminal, 1)?;
        }
        if task.category == "Browsing"
            || contains_any(
                &text,
                &[
                    "internet",
                    "website",
                    "web search",
                    "browse",
                    "download",
                    "upload",
                    "curl",
                    "wget",
                    "url",
                    "online",
                ],
            )
        {
            self.require_scope(Scope::Internet, 1)?;
        }
        if contains_any(&text, &["clipboard", "copy to", "paste from"]) {
            return Err(PolicyDenial::new(
                "RUN_CLIPBOARD_UNSUPPORTED",
                "AI runs do not receive direct clipboard authority in this release.",
            ));
        }
        if task.category == "System Control"
            || contains_any(
                &text,
                &[
                    "systemctl",
                    "reboot",
                    "shutdown",
                    "power off",
                    "desktop control",
                    "computer control",
                    "open app",
                    "close app",
                ],
            )
        {
            return Err(PolicyDenial::new(
                "RUN_SYSTEM_CONTROL_UNSUPPORTED",
                "AI runs cannot execute system-control actions directly.",
            ));
        }

        self.risk_level = if destructive || task.priority == "Critical" {
            "High"
        } else if writes || self.scopes.iter().any(|scope| *scope != Scope::Files) {
            "Medium"
        } else {
            "Low"
        };
        self.force_approval = destructive || task.priority == "Critical";
        self.reason = format!(
            "{}-risk task requests {}.",
            self.risk_level,
            self.scopes
                .iter()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(())
    }

    fn evaluate_system_action(
        &mut self,
        title: &str,
        force_approval: bool,
    ) -> Result<(), PolicyDenial> {
        self.title = title.to_string();
        self.risk_level = "High";
        self.force_approval = force_approval;
        self.require_scope(Scope::System, 3)
    }

    fn replace_scope_requirement(
        &mut self,
        scope: Scope,
        minimum_level: u8,
    ) -> Result<(), PolicyDenial> {
        self.scopes.retain(|candidate| *candidate != scope);
        self.require_scope(scope, minimum_level)
    }

    fn require_scope(&mut self, scope: Scope, minimum_level: u8) -> Result<(), PolicyDenial> {
        let capability = capability_level(self.agent, scope)?;
        if capability < minimum_level {
            return Err(PolicyDenial::new(
                "CAPABILITY_DENIED",
                format!(
                    "The selected agent lacks the required {} capability.",
                    scope.as_str()
                ),
            ));
        }
        if scope == Scope::Terminal && capability >= 3 {
            return Err(PolicyDenial::new(
                "ADMIN_TERMINAL_BLOCKED",
                "Administrator terminal access is blocked.",
            ));
        }
        if !self.scopes.contains(&scope) {
            self.scopes.push(scope);
        }
        match approval_mode(self.agent, scope) {
            "deny" => Err(PolicyDenial::new(
                "APPROVAL_POLICY_DENIED",
                format!("The agent policy denies {} access.", scope.as_str()),
            )),
            "ask" => {
                self.approval_required = true;
                Ok(())
            }
            "allow" => Ok(()),
            _ => Err(PolicyDenial::new(
                "INVALID_POLICY",
                "The stored agent policy contains an unsupported approval mode.",
            )),
        }
    }

    fn finish(mut self) -> Result<PolicyEvaluation, PolicyDenial> {
        let safety_mode = self.state.preferences.safety_mode.as_str();
        if safety_mode == "locked" && self.scopes.iter().any(|scope| *scope != Scope::Files) {
            return Err(PolicyDenial::new(
                "SAFETY_MODE_LOCKED",
                "Locked safety mode permits read-only workspace inspection only.",
            ));
        }
        if safety_mode == "locked"
            && matches!(self.intent, ActionIntent::RunTask { .. })
            && self.risk_level != "Low"
        {
            return Err(PolicyDenial::new(
                "SAFETY_MODE_LOCKED",
                "Locked safety mode permits read-only workspace inspection only.",
            ));
        }
        if safety_mode == "strict" {
            self.approval_required = true;
        }
        if self.force_approval {
            self.approval_required = true;
        }
        self.scopes.sort_by_key(|scope| scope.as_str());

        let task_snapshot = self.task.map(|task| task.title.clone()).unwrap_or_default();
        let workspace_id = self.workspace.map(|workspace| workspace.id.clone());
        let workspace_fingerprint = match self.workspace {
            Some(workspace) => format!(
                "{WORKSPACE_FINGERPRINT_VERSION}|{}",
                serde_json::to_string(workspace).map_err(|_| {
                    PolicyDenial::new(
                        "INVALID_WORKSPACE",
                        "The selected workspace could not be normalized.",
                    )
                })?
            ),
            None => format!("{WORKSPACE_FINGERPRINT_VERSION}|null"),
        };
        let intent_json = serde_json::to_string(self.intent).map_err(|_| {
            PolicyDenial::new(
                "INVALID_INTENT",
                "The action intent could not be normalized.",
            )
        })?;
        let model = if self.task.is_some() {
            let identity = resolve_model_identity(
                &self.state.models,
                &self.agent.model,
                &self.state.preferences.active_ai_provider,
            )
            .map_err(|error| PolicyDenial::new(error.code.as_str(), error.message))?;
            let model = self
                .state
                .models
                .iter()
                .find(|model| model.id == identity.catalog_model_id)
                .ok_or_else(|| {
                    PolicyDenial::new(
                        "MODEL_NOT_FOUND",
                        "The resolved model identity is no longer registered in backend state.",
                    )
                })?;
            Some(serde_json::json!({
                "catalogModelId": identity.catalog_model_id,
                "name": model.name,
                "catalogProvider": model.provider,
                "runtimeProvider": identity.provider_id,
            }))
        } else {
            None
        };
        let task_policy = self.task.map(|task| {
            serde_json::json!({
                "id": task.id,
                "title": task.title,
                "category": task.category,
                "priority": task.priority,
                "assignedAgentId": task.assigned_agent_id,
                "workspaceId": task.workspace_id,
                "reviewAgentId": task.review_agent_id,
                "reviewStatus": task.review_status,
            })
        });
        let required_scopes = self
            .scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>();
        let policy_context = serde_json::json!({
            "agent": {
                "id": self.agent.id,
                "status": self.agent.status,
                "role": self.agent.role,
                "model": self.agent.model,
                "performance": {
                    "strength": self.agent.performance.strength,
                    "focus": self.agent.performance.focus,
                },
                "capabilities": self.agent.capabilities,
                "approvals": self.agent.approvals,
            },
            "task": task_policy,
            "model": model,
            "preferences": {
                "safetyMode": self.state.preferences.safety_mode,
                "approvalExpiryMinutes": self.state.preferences.approval_expiry_minutes,
                "agentTimeoutMinutes": self.state.preferences.agent_timeout_minutes,
                "activeAiProvider": self.state.preferences.active_ai_provider,
            },
            "effective": {
                "requiredScopes": required_scopes,
                "riskLevel": self.risk_level,
                "forceApproval": self.force_approval,
            },
        });
        let policy_fingerprint = format!(
            "{POLICY_FINGERPRINT_VERSION}|{}",
            serde_json::to_string(&policy_context).map_err(|_| {
                PolicyDenial::new(
                    "INVALID_POLICY",
                    "The current execution policy could not be normalized.",
                )
            })?
        );
        let reason = if self.reason.is_empty() {
            format!(
                "{}-risk action requests {}.",
                self.risk_level,
                self.scopes
                    .iter()
                    .map(|scope| scope.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            self.reason
        };

        Ok(PolicyEvaluation {
            disposition: if self.approval_required {
                PolicyDisposition::ApprovalRequired
            } else {
                PolicyDisposition::Allow
            },
            agent_id: self.agent.id,
            task_id: self.task.map(|task| task.id),
            workspace_id,
            task_snapshot,
            title: self.title,
            reason,
            risk_level: self.risk_level.to_string(),
            scopes: self.scopes,
            intent_kind: self.intent.kind_name().to_string(),
            intent_fingerprint: format!("{INTENT_FINGERPRINT_VERSION}|{intent_json}"),
            policy_fingerprint,
            workspace_fingerprint,
            expires_in_ms: self
                .state
                .preferences
                .approval_expiry_minutes
                .saturating_mul(60_000),
        })
    }
}

fn find_workspace<'a>(
    state: &'a ApplicationState,
    workspace_id: &str,
) -> Result<&'a WorkspaceDefinition, PolicyDenial> {
    let workspace = state
        .preferences
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace_id)
        .ok_or_else(|| {
            PolicyDenial::new(
                "WORKSPACE_NOT_FOUND",
                "The selected workspace does not exist.",
            )
        })?;
    if workspace.path.trim().is_empty() {
        return Err(PolicyDenial::new(
            "WORKSPACE_REQUIRED",
            "The selected workspace has no configured root path.",
        ));
    }
    Ok(workspace)
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn capability_level(agent: &Agent, scope: Scope) -> Result<u8, PolicyDenial> {
    let value = match scope {
        Scope::Files => agent.capabilities.files.as_str(),
        Scope::Internet => agent.capabilities.internet.as_str(),
        Scope::Clipboard => agent.capabilities.clipboard.as_str(),
        Scope::Terminal => agent.capabilities.terminal.as_str(),
        Scope::System => agent.capabilities.system.as_str(),
    };
    let level = match (scope, value) {
        (Scope::Files | Scope::Internet | Scope::Clipboard, "none") => 0,
        (Scope::Files | Scope::Internet | Scope::Clipboard, "read") => 1,
        (Scope::Files | Scope::Internet | Scope::Clipboard, "write") => 2,
        (Scope::Files | Scope::Internet | Scope::Clipboard, "full") => 3,
        (Scope::Terminal, "none") => 0,
        (Scope::Terminal, "safe") => 1,
        (Scope::Terminal, "user") => 2,
        (Scope::Terminal, "admin") => 3,
        (Scope::System, "none") => 0,
        (Scope::System, "notifications") => 1,
        (Scope::System, "power") => 2,
        (Scope::System, "full") => 3,
        _ => {
            return Err(PolicyDenial::new(
                "INVALID_POLICY",
                "The stored agent capability contains an unsupported value.",
            ))
        }
    };
    Ok(level)
}

fn approval_mode(agent: &Agent, scope: Scope) -> &str {
    match scope {
        Scope::Files => &agent.approvals.files,
        Scope::Internet => &agent.approvals.internet,
        Scope::Clipboard => &agent.approvals.clipboard,
        Scope::Terminal => &agent.approvals.terminal,
        Scope::System => &agent.approvals.system,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::default_application_state;

    fn state_with_task() -> ApplicationState {
        let mut state = default_application_state().unwrap();
        state.preferences.workspaces.push(WorkspaceDefinition {
            id: "workspace-1".to_string(),
            name: "Fixture".to_string(),
            path: "/tmp/fixture".to_string(),
        });
        state.preferences.active_workspace_id = Some("workspace-1".to_string());
        state.preferences.workspace_path = "/tmp/fixture".to_string();
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
            workspace_id: Some("workspace-1".to_string()),
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

    fn run_intent() -> ActionIntent {
        ActionIntent::RunTask {
            agent_id: 2,
            task_owner_agent_id: 2,
            task_id: 41,
            run_mode: RunMode::Execute,
        }
    }

    #[test]
    fn run_policy_is_derived_from_backend_state() {
        let evaluation = evaluate_policy(&state_with_task(), &run_intent()).unwrap();
        assert_eq!(evaluation.disposition, PolicyDisposition::ApprovalRequired);
        assert_eq!(evaluation.agent_id, 2);
        assert_eq!(evaluation.task_id, Some(41));
        assert_eq!(evaluation.workspace_id.as_deref(), Some("workspace-1"));
        assert!(evaluation.scopes.contains(&Scope::Files));
        assert!(evaluation.policy_fingerprint.starts_with("policy-v3|"));
        assert!(evaluation.intent_fingerprint.starts_with("intent-v1|"));
    }

    #[test]
    fn task_0006_run_policy_rejects_inactive_and_unsupported_model_providers() {
        let mut inactive = state_with_task();
        inactive.preferences.active_ai_provider = "codex".to_string();
        assert_eq!(
            evaluate_policy(&inactive, &run_intent()).unwrap_err().code,
            "PROVIDER_MODEL_MISMATCH"
        );

        let mut unsupported = state_with_task();
        unsupported.agents[1].model = "claude-sonnet".to_string();
        assert_eq!(
            evaluate_policy(&unsupported, &run_intent())
                .unwrap_err()
                .code,
            "UNSUPPORTED_PROVIDER"
        );
    }

    #[test]
    fn paused_agent_and_wrong_task_subject_are_denied() {
        let mut state = state_with_task();
        state.agents[1].status = "Paused".to_string();
        assert_eq!(
            evaluate_policy(&state, &run_intent()).unwrap_err().code,
            "AGENT_PAUSED"
        );

        state.agents[1].status = "Working".to_string();
        let wrong = ActionIntent::RunTask {
            agent_id: 1,
            task_owner_agent_id: 2,
            task_id: 41,
            run_mode: RunMode::Execute,
        };
        assert_eq!(
            evaluate_policy(&state, &wrong).unwrap_err().code,
            "WRONG_TASK_AGENT"
        );
    }

    #[test]
    fn strict_mode_requires_approval_even_for_preallowed_read() {
        let mut state = state_with_task();
        state.agents[1].tasks[0].title = "Inspect the parser".to_string();
        state.agents[1].tasks[0].category = "General".to_string();
        state.agents[1].approvals.files = "allow".to_string();
        state.preferences.safety_mode = "strict".to_string();
        assert_eq!(
            evaluate_policy(&state, &run_intent()).unwrap().disposition,
            PolicyDisposition::ApprovalRequired
        );
    }

    #[test]
    fn notification_level_system_capability_cannot_control_desktop() {
        let state = state_with_task();
        let intent = ActionIntent::LaunchAllowedApplication {
            agent_id: 7,
            application: "firefox".to_string(),
        };
        assert_eq!(
            evaluate_policy(&state, &intent).unwrap_err().code,
            "AGENT_PAUSED"
        );

        let mut state = state;
        state.agents[6].status = "Working".to_string();
        assert_eq!(
            evaluate_policy(&state, &intent).unwrap_err().code,
            "CAPABILITY_DENIED"
        );
    }

    #[test]
    fn every_privileged_ipc_intent_requires_backend_policy_authority() {
        let mut state = state_with_task();
        state.agents[1].approvals.files = "ask".to_string();
        let pc_agent = state
            .agents
            .iter_mut()
            .find(|agent| agent.name == "PC Control Agent")
            .unwrap();
        pc_agent.status = "Working".to_string();
        pc_agent.capabilities.system = "full".to_string();
        pc_agent.capabilities.internet = "read".to_string();
        pc_agent.capabilities.terminal = "safe".to_string();
        pc_agent.capabilities.clipboard = "full".to_string();
        pc_agent.approvals.system = "allow".to_string();
        pc_agent.approvals.internet = "allow".to_string();
        pc_agent.approvals.terminal = "allow".to_string();
        pc_agent.approvals.clipboard = "allow".to_string();

        let intents = vec![
            run_intent(),
            ActionIntent::OpenWorkspaceItem {
                agent_id: 2,
                workspace_id: "workspace-1".to_string(),
                item_path: ".".to_string(),
            },
            ActionIntent::LaunchAllowedApplication {
                agent_id: 7,
                application: "firefox".to_string(),
            },
            ActionIntent::LaunchDesktopApplication {
                agent_id: 7,
                application: "org.kde.dolphin".to_string(),
            },
            ActionIntent::OpenStandardFolder {
                agent_id: 7,
                folder: "home".to_string(),
            },
            ActionIntent::CloseAllowedApplication {
                agent_id: 7,
                application: "firefox".to_string(),
            },
            ActionIntent::CloseActiveApplication { agent_id: 7 },
            ActionIntent::DesktopKeyboard {
                agent_id: 7,
                action: "copy".to_string(),
            },
            ActionIntent::DesktopWindow {
                agent_id: 7,
                application: "dolphin".to_string(),
                action: "minimize".to_string(),
            },
            ActionIntent::TypeDesktopText {
                agent_id: 7,
                text: "safe text".to_string(),
            },
            ActionIntent::EnableDesktopControl { agent_id: 7 },
            ActionIntent::DesktopPointer {
                agent_id: 7,
                action: "click".to_string(),
            },
            ActionIntent::InstallVoiceRuntime { agent_id: 7 },
            ActionIntent::InstallHighAccuracyVoiceRuntime { agent_id: 7 },
            ActionIntent::StartVoiceListener { agent_id: 7 },
        ];
        for intent in intents {
            assert_eq!(
                evaluate_policy(&state, &intent).unwrap().disposition,
                PolicyDisposition::ApprovalRequired,
                "{} must not trust renderer state",
                intent.kind_name()
            );
        }

        state.agents[6].status = "Paused".to_string();
        assert_eq!(
            evaluate_policy(
                &state,
                &ActionIntent::DesktopPointer {
                    agent_id: 7,
                    action: "click".to_string()
                }
            )
            .unwrap_err()
            .code,
            "AGENT_PAUSED"
        );
    }

    #[test]
    fn malformed_intent_and_unknown_workspace_fail_closed() {
        let state = state_with_task();
        let malformed = ActionIntent::OpenWorkspaceItem {
            agent_id: 2,
            workspace_id: "workspace-1".to_string(),
            item_path: "\n".to_string(),
        };
        assert_eq!(
            evaluate_policy(&state, &malformed).unwrap_err().code,
            "INVALID_INTENT"
        );
        let missing = ActionIntent::OpenWorkspaceItem {
            agent_id: 2,
            workspace_id: "missing".to_string(),
            item_path: "src/main.ts".to_string(),
        };
        assert_eq!(
            evaluate_policy(&state, &missing).unwrap_err().code,
            "WORKSPACE_NOT_FOUND"
        );
    }
}
