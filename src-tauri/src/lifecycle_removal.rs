//! TASK-0019 — privacy-safe application removal and full data purge.
//!
//! This module is the single source of truth for every on-disk location the
//! application owns on Linux. The removal scripts (`uninstall-kde.sh` and the
//! Arch package helper) call the compiled binary instead of hard-coding paths,
//! so the removal inventory cannot drift from the code that writes the data.
//!
//! Two removal modes are supported:
//!
//! * [`RemovalScope::KeepUserData`] removes the application and its transient or
//!   privacy-sensitive state (caches, logs, voice configuration, the KDE portal
//!   restore token, runtime sockets) while preserving durable user data: the
//!   SQLite database and any downloaded offline-voice models.
//! * [`RemovalScope::Purge`] irreversibly removes every owned location within
//!   the declared scope.
//!
//! Neither mode can revoke the persistent `xdg-desktop-portal` permission grant,
//! which is owned by KDE System Settings; callers surface that separately.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Tauri bundle identifier. Tauri v2 on Linux stores the SQLite database and the
/// WebKitGTK data/cache/cookies under `data_dir()/<identifier>` and (debug
/// builds only) rotating logs under `data_dir()/<identifier>/logs`.
pub const BUNDLE_IDENTIFIER: &str = "com.aivarsrocens.aiagentcontrolcenter";

/// Namespace used by [`crate::linux_paths`] for offline-voice data, voice
/// configuration (which includes the KDE portal restore token), the voice
/// download cache, and the compositor/voice runtime directory.
pub const APPLICATION_NAMESPACE: &str = "ai-agent-control-center";

/// SQLite database file created in the local-data directory.
pub const DATABASE_FILE_NAME: &str = "application-state.sqlite3";

/// Literal token a caller must supply to authorise an irreversible purge.
pub const PURGE_CONFIRMATION_TOKEN: &str = "PURGE";

const DEFAULT_TERM_WAIT: Duration = Duration::from_millis(4_000);
const DEFAULT_KILL_WAIT: Duration = Duration::from_millis(1_500);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Which removal mode to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalScope {
    /// Preserve the durable user data set; remove everything else.
    KeepUserData,
    /// Remove every owned location within the declared scope.
    Purge,
}

impl RemovalScope {
    pub fn label(self) -> &'static str {
        match self {
            RemovalScope::KeepUserData => "keep-user-data",
            RemovalScope::Purge => "purge",
        }
    }
}

/// A category of owned on-disk state. Ordering places child locations before
/// their parents so removal never trips over an already-removed ancestor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCategory {
    /// Debug-build rotating logs: `<local-data>/<identifier>/logs`.
    ApplicationLogs,
    /// SQLite database plus all WebKitGTK data/cache/cookies:
    /// `<local-data>/<identifier>`.
    LocalDataAndWebview,
    /// Tauri per-app config directory: `<config>/<identifier>`.
    ApplicationConfig,
    /// Tauri per-app cache directory: `<cache>/<identifier>`.
    ApplicationCache,
    /// Offline-voice models, virtual environment, and release manifests:
    /// `<data>/ai-agent-control-center`.
    VoiceData,
    /// Voice listener configuration and the KDE portal restore token:
    /// `<config>/ai-agent-control-center`.
    VoiceConfigAndPortalToken,
    /// Offline-voice download cache: `<cache>/ai-agent-control-center`.
    VoiceCache,
    /// Compositor scripts, voice runtime working files, and sockets:
    /// `<runtime>/ai-agent-control-center`.
    RuntimeState,
}

