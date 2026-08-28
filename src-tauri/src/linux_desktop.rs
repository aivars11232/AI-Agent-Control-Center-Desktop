use crate::linux_paths::LinuxPaths;
use crate::system_actions::{
    sha256_hex, AuthorizedSystemAction, KeyboardAction, PointerAction, StandardFolder, VoiceIntent,
    WindowAction,
};
use ashpd::zbus;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_DESKTOP_ENTRY_BYTES: u64 = 1024 * 1024;
const MAX_DESKTOP_ENTRIES: usize = 20_000;
const MAX_XDG_USER_DIR_CONFIG_BYTES: u64 = 64 * 1024;
const KWIN_RESULT_TIMEOUT: Duration = Duration::from_secs(3);
const KWIN_RESULT_INTERFACE: &str = "org.aiagentcontrolcenter.KWinResult";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxDesktopError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl LinuxDesktopError {
    pub fn new(code: &str, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            recoverable,
        }
    }
}

#[derive(Clone)]
struct KwinResultReceiver {
    token: String,
    expected_sender: String,
    sender: tokio::sync::mpsc::Sender<String>,
}

#[zbus::interface(name = "org.aiagentcontrolcenter.KWinResult", crate = "ashpd::zbus")]
impl KwinResultReceiver {
    fn report(
        &self,
        token: &str,
        result: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> bool {
        header
            .sender()
            .is_some_and(|sender| sender.as_str() == self.expected_sender)
            && token == self.token
            && result.len() <= 2048
            && self.sender.try_send(result.to_string()).is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntryTarget {
    pub desktop_id: String,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KwinWindowTarget {
    pub window_id: String,
    pub desktop_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopExecution {
    LaunchApplication {
        desktop_id: String,
        desktop_file: PathBuf,
    },
    OpenStandardFolder {
        folder: StandardFolder,
        path: PathBuf,
    },
    CloseWindow {
        target: KwinWindowTarget,
    },
    Pointer {
        action: PointerAction,
        window_id: String,
    },
    Keyboard {
        action: KeyboardAction,
        window_id: Option<String>,
    },
    Window {
        target: KwinWindowTarget,
        action: WindowAction,
    },
    TypeText {
        text: String,
        window_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSystemAction {
    pub authorized: AuthorizedSystemAction,
    pub execution: DesktopExecution,
}

pub async fn prepare_system_action(
    intent: &VoiceIntent,
    runtime_directory: &Path,
) -> Result<PreparedSystemAction, LinuxDesktopError> {
    match intent {
        VoiceIntent::CreateCodingTask { .. } => Err(LinuxDesktopError::new(
            "SYSTEM_ACTION_REQUIRED",
            "Coding tasks are prepared by the authoritative task gateway.",
            false,
        )),
        VoiceIntent::LaunchApplication { application } => {
            let entry = resolve_desktop_entry(application)?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::LaunchApplication {
                    desktop_id: entry.desktop_id.clone(),
                },
                execution: DesktopExecution::LaunchApplication {
                    desktop_id: entry.desktop_id,
                    desktop_file: entry.path,
                },
            })
        }
        VoiceIntent::OpenStandardFolder { folder } => {
            let path = resolve_standard_folder(folder)?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::OpenStandardFolder {
                    folder: folder.clone(),
                    path_sha256: path_sha256(&path),
                },
                execution: DesktopExecution::OpenStandardFolder {
                    folder: folder.clone(),
                    path,
                },
            })
        }
        VoiceIntent::CloseApplication { application } => {
            let entry = resolve_desktop_entry(application)?;
            let target = resolve_named_kwin_window(runtime_directory, &entry.desktop_id).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::CloseWindow {
                    window_id: target.window_id.clone(),
                    desktop_id: target.desktop_id.clone(),
                },
                execution: DesktopExecution::CloseWindow { target },
            })
        }
        VoiceIntent::CloseActiveWindow => {
            let target = resolve_active_kwin_window(runtime_directory).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::CloseWindow {
                    window_id: target.window_id.clone(),
                    desktop_id: target.desktop_id.clone(),
                },
                execution: DesktopExecution::CloseWindow { target },
            })
        }
        VoiceIntent::PointerAction { action } => {
            let target = resolve_active_kwin_window(runtime_directory).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::Pointer {
                    action: action.clone(),
                    window_id: target.window_id.clone(),
                },
                execution: DesktopExecution::Pointer {
                    action: action.clone(),
                    window_id: target.window_id,
                },
            })
        }
        VoiceIntent::KeyboardAction { action } => {
            let window_id = if action.needs_active_window() {
                Some(
                    resolve_active_kwin_window(runtime_directory)
                        .await?
                        .window_id,
                )
            } else {
                None
            };
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::Keyboard {
                    action: action.clone(),
                    window_id: window_id.clone(),
                },
                execution: DesktopExecution::Keyboard {
                    action: action.clone(),
                    window_id,
                },
            })
        }
        VoiceIntent::ActiveWindowAction { action } => {
            let target = resolve_active_kwin_window(runtime_directory).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::Window {
                    action: action.clone(),
                    window_id: target.window_id.clone(),
                    desktop_id: target.desktop_id.clone(),
                },
                execution: DesktopExecution::Window {
                    target,
                    action: action.clone(),
                },
            })
        }
        VoiceIntent::NamedWindowAction {
            application,
            action,
        } => {
            let entry = resolve_desktop_entry(application)?;
            let target = resolve_named_kwin_window(runtime_directory, &entry.desktop_id).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::Window {
                    action: action.clone(),
                    window_id: target.window_id.clone(),
                    desktop_id: target.desktop_id.clone(),
                },
                execution: DesktopExecution::Window {
                    target,
                    action: action.clone(),
                },
            })
        }
        VoiceIntent::TypeText { text } => {
            let target = resolve_active_kwin_window(runtime_directory).await?;
            Ok(PreparedSystemAction {
                authorized: AuthorizedSystemAction::TypeText {
                    window_id: target.window_id.clone(),
                    text_sha256: sha256_hex(text.as_bytes()),
                    text_length: text.len(),
                },
                execution: DesktopExecution::TypeText {
                    text: text.clone(),
                    window_id: target.window_id,
                },
            })
        }
    }
}

