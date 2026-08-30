use crate::{
    provider_runtime::{
        ProviderError, ProviderErrorCode, ProviderEventKind, ProviderRunContext,
        ProviderRunEvidence, ProviderRunUsage, RuntimeProviderId,
    },
    run_coordinator::{MAX_STDERR_CAPTURE_BYTES, MAX_STDOUT_CAPTURE_BYTES},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    time::{Duration, Instant, SystemTime},
};

#[cfg(target_os = "linux")]
use std::os::unix::{
    fs::{MetadataExt, PermissionsExt},
    io::AsRawFd,
    process::CommandExt,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const TERMINATION_GRACE: Duration = Duration::from_millis(500);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PROBE_BYTES: usize = 64 * 1024;
const MAX_JSON_LINE_BYTES: usize = 64 * 1024;
const MAX_PROMPT_BYTES: usize = 256 * 1024;
const MAX_CURATED_PROGRESS_EVENTS: usize = 64;

type JsonLineHandler<'a> = dyn FnMut(&[u8]) -> Result<(), ProviderError> + 'a;

const ALWAYS_DISABLED_FEATURES: &[&str] = &[
    "apps",
    "auth_elicitation",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "code_mode_host",
    "computer_use",
    "goals",
    "hooks",
    "image_generation",
    "in_app_browser",
    "memories",
    "plugins",
    "plugin_sharing",
    "recommended_plugins",
    "remote_plugin",
    "skill_mcp_dependency_install",
    "skill_search",
    "tool_call_mcp_elicitation",
    "tool_suggest",
    "workspace_dependencies",
];

#[derive(Debug, Clone)]
struct CodexCompatibility {
    features: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableIdentity {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(target_os = "linux")]
    device: u64,
    #[cfg(target_os = "linux")]
    inode: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct CodexLaunch {
    binary: PathBuf,
    containment_binary: PathBuf,
    binary_identity: ExecutableIdentity,
    containment_identity: ExecutableIdentity,
    compatibility: CodexCompatibility,
    extra_environment: Vec<(OsString, OsString)>,
}

#[derive(Debug, Clone)]
pub(crate) struct CodexInspection {
    pub(crate) installed: bool,
    pub(crate) authenticated: bool,
    pub(crate) compatible: bool,
    pub(crate) version: Option<String>,
    pub(crate) binary_path: Option<String>,
    pub(crate) message: String,
    launch: Option<CodexLaunch>,
}

impl CodexInspection {
    pub(crate) fn is_ready(&self) -> bool {
        self.installed && self.authenticated && self.compatible && self.launch.is_some()
    }

    pub(crate) fn launch(&self) -> Option<CodexLaunch> {
        self.launch.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CodexRunSpec {
    pub(crate) launch: CodexLaunch,
    pub(crate) workspace: PathBuf,
    pub(crate) model: String,
    pub(crate) reasoning_effort: String,
    pub(crate) file_access: String,
    pub(crate) terminal_access: String,
    pub(crate) enable_web_search: bool,
    pub(crate) prompt: String,
    pub(crate) timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexRunOutput {
    pub(crate) output: String,
    pub(crate) response_id: Option<String>,
    pub(crate) usage: ProviderRunUsage,
    pub(crate) evidence: ProviderRunEvidence,
}

pub(crate) fn inspect_codex_runtime() -> CodexInspection {
    inspect_codex_runtime_at(find_codex_binary(), find_containment_binary(), Vec::new())
}

pub(crate) fn inspect_codex_runtime_at(
    codex_binary: Option<PathBuf>,
    containment_binary: Option<PathBuf>,
    extra_environment: Vec<(OsString, OsString)>,
) -> CodexInspection {
    let Some(binary) = codex_binary.and_then(canonical_executable) else {
        return CodexInspection {
            installed: false,
            authenticated: false,
            compatible: false,
            version: None,
            binary_path: None,
            message: "Codex CLI is not installed. Install it, then refresh this status."
                .to_string(),
            launch: None,
        };
    };
    let binary_path = Some(binary.to_string_lossy().to_string());

    #[cfg(not(target_os = "linux"))]
    {
        let _ = containment_binary;
        let _ = extra_environment;
        return CodexInspection {
            installed: true,
            authenticated: false,
            compatible: false,
            version: None,
            binary_path,
            message: "The hardened Codex runtime is currently available only on Linux.".to_string(),
            launch: None,
        };
    }

    #[cfg(target_os = "linux")]
    {
        let Some(containment_binary) = containment_binary.and_then(canonical_executable) else {
            return CodexInspection {
                installed: true,
                authenticated: false,
                compatible: false,
                version: None,
                binary_path,
                message: "Codex is installed, but Bubblewrap process containment is unavailable."
                    .to_string(),
                launch: None,
            };
        };

        let mut containment_probe = Command::new(&containment_binary);
        containment_probe.arg("--version");
        apply_environment(&mut containment_probe, &extra_environment);
        if run_probe(containment_probe, None).is_err() {
            return CodexInspection {
                installed: true,
                authenticated: false,
                compatible: false,
                version: None,
                binary_path,
                message:
                    "Codex is installed, but Bubblewrap process containment could not be verified."
                        .to_string(),
                launch: None,
            };
        }

        let launch = CodexLaunch {
            binary_identity: executable_identity(&binary)
                .expect("the canonical Codex binary was validated above"),
            containment_identity: executable_identity(&containment_binary)
                .expect("the canonical containment binary was validated above"),
            binary,
            containment_binary,
            compatibility: CodexCompatibility {
                features: BTreeMap::new(),
            },
            extra_environment,
        };

        let version = match contained_probe_text(&launch, &["--version"]) {
            Ok(version) if valid_codex_version(&version) => version,
            _ => {
                return CodexInspection {
                    installed: true,
                    authenticated: false,
                    compatible: false,
                    version: None,
                    binary_path,
                    message: "Codex is installed, but its version response is unsupported."
                        .to_string(),
                    launch: None,
                }
            }
        };

        let top_level_help = match contained_probe_text(&launch, &["--help"]) {
            Ok(help) => help,
            Err(message) => return incompatible_inspection(binary_path, Some(version), message),
        };
        let exec_help = match contained_probe_text(&launch, &["exec", "--help"]) {
            Ok(help) => help,
            Err(message) => return incompatible_inspection(binary_path, Some(version), message),
        };
        let feature_text = match contained_probe_text(&launch, &["features", "list"]) {
            Ok(features) => features,
            Err(message) => return incompatible_inspection(binary_path, Some(version), message),
        };
        let features = parse_features(&feature_text);
        if let Err(message) = validate_compatibility(&top_level_help, &exec_help, &features) {
            return incompatible_inspection(binary_path, Some(version), message);
        }

        let mut compatible_launch = launch;
        compatible_launch.compatibility = CodexCompatibility { features };
        let authenticated = contained_probe(&compatible_launch, &["login", "status"])
            .map(|result| result.status.success())
            .unwrap_or(false);

        if !launch_identity_is_current(&compatible_launch) {
            return incompatible_inspection(
                binary_path,
                Some(version),
                "a runtime executable changed while its capabilities were being inspected",
            );
        }

        CodexInspection {
            installed: true,
            authenticated,
            compatible: true,
            version: Some(version),
            binary_path,
            message: if authenticated {
                "Codex is installed, compatible, contained, and signed in with ChatGPT.".to_string()
            } else {
                "Codex is installed and compatible, but authentication is unavailable. Run `codex login` in Kitty, then refresh this status."
                    .to_string()
            },
            launch: authenticated.then_some(compatible_launch),
        }
    }
}

fn incompatible_inspection(
    binary_path: Option<String>,
    version: Option<String>,
    detail: impl AsRef<str>,
) -> CodexInspection {
    let detail = detail.as_ref();
    let message = if detail.is_empty() {
        "Codex is installed, but this CLI build lacks required non-interactive safety capabilities."
            .to_string()
    } else {
        format!(
            "Codex is installed, but this CLI build is incompatible with the hardened runtime: {detail}"
        )
    };
    CodexInspection {
        installed: true,
        authenticated: false,
        compatible: false,
        version,
        binary_path,
        message,
        launch: None,
    }
}

fn validate_compatibility(
    top_level_help: &str,
    exec_help: &str,
    features: &BTreeMap<String, bool>,
) -> Result<(), String> {
    for required in ["--ask-for-approval", "--search", "--sandbox", "--disable"] {
        if !top_level_help.contains(required) {
            return Err(format!("required flag `{required}` is missing"));
        }
    }
    for required in [
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--strict-config",
        "--json",
        "--skip-git-repo-check",
    ] {
        if !exec_help.contains(required) {
            return Err(format!("required `exec` flag `{required}` is missing"));
        }
    }
    if !top_level_help.contains("never") {
        return Err("approval policy `never` is unavailable".to_string());
    }
    if !features.contains_key("multi_agent") {
        return Err("the `multi_agent` feature cannot be explicitly disabled".to_string());
    }
    Ok(())
}

fn valid_codex_version(value: &str) -> bool {
    let Some(version) = value.trim().strip_prefix("codex-cli ") else {
        return false;
    };
    let core = version.split('-').next().unwrap_or_default();
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

fn parse_features(value: &str) -> BTreeMap<String, bool> {
    value
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 3 {
                return None;
            }
            let enabled = match columns.last().copied() {
                Some("true") => true,
                Some("false") => false,
                _ => return None,
            };
            Some((columns[0].to_string(), enabled))
        })
        .collect()
}

fn find_codex_binary() -> Option<PathBuf> {
    if let Some(configured) = env::var_os("CODEX_BINARY") {
        let path = PathBuf::from(configured);
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let user_install = PathBuf::from(home).join(".local/bin/codex");
        if is_executable_file(&user_install) {
            return Some(user_install);
        }
    }
    find_in_path("codex").or_else(|| {
        ["/usr/local/bin/codex", "/usr/bin/codex"]
            .into_iter()
            .map(PathBuf::from)
            .find(|candidate| is_executable_file(candidate))
    })
}

fn find_containment_binary() -> Option<PathBuf> {
    ["/usr/bin/bwrap", "/usr/local/bin/bwrap"]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| is_executable_file(candidate))
        .or_else(|| find_in_path("bwrap"))
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path_value| {
        env::split_paths(&path_value)
            .map(|directory| directory.join(binary))
            .find(|candidate| is_executable_file(candidate))
    })
}

fn canonical_executable(path: PathBuf) -> Option<PathBuf> {
    if !is_executable_file(&path) {
        return None;
    }
    fs::canonicalize(path)
        .ok()
        .filter(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

fn executable_identity(path: &Path) -> Option<ExecutableIdentity> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(ExecutableIdentity {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(target_os = "linux")]
        device: metadata.dev(),
        #[cfg(target_os = "linux")]
        inode: metadata.ino(),
    })
}

fn launch_identity_is_current(launch: &CodexLaunch) -> bool {
    executable_identity(&launch.binary).as_ref() == Some(&launch.binary_identity)
        && executable_identity(&launch.containment_binary).as_ref()
            == Some(&launch.containment_identity)
}

fn contained_probe_text(launch: &CodexLaunch, arguments: &[&str]) -> Result<String, String> {
    let result = contained_probe(launch, arguments)?;
    if !result.status.success() {
        return Err("a required CLI probe returned a nonzero status".to_string());
    }
    let text = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if text.is_empty() {
        Err("a required CLI probe returned no output".to_string())
    } else {
        Ok(text)
    }
}

fn contained_probe(launch: &CodexLaunch, arguments: &[&str]) -> Result<SupervisedOutput, String> {
    let mut command = contained_command(launch, arguments.iter().map(OsString::from));
    apply_environment(&mut command, &launch.extra_environment);
    run_probe(command, None).map_err(|_| "a required CLI probe could not complete".to_string())
}

fn run_probe(command: Command, input: Option<&[u8]>) -> Result<SupervisedOutput, ProviderError> {
    supervise_command(
        command,
        input,
        PROBE_TIMEOUT,
        MAX_PROBE_BYTES,
        MAX_PROBE_BYTES,
        true,
        None,
        None,
    )
}

fn contained_command(
    launch: &CodexLaunch,
    arguments: impl IntoIterator<Item = OsString>,
) -> Command {
    let mut command = Command::new(&launch.containment_binary);
    command
        .arg("--unshare-user")
        .arg("--unshare-pid")
        .arg("--die-with-parent")
        .arg("--cap-drop")
        .arg("ALL")
        .arg("--dev-bind")
        .arg("/")
        .arg("/")
        .arg("--proc")
        .arg("/proc")
        .arg("--")
        .arg(&launch.binary)
        .args(arguments);
    command
}

fn apply_environment(command: &mut Command, environment: &[(OsString, OsString)]) {
    for (key, value) in environment {
        command.env(key, value);
    }
}

fn sanitized_path(binary: &Path) -> Option<OsString> {
    let parent = binary.parent()?;
    if matches!(
        parent.to_str(),
        Some("/bin" | "/usr/bin" | "/usr/local/bin")
    ) {
        return None;
    }
    let value = env::var_os("PATH")?;
    let paths = env::split_paths(&value)
        .filter(|entry| entry != parent)
        .collect::<Vec<_>>();
    env::join_paths(paths).ok()
}

fn build_codex_arguments(spec: &CodexRunSpec) -> Result<Vec<OsString>, ProviderError> {
    if spec.prompt.trim().is_empty() || spec.prompt.len() > MAX_PROMPT_BYTES {
        return Err(codex_error(
            ProviderErrorCode::CapabilityUnsupported,
            format!("The Codex prompt must contain between 1 and {MAX_PROMPT_BYTES} bytes."),
            false,
            &spec.model,
        ));
    }
    let sandbox = match spec.file_access.as_str() {
        "read" => "read-only",
        "write" | "full" => "workspace-write",
        "none" => {
            return Err(codex_error(
                ProviderErrorCode::CapabilityUnsupported,
                "The installed Codex CLI cannot guarantee a no-file-access run.",
                false,
                &spec.model,
            ))
        }
        _ => {
            return Err(codex_error(
                ProviderErrorCode::CapabilityUnsupported,
                "The Codex file-access policy is unsupported.",
                false,
                &spec.model,
            ))
        }
    };
    if !matches!(spec.terminal_access.as_str(), "none" | "safe" | "user") {
        return Err(codex_error(
            ProviderErrorCode::CapabilityUnsupported,
            "The Codex terminal-access policy is unsupported.",
            false,
            &spec.model,
        ));
    }
    if spec.terminal_access == "none"
        && (!spec
            .launch
            .compatibility
            .features
            .contains_key("shell_tool")
            || !spec
                .launch
                .compatibility
                .features
                .contains_key("unified_exec"))
    {
        return Err(codex_error(
            ProviderErrorCode::CapabilityUnsupported,
            "This Codex CLI cannot explicitly disable its terminal tools.",
            false,
            &spec.model,
        ));
    }

    let mut arguments = vec![
        OsString::from("--ask-for-approval"),
        OsString::from("never"),
        OsString::from("--sandbox"),
        OsString::from(sandbox),
        OsString::from("--model"),
        OsString::from(&spec.model),
        OsString::from("-C"),
        spec.workspace.as_os_str().to_os_string(),
        OsString::from("--strict-config"),
        OsString::from("-c"),
        OsString::from(format!(
            "model_reasoning_effort=\"{}\"",
            spec.reasoning_effort
        )),
        OsString::from("-c"),
        OsString::from(if spec.enable_web_search {
            "web_search=\"live\""
        } else {
            "web_search=\"disabled\""
        }),
        OsString::from("-c"),
        OsString::from("sandbox_workspace_write.network_access=false"),
        OsString::from("-c"),
        OsString::from("shell_environment_policy.inherit=\"core\""),
        OsString::from("-c"),
        OsString::from("shell_environment_policy.ignore_default_excludes=false"),
        OsString::from("-c"),
        OsString::from("mcp_servers={}"),
    ];
    if spec.enable_web_search {
        arguments.push(OsString::from("--search"));
    }

    let mut disabled = vec!["multi_agent"];
    for feature in ALWAYS_DISABLED_FEATURES {
        if spec
            .launch
            .compatibility
            .features
            .get(*feature)
            .copied()
            .unwrap_or(false)
        {
            disabled.push(feature);
        }
    }
    if spec.terminal_access == "none" {
        for feature in ["shell_tool", "unified_exec", "shell_snapshot"] {
            if spec.launch.compatibility.features.contains_key(feature) {
                disabled.push(feature);
            }
        }
    }
    disabled.sort_unstable();
    disabled.dedup();
    for feature in disabled {
        arguments.push(OsString::from("--disable"));
        arguments.push(OsString::from(feature));
    }
    arguments.extend([
        OsString::from("exec"),
        OsString::from("--ephemeral"),
        OsString::from("--ignore-user-config"),
        OsString::from("--ignore-rules"),
        OsString::from("--json"),
        OsString::from("--skip-git-repo-check"),
        OsString::from("--color"),
        OsString::from("never"),
        OsString::from("-"),
    ]);
    Ok(arguments)
}

pub(crate) fn run_codex(
    context: &ProviderRunContext,
    spec: CodexRunSpec,
) -> Result<CodexRunOutput, ProviderError> {
    if !launch_identity_is_current(&spec.launch) {
        return Err(codex_error(
            ProviderErrorCode::RuntimeIncompatible,
            "A Codex runtime executable changed after compatibility inspection.",
            false,
            &spec.model,
        ));
    }
    if context.is_cancelled() {
        return Err(codex_error(
            ProviderErrorCode::Cancelled,
            "Agent run cancelled by the user.",
            true,
            &spec.model,
        ));
    }
    let arguments = build_codex_arguments(&spec)?;
    let mut command = contained_command(&spec.launch, arguments);
    command.current_dir(&spec.workspace);
    apply_environment(&mut command, &spec.launch.extra_environment);
    command.env("AI_AGENT_CONTROL_CENTER_CODEX_RUN", "1");
    if let Some(path) = sanitized_path(&spec.launch.binary) {
        command.env("PATH", path);
    }

    let mut parser = CodexJsonl::default();
    let mut line_handler = |line: &[u8]| parser.handle_line(line, context);
    let supervised = supervise_command(
        command,
        Some(spec.prompt.as_bytes()),
        spec.timeout,
        MAX_STDOUT_CAPTURE_BYTES,
        MAX_STDERR_CAPTURE_BYTES,
        false,
        Some(context),
        Some(&mut line_handler),
    )
    .map_err(|error| with_codex_identity(error, &spec.model))?;

    if !supervised.status.success() {
        return Err(codex_error(
            ProviderErrorCode::ExecutionFailed,
            format!(
                "Codex exited without completing the task ({}).",
                exit_status_label(&supervised.status)
            ),
            true,
            &spec.model,
        )
        .with_evidence(supervised.evidence()));
    }
    let parsed = parser.finish().map_err(|error| {
        with_codex_identity(error, &spec.model).with_evidence(supervised.evidence())
    })?;
    Ok(CodexRunOutput {
        output: parsed.output,
        response_id: parsed.response_id,
        usage: parsed.usage,
        evidence: supervised.evidence(),
    })
}

fn exit_status_label(status: &ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

fn codex_error(
    code: ProviderErrorCode,
    message: impl Into<String>,
    retryable: bool,
    model: &str,
) -> ProviderError {
    ProviderError::new(code, message, retryable)
        .with_provider(RuntimeProviderId::Codex)
        .with_model(model)
}

fn with_codex_identity(error: ProviderError, model: &str) -> ProviderError {
    error
        .with_provider(RuntimeProviderId::Codex)
        .with_model(model)
}

#[derive(Default)]
struct CodexJsonl {
    response_id: Option<String>,
    final_message: Option<String>,
    usage: ProviderRunUsage,
    turn_completed: bool,
    progress_events: usize,
}

struct ParsedCodexOutput {
    output: String,
    response_id: Option<String>,
    usage: ProviderRunUsage,
}

impl CodexJsonl {
    fn handle_line(
        &mut self,
        line: &[u8],
        context: &ProviderRunContext,
    ) -> Result<(), ProviderError> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        let event: Value = serde_json::from_slice(line).map_err(|_| {
            ProviderError::new(
                ProviderErrorCode::ProtocolError,
                "Codex returned malformed JSONL output.",
                true,
            )
        })?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "thread.started" => {
                self.response_id = event
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string);
            }
            "turn.started" => {
                self.emit_progress(context, ProviderEventKind::Status, "Codex turn started.")?;
            }
            "item.started" => {
                if event
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("command_execution")
                {
                    self.emit_progress(
                        context,
                        ProviderEventKind::Progress,
                        "Codex started a sandboxed command.",
                    )?;
                }
            }
            "item.completed" => {
                let item = event.get("item").unwrap_or(&Value::Null);
                match item.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "agent_message" => {
                        self.final_message = item
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_string);
                    }
                    "command_execution" => self.emit_progress(
                        context,
                        ProviderEventKind::Progress,
                        "Codex completed a sandboxed command.",
                    )?,
                    "file_change" => self.emit_progress(
                        context,
                        ProviderEventKind::Progress,
                        "Codex reported a workspace file change.",
                    )?,
                    "mcp_tool_call" => {
                        return Err(ProviderError::new(
                            ProviderErrorCode::ProtocolError,
                            "Codex attempted to use a disabled external tool.",
                            false,
                        ))
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                self.turn_completed = true;
                if let Some(usage) = event.get("usage") {
                    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
                    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
                    let total_tokens =
                        usage
                            .get("total_tokens")
                            .and_then(Value::as_u64)
                            .or_else(|| match (input_tokens, output_tokens) {
                                (Some(input), Some(output)) => input.checked_add(output),
                                _ => None,
                            });
                    self.usage = ProviderRunUsage {
                        input_tokens,
                        output_tokens,
                        total_tokens,
                    };
                }
            }
            "turn.failed" | "error" => {
                return Err(ProviderError::new(
                    ProviderErrorCode::ExecutionFailed,
                    "Codex reported a failed turn.",
                    true,
                ))
            }
            _ => {}
        }
        Ok(())
    }

    fn emit_progress(
        &mut self,
        context: &ProviderRunContext,
        kind: ProviderEventKind,
        message: &str,
    ) -> Result<(), ProviderError> {
        if self.progress_events >= MAX_CURATED_PROGRESS_EVENTS {
            return Ok(());
        }
        self.progress_events += 1;
        context.emit(kind, message)
    }

    fn finish(self) -> Result<ParsedCodexOutput, ProviderError> {
        if !self.turn_completed {
            return Err(ProviderError::new(
                ProviderErrorCode::ProtocolError,
                "Codex completed without a terminal `turn.completed` event.",
                true,
            ));
        }
        let output = self.final_message.ok_or_else(|| {
            ProviderError::new(
                ProviderErrorCode::ProtocolError,
                "Codex completed without returning a final agent message.",
                true,
            )
        })?;
        Ok(ParsedCodexOutput {
            output,
            response_id: self.response_id,
            usage: self.usage,
        })
    }
}