impl DataCategory {
    pub fn key(self) -> &'static str {
        match self {
            DataCategory::ApplicationLogs => "ApplicationLogs",
            DataCategory::LocalDataAndWebview => "LocalDataAndWebview",
            DataCategory::ApplicationConfig => "ApplicationConfig",
            DataCategory::ApplicationCache => "ApplicationCache",
            DataCategory::VoiceData => "VoiceData",
            DataCategory::VoiceConfigAndPortalToken => "VoiceConfigAndPortalToken",
            DataCategory::VoiceCache => "VoiceCache",
            DataCategory::RuntimeState => "RuntimeState",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            DataCategory::ApplicationLogs => "debug application logs",
            DataCategory::LocalDataAndWebview => "SQLite database and WebView data",
            DataCategory::ApplicationConfig => "application configuration directory",
            DataCategory::ApplicationCache => "application cache directory",
            DataCategory::VoiceData => "offline-voice models and environment",
            DataCategory::VoiceConfigAndPortalToken => {
                "voice configuration and KDE portal restore token"
            }
            DataCategory::VoiceCache => "offline-voice download cache",
            DataCategory::RuntimeState => "compositor and voice runtime state",
        }
    }

    /// Whether [`RemovalScope::KeepUserData`] preserves this category.
    pub fn retained_on_keep_user_data(self) -> bool {
        matches!(
            self,
            DataCategory::LocalDataAndWebview | DataCategory::VoiceData
        )
    }
}

/// A single owned filesystem location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedLocation {
    pub category: DataCategory,
    pub path: PathBuf,
}

/// Absolute XDG roots the application derives its locations from. Mirrors the
/// resolution in [`crate::linux_paths`] so the two never disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPaths {
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    runtime_root: Option<PathBuf>,
    locations: Vec<OwnedLocation>,
}

impl RemovalPaths {
    /// Resolve the removal model from the current process environment.
    pub fn discover() -> Result<Self, String> {
        Self::from_values(
            std::env::var_os("HOME"),
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("XDG_CACHE_HOME"),
            std::env::var_os("XDG_RUNTIME_DIR"),
        )
    }

    /// Resolve the removal model from explicit environment values. Relative or
    /// empty XDG overrides fall back to the documented `$HOME`-relative default,
    /// exactly as [`crate::linux_paths`] does.
    pub fn from_values(
        home: Option<OsString>,
        data_home: Option<OsString>,
        config_home: Option<OsString>,
        cache_home: Option<OsString>,
        runtime_dir: Option<OsString>,
    ) -> Result<Self, String> {
        let home = absolute_path(home.as_deref())
            .ok_or_else(|| "The absolute home directory is unavailable.".to_string())?;
        let data_home = absolute_path(data_home.as_deref())
            .unwrap_or_else(|| home.join(".local").join("share"));
        let config_home =
            absolute_path(config_home.as_deref()).unwrap_or_else(|| home.join(".config"));
        let cache_home =
            absolute_path(cache_home.as_deref()).unwrap_or_else(|| home.join(".cache"));
        let runtime_root = absolute_path(runtime_dir.as_deref());

        let mut locations = vec![
            OwnedLocation {
                category: DataCategory::ApplicationLogs,
                path: data_home.join(BUNDLE_IDENTIFIER).join("logs"),
            },
            OwnedLocation {
                category: DataCategory::LocalDataAndWebview,
                path: data_home.join(BUNDLE_IDENTIFIER),
            },
            OwnedLocation {
                category: DataCategory::ApplicationConfig,
                path: config_home.join(BUNDLE_IDENTIFIER),
            },
            OwnedLocation {
                category: DataCategory::ApplicationCache,
                path: cache_home.join(BUNDLE_IDENTIFIER),
            },
            OwnedLocation {
                category: DataCategory::VoiceData,
                path: data_home.join(APPLICATION_NAMESPACE),
            },
            OwnedLocation {
                category: DataCategory::VoiceConfigAndPortalToken,
                path: config_home.join(APPLICATION_NAMESPACE),
            },
            OwnedLocation {
                category: DataCategory::VoiceCache,
                path: cache_home.join(APPLICATION_NAMESPACE),
            },
        ];
        // The runtime directory only exists as a distinct location when
        // `XDG_RUNTIME_DIR` is set; otherwise `linux_paths` nests it inside the
        // voice cache, which is already covered above.
        if let Some(runtime_root) = &runtime_root {
            locations.push(OwnedLocation {
                category: DataCategory::RuntimeState,
                path: runtime_root.join(APPLICATION_NAMESPACE),
            });
        }

        Ok(Self {
            home,
            data_home,
            config_home,
            cache_home,
            runtime_root,
            locations,
        })
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_home
            .join(BUNDLE_IDENTIFIER)
            .join(DATABASE_FILE_NAME)
    }

