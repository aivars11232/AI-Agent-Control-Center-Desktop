use crate::linux_paths::LinuxPaths;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const BASE_RELEASE: &str = "base-v1";
pub const HIGH_RELEASE: &str = "high-v1";
pub const VOSK_MODEL: &str = "vosk-model-small-en-us-0.15";
pub const WHISPER_MODEL: &str = "ggml-base.en.bin";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const WHISPER_MODEL_BYTES: u64 = 147_964_211;
const WHISPER_MODEL_SHA256: &str =
    "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Base,
    High,
}

impl InstallKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::High => "high",
        }
    }

    fn release(self) -> &'static str {
        match self {
            Self::Base => BASE_RELEASE,
            Self::High => HIGH_RELEASE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRuntimePaths {
    data_directory: PathBuf,
    config_directory: PathBuf,
    cache_directory: PathBuf,
    runtime_directory: PathBuf,
}

impl VoiceRuntimePaths {
    pub fn discover() -> Result<Self, String> {
        let paths = LinuxPaths::discover()?;
        Ok(Self {
            data_directory: paths.voice_data_directory(),
            config_directory: paths.voice_config_directory(),
            cache_directory: paths.voice_cache_directory(),
            runtime_directory: paths.voice_runtime_directory(),
        })
    }

    #[cfg(test)]
    fn fixture(root: &Path) -> Self {
        Self {
            data_directory: root.join("data"),
            config_directory: root.join("config"),
            cache_directory: root.join("cache"),
            runtime_directory: root.join("runtime"),
        }
    }

    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    pub fn cache_directory(&self) -> &Path {
        &self.cache_directory
    }

    pub fn runtime_directory(&self) -> &Path {
        &self.runtime_directory
    }

    pub fn base_release_directory(&self) -> PathBuf {
        self.data_directory.join(BASE_RELEASE)
    }

    pub fn high_release_directory(&self) -> PathBuf {
        self.data_directory.join(HIGH_RELEASE)
    }

    pub fn release_directory(&self, kind: InstallKind) -> PathBuf {
        self.data_directory.join(kind.release())
    }

    pub fn stage_directory(&self, kind: InstallKind, operation_id: &str) -> PathBuf {
        self.data_directory
            .join(format!(".{}.{}.staging", kind.release(), operation_id))
    }

    fn previous_directory(&self, kind: InstallKind, operation_id: &str) -> PathBuf {
        self.data_directory
            .join(format!(".{}.{}.previous", kind.release(), operation_id))
    }

    pub fn python(&self) -> PathBuf {
        self.base_release_directory()
            .join("venv")
            .join("bin")
            .join("python")
    }

    pub fn vosk_model(&self) -> PathBuf {
        self.base_release_directory()
            .join("models")
            .join(VOSK_MODEL)
    }

    pub fn whisper_binary(&self) -> PathBuf {
        self.high_release_directory().join("whisper-cli")
    }

    pub fn whisper_model(&self) -> PathBuf {
        self.high_release_directory().join(WHISPER_MODEL)
    }

    pub fn listener_config(&self) -> PathBuf {
        self.config_directory.join("listener-config.json")
    }

    pub fn desktop_control_token(&self) -> PathBuf {
        self.config_directory.join("desktop-control-restore-token")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BaseManifest {
    schema_version: u32,
    kind: String,
    release: String,
    python: String,
    architecture: String,
    vosk_version: String,
    vosk_wheel_sha256: String,
    model: String,
    model_archive_bytes: u64,
    model_archive_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HighManifest {
    schema_version: u32,
    kind: String,
    release: String,
    whisper_version: String,
    whisper_commit: String,
    source_archive_sha256: String,
    model: String,
    model_bytes: u64,
    model_sha256: String,
}

fn read_manifest<T: for<'de> Deserialize<'de>>(directory: &Path) -> Option<T> {
    let path = directory.join("install-manifest.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn real_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn executable_file(path: &Path) -> bool {
    if !real_file(path) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

pub fn base_release_ready(directory: &Path) -> bool {
    let Some(manifest) = read_manifest::<BaseManifest>(directory) else {
        return false;
    };
    manifest.schema_version == 1
        && manifest.kind == "base"
        && manifest.release == BASE_RELEASE
        && manifest.python == "3.14"
        && manifest.architecture == "x86_64"
        && manifest.vosk_version == "0.3.45"
        && manifest.vosk_wheel_sha256
            == "25e025093c4399d7278f543568ed8cc5460ac3a4bf48c23673ace1e25d26619f"
        && manifest.model == VOSK_MODEL
        && manifest.model_archive_bytes == 41_205_931
        && manifest.model_archive_sha256
            == "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498"
        && executable_file(&directory.join("venv").join("bin").join("python"))
        && real_directory(&directory.join("models").join(VOSK_MODEL))
}

pub fn high_release_ready(directory: &Path, verify_model_hash: bool) -> bool {
    let Some(manifest) = read_manifest::<HighManifest>(directory) else {
        return false;
    };
    let binary = directory.join("whisper-cli");
    let model = directory.join(WHISPER_MODEL);
    if manifest.schema_version != 1
        || manifest.kind != "high"
        || manifest.release != HIGH_RELEASE
        || manifest.whisper_version != "1.9.1"
        || manifest.whisper_commit != "f049fff95a089aa9969deb009cdd4892b3e74916"
        || manifest.source_archive_sha256
            != "279af4ce60dbf397362868f3bacc75b56a4332ac2541cae155070093f6aaf0e3"
        || manifest.model != WHISPER_MODEL
        || manifest.model_bytes != WHISPER_MODEL_BYTES
        || manifest.model_sha256 != WHISPER_MODEL_SHA256
        || !executable_file(&binary)
        || !real_file(&model)
        || fs::metadata(&model).map_or(true, |metadata| metadata.len() != WHISPER_MODEL_BYTES)
    {
        return false;
    }
    !verify_model_hash || sha256_file(&model).is_ok_and(|digest| digest == WHISPER_MODEL_SHA256)
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn prepare_install(
    paths: &VoiceRuntimePaths,
    kind: InstallKind,
    operation_id: &str,
) -> Result<PathBuf, String> {
    validate_operation_id(operation_id)?;
    ensure_private_directory(paths.data_directory())?;
    ensure_private_directory(paths.cache_directory())?;
    let stage = paths.stage_directory(kind, operation_id);
    if fs::symlink_metadata(&stage).is_ok() {
        return Err(
            "VOICE_INSTALL_STAGE_EXISTS: The private staging directory already exists.".to_string(),
        );
    }
    fs::create_dir(&stage).map_err(|_| {
        "VOICE_INSTALL_STAGE_FAILED: Could not create the staging directory.".to_string()
    })?;
    protect_directory(&stage)?;
    Ok(stage)
}

pub fn promote_install(
    paths: &VoiceRuntimePaths,
    kind: InstallKind,
    operation_id: &str,
) -> Result<(), String> {
    validate_operation_id(operation_id)?;
    let stage = paths.stage_directory(kind, operation_id);
    let valid = match kind {
        InstallKind::Base => base_release_ready(&stage),
        InstallKind::High => high_release_ready(&stage, true),
    };
    if !valid {
        return Err(
            "VOICE_INSTALL_VALIDATION_FAILED: The staged runtime did not match its pinned manifest."
                .to_string(),
        );
    }
    let destination = paths.release_directory(kind);
    let previous = paths.previous_directory(kind, operation_id);
    remove_managed_directory(&previous, paths.data_directory())?;
    let had_previous = fs::symlink_metadata(&destination).is_ok();
    if had_previous {
        fs::rename(&destination, &previous).map_err(|_| {
            "VOICE_INSTALL_PROMOTION_FAILED: Could not preserve the previous runtime.".to_string()
        })?;
    }
    if fs::rename(&stage, &destination).is_err() {
        if had_previous {
            let _ = fs::rename(&previous, &destination);
        }
        return Err(
            "VOICE_INSTALL_PROMOTION_FAILED: Could not activate the staged runtime.".to_string(),
        );
    }
    remove_managed_directory(&previous, paths.data_directory())?;
    Ok(())
}

pub fn cleanup_stage(
    paths: &VoiceRuntimePaths,
    kind: InstallKind,
    operation_id: &str,
) -> Result<(), String> {
    validate_operation_id(operation_id)?;
    remove_managed_directory(
        &paths.stage_directory(kind, operation_id),
        paths.data_directory(),
    )
}

fn validate_operation_id(operation_id: &str) -> Result<(), String> {
    if operation_id.is_empty()
        || operation_id.len() > 96
        || !operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(
            "VOICE_INSTALL_OPERATION_INVALID: The install operation ID is invalid.".to_string(),
        );
    }
    Ok(())
}

fn remove_managed_directory(path: &Path, parent: &Path) -> Result<(), String> {
    if path.parent() != Some(parent) {
        return Err(
            "VOICE_INSTALL_PATH_INVALID: A managed runtime path escaped its private root."
                .to_string(),
        );
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|_| {
        "VOICE_INSTALL_CLEANUP_FAILED: Could not remove a private staging directory.".to_string()
    })
}

pub fn ensure_private_directory(path: &Path) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(
                "PRIVATE_DIRECTORY_UNSAFE: A private runtime path is not a real directory."
                    .to_string(),
            );
        }
    } else {
        fs::create_dir_all(path).map_err(|_| {
            "PRIVATE_DIRECTORY_UNAVAILABLE: Could not create a private runtime directory."
                .to_string()
        })?;
    }
    protect_directory(path)
}

fn protect_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
            "PRIVATE_DIRECTORY_PERMISSION_FAILED: Could not protect a private runtime directory."
                .to_string()
        })?;
    }
    Ok(())
}