struct StreamCapture {
    retained: Vec<u8>,
    original_bytes: u64,
    limit: usize,
    retain: bool,
    truncated: bool,
}

impl StreamCapture {
    fn new(limit: usize, retain: bool) -> Self {
        Self {
            retained: Vec::with_capacity(if retain { limit.min(64 * 1024) } else { 0 }),
            original_bytes: 0,
            limit,
            retain,
            truncated: false,
        }
    }

    fn record(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        self.record_for_cleanup(bytes);
        if self.original_bytes > self.limit as u64 {
            return Err(ProviderError::new(
                ProviderErrorCode::OutputLimitExceeded,
                format!(
                    "Provider output exceeded its {}-byte stream limit.",
                    self.limit
                ),
                false,
            ));
        }
        Ok(())
    }

    fn record_for_cleanup(&mut self, bytes: &[u8]) {
        self.original_bytes = self.original_bytes.saturating_add(bytes.len() as u64);
        if self.retain {
            let available = self.limit.saturating_sub(self.retained.len());
            self.retained
                .extend_from_slice(&bytes[..bytes.len().min(available)]);
        }
        if self.original_bytes > self.limit as u64 {
            self.truncated = true;
        }
    }
}

struct LineDecoder {
    pending: Vec<u8>,
}

struct JsonLineProcessor<'a> {
    decoder: LineDecoder,
    handler: &'a mut JsonLineHandler<'a>,
    finished: bool,
}