    pub fn portal_restore_token_file(&self) -> PathBuf {
        self.config_home
            .join(APPLICATION_NAMESPACE)
            .join("voice-runtime")
            .join("desktop-control-restore-token")
    }

    pub fn locations(&self) -> &[OwnedLocation] {
        &self.locations
    }

    fn known_roots(&self) -> Vec<&Path> {
        let mut roots = vec![
            self.data_home.as_path(),
            self.config_home.as_path(),
            self.cache_home.as_path(),
        ];
        if let Some(runtime_root) = &self.runtime_root {
            roots.push(runtime_root.as_path());
        }
        roots
    }

    /// Defence in depth against a hostile `XDG_*` value: a removable path must be
    /// absolute, contain no `..`, sit strictly below a known root, and carry one
    /// of the application namespace components.
    fn is_owned_removable(&self, path: &Path) -> bool {
        if !path.is_absolute() {
            return false;
        }
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return false;
        }
        if path == self.home {
            return false;
        }
        let under_known_root = self
            .known_roots()
            .iter()
            .any(|root| path.starts_with(root) && path != *root);
        if !under_known_root {
            return false;
        }
        path.components().any(|component| {
            matches!(
                component,
                Component::Normal(name)
                    if name == OsStr::new(BUNDLE_IDENTIFIER)
                        || name == OsStr::new(APPLICATION_NAMESPACE)
            )
        })
    }

    /// Stat every owned location.
    pub fn inventory(&self) -> Vec<LocationInventory> {
        self.locations
            .iter()
            .map(|location| {
                let (present, entries, bytes) = measure_path(&location.path);
                LocationInventory {
                    category: location.category,
                    path: location.path.clone(),
                    present,
                    entry_count: entries,
                    byte_size: bytes,
                }
            })
            .collect()
    }

    /// Execute (or, with `dry_run`, only plan) a removal.
    pub fn execute(&self, scope: RemovalScope, dry_run: bool) -> RemovalReport {
        let mut outcomes = Vec::with_capacity(self.locations.len());
        for location in &self.locations {
            let retained = matches!(scope, RemovalScope::KeepUserData)
                && location.category.retained_on_keep_user_data();
            let action = if retained {
                RemovalAction::Retained
            } else if !path_exists(&location.path) {
                RemovalAction::Absent
            } else if !self.is_owned_removable(&location.path) {
                RemovalAction::Failed {
                    reason: "path failed the owned-location safety check".to_string(),
                }
            } else if dry_run {
                RemovalAction::WouldRemove
            } else {
                match remove_path(&location.path) {
                    Ok(()) => RemovalAction::Removed,
                    Err(reason) => RemovalAction::Failed { reason },
                }
            };
            outcomes.push(LocationOutcome {
                category: location.category,
                path: location.path.clone(),
                action,
            });
        }
        let had_failure = outcomes
            .iter()
            .any(|outcome| matches!(outcome.action, RemovalAction::Failed { .. }));
        RemovalReport {
            scope,
            dry_run,
            outcomes,
            had_failure,
        }
    }

    /// Render the inventory as stable, greppable text.
    pub fn inventory_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(
            text,
            "Owned data locations for {BUNDLE_IDENTIFIER} / {APPLICATION_NAMESPACE}:"
        );
        for entry in self.inventory() {
            let disposition = if entry.category.retained_on_keep_user_data() {
                "keep-user-data: retained"
            } else {
                "keep-user-data: removed"
            };
            let status = if entry.present {
                format!(
                    "present  {:>4} entries  {:>10}",
                    entry.entry_count,
                    human_bytes(entry.byte_size)
                )
            } else {
                format!("absent   {:>4} entries  {:>10}", "-", "-")
            };
            let _ = writeln!(
                text,
                "  {:<26} {}  {}  [{}]  ({})",
                entry.category.key(),
                status,
                entry.path.display(),
                disposition,
                entry.category.description()
            );
        }
        text
    }
}