pub fn execute_xdg_action(execution: &DesktopExecution) -> Result<bool, LinuxDesktopError> {
    match execution {
        DesktopExecution::LaunchApplication {
            desktop_id,
            desktop_file,
        } => {
            let mut command = if let Some(program) = find_in_path("gtk-launch") {
                let mut command = Command::new(program);
                command.arg(desktop_id);
                command
            } else if let Some(program) = find_in_path("gio") {
                let mut command = Command::new(program);
                command.arg("launch").arg(desktop_file);
                command
            } else {
                return Err(LinuxDesktopError::new(
                    "DESKTOP_LAUNCHER_UNAVAILABLE",
                    "Neither gtk-launch nor the GIO desktop-entry launcher is available.",
                    true,
                ));
            };
            command.spawn().map_err(|error| {
                LinuxDesktopError::new(
                    "XDG_ACTION_FAILED",
                    format!("Could not open the exact desktop application: {error}"),
                    true,
                )
            })?;
            Ok(true)
        }
        DesktopExecution::OpenStandardFolder { path, .. } => {
            let program = find_in_path("xdg-open").ok_or_else(|| {
                LinuxDesktopError::new(
                    "XDG_OPEN_UNAVAILABLE",
                    "The XDG opener is unavailable.",
                    true,
                )
            })?;
            let argument = path.to_str().ok_or_else(|| {
                LinuxDesktopError::new(
                    "STANDARD_FOLDER_INVALID",
                    "The configured standard folder cannot be represented safely.",
                    false,
                )
            })?;
            Command::new(program)
                .arg(argument)
                .spawn()
                .map_err(|error| {
                    LinuxDesktopError::new(
                        "XDG_ACTION_FAILED",
                        format!("Could not open the exact standard folder: {error}"),
                        true,
                    )
                })?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub async fn execute_kwin_action(
    runtime_directory: &Path,
    execution: &DesktopExecution,
) -> Result<bool, LinuxDesktopError> {
    let (target, action) = match execution {
        DesktopExecution::CloseWindow { target } => (target, "close"),
        DesktopExecution::Window { target, action } => (target, action.as_str()),
        _ => return Ok(false),
    };
    let script = kwin_action_script(target, action)?;
    let response = run_kwin_script(runtime_directory, "action", &script).await?;
    if response.status != "applied" {
        return Err(response.into_error(
            "KWIN_TARGET_CHANGED",
            "KWin refused the exact window action because the target changed or disappeared.",
        ));
    }
    Ok(true)
}

pub async fn ensure_active_window(
    runtime_directory: &Path,
    expected_window_id: &str,
) -> Result<(), LinuxDesktopError> {
    let current = resolve_active_kwin_window(runtime_directory).await?;
    if current.window_id != expected_window_id {
        return Err(LinuxDesktopError::new(
            "ACTIVE_WINDOW_CHANGED",
            "The active window changed after authorization; no desktop input was sent.",
            true,
        ));
    }
    Ok(())
}

fn resolve_desktop_entry(query: &str) -> Result<DesktopEntryTarget, LinuxDesktopError> {
    resolve_desktop_entry_in_context(
        query,
        &xdg_data_directories()?,
        &DesktopEntryContext::discover(),
    )
}

#[cfg(test)]
fn resolve_desktop_entry_in(
    query: &str,
    data_directories: &[PathBuf],
) -> Result<DesktopEntryTarget, LinuxDesktopError> {
    resolve_desktop_entry_in_context(query, data_directories, &DesktopEntryContext::discover())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopEntryContext {
    locale_candidates: Vec<String>,
    current_desktops: HashSet<String>,
    executable_path: Option<OsString>,
}

impl DesktopEntryContext {
    fn discover() -> Self {
        let locale = env::var("LC_ALL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                env::var("LC_MESSAGES")
                    .ok()
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| env::var("LANG").ok().filter(|value| !value.is_empty()));
        let current_desktops = env::var("XDG_CURRENT_DESKTOP")
            .ok()
            .into_iter()
            .flat_map(|value| {
                value
                    .split(':')
                    .map(str::trim)
                    .filter(|desktop| !desktop.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();
        Self {
            locale_candidates: locale_candidates(locale.as_deref()),
            current_desktops,
            executable_path: env::var_os("PATH"),
        }
    }
}

fn resolve_desktop_entry_in_context(
    query: &str,
    data_directories: &[PathBuf],
    context: &DesktopEntryContext,
) -> Result<DesktopEntryTarget, LinuxDesktopError> {
    let normalized_query = normalized_lookup(query);
    if normalized_query.is_empty() {
        return Err(LinuxDesktopError::new(
            "APPLICATION_TARGET_REQUIRED",
            "Say an exact installed application name or desktop-entry ID.",
            true,
        ));
    }
    let mut by_id = HashMap::<String, DesktopEntryTarget>::new();
    let mut seen_ids = HashSet::<String>::new();
    for data_directory in data_directories {
        let applications = data_directory.join("applications");
        for (desktop_id, path) in desktop_entry_paths(&applications)? {
            let key = desktop_id.to_ascii_lowercase();
            if !seen_ids.insert(key.clone()) {
                continue;
            }
            if let Some(entry) = parse_desktop_entry(&path, desktop_id, context)? {
                by_id.insert(key, entry);
            }
            if seen_ids.len() > MAX_DESKTOP_ENTRIES {
                return Err(LinuxDesktopError::new(
                    "DESKTOP_ENTRY_LIMIT_EXCEEDED",
                    "The XDG application registry exceeds the bounded resolver limit.",
                    false,
                ));
            }
        }
    }
    let query_with_suffix = if normalized_query.ends_with(".desktop") {
        normalized_query.clone()
    } else {
        format!("{normalized_query}.desktop")
    };
    if let Some(entry) = by_id.get(&query_with_suffix) {
        return Ok(entry.clone());
    }
    let mut matches = by_id
        .values()
        .filter(|entry| normalized_lookup(&entry.name) == normalized_query)
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.desktop_id.cmp(&right.desktop_id));
    match matches.len() {
        0 => Err(LinuxDesktopError::new(
            "APPLICATION_NOT_FOUND",
            "No exact installed desktop entry matches that application.",
            true,
        )),
        1 => Ok(matches.remove(0)),
        _ => Err(LinuxDesktopError::new(
            "APPLICATION_TARGET_AMBIGUOUS",
            "More than one installed desktop entry has that exact name; use its desktop-entry ID.",
            true,
        )),
    }
}

fn xdg_data_directories() -> Result<Vec<PathBuf>, LinuxDesktopError> {
    LinuxPaths::discover()
        .map(|paths| paths.data_directories().to_vec())
        .map_err(|message| LinuxDesktopError::new("XDG_PATHS_UNAVAILABLE", message, false))
}

fn desktop_entry_paths(root: &Path) -> Result<Vec<(String, PathBuf)>, LinuxDesktopError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut pending = vec![(root.to_path_buf(), PathBuf::new())];
    let mut result = Vec::new();
    let mut inspected_entries = 0usize;
    while let Some((directory, relative)) = pending.pop() {
        let remaining = MAX_DESKTOP_ENTRIES.saturating_sub(inspected_entries);
        let mut entries = fs::read_dir(&directory)
            .map_err(|error| {
                LinuxDesktopError::new(
                    "DESKTOP_ENTRY_READ_FAILED",
                    format!("Could not read the XDG application registry: {error}"),
                    true,
                )
            })?
            .take(remaining.saturating_add(1))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                LinuxDesktopError::new(
                    "DESKTOP_ENTRY_READ_FAILED",
                    format!("Could not read the XDG application registry: {error}"),
                    true,
                )
            })?;
        if entries.len() > remaining {
            return Err(LinuxDesktopError::new(
                "DESKTOP_ENTRY_LIMIT_EXCEEDED",
                "The XDG application registry exceeds the bounded resolver limit.",
                false,
            ));
        }
        inspected_entries = inspected_entries.saturating_add(entries.len());
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let file_type = entry.file_type().map_err(|error| {
                LinuxDesktopError::new(
                    "DESKTOP_ENTRY_READ_FAILED",
                    format!("Could not inspect the XDG application registry: {error}"),
                    true,
                )
            })?;
            if file_type.is_symlink() {
                continue;
            }
            let relative_path = relative.join(entry.file_name());
            if file_type.is_dir() {
                pending.push((entry.path(), relative_path));
            } else if file_type.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "desktop")
            {
                let desktop_id = relative_path
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("-");
                result.push((desktop_id, entry.path()));
            }
        }
    }
    result.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(result)
}

fn parse_desktop_entry(
    path: &Path,
    desktop_id: String,
    context: &DesktopEntryContext,
) -> Result<Option<DesktopEntryTarget>, LinuxDesktopError> {
    let metadata = fs::metadata(path).map_err(|error| {
        LinuxDesktopError::new(
            "DESKTOP_ENTRY_READ_FAILED",
            format!("Could not inspect an XDG desktop entry: {error}"),
            true,
        )
    })?;
    if metadata.len() > MAX_DESKTOP_ENTRY_BYTES {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|error| {
        LinuxDesktopError::new(
            "DESKTOP_ENTRY_READ_FAILED",
            format!("Could not read an XDG desktop entry: {error}"),
            true,
        )
    })?;
    let mut in_desktop_entry = false;
    let mut values = HashMap::<String, String>::new();
    let mut duplicate_key = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if values
                .insert(key.to_string(), value.trim().to_string())
                .is_some()
            {
                duplicate_key = true;
            }
        }
    }
    if duplicate_key
        || values.get("Type").map(String::as_str) != Some("Application")
        || values.get("Hidden").is_some_and(|value| value == "true")
        || values.get("NoDisplay").is_some_and(|value| value == "true")
        || values.get("OnlyShowIn").is_some_and(|value| {
            !desktop_list(value).any(|desktop| context.current_desktops.contains(desktop))
        })
        || values.get("NotShowIn").is_some_and(|value| {
            desktop_list(value).any(|desktop| context.current_desktops.contains(desktop))
        })
        || values.get("TryExec").is_some_and(|value| {
            !desktop_try_exec_available(value, context.executable_path.as_deref())
        })
    {
        return Ok(None);
    }
    let Some(default_name) = values.get("Name").filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let name = context
        .locale_candidates
        .iter()
        .find_map(|locale| values.get(&format!("Name[{locale}]")))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default_name);
    Ok(Some(DesktopEntryTarget {
        desktop_id,
        name: unescape_desktop_string(name),
        path: path.to_path_buf(),
    }))
}

