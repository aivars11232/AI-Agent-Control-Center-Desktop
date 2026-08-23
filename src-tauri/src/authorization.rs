use crate::app_state::ApprovalRequest;
use crate::policy::{ActionIntent, RunMode};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationDecision {
    Allowed,
    ApprovalRequired,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationOutcome {
    pub decision: AuthorizationDecision,
    pub approval: Option<ApprovalRequest>,
}

impl AuthorizationOutcome {
    pub fn allowed() -> Self {
        Self {
            decision: AuthorizationDecision::Allowed,
            approval: None,
        }
    }

    pub fn approval_required(approval: ApprovalRequest) -> Self {
        Self {
            decision: AuthorizationDecision::ApprovalRequired,
            approval: Some(approval),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationGrant {
    pub approval: Option<ApprovalRequest>,
}

impl AuthorizationGrant {
    pub fn policy_allowed() -> Self {
        Self { approval: None }
    }

    pub fn consumed(approval: ApprovalRequest) -> Self {
        Self {
            approval: Some(approval),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalResolution {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveApprovalRequest {
    pub approval_id: i64,
    pub resolution: ApprovalResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalConfirmation {
    pub title: String,
    pub message: String,
}

pub fn build_approval_confirmation(
    approval: &ApprovalRequest,
    intent: &ActionIntent,
) -> ApprovalConfirmation {
    let action = match intent {
        ActionIntent::RunTask {
            task_owner_agent_id,
            task_id,
            run_mode,
            ..
        } => format!(
            "{} task ID {task_id} owned by agent ID {task_owner_agent_id}",
            match run_mode {
                RunMode::Execute => "Execute",
                RunMode::Review => "Review",
            }
        ),
        ActionIntent::OpenWorkspaceItem {
            workspace_id,
            item_path,
            ..
        } => format!(
            "Open item {} in workspace {}",
            dialog_literal(item_path),
            dialog_literal(workspace_id)
        ),
        ActionIntent::LaunchAllowedApplication { application, .. }
        | ActionIntent::LaunchDesktopApplication { application, .. } => {
            format!("Launch application {}", dialog_literal(application))
        }
        ActionIntent::OpenStandardFolder { folder, .. } => {
            format!("Open standard folder {}", dialog_literal(folder))
        }
        ActionIntent::CloseAllowedApplication { application, .. } => {
            format!("Close application {}", dialog_literal(application))
        }
        ActionIntent::CloseActiveApplication { .. } => {
            "Close the active desktop application".to_string()
        }
        ActionIntent::DesktopKeyboard { action, .. } => {
            format!("Send desktop keyboard action {}", dialog_literal(action))
        }
        ActionIntent::DesktopWindow {
            application,
            action,
            ..
        } => format!(
            "Apply window action {} to application {}",
            dialog_literal(action),
            dialog_literal(application)
        ),
        ActionIntent::TypeDesktopText { text, .. } => {
            format!("Type desktop text exactly as {}", dialog_literal(text))
        }
        ActionIntent::EnableDesktopControl { .. } => {
            "Request KDE keyboard and pointer control".to_string()
        }
        ActionIntent::DesktopPointer { action, .. } => {
            format!("Send desktop pointer action {}", dialog_literal(action))
        }
        ActionIntent::InstallVoiceRuntime { .. } => "Install the offline voice runtime".to_string(),
        ActionIntent::InstallHighAccuracyVoiceRuntime { .. } => {
            "Install the high-accuracy offline voice runtime".to_string()
        }
        ActionIntent::StartVoiceListener { .. } => {
            "Start microphone capture for the offline voice listener".to_string()
        }
    };
    let task = approval
        .task_id
        .map(|task_id| format!("{} (ID {task_id})", dialog_literal(&approval.task_snapshot)))
        .unwrap_or_else(|| "None".to_string());
    let workspace = approval
        .workspace_id
        .as_deref()
        .map(dialog_literal)
        .unwrap_or_else(|| "None".to_string());
    let scopes = if approval.scopes.is_empty() {
        "none".to_string()
    } else {
        approval.scopes.join(", ")
    };
    ApprovalConfirmation {
        title: "Confirm one-time authorization".to_string(),
        message: format!(
            "Action: {action}\nAgent ID: {}\nTask: {task}\nWorkspace: {workspace}\nRisk: {}\nScopes: {scopes}\nExpires: {}\n\nApprove exactly one use?",
            approval.agent_id,
            approval.risk_level,
            approval.expires_at,
        ),
    }
}

pub(crate) fn dialog_literal(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"unavailable\"".to_string())
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

pub fn request_native_confirmation(title: &str, message: &str) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("/usr/bin/kdialog")
            .args(["--title", title, "--warningyesno", message])
            .status()
            .map_err(|_| {
                "The trusted desktop confirmation dialog is unavailable. No authorization was granted."
                    .to_string()
            })?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(
                "The trusted desktop confirmation dialog ended unexpectedly. No authorization was granted."
                    .to_string(),
            ),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (title, message);
        Err(
            "Trusted desktop confirmation is not available on this operating system. No authorization was granted."
                .to_string(),
        )
    }
}

pub fn format_unix_ms(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let day_ms = timestamp_ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let hour = day_ms / 3_600_000;
    let minute = (day_ms % 3_600_000) / 60_000;
    let second = (day_ms % 60_000) / 1_000;
    let millisecond = day_ms % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + 719_468;
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
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_milliseconds_use_stable_utc_display_format() {
        assert_eq!(format_unix_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_unix_ms(946_684_800_123), "2000-01-01T00:00:00.123Z");
    }

    #[test]
    fn trusted_confirmation_describes_the_exact_normalized_action() {
        let approval = ApprovalRequest {
            id: 1,
            agent_id: 7,
            task_id: None,
            title: "Type desktop text".to_string(),
            reason: "High-risk action requests system.".to_string(),
            status: "Pending".to_string(),
            created_at: "2026-08-23T10:00:00.000Z".to_string(),
            resolved_at: None,
            risk_level: "High".to_string(),
            scopes: vec!["system".to_string()],
            workspace_id: None,
            task_snapshot: String::new(),
            expires_at: "2026-08-23T10:10:00.000Z".to_string(),
            consumed_at: None,
        };
        let confirmation = build_approval_confirmation(
            &approval,
            &ActionIntent::TypeDesktopText {
                agent_id: 7,
                text: "literal <text>".to_string(),
            },
        );
        assert!(confirmation.message.contains("literal \\u003ctext\\u003e"));
        assert!(confirmation.message.contains("Agent ID: 7"));
        assert!(confirmation.message.contains("Scopes: system"));
    }
}