/// Stat result for one owned location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationInventory {
    pub category: DataCategory,
    pub path: PathBuf,
    pub present: bool,
    pub entry_count: usize,
    pub byte_size: u64,
}

/// What happened (or would happen) to one owned location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalAction {
    Removed,
    WouldRemove,
    Retained,
    Absent,
    Failed { reason: String },
}

impl RemovalAction {
    pub fn label(&self) -> &'static str {
        match self {
            RemovalAction::Removed => "removed",
            RemovalAction::WouldRemove => "would remove",
            RemovalAction::Retained => "retained",
            RemovalAction::Absent => "absent",
            RemovalAction::Failed { .. } => "FAILED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationOutcome {
    pub category: DataCategory,
    pub path: PathBuf,
    pub action: RemovalAction,
}

/// The outcome of a whole removal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalReport {
    pub scope: RemovalScope,
    pub dry_run: bool,
    pub outcomes: Vec<LocationOutcome>,
    pub had_failure: bool,
}

impl RemovalReport {
    /// `true` when nothing owned remains on disk after a non-dry run.
    pub fn fully_removed(&self) -> bool {
        !self.dry_run
            && self.outcomes.iter().all(|outcome| {
                matches!(
                    outcome.action,
                    RemovalAction::Removed | RemovalAction::Absent
                )
            })
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        let mode = if self.dry_run { " (dry run)" } else { "" };
        let _ = writeln!(text, "[{}]{}", self.scope.label(), mode);
        for outcome in &self.outcomes {
            match &outcome.action {
                RemovalAction::Failed { reason } => {
                    let _ = writeln!(
                        text,
                        "  {:<12} {:<26} {}  ({reason})",
                        outcome.action.label(),
                        outcome.category.key(),
                        outcome.path.display()
                    );
                }
                other => {
                    let _ = writeln!(
                        text,
                        "  {:<12} {:<26} {}",
                        other.label(),
                        outcome.category.key(),
                        outcome.path.display()
                    );
                }
            }
        }
        text
    }
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn absolute_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("could not inspect the path: {error}")),
    };
    let result = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove the path: {error}")),
    }
}

fn measure_path(path: &Path) -> (bool, usize, u64) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return (false, 0, 0);
    };
    if !metadata.is_dir() {
        return (true, 1, metadata.len());
    }
    let mut entries = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in read_dir.flatten() {
            entries += 1;
            let Ok(entry_metadata) = entry.metadata() else {
                continue;
            };
            if entry_metadata.is_dir() {
                stack.push(entry.path());
            } else {
                bytes = bytes.saturating_add(entry_metadata.len());
            }
        }
    }
    (true, entries, bytes)
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// ---------------------------------------------------------------------------
// Process shutdown
// ---------------------------------------------------------------------------

/// Kind of owned process discovered on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedProcessKind {
    /// The tray/main application process (same executable as this binary).
    MainApplication,
    /// The offline-voice Python listener child (`.../voice-runtime/listener.py`).
    VoiceListener,
}

