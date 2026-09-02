use super::*;
use crate::provider_runtime::{
    test_support::RecordingObserver, ProviderCancellation, ProviderRunEvent, ProviderRunObserver,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct FixtureWorkspace {
    path: PathBuf,
}

impl FixtureWorkspace {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!(
            "ai-agent-control-center-task-0007-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for FixtureWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn fake_codex() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_codex.sh")
}

fn fixture_environment(scenario: &str, marker: Option<&str>) -> Vec<(OsString, OsString)> {
    let mut environment = vec![(
        OsString::from("FAKE_CODEX_SCENARIO"),
        OsString::from(scenario),
    )];
    if let Some(marker) = marker {
        environment.push((OsString::from("FAKE_CODEX_MARKER"), OsString::from(marker)));
    }
    environment
}

fn inspection(scenario: &str, marker: Option<&str>) -> CodexInspection {
    inspect_codex_runtime_at(
        Some(fake_codex()),
        Some(PathBuf::from("/usr/bin/bwrap")),
        fixture_environment(scenario, marker),
    )
}

fn specification(
    launch: CodexLaunch,
    workspace: &FixtureWorkspace,
    file_access: &str,
    terminal_access: &str,
    enable_web_search: bool,
    timeout: Duration,
) -> CodexRunSpec {
    CodexRunSpec {
        launch,
        workspace: workspace.path.clone(),
        model: "gpt-fixture".to_string(),
        reasoning_effort: "medium".to_string(),
        file_access: file_access.to_string(),
        terminal_access: terminal_access.to_string(),
        enable_web_search,
        prompt: "Inspect the fixture workspace without recursion.".to_string(),
        timeout,
    }
}

fn context(
    observer: Arc<dyn ProviderRunObserver>,
    cancellation: Arc<AtomicBool>,
) -> ProviderRunContext {
    ProviderRunContext::new(observer, ProviderCancellation::new(cancellation))
}

fn marker(prefix: &str) -> String {
    format!(
        "task-0007-{prefix}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst)
    )
}

fn marker_process_exists(marker: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        if !name.chars().all(|character| character.is_ascii_digit()) {
            return false;
        }
        fs::read(entry.path().join("cmdline"))
            .ok()
            .is_some_and(|command| {
                command
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes())
            })
    })
}

#[derive(Default)]
struct CancelOnTurnObserver {
    cancellation: Arc<AtomicBool>,
    entries: Mutex<Vec<String>>,
}

struct FailingEventObserver;

impl ProviderRunObserver for FailingEventObserver {
    fn emit(&self, _event: ProviderRunEvent) -> Result<(), ProviderError> {
        Err(ProviderError::new(
            ProviderErrorCode::EventSinkFailed,
            "fixture event sink failed",
            true,
        ))
    }