fn desktop_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
}

fn locale_candidates(locale: Option<&str>) -> Vec<String> {
    let Some(locale) = locale.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    if locale == "C" || locale == "POSIX" {
        return Vec::new();
    }
    let normalized = locale.split_once('.').map_or_else(
        || locale.to_string(),
        |(prefix, suffix)| {
            suffix.split_once('@').map_or_else(
                || prefix.to_string(),
                |(_, modifier)| format!("{prefix}@{modifier}"),
            )
        },
    );
    let without_encoding = normalized.as_str();
    let (base, modifier) = without_encoding
        .split_once('@')
        .map_or((without_encoding, None), |(base, modifier)| {
            (base, Some(modifier))
        });
    let (language, country) = base
        .split_once('_')
        .map_or((base, None), |(language, country)| {
            (language, Some(country))
        });
    let mut candidates = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        candidates.push(format!("{language}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        candidates.push(format!("{language}_{country}"));
    }
    if let Some(modifier) = modifier {
        candidates.push(format!("{language}@{modifier}"));
    }
    candidates.push(language.to_string());
    candidates.dedup();
    candidates
}

fn unescape_desktop_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('s') => result.push(' '),
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

fn desktop_try_exec_available(value: &str, path: Option<&OsStr>) -> bool {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return false;
    }
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        is_executable_file(candidate)
    } else if candidate.components().count() == 1 {
        find_in_path_value(value, path).is_some()
    } else {
        false
    }
}

