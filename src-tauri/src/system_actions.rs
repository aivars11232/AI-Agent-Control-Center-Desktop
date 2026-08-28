use crate::app_state::{ApprovalRequest, MAX_SAFE_INTEGER};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_SYSTEM_ACTION_AUDITS: i64 = 10_000;
pub const MAX_SYSTEM_ACTION_AUDIT_PAGE: i64 = 100;
pub const MAX_VOICE_TASK_TEXT: usize = 4 * 1024;
pub const MAX_VOICE_TYPED_TEXT: usize = 280;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StandardFolder {
    Home,
    Desktop,
    Documents,
    Downloads,
}

impl StandardFolder {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointerAction {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Click,
    DoubleClick,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyboardAction {
    OpenLauncher,
    VolumeUp,
    VolumeDown,
    ToggleMute,
    NextWindow,
    PreviousWindow,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    ShiftTab,
    Enter,
    Escape,
    Backspace,
    Delete,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
}

impl KeyboardAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenLauncher => "openLauncher",
            Self::VolumeUp => "volumeUp",
            Self::VolumeDown => "volumeDown",
            Self::ToggleMute => "toggleMute",
            Self::NextWindow => "nextWindow",
            Self::PreviousWindow => "previousWindow",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "pageUp",
            Self::PageDown => "pageDown",
            Self::Tab => "tab",
            Self::ShiftTab => "shiftTab",
            Self::Enter => "enter",
            Self::Escape => "escape",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::SelectAll => "selectAll",
            Self::Copy => "copy",
            Self::Cut => "cut",
            Self::Paste => "paste",
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }

    pub fn needs_active_window(&self) -> bool {
        !matches!(
            self,
            Self::OpenLauncher
                | Self::VolumeUp
                | Self::VolumeDown
                | Self::ToggleMute
                | Self::NextWindow
                | Self::PreviousWindow
        )
    }