pub fn write_private_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        "PRIVATE_FILE_PATH_INVALID: A private file has no parent directory.".to_string()
    })?;
    ensure_private_directory(parent)?;
    let sequence = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".private-{}-{sequence}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| {
        "PRIVATE_FILE_WRITE_FAILED: Could not create a private temporary file.".to_string()
    })?;
    let result = file
        .write_all(contents)
        .and_then(|_| file.sync_all())
        .and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(
            "PRIVATE_FILE_WRITE_FAILED: Could not atomically save a private file.".to_string(),
        );
    }
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| {
            "PRIVATE_FILE_SYNC_FAILED: Could not durably activate a private file.".to_string()
        })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub install_state: String,
    pub listener_state: String,
    pub operation_id: Option<String>,
    pub can_cancel: bool,
    pub message: String,
}

#[derive(Clone)]
struct ActiveInstall {
    operation_id: String,
    cancel: Arc<AtomicBool>,
    process_group: Option<i32>,
}

struct RuntimeState {
    install_state: String,
    listener_state: String,
    active_install: Option<ActiveInstall>,
    message: String,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            install_state: "missing".to_string(),
            listener_state: "stopped".to_string(),
            active_install: None,
            message: "Offline voice is not installed.".to_string(),
        }
    }
}

#[derive(Clone, Default)]
pub struct VoiceRuntime {
    state: Arc<Mutex<RuntimeState>>,
    listener: Arc<Mutex<Option<Child>>>,
    listener_starting: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct InstallReservation {
    pub operation_id: String,
    pub cancel: Arc<AtomicBool>,
}

impl VoiceRuntime {
    pub fn snapshot(&self) -> Result<RuntimeSnapshot, String> {
        let state = self.state.lock().map_err(|_| {
            "VOICE_RUNTIME_UNAVAILABLE: The runtime registry is unavailable.".to_string()
        })?;
        Ok(RuntimeSnapshot {
            install_state: state.install_state.clone(),
            listener_state: state.listener_state.clone(),
            operation_id: state
                .active_install
                .as_ref()
                .map(|install| install.operation_id.clone()),
            can_cancel: state.active_install.is_some() && state.install_state == "installing",
            message: state.message.clone(),
        })
    }