impl OwnedProcessKind {
    pub fn label(self) -> &'static str {
        match self {
            OwnedProcessKind::MainApplication => "main application",
            OwnedProcessKind::VoiceListener => "voice listener",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStopOutcome {
    /// No such process was running.
    NotRunning,
    /// The process exited after `SIGTERM`.
    Terminated,
    /// The process required `SIGKILL`.
    Killed,
    /// The process could not be stopped.
    Failed,
}

impl ProcessStopOutcome {
    pub fn label(self) -> &'static str {
        match self {
            ProcessStopOutcome::NotRunning => "not running",
            ProcessStopOutcome::Terminated => "terminated",
            ProcessStopOutcome::Killed => "killed",
            ProcessStopOutcome::Failed => "FAILED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedProcess {
    pub pid: i32,
    pub kind: OwnedProcessKind,
    pub outcome: ProcessStopOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStopReport {
    pub supported: bool,
    pub processes: Vec<StoppedProcess>,
}

impl ProcessStopReport {
    pub fn had_failure(&self) -> bool {
        self.processes
            .iter()
            .any(|process| matches!(process.outcome, ProcessStopOutcome::Failed))
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if !self.supported {
            let _ = writeln!(text, "process shutdown: unsupported on this platform");
            return text;
        }
        if self.processes.is_empty() {
            let _ = writeln!(text, "process shutdown: no owned processes were running");
            return text;
        }
        for process in &self.processes {
            let _ = writeln!(
                text,
                "  {:<12} {:<18} pid {}",
                process.outcome.label(),
                process.kind.label(),
                process.pid
            );
        }
        text
    }
}

/// Stop every owned process: the tray/main application first, then any
/// remaining offline-voice listener (which may be orphaned if the application
/// was killed rather than quit).
pub fn stop_owned_processes() -> ProcessStopReport {
    stop_owned_processes_with(DEFAULT_TERM_WAIT, DEFAULT_KILL_WAIT)
}

#[cfg(target_os = "linux")]
fn stop_owned_processes_with(term_wait: Duration, kill_wait: Duration) -> ProcessStopReport {
    use linux_process::{discover_owned_processes, process_group_of, terminate, TerminationTarget};

    let mut processes = Vec::new();
    let self_pid = std::process::id() as i32;
    let self_exe = std::env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok());

    let discovered = discover_owned_processes(self_pid, self_exe.as_deref());

    for candidate in discovered
        .iter()
        .filter(|candidate| candidate.kind == OwnedProcessKind::MainApplication)
    {
        let outcome = terminate(
            TerminationTarget::Pid(candidate.pid),
            term_wait,
            kill_wait,
            PROCESS_POLL_INTERVAL,
        );
        processes.push(StoppedProcess {
            pid: candidate.pid,
            kind: candidate.kind,
            outcome,
        });
    }

    // Re-scan for listeners: the main application may already have stopped its
    // child, or the child may have been orphaned.
    let remaining = discover_owned_processes(self_pid, self_exe.as_deref());
    for candidate in remaining
        .iter()
        .filter(|candidate| candidate.kind == OwnedProcessKind::VoiceListener)
    {
        let target = match process_group_of(candidate.pid) {
            Some(pgid) if pgid > 1 => TerminationTarget::ProcessGroup(pgid),
            _ => TerminationTarget::Pid(candidate.pid),
        };
        let outcome = terminate(target, term_wait, kill_wait, PROCESS_POLL_INTERVAL);
        processes.push(StoppedProcess {
            pid: candidate.pid,
            kind: candidate.kind,
            outcome,
        });
    }

    ProcessStopReport {
        supported: true,
        processes,
    }
}

#[cfg(not(target_os = "linux"))]
fn stop_owned_processes_with(_term_wait: Duration, _kill_wait: Duration) -> ProcessStopReport {
    ProcessStopReport {
        supported: false,
        processes: Vec::new(),
    }
}

#[cfg(target_os = "linux")]
mod linux_process {
    use super::{OwnedProcessKind, ProcessStopOutcome, PROCESS_POLL_INTERVAL};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    pub(super) struct OwnedProcess {
        pub pid: i32,
        pub kind: OwnedProcessKind,
    }

    #[derive(Clone, Copy)]
    pub(super) enum TerminationTarget {
        Pid(i32),
        ProcessGroup(i32),
    }

    /// Parse the process-group id (field 5) from `/proc/<pid>/stat`. The `comm`
    /// field (field 2) is wrapped in parentheses and may itself contain spaces
    /// or parentheses, so fields are read after the final `)`.
    pub(super) fn process_group_of(pid: i32) -> Option<i32> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = stat.rsplit_once(')').map(|(_, rest)| rest.trim())?;
        // rest = "<state> <ppid> <pgrp> ..."
        rest.split_whitespace().nth(2)?.parse().ok()
    }

    /// A process still counts as alive only while it holds a runnable state. A
    /// zombie (`Z`) whose parent has not yet reaped it, or a fully dead entry
    /// (`X`/`x`), is treated as gone.
    fn process_is_alive(pid: i32) -> bool {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        match stat.rsplit_once(')').map(|(_, rest)| rest.trim()) {
            Some(rest) => !matches!(
                rest.as_bytes().first(),
                Some(b'Z') | Some(b'X') | Some(b'x') | None
            ),
            None => false,
        }
    }

    fn signal(target: TerminationTarget, signal: i32) {
        let raw = match target {
            TerminationTarget::Pid(pid) => pid,
            TerminationTarget::ProcessGroup(pgid) => -pgid,
        };
        // SAFETY: `kill` with a plain signal number has no memory effects.
        unsafe {
            libc::kill(raw, signal);
        }
    }

    fn representative_pid(target: TerminationTarget) -> i32 {
        match target {
            TerminationTarget::Pid(pid) => pid,
            TerminationTarget::ProcessGroup(pgid) => pgid,
        }
    }

    pub(super) fn terminate(
        target: TerminationTarget,
        term_wait: Duration,
        kill_wait: Duration,
        poll: Duration,
    ) -> ProcessStopOutcome {
        let pid = representative_pid(target);
        if !process_is_alive(pid) {
            return ProcessStopOutcome::NotRunning;
        }
        signal(target, libc::SIGTERM);
        if wait_until_gone(pid, term_wait, poll) {
            return ProcessStopOutcome::Terminated;
        }
        signal(target, libc::SIGKILL);
        if wait_until_gone(pid, kill_wait, poll) {
            ProcessStopOutcome::Killed
        } else {
            ProcessStopOutcome::Failed
        }
    }

    fn wait_until_gone(pid: i32, budget: Duration, poll: Duration) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            if !process_is_alive(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                return !process_is_alive(pid);
            }
            std::thread::sleep(poll.min(PROCESS_POLL_INTERVAL));
        }
    }