fn resolve_standard_folder(folder: &StandardFolder) -> Result<PathBuf, LinuxDesktopError> {
    let paths = LinuxPaths::discover()
        .map_err(|message| LinuxDesktopError::new("XDG_PATHS_UNAVAILABLE", message, false))?;
    let home = paths.home().to_path_buf();
    if folder == &StandardFolder::Home {
        return ensure_standard_folder(home, folder);
    }
    let config_path = paths.config_home().join("user-dirs.dirs");
    let metadata = fs::metadata(&config_path).map_err(|_| {
        LinuxDesktopError::new(
            "XDG_USER_DIRS_UNAVAILABLE",
            "The XDG user-directory configuration is unavailable; no guessed folder was opened.",
            true,
        )
    })?;
    if metadata.len() > MAX_XDG_USER_DIR_CONFIG_BYTES {
        return Err(LinuxDesktopError::new(
            "XDG_USER_DIR_INVALID",
            "The XDG user-directory configuration exceeds the bounded input limit.",
            false,
        ));
    }
    let config = fs::read_to_string(config_path).map_err(|_| {
        LinuxDesktopError::new(
            "XDG_USER_DIRS_UNAVAILABLE",
            "The XDG user-directory configuration is unavailable; no guessed folder was opened.",
            true,
        )
    })?;
    let path = parse_xdg_user_directory(&config, &home, folder)?;
    ensure_standard_folder(path, folder)
}

fn parse_xdg_user_directory(
    config: &str,
    home: &Path,
    folder: &StandardFolder,
) -> Result<PathBuf, LinuxDesktopError> {
    let key = match folder {
        StandardFolder::Desktop => "XDG_DESKTOP_DIR",
        StandardFolder::Documents => "XDG_DOCUMENTS_DIR",
        StandardFolder::Downloads => "XDG_DOWNLOAD_DIR",
        StandardFolder::Home => return Ok(home.to_path_buf()),
    };
    let value = config
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            line.split_once('=')
                .and_then(|(candidate, value)| (candidate.trim() == key).then(|| value.trim()))
        })
        .ok_or_else(|| {
            LinuxDesktopError::new(
                "STANDARD_FOLDER_NOT_CONFIGURED",
                format!("The {} folder is not configured by XDG.", folder.as_str()),
                true,
            )
        })?;
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(LinuxDesktopError::new(
            "XDG_USER_DIR_INVALID",
            "The XDG user-directory configuration is malformed.",
            false,
        ));
    }
    let unquoted = &value[1..value.len() - 1];
    if unquoted.contains('$') && !unquoted.starts_with("$HOME") {
        return Err(LinuxDesktopError::new(
            "XDG_USER_DIR_INVALID",
            "The XDG user-directory configuration contains an unsupported variable.",
            false,
        ));
    }
    let path = if unquoted == "$HOME" {
        home.to_path_buf()
    } else if let Some(relative) = unquoted.strip_prefix("$HOME/") {
        if relative.split('/').any(|segment| segment == "..") {
            return Err(LinuxDesktopError::new(
                "XDG_USER_DIR_INVALID",
                "The XDG user-directory configuration contains traversal.",
                false,
            ));
        }
        home.join(relative.replace("\\\"", "\"").replace("\\\\", "\\"))
    } else {
        let candidate = PathBuf::from(unquoted);
        if !candidate.is_absolute() {
            return Err(LinuxDesktopError::new(
                "XDG_USER_DIR_INVALID",
                "The XDG user-directory configuration is not absolute.",
                false,
            ));
        }
        candidate
    };
    Ok(path)
}

fn ensure_standard_folder(
    path: PathBuf,
    folder: &StandardFolder,
) -> Result<PathBuf, LinuxDesktopError> {
    if !path.is_dir() {
        return Err(LinuxDesktopError::new(
            "STANDARD_FOLDER_NOT_FOUND",
            format!("The configured {} folder does not exist.", folder.as_str()),
            true,
        ));
    }
    Ok(path)
}

async fn resolve_named_kwin_window(
    runtime_directory: &Path,
    desktop_id: &str,
) -> Result<KwinWindowTarget, LinuxDesktopError> {
    let script = kwin_resolution_script("named", Some(desktop_id))?;
    let response = run_kwin_script(runtime_directory, "resolve-named", &script).await?;
    response.into_window_target(
        "WINDOW_NOT_FOUND",
        "No unique normal KWin window matches that exact desktop entry.",
    )
}