    pub fn is_destructive(&self) -> bool {
        matches!(self, Self::Cut | Self::Delete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowAction {
    Restore,
    Minimize,
    Maximize,
    SnapLeft,
    SnapRight,
}

impl WindowAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Restore => "restore",
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::SnapLeft => "snapLeft",
            Self::SnapRight => "snapRight",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VoiceIntent {
    CreateCodingTask {
        request: String,
    },
    LaunchApplication {
        application: String,
    },
    OpenStandardFolder {
        folder: StandardFolder,
    },
    CloseApplication {
        application: String,
    },
    CloseActiveWindow,
    PointerAction {
        action: PointerAction,
    },
    KeyboardAction {
        action: KeyboardAction,
    },
    ActiveWindowAction {
        action: WindowAction,
    },
    NamedWindowAction {
        application: String,
        action: WindowAction,
    },
    TypeText {
        text: String,
    },
}

impl VoiceIntent {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::CreateCodingTask { .. } => "createCodingTask",
            Self::LaunchApplication { .. } => "launchApplication",
            Self::OpenStandardFolder { .. } => "openStandardFolder",
            Self::CloseApplication { .. } => "closeApplication",
            Self::CloseActiveWindow => "closeActiveWindow",
            Self::PointerAction { .. } => "pointerAction",
            Self::KeyboardAction { .. } => "keyboardAction",
            Self::ActiveWindowAction { .. } => "activeWindowAction",
            Self::NamedWindowAction { .. } => "namedWindowAction",
            Self::TypeText { .. } => "typeText",
        }
    }

    pub fn validate(&self) -> Result<(), SystemActionValidationError> {
        match self {
            Self::CreateCodingTask { request } => {
                validate_user_text("coding request", request, MAX_VOICE_TASK_TEXT, false)
            }
            Self::LaunchApplication { application }
            | Self::CloseApplication { application }
            | Self::NamedWindowAction { application, .. } => {
                validate_user_text("application", application, 256, false)
            }
            Self::TypeText { text } => {
                validate_user_text("dictated text", text, MAX_VOICE_TYPED_TEXT, true)?;
                if !text.chars().all(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(
                            character,
                            ' ' | '\n' | '-' | '.' | '/' | '_' | ':' | ',' | '=' | '+' | '?' | '@'
                        )
                }) {
                    return Err(SystemActionValidationError::new(
                        "INVALID_TYPED_TEXT",
                        "Dictated text contains unsupported characters.",
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn content_digest(&self) -> Option<(String, usize)> {
        match self {
            Self::CreateCodingTask { request } => {
                Some((sha256_hex(request.as_bytes()), request.len()))
            }
            Self::TypeText { text } => Some((sha256_hex(text.as_bytes()), text.len())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitVoiceIntentRequest {
    pub request_id: String,
    pub intent: VoiceIntent,
}

impl SubmitVoiceIntentRequest {
    pub fn validate(&self) -> Result<(), SystemActionValidationError> {
        validate_request_id(&self.request_id)?;
        self.intent.validate()
    }

    pub fn fingerprint(&self) -> Result<String, SystemActionValidationError> {
        let normalized = serde_json::to_vec(self).map_err(|_| {
            SystemActionValidationError::new(
                "INVALID_VOICE_INTENT",
                "The voice intent could not be normalized.",
            )
        })?;
        Ok(format!("voice-intent-v1|{}", sha256_hex(&normalized)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AuthorizedSystemAction {
    LaunchApplication {
        desktop_id: String,
    },
    OpenStandardFolder {
        folder: StandardFolder,
        path_sha256: String,
    },
    CloseWindow {
        window_id: String,
        desktop_id: String,
    },
    Pointer {
        action: PointerAction,
        window_id: String,
    },
    Keyboard {
        action: KeyboardAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_id: Option<String>,
    },
    Window {
        action: WindowAction,
        window_id: String,
        desktop_id: String,
    },
    TypeText {
        window_id: String,
        text_sha256: String,
        text_length: usize,
    },
}

impl AuthorizedSystemAction {
    pub fn validate(&self) -> Result<(), SystemActionValidationError> {
        match self {
            Self::LaunchApplication { desktop_id } => {
                validate_exact_id("desktop entry", desktop_id)
            }
            Self::OpenStandardFolder { path_sha256, .. } => validate_sha256(path_sha256),
            Self::CloseWindow {
                window_id,
                desktop_id,
            }
            | Self::Window {
                window_id,
                desktop_id,
                ..
            } => {
                validate_exact_id("window", window_id)?;
                validate_exact_id("desktop entry", desktop_id)
            }
            Self::Pointer { window_id, .. } => validate_exact_id("window", window_id),
            Self::Keyboard { action, window_id } => {
                if action.needs_active_window() {
                    validate_exact_id(
                        "window",
                        window_id.as_deref().ok_or_else(|| {
                            SystemActionValidationError::new(
                                "WINDOW_TARGET_REQUIRED",
                                "That keyboard action requires an exact active-window target.",
                            )
                        })?,
                    )
                } else if window_id.is_some() {
                    Err(SystemActionValidationError::new(
                        "UNEXPECTED_WINDOW_TARGET",
                        "That global keyboard action must not carry a window target.",
                    ))
                } else {
                    Ok(())
                }
            }
            Self::TypeText {
                window_id,
                text_sha256,
                text_length,
            } => {
                validate_exact_id("window", window_id)?;
                validate_sha256(text_sha256)?;
                if *text_length == 0 || *text_length > MAX_VOICE_TYPED_TEXT {
                    return Err(SystemActionValidationError::new(
                        "INVALID_TYPED_TEXT",
                        "The dictated-text authorization has an invalid length.",
                    ));
                }
                Ok(())
            }
        }
    }

    pub fn risk_class(&self) -> &'static str {
        match self {
            Self::CloseWindow { .. } => "destructive",
            Self::Keyboard { action, .. } if action.is_destructive() => "destructive",
            Self::Pointer {
                action: PointerAction::Click | PointerAction::DoubleClick,
                ..
            }
            | Self::Keyboard { .. }
            | Self::Window { .. }
            | Self::TypeText { .. } => "meaningful",
            Self::LaunchApplication { .. }
            | Self::OpenStandardFolder { .. }
            | Self::Pointer { .. } => "reversible",
        }
    }

    pub fn force_approval(&self) -> bool {
        self.risk_class() == "destructive"
    }

    pub fn target(&self) -> (String, String) {
        match self {
            Self::LaunchApplication { desktop_id } => {
                ("desktopEntry".to_string(), desktop_id.clone())
            }
            Self::OpenStandardFolder {
                folder,
                path_sha256,
            } => (
                "standardFolder".to_string(),
                format!("{}:sha256:{path_sha256}", folder.as_str()),
            ),
            Self::CloseWindow {
                window_id,
                desktop_id,
            }
            | Self::Window {
                window_id,
                desktop_id,
                ..
            } => (
                "kwinWindow".to_string(),
                format!("{window_id}:desktop:{desktop_id}"),
            ),
            Self::Pointer { window_id, .. } | Self::TypeText { window_id, .. } => {
                ("kwinWindow".to_string(), window_id.clone())
            }
            Self::Keyboard { action, window_id } => window_id
                .as_ref()
                .map(|window_id| ("kwinWindow".to_string(), window_id.clone()))
                .unwrap_or_else(|| ("desktopInput".to_string(), action.as_str().to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemActionValidationError {
    pub code: String,
    pub message: String,
}

impl SystemActionValidationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemActionAuditRecord {
    pub id: i64,
    pub request_id: String,
    pub request_fingerprint: String,
    pub intent_kind: String,
    pub risk_class: String,
    pub target_kind: String,
    pub target_id: String,
    pub agent_id: i64,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub approval_id: Option<i64>,
    pub authorization_kind: String,
    pub intent_fingerprint_sha256: String,
    pub policy_fingerprint_sha256: String,
    pub status: String,
    pub detail_code: Option<String>,
    pub detail_message: Option<String>,
    pub content_sha256: Option<String>,
    pub content_length: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemActionAuditPage {
    pub records: Vec<SystemActionAuditRecord>,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditWrite {
    pub request_id: String,
    pub request_fingerprint: String,
    pub intent_kind: String,
    pub risk_class: String,
    pub target_kind: String,
    pub target_id: String,
    pub agent_id: i64,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub approval_id: Option<i64>,
    pub authorization_kind: String,
    pub intent_fingerprint_sha256: String,
    pub policy_fingerprint_sha256: String,
    pub status: String,
    pub detail_code: Option<String>,
    pub detail_message: Option<String>,
    pub content_sha256: Option<String>,
    pub content_length: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceIntentResult {
    pub request_id: String,
    pub status: String,
    pub message: String,
    pub approval: Option<ApprovalRequest>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub audit: SystemActionAuditRecord,
}

pub fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_request_id(value: &str) -> Result<(), SystemActionValidationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        return Err(SystemActionValidationError::new(
            "INVALID_VOICE_REQUEST_ID",
            "The voice request identifier is empty or malformed.",
        ));
    }
    Ok(())
}

fn validate_user_text(
    label: &str,
    value: &str,
    maximum: usize,
    allow_newline: bool,
) -> Result<(), SystemActionValidationError> {
    if value.trim().is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && !(allow_newline && character == '\n'))
    {
        return Err(SystemActionValidationError::new(
            "INVALID_VOICE_INTENT",
            format!("The {label} is empty, too long, or contains control characters."),
        ));
    }
    Ok(())
}

fn validate_exact_id(label: &str, value: &str) -> Result<(), SystemActionValidationError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(SystemActionValidationError::new(
            "INVALID_SYSTEM_ACTION_TARGET",
            format!("The exact {label} target is invalid."),
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), SystemActionValidationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SystemActionValidationError::new(
            "INVALID_CONTENT_DIGEST",
            "The content digest is not a valid SHA-256 value.",
        ));
    }
    Ok(())
}

pub fn validate_audit_write(write: &AuditWrite) -> Result<(), SystemActionValidationError> {
    validate_request_id(&write.request_id)?;
    validate_exact_id("request fingerprint", &write.request_fingerprint)?;
    validate_exact_id("intent kind", &write.intent_kind)?;
    validate_exact_id("risk class", &write.risk_class)?;
    validate_exact_id("target kind", &write.target_kind)?;
    validate_exact_id("target", &write.target_id)?;
    if write.agent_id <= 0 || write.agent_id > MAX_SAFE_INTEGER {
        return Err(SystemActionValidationError::new(
            "INVALID_AGENT",
            "The audit record has an invalid agent identifier.",
        ));
    }
    if let Some(length) = write.content_length {
        if length <= 0 || length > MAX_VOICE_TASK_TEXT as i64 {
            return Err(SystemActionValidationError::new(
                "INVALID_CONTENT_LENGTH",
                "The audit record has an invalid redacted-content length.",
            ));
        }
    }
    if let Some(digest) = &write.content_sha256 {
        validate_sha256(digest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_0015_gateway_fingerprint_binds_content_without_exposing_it() {
        let request = SubmitVoiceIntentRequest {
            request_id: "voice:42".to_string(),
            intent: VoiceIntent::TypeText {
                text: "private typed text".to_string(),
            },
        };
        let fingerprint = request.fingerprint().unwrap();
        let (digest, length) = request.intent.content_digest().unwrap();

        assert!(fingerprint.starts_with("voice-intent-v1|"));
        assert!(!fingerprint.contains("private typed text"));
        assert_eq!(digest.len(), 64);
        assert_eq!(length, 18);
    }

    #[test]
    fn task_0015_destructive_actions_always_force_approval() {
        let close = AuthorizedSystemAction::CloseWindow {
            window_id: "cafe-window".to_string(),
            desktop_id: "org.kde.dolphin.desktop".to_string(),
        };
        let cut = AuthorizedSystemAction::Keyboard {
            action: KeyboardAction::Cut,
            window_id: Some("cafe-window".to_string()),
        };
        let copy = AuthorizedSystemAction::Keyboard {
            action: KeyboardAction::Copy,
            window_id: Some("cafe-window".to_string()),
        };

        assert!(close.force_approval());
        assert_eq!(
            close.target().1,
            "cafe-window:desktop:org.kde.dolphin.desktop"
        );
        assert!(cut.force_approval());
        assert!(!copy.force_approval());
    }

    #[test]
    fn task_0015_typed_text_is_bounded_and_active_target_is_mandatory() {
        assert!(VoiceIntent::TypeText {
            text: "safe text".to_string()
        }
        .validate()
        .is_ok());
        assert!(VoiceIntent::TypeText {
            text: "\u{1b}".to_string()
        }
        .validate()
        .is_err());
        assert!(AuthorizedSystemAction::Keyboard {
            action: KeyboardAction::Paste,
            window_id: None,
        }
        .validate()
        .is_err());

        let folder = AuthorizedSystemAction::OpenStandardFolder {
            folder: StandardFolder::Documents,
            path_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        };
        assert!(folder.validate().is_ok());
        assert_eq!(
            folder.target().1,
            "documents:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }
}