    pub(super) fn discover_owned_processes(
        self_pid: i32,
        self_exe: Option<&Path>,
    ) -> Vec<OwnedProcess> {
        let mut found = Vec::new();
        let Ok(read_dir) = fs::read_dir("/proc") else {
            return found;
        };
        for entry in read_dir.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if pid == self_pid {
                continue;
            }
            if let Some(self_exe) = self_exe {
                if let Ok(link) = fs::read_link(format!("/proc/{pid}/exe")) {
                    let resolved = fs::canonicalize(&link).unwrap_or(link);
                    if resolved == self_exe {
                        found.push(OwnedProcess {
                            pid,
                            kind: OwnedProcessKind::MainApplication,
                        });
                        continue;
                    }
                }
            }
            if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) {
                if cmdline_is_owned_listener(&cmdline) {
                    found.push(OwnedProcess {
                        pid,
                        kind: OwnedProcessKind::VoiceListener,
                    });
                }
            }
        }
        found
    }

    /// The listener is launched as `python <…>/voice-runtime/listener.py <args…>`.
    pub(super) fn cmdline_is_owned_listener(cmdline: &[u8]) -> bool {
        cmdline
            .split(|byte| *byte == 0)
            .filter(|argument| !argument.is_empty())
            .any(|argument| {
                let text = String::from_utf8_lossy(argument);
                text.ends_with("voice-runtime/listener.py")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(target_os = "linux")]
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("aacc-removal-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(paths: &RemovalPaths) {
        for location in paths.locations() {
            fs::create_dir_all(&location.path).unwrap();
            fs::write(location.path.join("seed.bin"), b"payload").unwrap();
        }
        fs::write(paths.database_file(), b"sqlite").unwrap();
        fs::create_dir_all(paths.portal_restore_token_file().parent().unwrap()).unwrap();
        fs::write(paths.portal_restore_token_file(), b"token").unwrap();
    }

    fn model(root: &Path) -> RemovalPaths {
        RemovalPaths::from_values(
            Some(root.join("home").into()),
            Some(root.join("data").into()),
            Some(root.join("config").into()),
            Some(root.join("cache").into()),
            Some(root.join("runtime").into()),
        )
        .unwrap()
    }

    #[test]
    fn task_0019_relative_xdg_values_fall_back_to_home_defaults() {
        let paths = RemovalPaths::from_values(
            Some("/home/example".into()),
            Some("relative".into()),
            Some(OsString::new()),
            None,
            Some("also-relative".into()),
        )
        .unwrap();
        assert_eq!(
            paths.database_file(),
            Path::new(
                "/home/example/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3"
            )
        );
        assert_eq!(
            paths.portal_restore_token_file(),
            Path::new(
                "/home/example/.config/ai-agent-control-center/voice-runtime/desktop-control-restore-token"
            )
        );
        // No absolute XDG_RUNTIME_DIR -> no distinct runtime location.
        assert!(paths
            .locations()
            .iter()
            .all(|location| location.category != DataCategory::RuntimeState));
    }

    #[test]
    fn task_0019_runtime_location_is_present_only_with_absolute_runtime_dir() {
        let root = scratch("runtime");
        let paths = model(&root);
        assert!(paths
            .locations()
            .iter()
            .any(|location| location.category == DataCategory::RuntimeState));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_0019_keep_user_data_retains_database_and_voice_models_only() {
        let root = scratch("keep");
        let paths = model(&root);
        seed(&paths);

        let report = paths.execute(RemovalScope::KeepUserData, false);
        assert!(!report.had_failure, "{}", report.to_text());

        // Retained: database + voice models.
        assert!(paths.database_file().exists());
        assert!(paths
            .locations()
            .iter()
            .find(|location| location.category == DataCategory::VoiceData)
            .map(|location| location.path.exists())
            .unwrap());

        // Removed: caches, logs, voice config (portal token), runtime, app config.
        for category in [
            DataCategory::ApplicationLogs,
            DataCategory::ApplicationConfig,
            DataCategory::ApplicationCache,
            DataCategory::VoiceConfigAndPortalToken,
            DataCategory::VoiceCache,
            DataCategory::RuntimeState,
        ] {
            let location = paths
                .locations()
                .iter()
                .find(|location| location.category == category)
                .unwrap();
            assert!(
                !path_exists(&location.path),
                "{category:?} should have been removed"
            );
        }
        assert!(!path_exists(&paths.portal_restore_token_file()));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_0019_purge_leaves_no_owned_location() {
        let root = scratch("purge");
        let paths = model(&root);
        seed(&paths);

        let report = paths.execute(RemovalScope::Purge, false);
        assert!(!report.had_failure, "{}", report.to_text());
        assert!(report.fully_removed(), "{}", report.to_text());

        for location in paths.locations() {
            assert!(
                !path_exists(&location.path),
                "{:?} still present after purge",
                location.category
            );
        }
        assert!(!paths.database_file().exists());
        assert!(!path_exists(&paths.portal_restore_token_file()));

        // Idempotent: a second purge is clean and reports everything absent.
        let again = paths.execute(RemovalScope::Purge, false);
        assert!(!again.had_failure);
        assert!(again
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome.action, RemovalAction::Absent)));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_0019_dry_run_changes_nothing() {
        let root = scratch("dry");
        let paths = model(&root);
        seed(&paths);

        let report = paths.execute(RemovalScope::Purge, true);
        assert!(report
            .outcomes
            .iter()
            .any(|outcome| matches!(outcome.action, RemovalAction::WouldRemove)));
        for location in paths.locations() {
            assert!(path_exists(&location.path), "dry run must not delete");
        }
        assert!(!report.fully_removed());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_0019_hostile_xdg_root_is_rejected_by_the_safety_check() {
        // XDG_DATA_HOME points at the home directory itself; the derived
        // "<home>/com.aivarsrocens…" is still namespaced, but a crafted bare
        // root is not removable.
        let paths = RemovalPaths::from_values(
            Some("/home/victim".into()),
            Some("/home/victim".into()),
            Some("/home/victim/.config".into()),
            Some("/home/victim/.cache".into()),
            None,
        )
        .unwrap();
        assert!(!paths.is_owned_removable(Path::new("/home/victim")));
        assert!(!paths.is_owned_removable(Path::new("/")));
        assert!(!paths.is_owned_removable(Path::new("/home/victim/Documents")));
        assert!(!paths.is_owned_removable(Path::new(
            "/home/victim/com.aivarsrocens.aiagentcontrolcenter/../.ssh"
        )));
        assert!(paths.is_owned_removable(Path::new(
            "/home/victim/com.aivarsrocens.aiagentcontrolcenter"
        )));
    }

    #[test]
    fn task_0019_inventory_text_is_greppable_and_reports_absence() {
        let root = scratch("inv");
        let paths = model(&root);
        let text = paths.inventory_text();
        assert!(text.contains("LocalDataAndWebview"));
        assert!(text.contains("keep-user-data: retained"));
        assert!(text.contains("keep-user-data: removed"));
        assert!(text.contains("absent"));
        assert!(!text.contains("present"));

        seed(&paths);
        let text = paths.inventory_text();
        assert!(text.contains("present"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn task_0019_process_group_parser_handles_comm_with_spaces_and_parens() {
        // Field 2 (comm) contains spaces and parentheses; pgrp is field 5.
        let sample = "1234 (weird )(name) ) S 1 4242 4242 0 -1 4194560";
        let rest = sample
            .rsplit_once(')')
            .map(|(_, rest)| rest.trim())
            .unwrap();
        let pgrp: i32 = rest.split_whitespace().nth(2).unwrap().parse().unwrap();
        assert_eq!(pgrp, 4242);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn task_0019_listener_cmdline_matcher_is_specific() {
        use linux_process::cmdline_is_owned_listener;
        let owned = b"python\0/home/u/.local/lib/ai-agent-control-center/voice-runtime/listener.py\0model\0";
        assert!(cmdline_is_owned_listener(owned));
        let unrelated = b"python\0/home/u/projects/listener.py\0";
        assert!(!cmdline_is_owned_listener(unrelated));
        let editor = b"vim\0notes.txt\0";
        assert!(!cmdline_is_owned_listener(editor));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn task_0019_terminate_escalates_from_sigterm_to_the_process() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        // Spawn a child and block until it confirms its signal handler is
        // installed, removing the spawn/signal race.
        fn spawn_ready(script: &str) -> std::process::Child {
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "{script}; echo ready; while true; do sleep 1; done"
                ))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(line.trim(), "ready");
            child
        }

        // A child that ignores SIGTERM forces the SIGKILL escalation path.
        let mut child = spawn_ready("trap '' TERM");
        let outcome = linux_process::terminate(
            linux_process::TerminationTarget::Pid(child.id() as i32),
            Duration::from_millis(300),
            Duration::from_millis(2_000),
            Duration::from_millis(50),
        );
        assert_eq!(outcome, ProcessStopOutcome::Killed);
        let _ = child.wait();

        // A cooperative child exits on SIGTERM.
        let mut child = spawn_ready(":");
        let outcome = linux_process::terminate(
            linux_process::TerminationTarget::Pid(child.id() as i32),
            Duration::from_millis(2_000),
            Duration::from_millis(1_000),
            Duration::from_millis(50),
        );
        assert_eq!(outcome, ProcessStopOutcome::Terminated);
        let _ = child.wait();

        // A pid that is not running reports NotRunning.
        let outcome = linux_process::terminate(
            linux_process::TerminationTarget::Pid(999_999),
            Duration::from_millis(50),
            Duration::from_millis(50),
            Duration::from_millis(10),
        );
        assert_eq!(outcome, ProcessStopOutcome::NotRunning);
    }
}