async fn resolve_active_kwin_window(
    runtime_directory: &Path,
) -> Result<KwinWindowTarget, LinuxDesktopError> {
    let script = kwin_resolution_script("active", None)?;
    let response = run_kwin_script(runtime_directory, "resolve-active", &script).await?;
    response.into_window_target(
        "ACTIVE_WINDOW_NOT_FOUND",
        "KWin has no exact active normal-window target.",
    )
}

fn kwin_resolution_script(
    mode: &str,
    desktop_id: Option<&str>,
) -> Result<String, LinuxDesktopError> {
    let mode = serde_json::to_string(mode).map_err(kwin_script_error)?;
    let desktop_id = serde_json::to_string(&desktop_id).map_err(kwin_script_error)?;
    Ok(format!(
        r#"const mode = {mode};
const requestedDesktopId = {desktop_id};

function normalizedDesktopId(value) {{
  const result = String(value || "").trim().toLowerCase();
  return result.endsWith(".desktop") ? result : result + ".desktop";
}}

function exactTarget(window) {{
  const windowId = String(window.internalId || "");
  const desktopId = String(window.desktopFileName || "").trim();
  return {{
    status: "ok",
    windowId: windowId,
    desktopId: desktopId || "kwin-unidentified"
  }};
}}

let candidates = [];
if (mode === "active") {{
  const active = workspace.activeWindow;
  if (active && !active.deleted && active.normalWindow) candidates = [active];
}} else {{
  const expected = normalizedDesktopId(requestedDesktopId);
  candidates = workspace.stackingOrder.filter((window) =>
    window && !window.deleted && window.normalWindow &&
    normalizedDesktopId(window.desktopFileName) === expected
  );
}}

if (candidates.length === 1 && String(candidates[0].internalId || "")) {{
  aaccReport(exactTarget(candidates[0]));
}} else if (candidates.length > 1) {{
  aaccReport({{status: "ambiguous", code: "WINDOW_TARGET_AMBIGUOUS"}});
}} else {{
  aaccReport({{status: "notFound", code: "WINDOW_NOT_FOUND"}});
}}
"#
    ))
}

fn kwin_action_script(
    target: &KwinWindowTarget,
    action: &str,
) -> Result<String, LinuxDesktopError> {
    let window_id = serde_json::to_string(&target.window_id).map_err(kwin_script_error)?;
    let desktop_id = serde_json::to_string(&target.desktop_id).map_err(kwin_script_error)?;
    let action = serde_json::to_string(action).map_err(kwin_script_error)?;
    Ok(format!(
        r#"const requestedWindowId = {window_id};
const requestedDesktopId = {desktop_id};
const action = {action};

function normalizedDesktopId(value) {{
  const result = String(value || "").trim().toLowerCase();
  return result.endsWith(".desktop") ? result : result + ".desktop";
}}

const matches = workspace.stackingOrder.filter((window) =>
  window && !window.deleted && window.normalWindow &&
  String(window.internalId || "") === requestedWindowId &&
  (requestedDesktopId === "kwin-unidentified" ||
   normalizedDesktopId(window.desktopFileName) === normalizedDesktopId(requestedDesktopId))
);

if (matches.length !== 1) {{
  aaccReport({{status: "targetChanged", code: "KWIN_TARGET_CHANGED"}});
}} else {{
  const selected = matches[0];
  if (action === "close") {{
    if (!selected.closeable) {{
      aaccReport({{status: "refused", code: "WINDOW_NOT_CLOSEABLE"}});
    }} else {{
      selected.closeWindow();
      aaccReport({{status: "applied"}});
    }}
  }} else if (action === "minimize") {{
    selected.minimized = true;
    aaccReport({{status: "applied"}});
  }} else if (action === "maximize") {{
    selected.minimized = false;
    selected.setMaximize(true, true);
    workspace.activeWindow = selected;
    aaccReport({{status: "applied"}});
  }} else if (action === "restore") {{
    selected.minimized = false;
    selected.setMaximize(false, false);
    workspace.activeWindow = selected;
    aaccReport({{status: "applied"}});
  }} else if (action === "snapLeft" || action === "snapRight") {{
    const area = workspace.clientArea(KWin.MaximizeArea, selected);
    selected.setMaximize(false, false);
    selected.frameGeometry = {{
      x: action === "snapLeft" ? area.x : area.x + Math.floor(area.width / 2),
      y: area.y,
      width: Math.floor(area.width / 2),
      height: area.height
    }};
    workspace.activeWindow = selected;
    aaccReport({{status: "applied"}});
  }} else {{
    aaccReport({{status: "refused", code: "WINDOW_ACTION_UNSUPPORTED"}});
  }}
}}
"#
    ))
}