impl<'a> JsonLineProcessor<'a> {
    fn new(handler: &'a mut JsonLineHandler<'a>) -> Self {
        Self {
            decoder: LineDecoder::new(),
            handler,
            finished: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        self.decoder.push(bytes, self.handler)
    }

    fn finish(&mut self) -> Result<(), ProviderError> {
        if !self.finished {
            self.decoder.finish(self.handler)?;
            self.finished = true;
        }
        Ok(())
    }
}

impl LineDecoder {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn push(
        &mut self,
        bytes: &[u8],
        handler: &mut dyn FnMut(&[u8]) -> Result<(), ProviderError>,
    ) -> Result<(), ProviderError> {
        for byte in bytes {
            if *byte == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                handler(&self.pending)?;
                self.pending.clear();
            } else {
                self.pending.push(*byte);
                if self.pending.len() > MAX_JSON_LINE_BYTES {
                    return Err(ProviderError::new(
                        ProviderErrorCode::OutputLimitExceeded,
                        format!(
                            "Codex JSONL output exceeded the {MAX_JSON_LINE_BYTES}-byte line limit."
                        ),
                        false,
                    ));
                }
            }
        }
        Ok(())
    }

    fn finish(
        &mut self,
        handler: &mut dyn FnMut(&[u8]) -> Result<(), ProviderError>,
    ) -> Result<(), ProviderError> {
        if !self.pending.is_empty() {
            handler(&self.pending)?;
            self.pending.clear();
        }
        Ok(())
    }
}