    pub fn begin_install(&self, kind: InstallKind) -> Result<InstallReservation, String> {
        let mut state = self.state.lock().map_err(|_| {
            "VOICE_RUNTIME_UNAVAILABLE: The runtime registry is unavailable.".to_string()
        })?;
        if state.active_install.is_some() {
            return Err(
                "VOICE_INSTALL_BUSY: Another voice runtime installation is already active."
                    .to_string(),
            );
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let sequence = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
        let operation_id = format!(
            "install-{}-{}-{timestamp}-{sequence}",
            kind.as_str(),
            std::process::id()
        );
        let cancel = Arc::new(AtomicBool::new(false));
        state.install_state = "installing".to_string();
        state.message = match kind {
            InstallKind::Base => "Installing the pinned offline voice runtime…",
            InstallKind::High => "Installing the optional high-accuracy runtime…",
        }
        .to_string();
        state.active_install = Some(ActiveInstall {
            operation_id: operation_id.clone(),
            cancel: cancel.clone(),
            process_group: None,
        });
        Ok(InstallReservation {
            operation_id,
            cancel,
        })
    }

    pub fn attach_install_process(
        &self,
        operation_id: &str,
        process_group: i32,
    ) -> Result<(), String> {
        let cancelled = {
            let mut state = self.state.lock().map_err(|_| {
                "VOICE_RUNTIME_UNAVAILABLE: The runtime registry is unavailable.".to_string()
            })?;
            let install = state.active_install.as_mut().ok_or_else(|| {
                "VOICE_INSTALL_NOT_ACTIVE: No voice runtime installation is active.".to_string()
            })?;
            if install.operation_id != operation_id {
                return Err(
                    "VOICE_INSTALL_OPERATION_MISMATCH: The active install operation changed."
                        .to_string(),
                );
            }
            install.process_group = Some(process_group);
            install.cancel.load(Ordering::Acquire)
        };
        if cancelled {
            signal_process_group(process_group, false);
        }
        Ok(())
    }

    pub fn cancel_install(&self, operation_id: &str) -> Result<RuntimeSnapshot, String> {
        let process_group = {
            let mut state = self.state.lock().map_err(|_| {
                "VOICE_RUNTIME_UNAVAILABLE: The runtime registry is unavailable.".to_string()
            })?;
            let install = state.active_install.as_mut().ok_or_else(|| {
                "VOICE_INSTALL_NOT_ACTIVE: No voice runtime installation is active.".to_string()
            })?;
            if install.operation_id != operation_id {
                return Err(
                    "VOICE_INSTALL_OPERATION_MISMATCH: The cancel request is stale.".to_string(),
                );
            }
            install.cancel.store(true, Ordering::Release);
            let process_group = install.process_group;
            state.install_state = "cancelling".to_string();
            state.message = "Cancelling the voice runtime installation…".to_string();
            process_group
        };
        if let Some(process_group) = process_group {
            signal_process_group(process_group, false);
            let state = self.state.clone();
            let operation_id = operation_id.to_string();
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(2));
                let still_cancelling = state.lock().is_ok_and(|state| {
                    state.active_install.as_ref().is_some_and(|install| {
                        install.operation_id == operation_id
                            && install.process_group == Some(process_group)
                            && install.cancel.load(Ordering::Acquire)
                    })
                });
                if still_cancelling {
                    signal_process_group(process_group, true);
                }
            });
        }
        self.snapshot()
    }

    pub fn finish_install(
        &self,
        operation_id: &str,
        install_state: &str,
        message: impl Into<String>,
    ) {
        if let Ok(mut state) = self.state.lock() {
            if state
                .active_install
                .as_ref()
                .is_some_and(|install| install.operation_id == operation_id)
            {
                state.active_install = None;
                state.install_state = install_state.to_string();
                state.message = message.into();
            }
        }
    }

    pub fn install_cancelled(&self, operation_id: &str) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.active_install.as_ref().is_some_and(|install| {
                install.operation_id == operation_id && install.cancel.load(Ordering::Acquire)
            })
        })
    }

    pub fn listener_is_running(&self) -> Result<bool, String> {
        let exited = {
            let mut listener = self.listener.lock().map_err(|_| {
                "VOICE_RUNTIME_UNAVAILABLE: The listener registry is unavailable.".to_string()
            })?;
            let Some(child) = listener.as_mut() else {
                return Ok(false);
            };
            match child.try_wait().map_err(|_| {
                "VOICE_LISTENER_INSPECTION_FAILED: Could not inspect the voice listener."
                    .to_string()
            })? {
                Some(_) => {
                    *listener = None;
                    true
                }
                None => false,
            }
        };
        if exited {
            let failed = self
                .state
                .lock()
                .is_ok_and(|state| state.listener_state == "failed");
            if !failed {
                self.set_listener_state("stopped", "Offline voice listener stopped.");
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }

    pub fn begin_listener_start(&self) -> Result<(), String> {
        if self
            .listener_starting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(
                "VOICE_LISTENER_BUSY: The offline listener is already starting.".to_string(),
            );
        }
        let occupied = self
            .listener
            .lock()
            .map(|listener| listener.is_some())
            .map_err(|_| {
                self.listener_starting.store(false, Ordering::Release);
                "VOICE_RUNTIME_UNAVAILABLE: The listener registry is unavailable.".to_string()
            })?;
        if occupied {
            self.listener_starting.store(false, Ordering::Release);
            return Err(
                "VOICE_LISTENER_BUSY: The offline listener is already running.".to_string(),
            );
        }
        self.set_listener_state("starting", "Starting the offline voice listener…");
        Ok(())
    }

    pub fn cancel_listener_start(&self, listener_state: &str, message: impl Into<String>) {
        self.listener_starting.store(false, Ordering::Release);
        self.set_listener_state(listener_state, message);
    }

    pub fn store_listener(&self, mut child: Child) -> Result<(), String> {
        if !self.listener_starting.swap(false, Ordering::AcqRel) {
            terminate_child_group(&mut child);
            return Err(
                "VOICE_LISTENER_START_CANCELLED: The listener start is no longer current."
                    .to_string(),
            );
        }
        let mut listener = match self.listener.lock() {
            Ok(listener) => listener,
            Err(_) => {
                terminate_child_group(&mut child);
                return Err(
                    "VOICE_RUNTIME_UNAVAILABLE: The listener registry is unavailable.".to_string(),
                );
            }
        };
        if listener.is_some() {
            drop(listener);
            terminate_child_group(&mut child);
            return Err(
                "VOICE_LISTENER_BUSY: The offline listener is already running.".to_string(),
            );
        }
        *listener = Some(child);
        Ok(())
    }

    pub fn set_listener_state(&self, listener_state: &str, message: impl Into<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.listener_state = listener_state.to_string();
            state.message = message.into();
        }
    }

    pub fn stop_listener(&self) {
        self.listener_starting.store(false, Ordering::Release);
        let child = self
            .listener
            .lock()
            .ok()
            .and_then(|mut listener| listener.take());
        if let Some(mut child) = child {
            terminate_child_group(&mut child);
        }
        self.set_listener_state("stopped", "Offline voice listener stopped.");
    }

    pub fn reap_listener(&self) {
        let child = self
            .listener
            .lock()
            .ok()
            .and_then(|mut listener| listener.take());
        if let Some(mut child) = child {
            let _ = child.wait();
        }
        let failed = self
            .state
            .lock()
            .is_ok_and(|state| state.listener_state == "failed");
        if !failed {
            self.set_listener_state("stopped", "Offline voice listener stopped.");
        }
    }
}