async fn run_kwin_script(
    runtime_directory: &Path,
    purpose: &str,
    script: &str,
) -> Result<KwinScriptResponse, LinuxDesktopError> {
    fs::create_dir_all(runtime_directory).map_err(|error| {
        LinuxDesktopError::new(
            "KWIN_RUNTIME_UNAVAILABLE",
            format!("Could not prepare the private KWin runtime directory: {error}"),
            true,
        )
    })?;
    let runtime_metadata = fs::symlink_metadata(runtime_directory).map_err(|error| {
        LinuxDesktopError::new(
            "KWIN_RUNTIME_UNAVAILABLE",
            format!("Could not inspect the private KWin runtime directory: {error}"),
            false,
        )
    })?;
    if runtime_metadata.file_type().is_symlink() || !runtime_metadata.is_dir() {
        return Err(LinuxDesktopError::new(
            "KWIN_RUNTIME_UNSAFE",
            "The private KWin runtime path is not a real directory.",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(runtime_directory, fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                LinuxDesktopError::new(
                    "KWIN_RUNTIME_PERMISSION_FAILED",
                    format!("Could not protect the private KWin runtime directory: {error}"),
                    false,
                )
            },
        )?;
    }
    let sequence = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            LinuxDesktopError::new(
                "CLOCK_UNAVAILABLE",
                "System time is unavailable for the KWin action token.",
                false,
            )
        })?
        .as_nanos();
    let token = format!("{}-{purpose}-{sequence}", std::process::id());
    let plugin_name = format!("ai-agent-control-center-{token}");
    let script_path = runtime_directory.join(format!("{token}.js"));
    let connection = zbus::Connection::session().await.map_err(kwin_dbus_error)?;
    let bus_proxy = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(kwin_dbus_error)?;
    let kwin_bus_name = zbus::names::BusName::try_from("org.kde.KWin").map_err(kwin_dbus_error)?;
    let expected_sender = bus_proxy
        .get_name_owner(kwin_bus_name)
        .await
        .map_err(kwin_dbus_error)?
        .to_string();
    let service = connection
        .unique_name()
        .map(ToString::to_string)
        .ok_or_else(|| {
            LinuxDesktopError::new(
                "KWIN_RESULT_CHANNEL_UNAVAILABLE",
                "The private KWin result channel has no D-Bus identity.",
                false,
            )
        })?;
    let object_path = format!(
        "/org/aiagentcontrolcenter/KWinResult/{}_{}",
        std::process::id(),
        sequence
    );
    let (result_sender, result_receiver) = tokio::sync::mpsc::channel(1);
    let registered = connection
        .object_server()
        .at(
            object_path.as_str(),
            KwinResultReceiver {
                token: token.clone(),
                expected_sender,
                sender: result_sender,
            },
        )
        .await
        .map_err(kwin_dbus_error)?;
    if !registered {
        return Err(LinuxDesktopError::new(
            "KWIN_RESULT_CHANNEL_CONFLICT",
            "The private KWin result channel is already registered.",
            false,
        ));
    }
    let service = serde_json::to_string(&service).map_err(kwin_script_error)?;
    let object_path_json = serde_json::to_string(&object_path).map_err(kwin_script_error)?;
    let token_json = serde_json::to_string(&token).map_err(kwin_script_error)?;
    let result_interface =
        serde_json::to_string(KWIN_RESULT_INTERFACE).map_err(kwin_script_error)?;
    let callback = format!(
        r#"const aaccResultService = {service};
const aaccResultPath = {object_path_json};
const aaccResultInterface = {result_interface};
const aaccResultToken = {token_json};
function aaccReport(result) {{
  callDBus(aaccResultService, aaccResultPath, aaccResultInterface, "Report", aaccResultToken, JSON.stringify(result));
}}
"#
    );
    let script = format!("{callback}\n{script}");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut script_file = options.open(&script_path).map_err(|error| {
        LinuxDesktopError::new(
            "KWIN_SCRIPT_WRITE_FAILED",
            format!("Could not prepare the bounded KWin action: {error}"),
            true,
        )
    })?;
    if let Err(error) = script_file.write_all(script.as_bytes()) {
        drop(script_file);
        let _ = fs::remove_file(&script_path);
        return Err(LinuxDesktopError::new(
            "KWIN_SCRIPT_WRITE_FAILED",
            format!("Could not write the bounded KWin action: {error}"),
            true,
        ));
    }
    drop(script_file);

    let result = run_kwin_script_inner(
        &connection,
        &script_path,
        &plugin_name,
        result_receiver,
        purpose == "action",
    )
    .await;
    let _ = connection
        .object_server()
        .remove::<KwinResultReceiver, _>(object_path.as_str())
        .await;
    let _ = fs::remove_file(&script_path);
    result
}