struct SupervisedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_original_bytes: u64,
    stderr_original_bytes: u64,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl SupervisedOutput {
    fn evidence(&self) -> ProviderRunEvidence {
        let stderr = String::from_utf8_lossy(&self.stderr).trim().to_string();
        ProviderRunEvidence {
            stderr_excerpt: (!stderr.is_empty()).then_some(stderr),
            stdout_truncated: self.stdout_truncated,
            stderr_truncated: self.stderr_truncated,
            original_stdout_bytes: self.stdout_original_bytes,
            original_stderr_bytes: self.stderr_original_bytes,
            ..ProviderRunEvidence::default()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn supervise_command<'a>(
    mut command: Command,
    input: Option<&[u8]>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    retain_stdout: bool,
    context: Option<&ProviderRunContext>,
    mut line_handler: Option<&'a mut JsonLineHandler<'a>>,
) -> Result<SupervisedOutput, ProviderError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        let _ = input;
        let _ = timeout;
        let _ = stdout_limit;
        let _ = stderr_limit;
        let _ = retain_stdout;
        let _ = context;
        let _ = line_handler;
        return Err(ProviderError::new(
            ProviderErrorCode::RuntimeIncompatible,
            "The hardened Codex supervisor is currently available only on Linux.",
            false,
        ));
    }

    #[cfg(target_os = "linux")]
    {
        if context.map(|value| value.is_cancelled()).unwrap_or(false) {
            return Err(ProviderError::new(
                ProviderErrorCode::Cancelled,
                "Agent run cancelled by the user.",
                true,
            ));
        }
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command.spawn().map_err(|_| {
            ProviderError::new(
                ProviderErrorCode::StartupFailed,
                "Could not start the contained Codex process.",
                true,
            )
        })?;
        let process_group = child.id() as i32;
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let mut stdin = if input.is_some() {
            child.stdin.take()
        } else {
            None
        };
        let mut stdout_capture = StreamCapture::new(stdout_limit, retain_stdout);
        let mut stderr_capture = StreamCapture::new(stderr_limit, true);

        let setup_result = (|| -> Result<(), ProviderError> {
            let stdout_fd = stdout.as_ref().ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorCode::StartupFailed,
                    "Could not capture Codex stdout.",
                    true,
                )
            })?;
            let stderr_fd = stderr.as_ref().ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorCode::StartupFailed,
                    "Could not capture Codex stderr.",
                    true,
                )
            })?;
            set_nonblocking(stdout_fd.as_raw_fd())?;
            set_nonblocking(stderr_fd.as_raw_fd())?;
            if let Some(stdin) = stdin.as_ref() {
                set_nonblocking(stdin.as_raw_fd())?;
            }
            Ok(())
        })();
        if let Err(error) = setup_result {
            // A partially configured pipe may still be blocking. Close both
            // capture ends before bounded termination rather than risk a
            // cleanup read waiting indefinitely on that exceptional path.
            stdout = None;
            stderr = None;
            return Err(terminate_with_error(
                &mut child,
                process_group,
                &mut stdin,
                &mut stdout,
                &mut stderr,
                &mut stdout_capture,
                &mut stderr_capture,
                error,
            ));
        }
        if let Some(context) = context {
            if let Err(error) = context.mark_started() {
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }
        }

        let started = Instant::now();
        let deadline = started + timeout;
        let mut input_offset = 0_usize;
        let mut line_processor = line_handler.take().map(JsonLineProcessor::new);
        let mut exit_status = None;
        let mut drain_deadline = None;

        loop {
            if context.map(|value| value.is_cancelled()).unwrap_or(false) {
                let error = ProviderError::new(
                    ProviderErrorCode::Cancelled,
                    "Agent run cancelled by the user.",
                    true,
                );
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                let error = ProviderError::new(
                    ProviderErrorCode::TimedOut,
                    format!(
                        "Codex stopped after reaching the {}-second timeout.",
                        timeout.as_secs()
                    ),
                    true,
                );
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }

            if exit_status.is_none() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        exit_status = Some(status);
                        drain_deadline = Some(Instant::now() + CLEANUP_TIMEOUT);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        let error = ProviderError::new(
                            ProviderErrorCode::ExecutionFailed,
                            "Could not monitor the contained Codex process.",
                            true,
                        );
                        return Err(terminate_with_error(
                            &mut child,
                            process_group,
                            &mut stdin,
                            &mut stdout,
                            &mut stderr,
                            &mut stdout_capture,
                            &mut stderr_capture,
                            error,
                        ));
                    }
                }
            }

            if let Some(drain_deadline) = drain_deadline {
                if Instant::now() >= drain_deadline && (stdout.is_some() || stderr.is_some()) {
                    let error = ProviderError::new(
                        ProviderErrorCode::ExecutionFailed,
                        "Codex exited without releasing all owned process pipes.",
                        false,
                    );
                    return Err(terminate_with_error(
                        &mut child,
                        process_group,
                        &mut stdin,
                        &mut stdout,
                        &mut stderr,
                        &mut stdout_capture,
                        &mut stderr_capture,
                        error,
                    ));
                }
            }

            if stdout.is_none() && stderr.is_none() {
                if let Some(status) = exit_status.take() {
                    if let Some(processor) = line_processor.as_mut() {
                        processor.finish().map_err(|error| {
                            error.with_evidence(capture_evidence(&stdout_capture, &stderr_capture))
                        })?;
                    }
                    return Ok(SupervisedOutput {
                        status,
                        stdout: stdout_capture.retained,
                        stderr: stderr_capture.retained,
                        stdout_original_bytes: stdout_capture.original_bytes,
                        stderr_original_bytes: stderr_capture.original_bytes,
                        stdout_truncated: stdout_capture.truncated,
                        stderr_truncated: stderr_capture.truncated,
                    });
                }
            }

            if let Err(error) = poll_process_fds(
                stdout.as_ref(),
                stderr.as_ref(),
                stdin
                    .as_ref()
                    .filter(|_| input_offset < input.map_or(0, |value| value.len())),
                poll_timeout(deadline, drain_deadline),
            ) {
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }

            if let Some(input) = input {
                if input_offset < input.len() {
                    if let Some(writer) = stdin.as_mut() {
                        match writer.write(&input[input_offset..]) {
                            Ok(0) => {
                                let error = ProviderError::new(
                                    ProviderErrorCode::ExecutionFailed,
                                    "Codex closed prompt input before reading it.",
                                    true,
                                );
                                return Err(terminate_with_error(
                                    &mut child,
                                    process_group,
                                    &mut stdin,
                                    &mut stdout,
                                    &mut stderr,
                                    &mut stdout_capture,
                                    &mut stderr_capture,
                                    error,
                                ));
                            }
                            Ok(written) => input_offset += written,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                            Err(_) => {
                                let error = ProviderError::new(
                                    ProviderErrorCode::ExecutionFailed,
                                    "Could not send the bounded prompt to Codex.",
                                    true,
                                );
                                return Err(terminate_with_error(
                                    &mut child,
                                    process_group,
                                    &mut stdin,
                                    &mut stdout,
                                    &mut stderr,
                                    &mut stdout_capture,
                                    &mut stderr_capture,
                                    error,
                                ));
                            }
                        }
                    }
                }
                if input_offset >= input.len() {
                    stdin = None;
                }
            }

            let stdout_result =
                pump_stdout(&mut stdout, &mut stdout_capture, line_processor.as_mut());
            if let Err(error) = stdout_result {
                if error.code == ProviderErrorCode::OutputLimitExceeded {
                    stdout_capture.truncated = true;
                }
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }
            if stdout.is_none() {
                if let Some(processor) = line_processor.as_mut() {
                    if let Err(error) = processor.finish() {
                        return Err(terminate_with_error(
                            &mut child,
                            process_group,
                            &mut stdin,
                            &mut stdout,
                            &mut stderr,
                            &mut stdout_capture,
                            &mut stderr_capture,
                            error,
                        ));
                    }
                }
            }
            if let Err(error) = pump_stderr(&mut stderr, &mut stderr_capture) {
                if error.code == ProviderErrorCode::OutputLimitExceeded {
                    stderr_capture.truncated = true;
                }
                return Err(terminate_with_error(
                    &mut child,
                    process_group,
                    &mut stdin,
                    &mut stdout,
                    &mut stderr,
                    &mut stdout_capture,
                    &mut stderr_capture,
                    error,
                ));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn set_nonblocking(fd: i32) -> Result<(), ProviderError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ProviderError::new(
            ProviderErrorCode::StartupFailed,
            "Could not configure bounded Codex process pipes.",
            true,
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn poll_process_fds(
    stdout: Option<&ChildStdout>,
    stderr: Option<&ChildStderr>,
    stdin: Option<&ChildStdin>,
    timeout: Duration,
) -> Result<(), ProviderError> {
    let mut descriptors = Vec::with_capacity(3);
    if let Some(stdout) = stdout {
        descriptors.push(libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    if let Some(stderr) = stderr {
        descriptors.push(libc::pollfd {
            fd: stderr.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    if let Some(stdin) = stdin {
        descriptors.push(libc::pollfd {
            fd: stdin.as_raw_fd(),
            events: libc::POLLOUT | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    let timeout_millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    let result = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout_millis,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(());
        }
        return Err(ProviderError::new(
            ProviderErrorCode::ExecutionFailed,
            "Could not poll the contained Codex process pipes.",
            true,
        ));
    }
    Ok(())
}

fn poll_timeout(deadline: Instant, drain_deadline: Option<Instant>) -> Duration {
    let now = Instant::now();
    let mut timeout = deadline.saturating_duration_since(now).min(POLL_INTERVAL);
    if let Some(drain_deadline) = drain_deadline {
        timeout = timeout.min(drain_deadline.saturating_duration_since(now));
    }
    timeout
}

fn pump_stdout(
    reader: &mut Option<ChildStdout>,
    capture: &mut StreamCapture,
    mut line_processor: Option<&mut JsonLineProcessor<'_>>,
) -> Result<(), ProviderError> {
    let Some(stream) = reader.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                *reader = None;
                return Ok(());
            }
            Ok(read) => {
                let bytes = &buffer[..read];
                capture.record(bytes)?;
                if let Some(processor) = line_processor.as_deref_mut() {
                    processor.push(bytes)?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(_) => {
                return Err(ProviderError::new(
                    ProviderErrorCode::ExecutionFailed,
                    "Could not read Codex stdout.",
                    true,
                ))
            }
        }
    }
}

fn pump_stderr(
    reader: &mut Option<ChildStderr>,
    capture: &mut StreamCapture,
) -> Result<(), ProviderError> {
    let Some(stream) = reader.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                *reader = None;
                return Ok(());
            }
            Ok(read) => capture.record(&buffer[..read])?,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(_) => {
                return Err(ProviderError::new(
                    ProviderErrorCode::ExecutionFailed,
                    "Could not read Codex stderr.",
                    true,
                ))
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn terminate_with_error(
    child: &mut Child,
    process_group: i32,
    stdin: &mut Option<ChildStdin>,
    stdout: &mut Option<ChildStdout>,
    stderr: &mut Option<ChildStderr>,
    stdout_capture: &mut StreamCapture,
    stderr_capture: &mut StreamCapture,
    error: ProviderError,
) -> ProviderError {
    *stdin = None;
    let cleaned = terminate_owned_process(
        child,
        process_group,
        stdout,
        stderr,
        stdout_capture,
        stderr_capture,
    );
    let evidence = capture_evidence(stdout_capture, stderr_capture);
    if cleaned {
        error.with_evidence(evidence)
    } else {
        ProviderError::new(
            ProviderErrorCode::CleanupFailed,
            format!(
                "Could not confirm Codex process-tree cleanup after {}.",
                error.code.as_str()
            ),
            false,
        )
        .with_evidence(evidence)
    }
}

#[cfg(target_os = "linux")]
fn terminate_owned_process(
    child: &mut Child,
    process_group: i32,
    stdout: &mut Option<ChildStdout>,
    stderr: &mut Option<ChildStderr>,
    stdout_capture: &mut StreamCapture,
    stderr_capture: &mut StreamCapture,
) -> bool {
    signal_group(process_group, libc::SIGTERM);
    if wait_for_cleanup(
        child,
        stdout,
        stderr,
        stdout_capture,
        stderr_capture,
        Instant::now() + TERMINATION_GRACE,
    ) {
        return true;
    }
    signal_group(process_group, libc::SIGKILL);
    wait_for_cleanup(
        child,
        stdout,
        stderr,
        stdout_capture,
        stderr_capture,
        Instant::now() + CLEANUP_TIMEOUT,
    )
}

#[cfg(target_os = "linux")]
fn signal_group(process_group: i32, signal: i32) {
    if process_group > 0 {
        unsafe {
            libc::kill(-process_group, signal);
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_for_cleanup(
    child: &mut Child,
    stdout: &mut Option<ChildStdout>,
    stderr: &mut Option<ChildStderr>,
    stdout_capture: &mut StreamCapture,
    stderr_capture: &mut StreamCapture,
    deadline: Instant,
) -> bool {
    loop {
        drain_for_cleanup(stdout, stdout_capture);
        drain_for_cleanup(stderr, stderr_capture);
        let exited = matches!(child.try_wait(), Ok(Some(_)));
        if exited && stdout.is_none() && stderr.is_none() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        let _ = poll_process_fds(
            stdout.as_ref(),
            stderr.as_ref(),
            None,
            POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn drain_for_cleanup<R: Read>(reader: &mut Option<R>, capture: &mut StreamCapture) {
    let Some(stream) = reader.as_mut() else {
        return;
    };
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                *reader = None;
                return;
            }
            Ok(read) => capture.record_for_cleanup(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(_) => {
                *reader = None;
                return;
            }
        }
    }
}

fn capture_evidence(
    stdout_capture: &StreamCapture,
    stderr_capture: &StreamCapture,
) -> ProviderRunEvidence {
    let stderr = String::from_utf8_lossy(&stderr_capture.retained)
        .trim()
        .to_string();
    ProviderRunEvidence {
        stderr_excerpt: (!stderr.is_empty()).then_some(stderr),
        stdout_truncated: stdout_capture.truncated,
        stderr_truncated: stderr_capture.truncated,
        original_stdout_bytes: stdout_capture.original_bytes,
        original_stderr_bytes: stderr_capture.original_bytes,
        ..ProviderRunEvidence::default()
    }
}

#[cfg(test)]
mod tests;