fn terminate_child_group(child: &mut Child) {
    let process_group = child.id() as i32;
    signal_process_group(process_group, false);
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    signal_process_group(process_group, true);
    let _ = child.wait();
}

fn signal_process_group(process_group: i32, force: bool) {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::kill(
            -process_group,
            if force { libc::SIGKILL } else { libc::SIGTERM },
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (process_group, force);
    }
}

pub fn drain_bounded<R: Read>(mut reader: R, limit: usize) -> io::Result<Vec<u8>> {
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0u8; 4 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        if remaining > 0 {
            retained.extend_from_slice(&buffer[..count.min(remaining)]);
        }
    }
    Ok(retained)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ai-agent-control-center-task-0016-voice-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn valid_base_release(directory: &Path) {
        let python = directory.join("venv").join("bin").join("python");
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&python, b"python").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&python, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir_all(directory.join("models").join(VOSK_MODEL)).unwrap();
        fs::write(
            directory.join("install-manifest.json"),
            br#"{"schemaVersion":1,"kind":"base","release":"base-v1","python":"3.14","architecture":"x86_64","voskVersion":"0.3.45","voskWheelSha256":"25e025093c4399d7278f543568ed8cc5460ac3a4bf48c23673ace1e25d26619f","model":"vosk-model-small-en-us-0.15","modelArchiveBytes":41205931,"modelArchiveSha256":"30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498"}"#,
        )
        .unwrap();
    }

    #[test]
    fn task_0016_base_manifest_is_exact_and_fail_closed() {
        let fixture = Fixture::new();
        valid_base_release(&fixture.0);
        assert!(base_release_ready(&fixture.0));
        let manifest = fixture.0.join("install-manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["unexpected"] = serde_json::Value::Bool(true);
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!base_release_ready(&fixture.0));

        valid_base_release(&fixture.0);
        fs::remove_dir_all(fixture.0.join("models").join(VOSK_MODEL)).unwrap();
        assert!(!base_release_ready(&fixture.0));
    }

    #[test]
    fn task_0016_promotion_replaces_only_the_managed_release() {
        let fixture = Fixture::new();
        let paths = VoiceRuntimePaths::fixture(&fixture.0);
        ensure_private_directory(paths.data_directory()).unwrap();
        let operation_id = "install-base-fixture-1";
        let stage = paths.stage_directory(InstallKind::Base, operation_id);
        valid_base_release(&stage);
        fs::write(stage.join("marker"), b"new").unwrap();
        let old = paths.base_release_directory();
        valid_base_release(&old);
        fs::write(old.join("marker"), b"old").unwrap();

        promote_install(&paths, InstallKind::Base, operation_id).unwrap();

        assert_eq!(fs::read(old.join("marker")).unwrap(), b"new");
        assert!(!stage.exists());
        assert!(!paths
            .previous_directory(InstallKind::Base, operation_id)
            .exists());

        let invalid_operation = "install-base-fixture-2";
        let invalid_stage = paths.stage_directory(InstallKind::Base, invalid_operation);
        valid_base_release(&invalid_stage);
        fs::remove_dir_all(invalid_stage.join("models").join(VOSK_MODEL)).unwrap();
        assert!(promote_install(&paths, InstallKind::Base, invalid_operation).is_err());
        assert_eq!(fs::read(old.join("marker")).unwrap(), b"new");
    }

    #[test]
    fn task_0016_cancel_requires_the_exact_active_operation() {
        let runtime = VoiceRuntime::default();
        let install = runtime.begin_install(InstallKind::Base).unwrap();
        assert_eq!(
            runtime.begin_install(InstallKind::High).err().unwrap(),
            "VOICE_INSTALL_BUSY: Another voice runtime installation is already active."
        );
        assert_eq!(
            runtime.cancel_install("stale-operation").unwrap_err(),
            "VOICE_INSTALL_OPERATION_MISMATCH: The cancel request is stale."
        );
        let cancelled = runtime.cancel_install(&install.operation_id).unwrap();
        assert_eq!(cancelled.install_state, "cancelling");
        assert!(!cancelled.can_cancel);
        assert!(install.cancel.load(Ordering::Acquire));
        runtime.finish_install(&install.operation_id, "failed", "Installation cancelled.");
        assert!(runtime.snapshot().unwrap().operation_id.is_none());
    }

    #[test]
    fn task_0016_private_atomic_files_are_mode_six_hundred() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config").join("private.json");
        write_private_atomic(&path, b"bounded").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"bounded");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn task_0016_bounded_diagnostics_drain_without_growing_memory() {
        let input = vec![b'x'; 64 * 1024];
        let output = drain_bounded(input.as_slice(), 1_024).unwrap();
        assert_eq!(output.len(), 1_024);
    }
}