async fn run_kwin_script_inner(
    connection: &zbus::Connection,
    script_path: &Path,
    plugin_name: &str,
    mut result_receiver: tokio::sync::mpsc::Receiver<String>,
    dispatched_action: bool,
) -> Result<KwinScriptResponse, LinuxDesktopError> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .await
    .map_err(kwin_dbus_error)?;
    let _: Result<bool, _> = proxy.call("unloadScript", &(plugin_name,)).await;
    let script_path = script_path.to_str().ok_or_else(|| {
        LinuxDesktopError::new(
            "KWIN_SCRIPT_PATH_INVALID",
            "The KWin action path cannot be represented safely.",
            false,
        )
    })?;
    let script_id: i32 = proxy
        .call("loadScript", &(script_path, plugin_name))
        .await
        .map_err(kwin_dbus_error)?;
    if script_id < 0 {
        return Err(LinuxDesktopError::new(
            "KWIN_SCRIPT_LOAD_FAILED",
            "KWin refused to load the bounded action script.",
            true,
        ));
    }
    let script_object_path = format!("/Scripting/Script{script_id}");
    let script_proxy = zbus::Proxy::new(
        connection,
        "org.kde.KWin",
        script_object_path.as_str(),
        "org.kde.kwin.Script",
    )
    .await;
    let run_result = match script_proxy {
        Ok(script_proxy) => script_proxy.call::<_, _, ()>("run", &()).await,
        Err(error) => Err(error),
    };
    if let Err(error) = run_result {
        let _: Result<bool, _> = proxy.call("unloadScript", &(plugin_name,)).await;
        return Err(kwin_dbus_error(error));
    }

    let response = match tokio::time::timeout(KWIN_RESULT_TIMEOUT, result_receiver.recv()).await {
        Ok(Some(json)) => parse_kwin_response(&json),
        Ok(None) => Err(LinuxDesktopError::new(
            "KWIN_RESULT_CHANNEL_ENDED",
            "KWin's private result channel ended without an acknowledgement.",
            false,
        )),
        Err(_) => Err(LinuxDesktopError::new(
            "KWIN_RESULT_TIMEOUT",
            if dispatched_action {
                "KWin did not acknowledge the bounded request; the dispatched action may be uncertain."
            } else {
                "KWin did not return an exact target; no action was dispatched."
            },
            false,
        )),
    };
    let _: Result<bool, _> = proxy.call("unloadScript", &(plugin_name,)).await;
    response
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KwinScriptResponse {
    status: String,
    #[serde(default)]
    window_id: Option<String>,
    #[serde(default)]
    desktop_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

impl KwinScriptResponse {
    fn into_window_target(
        self,
        default_code: &str,
        default_message: &str,
    ) -> Result<KwinWindowTarget, LinuxDesktopError> {
        if self.status != "ok" {
            return Err(self.into_error(default_code, default_message));
        }
        let window_id = self.window_id.filter(|value| !value.trim().is_empty());
        let desktop_id = self.desktop_id.filter(|value| !value.trim().is_empty());
        match (window_id, desktop_id) {
            (Some(window_id), Some(desktop_id)) => Ok(KwinWindowTarget {
                window_id,
                desktop_id,
            }),
            _ => Err(LinuxDesktopError::new(
                "KWIN_RESULT_INVALID",
                "KWin returned an incomplete exact-window result.",
                false,
            )),
        }
    }

    fn into_error(self, default_code: &str, default_message: &str) -> LinuxDesktopError {
        LinuxDesktopError::new(
            self.code.as_deref().unwrap_or(default_code),
            default_message,
            self.status != "ambiguous",
        )
    }
}

fn parse_kwin_response(json: &str) -> Result<KwinScriptResponse, LinuxDesktopError> {
    if json.len() > 2048 {
        return Err(LinuxDesktopError::new(
            "KWIN_RESULT_INVALID",
            "KWin returned an oversized action result.",
            false,
        ));
    }
    serde_json::from_str(json).map_err(|_| {
        LinuxDesktopError::new(
            "KWIN_RESULT_INVALID",
            "KWin returned a malformed action result.",
            false,
        )
    })
}

fn kwin_script_error(error: serde_json::Error) -> LinuxDesktopError {
    LinuxDesktopError::new(
        "KWIN_SCRIPT_INVALID",
        format!("Could not normalize the bounded KWin action: {error}"),
        false,
    )
}

fn kwin_dbus_error(error: impl std::fmt::Display) -> LinuxDesktopError {
    LinuxDesktopError::new(
        "KWIN_DBUS_UNAVAILABLE",
        format!("KWin's scripting interface is unavailable: {error}"),
        true,
    )
}

fn normalized_lookup(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn path_sha256(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        sha256_hex(path.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    sha256_hex(path.to_string_lossy().as_bytes())
}

#[cfg(test)]
fn absolute_environment_path(value: Option<OsString>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    find_in_path_value(binary, env::var_os("PATH").as_deref())
}

fn find_in_path_value(binary: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    path.and_then(|path| {
        env::split_paths(path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(binary))
            .find(|candidate| is_executable_file(candidate))
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        fn new() -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "ai-agent-control-center-task-0015-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(path.join("applications")).unwrap();
            Self(path)
        }

        fn desktop_entry(&self, id: &str, name: &str) {
            self.desktop_entry_content(
                id,
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
            );
        }

        fn desktop_entry_content(&self, id: &str, content: &str) {
            fs::write(self.0.join("applications").join(id), content).unwrap();
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn task_0015_xdg_resolver_uses_exact_id_and_rejects_ambiguous_names() {
        let user = FixtureDirectory::new();
        let system = FixtureDirectory::new();
        user.desktop_entry("org.example.Editor.desktop", "Editor");
        system.desktop_entry("org.other.Editor.desktop", "Editor");
        system.desktop_entry("org.example.Editor.desktop", "Shadowed Editor");

        let exact =
            resolve_desktop_entry_in("org.example.Editor", &[user.0.clone(), system.0.clone()])
                .unwrap();
        assert_eq!(exact.desktop_id, "org.example.Editor.desktop");
        assert_eq!(exact.name, "Editor");
        assert_eq!(
            resolve_desktop_entry_in("Editor", &[user.0.clone(), system.0.clone()])
                .unwrap_err()
                .code,
            "APPLICATION_TARGET_AMBIGUOUS"
        );
        assert_eq!(
            resolve_desktop_entry_in("Edit", &[user.0.clone(), system.0.clone()])
                .unwrap_err()
                .code,
            "APPLICATION_NOT_FOUND"
        );
    }

    #[test]
    fn task_0015_xdg_user_dirs_are_configured_not_guessed() {
        let home = Path::new("/home/fixture");
        let config =
            "XDG_DOCUMENTS_DIR=\"$HOME/My Documents\"\nXDG_DOWNLOAD_DIR=\"/data/downloads\"\n";
        assert_eq!(
            parse_xdg_user_directory(config, home, &StandardFolder::Documents).unwrap(),
            PathBuf::from("/home/fixture/My Documents")
        );
        assert_eq!(
            parse_xdg_user_directory(config, home, &StandardFolder::Downloads).unwrap(),
            PathBuf::from("/data/downloads")
        );
        assert_eq!(
            parse_xdg_user_directory(config, home, &StandardFolder::Desktop)
                .unwrap_err()
                .code,
            "STANDARD_FOLDER_NOT_CONFIGURED"
        );
        assert!(absolute_environment_path(Some(OsString::from("relative/path"))).is_none());
        assert_eq!(
            absolute_environment_path(Some(OsString::from("/absolute/path"))).unwrap(),
            PathBuf::from("/absolute/path")
        );
    }

    #[test]
    fn task_0016_desktop_entries_honor_locale_visibility_and_tombstones() {
        let user = FixtureDirectory::new();
        let system = FixtureDirectory::new();
        user.desktop_entry_content(
            "org.example.Hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden override\nHidden=true\n",
        );
        system.desktop_entry("org.example.Hidden.desktop", "System fallback");
        user.desktop_entry_content(
            "org.example.Localized.desktop",
            "[Desktop Entry]\nType=Application\nName=Editor\nName[nl]=Bewerker\\sLokaal\nOnlyShowIn=KDE;\nExec=/bin/true\n",
        );
        user.desktop_entry_content(
            "org.example.NoDisplay.desktop",
            "[Desktop Entry]\nType=Application\nName=Private app\nNoDisplay=true\n",
        );
        user.desktop_entry_content(
            "org.example.OtherDesktop.desktop",
            "[Desktop Entry]\nType=Application\nName=Other desktop\nOnlyShowIn=GNOME;\n",
        );
        let context = DesktopEntryContext {
            locale_candidates: locale_candidates(Some("nl_NL.UTF-8")),
            current_desktops: HashSet::from(["KDE".to_string()]),
            executable_path: Some(OsString::from("/usr/bin:/bin")),
        };

        let localized = resolve_desktop_entry_in_context(
            "Bewerker Lokaal",
            &[user.0.clone(), system.0.clone()],
            &context,
        )
        .unwrap();
        assert_eq!(localized.desktop_id, "org.example.Localized.desktop");
        assert_eq!(
            localized.path,
            user.0
                .join("applications")
                .join("org.example.Localized.desktop")
        );
        for unavailable in [
            "org.example.Hidden",
            "org.example.NoDisplay",
            "org.example.OtherDesktop",
        ] {
            assert_eq!(
                resolve_desktop_entry_in_context(
                    unavailable,
                    &[user.0.clone(), system.0.clone()],
                    &context,
                )
                .unwrap_err()
                .code,
                "APPLICATION_NOT_FOUND"
            );
        }
    }

    #[test]
    fn task_0016_desktop_entries_enforce_try_exec_and_locale_fallback_order() {
        let user = FixtureDirectory::new();
        let binaries = user.0.join("bin");
        fs::create_dir_all(&binaries).unwrap();
        let executable = binaries.join("available-app");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        user.desktop_entry_content(
            "org.example.Available.desktop",
            "[Desktop Entry]\nType=Application\nName=Default\nName[sr]=Jezik\nName[sr_RS]=Jezik Srbija\nName[sr_RS@latin]=Jezik Latinica\nTryExec=available-app\n",
        );
        user.desktop_entry_content(
            "org.example.Missing.desktop",
            "[Desktop Entry]\nType=Application\nName=Missing\nTryExec=missing-app\n",
        );
        let context = DesktopEntryContext {
            locale_candidates: locale_candidates(Some("sr_RS.UTF-8@latin")),
            current_desktops: HashSet::new(),
            executable_path: Some(binaries.into_os_string()),
        };

        assert_eq!(
            context.locale_candidates,
            ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert_eq!(
            resolve_desktop_entry_in_context(
                "Jezik Latinica",
                std::slice::from_ref(&user.0),
                &context,
            )
            .unwrap()
            .desktop_id,
            "org.example.Available.desktop"
        );
        assert_eq!(
            resolve_desktop_entry_in_context(
                "org.example.Missing",
                std::slice::from_ref(&user.0),
                &context,
            )
            .unwrap_err()
            .code,
            "APPLICATION_NOT_FOUND"
        );
    }

    #[test]
    fn task_0015_kwin_scripts_match_only_exact_ids_and_acknowledge_results() {
        let resolution = kwin_resolution_script("named", Some("org.kde.dolphin.desktop")).unwrap();
        let target = KwinWindowTarget {
            window_id: "8a6f-window".to_string(),
            desktop_id: "org.kde.dolphin.desktop".to_string(),
        };
        let close = kwin_action_script(&target, "close").unwrap();

        assert!(resolution.contains("normalizedDesktopId(window.desktopFileName) === expected"));
        assert!(!resolution.contains("caption"));
        assert!(!resolution.contains("includes("));
        assert!(close.contains("String(window.internalId || \"\") === requestedWindowId"));
        assert!(close.contains("selected.closeWindow()"));
        assert!(!close.contains("pkill"));
        assert!(close.contains("aaccReport({status: \"applied\"})"));
        assert!(!close.contains("print("));

        let adapter = include_str!("linux_desktop.rs");
        assert!(adapter.contains("/Scripting/Script{script_id}"));
        assert!(adapter.contains("org.kde.kwin.Script"));
        assert!(adapter.contains("\"Report\", aaccResultToken"));
        assert!(!adapter.contains("proxy.call(\"start\""));
    }

    #[test]
    fn task_0015_kwin_results_fail_closed_on_unknown_or_extra_fields() {
        assert_eq!(
            parse_kwin_response(
                r#"{"status":"ok","windowId":"exact","desktopId":"org.example.App"}"#,
            )
            .unwrap()
            .into_window_target("WINDOW_NOT_FOUND", "missing")
            .unwrap()
            .window_id,
            "exact"
        );
        assert_eq!(
            parse_kwin_response(r#"{"status":"ok","windowId":"exact","caption":"secret"}"#)
                .unwrap_err()
                .code,
            "KWIN_RESULT_INVALID"
        );
    }
}