    fn mark_started(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

impl ProviderRunObserver for CancelOnTurnObserver {
    fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError> {
        self.entries.lock().unwrap().push(event.message.clone());
        if event.message == "Codex turn started." {
            self.cancellation.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn mark_started(&self) -> Result<(), ProviderError> {
        self.entries.lock().unwrap().push("started".to_string());
        Ok(())
    }
}

#[test]
fn task_0007_compatibility_accepts_required_capabilities_and_sanitizes_auth_status() {
    let ready = inspection("success", None);
    assert!(ready.installed);
    assert!(ready.compatible);
    assert!(ready.authenticated);
    assert!(ready.is_ready());
    assert_eq!(ready.version.as_deref(), Some("codex-cli 1.2.3-fixture"));

    let missing_auth = inspection("missing_auth", None);
    assert!(missing_auth.installed);
    assert!(missing_auth.compatible);
    assert!(!missing_auth.authenticated);
    assert!(!missing_auth.is_ready());
    assert!(!missing_auth.message.contains("not signed in"));
}

#[test]
fn task_0007_compatibility_reports_missing_binary_and_containment_without_probing_auth() {
    let missing_binary =
        inspect_codex_runtime_at(None, Some(PathBuf::from("/usr/bin/bwrap")), Vec::new());
    assert!(!missing_binary.installed);
    assert!(!missing_binary.authenticated);

    let missing_containment = inspect_codex_runtime_at(Some(fake_codex()), None, Vec::new());
    assert!(missing_containment.installed);
    assert!(!missing_containment.compatible);
    assert!(!missing_containment.authenticated);
    assert!(missing_containment.message.contains("Bubblewrap"));
}

#[test]
fn task_0007_compatibility_rejects_unsupported_version_and_flags() {
    let version = inspection("unsupported_version", None);
    assert!(version.installed);
    assert!(!version.compatible);
    assert!(version.launch().is_none());

    let flags = inspection("unsupported_flags", None);
    assert!(flags.installed);
    assert!(!flags.compatible);
    assert!(flags.message.contains("--json"));
    assert!(flags.launch().is_none());
}

#[test]
fn task_0007_compatibility_command_contract_is_explicit_and_prompt_is_not_an_argument() {
    let workspace = FixtureWorkspace::new();
    let ready = inspection("success", None);
    let read_only = specification(
        ready.launch().unwrap(),
        &workspace,
        "read",
        "none",
        false,
        Duration::from_secs(3),
    );
    let arguments = build_codex_arguments(&read_only).unwrap();
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let joined = arguments.join(" ");
    assert!(joined.contains("--ask-for-approval never"));
    assert!(joined.contains("--sandbox read-only"));
    assert!(joined.contains("web_search=\"disabled\""));
    assert!(joined.contains("sandbox_workspace_write.network_access=false"));
    assert!(joined.contains("--disable multi_agent"));
    assert!(joined.contains("--disable shell_tool"));
    assert!(joined.contains("--disable unified_exec"));
    assert!(joined.contains("--ephemeral"));
    assert!(joined.contains("--ignore-user-config"));
    assert!(joined.contains("--ignore-rules"));
    assert!(joined.contains("--json"));
    assert!(!joined.contains(&read_only.prompt));
    assert_eq!(arguments.last().map(String::as_str), Some("-"));
    for forbidden in [
        "danger-full-access",
        "dangerously-bypass",
        "--add-dir",
        "--approve-for-me",
        "--profile",
    ] {
        assert!(!joined.contains(forbidden));
    }

    let writable = specification(
        inspection("success", None).launch().unwrap(),
        &workspace,
        "full",
        "safe",
        true,
        Duration::from_secs(3),
    );
    let writable = build_codex_arguments(&writable)
        .unwrap()
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(writable.contains("--sandbox workspace-write"));
    assert!(writable.contains("--search"));
    assert!(writable.contains("web_search=\"live\""));
    assert!(!writable.contains("--disable shell_tool"));
}

#[test]
fn task_0007_compatibility_rejects_unenforceable_file_none_policy() {
    let workspace = FixtureWorkspace::new();
    let spec = specification(
        inspection("success", None).launch().unwrap(),
        &workspace,
        "none",
        "none",
        false,
        Duration::from_secs(3),
    );
    assert_eq!(
        build_codex_arguments(&spec).unwrap_err().code,
        ProviderErrorCode::CapabilityUnsupported
    );
}

#[test]
fn task_0007_runtime_parses_success_jsonl_usage_and_curated_progress() {
    let workspace = FixtureWorkspace::new();
    let spec = specification(
        inspection("success", None).launch().unwrap(),
        &workspace,
        "read",
        "none",
        false,
        Duration::from_secs(3),
    );
    let observer = Arc::new(RecordingObserver::default());
    let result = run_codex(
        &context(observer.clone(), Arc::new(AtomicBool::new(false))),
        spec,
    )
    .unwrap();
    assert_eq!(result.output, "fixture complete");
    assert_eq!(result.response_id.as_deref(), Some("thread-fixture"));
    assert_eq!(result.usage.input_tokens, Some(11));
    assert_eq!(result.usage.output_tokens, Some(7));
    assert_eq!(result.usage.total_tokens, Some(18));
    assert!(result.evidence.original_stdout_bytes > 0);
    assert!(observer
        .entries()
        .iter()
        .any(|entry| entry == "status:Codex turn started."));
    assert!(observer
        .entries()
        .iter()
        .all(|entry| !entry.contains("fixture complete")));
}

#[test]
fn task_0007_runtime_reports_nonzero_malformed_missing_final_and_output_limit() {
    let workspace = FixtureWorkspace::new();
    let observer = Arc::new(RecordingObserver::default());
    for (scenario, expected) in [
        ("nonzero", ProviderErrorCode::ExecutionFailed),
        ("malformed", ProviderErrorCode::ProtocolError),
        ("missing_final", ProviderErrorCode::ProtocolError),
        ("huge_output", ProviderErrorCode::OutputLimitExceeded),
    ] {
        let spec = specification(
            inspection(scenario, None).launch().unwrap(),
            &workspace,
            "read",
            "none",
            false,
            Duration::from_secs(3),
        );
        let error = run_codex(
            &context(observer.clone(), Arc::new(AtomicBool::new(false))),
            spec,
        )
        .unwrap_err();
        assert_eq!(error.code, expected, "scenario {scenario}");
        if scenario == "nonzero" {
            assert_eq!(
                error.evidence.stderr_excerpt.as_deref(),
                Some("bounded fixture failure")
            );
        }
        if scenario == "huge_output" {
            assert!(error.evidence.stdout_truncated);
            assert!(error.evidence.original_stdout_bytes > MAX_JSON_LINE_BYTES as u64);
        }
    }
}

#[test]
fn task_0030_runtime_surfaces_the_reason_codex_gave_for_a_failed_turn() {
    // TASK-0030 live acceptance hit a Codex usage limit. The CLI said so, naming
    // the reset time, but the runtime replaced every `turn.failed` with a fixed
    // "Codex reported a failed turn." string, so the operator saw an
    // unactionable error and no way to tell a quota stop from a crash.
    let workspace = FixtureWorkspace::new();
    let observer = Arc::new(RecordingObserver::default());

    for (scenario, expected) in [
        (
            "turn_failed",
            Some("You've hit your usage limit. Try again at Sep 7th, 2026 12:44 PM."),
        ),
        ("turn_failed_bare", Some("upstream refused the request")),
        ("turn_failed_unlabelled", None),
    ] {
        let spec = specification(
            inspection(scenario, None).launch().unwrap(),
            &workspace,
            "read",
            "none",
            false,
            Duration::from_secs(3),
        );
        let error = run_codex(
            &context(observer.clone(), Arc::new(AtomicBool::new(false))),
            spec,
        )
        .unwrap_err();

        assert_eq!(
            error.code,
            ProviderErrorCode::ExecutionFailed,
            "scenario {scenario} stays a typed execution failure"
        );
        match expected {
            Some(reason) => assert_eq!(
                error.message,
                format!("Codex reported a failed turn: {reason}"),
                "scenario {scenario} must carry the reason Codex gave"
            ),
            // With nothing to quote the message stays exactly as it was, rather
            // than inventing a reason.
            None => assert_eq!(
                error.message, "Codex reported a failed turn.",
                "scenario {scenario} has no reason to surface"
            ),
        }
    }
}

#[test]
fn task_0007_containment_cleans_normal_and_detached_pipe_holding_descendants() {
    let workspace = FixtureWorkspace::new();
    for scenario in ["descendant", "detached_descendant"] {
        let process_marker = marker(scenario);
        let spec = specification(
            inspection(scenario, Some(&process_marker))
                .launch()
                .unwrap(),
            &workspace,
            "read",
            "none",
            false,
            Duration::from_secs(4),
        );
        let result = run_codex(
            &context(
                Arc::new(RecordingObserver::default()),
                Arc::new(AtomicBool::new(false)),
            ),
            spec,
        )
        .unwrap();
        assert_eq!(result.output, "fixture complete");
        assert!(
            !marker_process_exists(&process_marker),
            "scenario {scenario}"
        );
    }
}

#[test]
fn task_0007_containment_cancellation_escalates_and_cleans_detached_descendant() {
    let workspace = FixtureWorkspace::new();
    let process_marker = marker("cancel");
    let spec = specification(
        inspection("cancel", Some(&process_marker))
            .launch()
            .unwrap(),
        &workspace,
        "read",
        "none",
        false,
        Duration::from_secs(4),
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let observer = Arc::new(CancelOnTurnObserver {
        cancellation: cancellation.clone(),
        entries: Mutex::new(Vec::new()),
    });
    let error = run_codex(&context(observer, cancellation), spec).unwrap_err();
    assert_eq!(error.code, ProviderErrorCode::Cancelled);
    assert!(!marker_process_exists(&process_marker));
}

#[test]
fn task_0007_containment_timeout_escalates_and_cleans_detached_descendant() {
    let workspace = FixtureWorkspace::new();
    let process_marker = marker("timeout");
    let spec = specification(
        inspection("timeout", Some(&process_marker))
            .launch()
            .unwrap(),
        &workspace,
        "read",
        "none",
        false,
        Duration::from_millis(250),
    );
    let error = run_codex(
        &context(
            Arc::new(RecordingObserver::default()),
            Arc::new(AtomicBool::new(false)),
        ),
        spec,
    )
    .unwrap_err();
    assert_eq!(error.code, ProviderErrorCode::TimedOut);
    assert!(!marker_process_exists(&process_marker));
}

#[test]
fn task_0007_containment_event_sink_failure_cleans_detached_descendant() {
    let workspace = FixtureWorkspace::new();
    let process_marker = marker("event-sink");
    let spec = specification(
        inspection("cancel", Some(&process_marker))
            .launch()
            .unwrap(),
        &workspace,
        "read",
        "none",
        false,
        Duration::from_secs(4),
    );
    let error = run_codex(
        &context(
            Arc::new(FailingEventObserver),
            Arc::new(AtomicBool::new(false)),
        ),
        spec,
    )
    .unwrap_err();
    assert_eq!(error.code, ProviderErrorCode::EventSinkFailed);
    assert!(!marker_process_exists(&process_marker));
}
