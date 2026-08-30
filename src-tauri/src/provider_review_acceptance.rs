//! TASK-0024 — composed live-provider / review acceptance (S5).
//!
//! These `#[cfg(test)]` scenarios wire the post-dispatch provider path end to end
//! the way [`run_agent_task`](crate::run_agent_task) does it: admit → prepare →
//! dispatch through a real provider adapter helper (fake Codex CLI / fake Ollama
//! HTTP transport) → bounded workspace-evidence baseline/finish →
//! [`attach_workspace_evidence`](crate::attach_workspace_evidence) →
//! [`finalize_specialist_result`](crate::finalize_specialist_result) →
//! `complete_run` → the sequential review pipeline.
//!
//! [`orchestration_acceptance`](crate::orchestration_acceptance) already composes
//! the registry / routing / queue / approval / review / recovery path with a
//! *synthetic* dispatch boundary. This module is the regression home for the
//! seam that one leaves out: real provider transport (cancellation, timeout,
//! cleanup), the evidence / specialist-finalize glue, and a review verdict that
//! is actually parsed from provider output.

use crate::app_state::WorkspaceDefinition;
use crate::authorization::ApprovalResolution;
use crate::codex_runtime::{inspect_codex_runtime_at, run_codex, CodexInspection, CodexRunSpec};
use crate::ollama_runtime::OllamaSession;
use crate::persistence::{StateEnvelope, StateRepository};
use crate::policy::{ActionIntent, RunMode};
use crate::provider_runtime::{
    ProviderCancellation, ProviderError, ProviderErrorCode, ProviderModelIdentity,
    ProviderRunContext, ProviderRunEvent, ProviderRunMode, ProviderRunObserver, ProviderRunRequest,
    ProviderRunResult, RuntimeProviderId,
};
use crate::review_orchestration::{
    ReviewCheckKind, ReviewCheckResultV1, ReviewCheckStatus, ReviewRequestV1, ReviewResultV1,
    ReviewVerdict, StartReviewStageRequest, REQUIRED_REVIEW_CHECKS,
};
use crate::run_coordinator::{RunAttemptProjection, RunAttemptStatus};
use crate::specialist_capabilities::{
    CodingRequestV1, CodingResultV1, SpecialistResultV1, SpecialistRunContractV1,
    SpecialistTaskRequestV1, WorkspaceMutationClass, SPECIALIST_PROFILE_VERSION,
    SPECIALIST_SCHEMA_VERSION,
};
use crate::task_orchestration::CreateRoutedTaskRequest;
use crate::workspace_evidence::{WorkspaceChangeEvidenceV1, WorkspaceEvidenceBaseline};
use crate::{
    attach_workspace_evidence, finalize_specialist_result, provider_error_completion,
    provider_success_completion, terminal_status_for_provider_error,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MODEL: &str = "qwen2.5-coder:7b";
const WORKSPACE_ID: &str = "workspace-1";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------
// Throwaway directories
// ---------------------------------------------------------------------------

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "aacc-task-0024-{label}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scenario directory should be created");
        Self { path }
    }

    fn database_path(&self) -> PathBuf {
        self.path.join("application-state.sqlite3")
    }

    fn workspace(&self) -> PathBuf {
        let workspace = self.path.join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace directory should be created");
        workspace
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Repository fixture — active Ollama provider, one on-disk workspace, manual
// review, agent 2 (Coding Agent) as executor, reviewers on its chain (3 → 6 → 1).
// ---------------------------------------------------------------------------

fn base_repository(workspace_path: &Path) -> (StateRepository, i64) {
    let mut repository = StateRepository::open_in_memory().unwrap();
    let initialized = repository.initialize_fresh().unwrap();
    let revision = configure(&mut repository, initialized, workspace_path);
    (repository, revision)
}

fn base_repository_on_disk(database: &Path, workspace_path: &Path) -> (StateRepository, i64) {
    let mut repository = StateRepository::open(database).unwrap();
    let initialized = repository.initialize_fresh().unwrap();
    let revision = configure(&mut repository, initialized, workspace_path);
    (repository, revision)
}

fn configure(
    repository: &mut StateRepository,
    initialized: StateEnvelope,
    workspace_path: &Path,
) -> i64 {
    let mut configured = initialized.state;
    let workspace_path = workspace_path.to_string_lossy().into_owned();
    configured.preferences.active_ai_provider = "ollama".to_string();
    configured.preferences.workspaces.push(WorkspaceDefinition {
        id: WORKSPACE_ID.to_string(),
        name: "Fixture".to_string(),
        path: workspace_path.clone(),
    });
    configured.preferences.active_workspace_id = Some(WORKSPACE_ID.to_string());
    configured.preferences.workspace_path = workspace_path;
    configured.preferences.review_mode = "manual".to_string();
    for agent_id in [1, 3, 6] {
        let reviewer = configured
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .unwrap();
        reviewer.model = MODEL.to_string();
        reviewer.approvals.files = "allow".to_string();
    }
    repository
        .save(initialized.revision, &configured, true)
        .unwrap()
        .revision
}

fn coding_request(objective: &str) -> SpecialistTaskRequestV1 {
    SpecialistTaskRequestV1::Coding(CodingRequestV1 {
        schema_version: SPECIALIST_SCHEMA_VERSION,
        profile_version: SPECIALIST_PROFILE_VERSION.to_string(),
        objective: objective.to_string(),
        acceptance_criteria: vec!["The requested bounded change is verified.".to_string()],
        constraints: vec!["Preserve unrelated workspace state.".to_string()],
        mutation_classes: vec![
            WorkspaceMutationClass::Create,
            WorkspaceMutationClass::Modify,
        ],
        requested_checks: vec![],
        allow_web_research: false,
    })
}

/// Create one coding-specialist execute task owned and executed by agent 2.
fn create_task(repository: &mut StateRepository, revision: i64, title: &str) -> (i64, i64) {
    let created = repository
        .create_routed_task(
            CreateRoutedTaskRequest {
                expected_revision: revision,
                task_owner_agent_id: 2,
                title: title.to_string(),
                category: "Development".to_string(),
                priority: "Normal".to_string(),
                workspace_id: WORKSPACE_ID.to_string(),
                routing_mode: "selected".to_string(),
                preferred_agent_id: Some(2),
                selected_agent_id: Some(2),
                specialist_request: Some(coding_request(title)),
            },
            &crate::orchestration_acceptance::provider_snapshot(),
        )
        .unwrap();
    let task = created
        .state
        .agents
        .iter()
        .flat_map(|agent| &agent.tasks)
        .find(|task| task.title == title)
        .expect("created task should exist");
    (task.id, created.revision)
}

fn execute_intent(task_id: i64) -> ActionIntent {
    ActionIntent::RunTask {
        agent_id: 2,
        task_owner_agent_id: 2,
        task_id,
        run_mode: RunMode::Execute,
        review_context: None,
    }
}

fn approve(repository: &mut StateRepository, intent: &ActionIntent) {
    if let Some(pending) = repository.request_authorization(intent).unwrap().approval {
        repository
            .resolve_approval(pending.id, ApprovalResolution::Approve, true)
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Provider request builder + the real post-dispatch composition
// ---------------------------------------------------------------------------

fn provider_request(
    provider: RuntimeProviderId,
    workspace: &Path,
    run_mode: ProviderRunMode,
) -> ProviderRunRequest {
    provider_request_with_model(provider, MODEL, workspace, run_mode)
}

fn provider_request_with_model(
    provider: RuntimeProviderId,
    model: &str,
    workspace: &Path,
    run_mode: ProviderRunMode,
) -> ProviderRunRequest {
    let is_review = run_mode == ProviderRunMode::Review;
    let specialist_request =
        (!is_review).then(|| coding_request("Apply a bounded workspace change"));
    let specialist_contract = specialist_request.as_ref().map(|request| {
        SpecialistRunContractV1::for_request(
            request,
            provider.to_string(),
            model.to_string(),
            Some(7),
        )
        .expect("fixture specialist contract should build")
    });
    ProviderRunRequest {
        run_mode,
        agent_name: "Coding Agent".to_string(),
        description: "Bounded provider/review acceptance fixture.".to_string(),
        role: "Specialist".to_string(),
        category: "Development".to_string(),
        memory: String::new(),
        review_feedback: None,
        task_title: "Apply a bounded workspace change".to_string(),
        model: ProviderModelIdentity {
            catalog_model_id: 6,
            provider_id: provider,
            runtime_model: model.to_string(),
        },
        strength: 5,
        focus: "balanced".to_string(),
        enable_web_search: false,
        workspace_path: workspace.to_string_lossy().into_owned(),
        // The Coding specialist profile fixes write files + safe terminal +
        // {files, terminal} scopes; reviews are strictly read-only.
        file_access: if is_review { "read" } else { "write" }.to_string(),
        terminal_access: if is_review { "none" } else { "safe" }.to_string(),
        authorized_scopes: if is_review {
            Vec::new()
        } else {
            vec!["files".to_string(), "terminal".to_string()]
        },
        destructive_actions_approved: false,
        timeout_seconds: 120,
        specialist_request,
        specialist_contract,
    }
}

/// The `#[cfg(test)]` mirror of `run_agent_task`'s spawn_blocking worker body:
/// baseline finish → attach evidence → finalize specialist.
fn finalize_dispatch(
    run_mode: ProviderRunMode,
    specialist_request: Option<&SpecialistTaskRequestV1>,
    execution: Result<ProviderRunResult, ProviderError>,
    baseline: Option<WorkspaceEvidenceBaseline>,
) -> Result<ProviderRunResult, ProviderError> {
    let workspace_changes = match baseline {
        Some(baseline) => baseline.finish(),
        None if run_mode == ProviderRunMode::Review => WorkspaceChangeEvidenceV1::not_collected(
            "Read-only review attempts do not own workspace mutation evidence.",
        ),
        None => WorkspaceChangeEvidenceV1::legacy_unavailable(
            "The execution workspace could not be resolved for bounded evidence collection.",
        ),
    };
    attach_workspace_evidence(execution, workspace_changes)
        .and_then(|result| finalize_specialist_result(result, specialist_request))
}

/// Complete an admitted attempt from a real dispatch result the exact way
/// `run_agent_task` translates it into a `RunCompletion`.
fn complete_from_dispatch(
    repository: &mut StateRepository,
    attempt_id: i64,
    cancel_requested: bool,
    finalized: Result<ProviderRunResult, ProviderError>,
) -> RunAttemptProjection {
    match finalized {
        Ok(runtime) => repository
            .complete_run(attempt_id, &provider_success_completion(&runtime))
            .unwrap(),
        Err(error) => {
            let snapshot = repository.run_snapshot().unwrap();
            let active = snapshot
                .active_attempt
                .as_ref()
                .filter(|attempt| attempt.id == attempt_id);
            let status = terminal_status_for_provider_error(active, cancel_requested, &error);
            // In production the user's stop request moves the attempt to
            // `cancel_requested` before the provider observes the flag and
            // reports back; mirror that ordering so the transition is legal.
            if status == RunAttemptStatus::Cancelled
                && active.is_some_and(|attempt| attempt.status != RunAttemptStatus::CancelRequested)
            {
                repository.request_run_cancellation(attempt_id).unwrap();
            }
            repository
                .complete_run(attempt_id, &provider_error_completion(status, &error, 1))
                .unwrap()
        }
    }
}

fn context_with_cancel(flag: Arc<AtomicBool>) -> ProviderRunContext {
    ProviderRunContext::new(
        Arc::new(SilentObserver::default()),
        ProviderCancellation::new(flag),
    )
}

#[derive(Default)]
struct SilentObserver {
    events: Mutex<Vec<String>>,
}

impl ProviderRunObserver for SilentObserver {
    fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:{}", event.kind.as_str(), event.message));
        Ok(())
    }

    fn mark_started(&self) -> Result<(), ProviderError> {
        self.events.lock().unwrap().push("started".to_string());
        Ok(())
    }
}

/// Trips the cancel flag once the Codex turn is genuinely in flight.
struct CancelOnTurnObserver {
    flag: Arc<AtomicBool>,
}

impl ProviderRunObserver for CancelOnTurnObserver {
    fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError> {
        if event.message == "Codex turn started." {
            self.flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn mark_started(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Ollama execute / review dispatch
// ---------------------------------------------------------------------------

/// Admit + prepare + dispatch one execute attempt through a fake Ollama server.
fn execute_via_ollama(
    repository: &mut StateRepository,
    task_id: i64,
    request_id: &str,
    workspace: &Path,
    server: &FakeOllama,
) -> RunAttemptProjection {
    let intent = execute_intent(task_id);
    approve(repository, &intent);
    let admitted = repository.admit_run(request_id, &intent).unwrap();
    let attempt_id = admitted.attempt.id;
    repository
        .prepare_run_attempt(attempt_id, "Ollama", MODEL, Some(WORKSPACE_ID))
        .unwrap();
    repository.mark_run_dispatching(attempt_id).unwrap();
    repository.mark_run_started(attempt_id).unwrap();

    let request = provider_request(
        RuntimeProviderId::Ollama,
        workspace,
        ProviderRunMode::Execute,
    );
    let specialist = request.specialist_request.clone();
    let baseline = Some(WorkspaceEvidenceBaseline::begin(workspace));
    let session = OllamaSession::for_test_endpoint(&server.endpoint).unwrap();
    let execution = crate::run_ollama_task_with_session(
        context_with_cancel(Arc::new(AtomicBool::new(false))),
        request,
        session,
    );
    let finalized = finalize_dispatch(
        ProviderRunMode::Execute,
        specialist.as_ref(),
        execution,
        baseline,
    );
    complete_from_dispatch(repository, attempt_id, false, finalized)
}

// ---------------------------------------------------------------------------
// Fake Ollama HTTP transport (127.0.0.1 loopback, one bounded conversation)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum OllamaScript {
    /// One `create_workspace_file` tool call, then a final assistant message
    /// whose text is `final_text`.
    CreateThenText,
    /// Immediately return a final assistant message (no tool call).
    FinalTextOnly,
}

struct FakeOllama {
    endpoint: String,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeOllama {
    fn start(script: OllamaScript, final_text: impl Into<String>) -> Self {
        let final_text = final_text.into();
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake Ollama should bind loopback");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || serve_fake_ollama(&listener, script, &final_text));
        Self {
            endpoint,
            handle: Some(handle),
        }
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FakeOllama {
    fn drop(&mut self) {
        // The server thread self-terminates after `expected` requests or a
        // 15s idle deadline, so this never blocks even if a run stops early.
        self.join();
    }
}

fn serve_fake_ollama(listener: &TcpListener, script: OllamaScript, final_text: &str) {
    let expected = match script {
        OllamaScript::CreateThenText => 4, // tags, show, chat(tool), chat(final)
        OllamaScript::FinalTextOnly => 3,  // tags, show, chat(final)
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut chat_turn = 0_usize;
    for _ in 0..expected {
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        };
        stream
            .set_nonblocking(false)
            .expect("blocking accepted stream");
        let (_method, path, _body) = read_http_request(&stream);
        let response = match path.as_str() {
            "/api/tags" => json!({ "models": [{ "name": MODEL }] }),
            "/api/show" => json!({
                "capabilities": ["completion", "tools"],
                "model_info": {
                    "general.architecture": "fixture",
                    "fixture.context_length": 16384
                }
            }),
            "/api/chat" => {
                chat_turn += 1;
                if script == OllamaScript::CreateThenText && chat_turn == 1 {
                    json!({
                        "model": MODEL,
                        "prompt_eval_count": 9,
                        "eval_count": 4,
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "tool_calls": [{
                                "function": {
                                    "name": "create_workspace_file",
                                    "arguments": {
                                        "path": "created.txt",
                                        "content": "bounded fixture change\n"
                                    }
                                }
                            }]
                        }
                    })
                } else {
                    json!({
                        "model": MODEL,
                        "prompt_eval_count": 6,
                        "eval_count": 5,
                        "message": { "role": "assistant", "content": final_text }
                    })
                }
            }
            other => panic!("unexpected fake Ollama path: {other}"),
        };
        write_http_json(&mut stream, &response);
    }
}

fn read_http_request(stream: &TcpStream) -> (String, String, Value) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut content_length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut raw = vec![0_u8; content_length];
    reader.read_exact(&mut raw).unwrap();
    let body = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw).unwrap_or(Value::Null)
    };
    (method, path, body)
}

fn write_http_json(stream: &mut TcpStream, body: &Value) {
    let bytes = serde_json::to_vec(body).unwrap();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}

// ---------------------------------------------------------------------------
// Fake Codex CLI (reuses tests/fixtures/fake_codex.sh + real bwrap containment)
// ---------------------------------------------------------------------------

fn fake_codex_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_codex.sh")
}

fn codex_inspection(scenario: &str, marker: Option<&str>) -> CodexInspection {
    let mut environment = vec![(
        std::ffi::OsString::from("FAKE_CODEX_SCENARIO"),
        std::ffi::OsString::from(scenario),
    )];
    if let Some(marker) = marker {
        environment.push((
            std::ffi::OsString::from("FAKE_CODEX_MARKER"),
            std::ffi::OsString::from(marker),
        ));
    }
    inspect_codex_runtime_at(
        Some(fake_codex_path()),
        Some(PathBuf::from("/usr/bin/bwrap")),
        environment,
    )
}

fn codex_marker(prefix: &str) -> String {
    format!(
        "task-0024-{prefix}-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    )
}

fn codex_marker_process_exists(marker: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
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
        std::fs::read(entry.path().join("cmdline"))
            .ok()
            .is_some_and(|command| {
                command
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes())
            })
    })
}

fn codex_spec(inspection: &CodexInspection, workspace: &Path, timeout: Duration) -> CodexRunSpec {
    CodexRunSpec {
        launch: inspection
            .launch()
            .expect("fake Codex launch should be ready"),
        workspace: workspace.to_path_buf(),
        model: MODEL.to_string(),
        reasoning_effort: "medium".to_string(),
        file_access: "read".to_string(),
        terminal_access: "none".to_string(),
        enable_web_search: false,
        prompt: "Inspect the fixture workspace without recursion.".to_string(),
        timeout,
    }
}

/// One fake-Codex dispatch scenario.
struct CodexDispatch {
    scenario: &'static str,
    marker: Option<String>,
    timeout: Duration,
    /// Trip the cancel flag once the Codex turn is genuinely in flight.
    cancel_on_turn: bool,
}

/// Admit + prepare + dispatch one execute attempt through the fake Codex CLI.
fn execute_via_codex(
    repository: &mut StateRepository,
    task_id: i64,
    request_id: &str,
    workspace: &Path,
    dispatch: &CodexDispatch,
) -> RunAttemptProjection {
    let inspection = codex_inspection(dispatch.scenario, dispatch.marker.as_deref());
    assert!(
        inspection.is_ready(),
        "fake Codex inspection should be ready"
    );

    let intent = execute_intent(task_id);
    approve(repository, &intent);
    let admitted = repository.admit_run(request_id, &intent).unwrap();
    let attempt_id = admitted.attempt.id;
    repository
        .prepare_run_attempt(attempt_id, "Codex", MODEL, Some(WORKSPACE_ID))
        .unwrap();
    repository.mark_run_dispatching(attempt_id).unwrap();
    repository.mark_run_started(attempt_id).unwrap();

    let request = provider_request(
        RuntimeProviderId::Codex,
        workspace,
        ProviderRunMode::Execute,
    );
    let specialist = request.specialist_request.clone();
    let baseline = Some(WorkspaceEvidenceBaseline::begin(workspace));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let observer: Arc<dyn ProviderRunObserver> = if dispatch.cancel_on_turn {
        Arc::new(CancelOnTurnObserver {
            flag: cancel_flag.clone(),
        })
    } else {
        Arc::new(SilentObserver::default())
    };
    let context = ProviderRunContext::new(observer, ProviderCancellation::new(cancel_flag.clone()));
    let execution = run_codex(
        &context,
        codex_spec(&inspection, workspace, dispatch.timeout),
    )
    .map(|runtime| ProviderRunResult {
        provider_id: RuntimeProviderId::Codex,
        output: runtime.output,
        response_id: runtime.response_id,
        model: MODEL.to_string(),
        usage: runtime.usage,
        changed_files: Vec::new(),
        diff: None,
        duration_seconds: 1,
        evidence: runtime.evidence,
        specialist_result: None,
    });
    let finalized = finalize_dispatch(
        ProviderRunMode::Execute,
        specialist.as_ref(),
        execution,
        baseline,
    );
    complete_from_dispatch(
        repository,
        attempt_id,
        cancel_flag.load(Ordering::SeqCst),
        finalized,
    )
}

// ---------------------------------------------------------------------------
// Review helpers
// ---------------------------------------------------------------------------

fn coding_result_json(summary: &str) -> String {
    serde_json::to_string(&SpecialistResultV1::Coding(CodingResultV1 {
        summary: summary.to_string(),
        changes: vec!["created.txt".to_string()],
        verification: vec![],
        evidence_refs: vec![],
        limitations: vec![],
    }))
    .unwrap()
}

fn review_result_json(request_json: &str, verdict: ReviewVerdict) -> String {
    let request: ReviewRequestV1 = serde_json::from_str(request_json).unwrap();
    serde_json::to_string(&ReviewResultV1 {
        schema_version: 1,
        flow_id: request.flow_id,
        task_id: request.subject.task_id,
        revision_round: request.revision_round,
        level: request.level,
        stage_attempt_id: request.stage_attempt_id,
        request_fingerprint: request.request_fingerprint,
        verdict,
        checks: REQUIRED_REVIEW_CHECKS
            .iter()
            .copied()
            .map(|check| ReviewCheckResultV1 {
                check,
                status: if verdict == ReviewVerdict::Approved {
                    ReviewCheckStatus::Pass
                } else if check == ReviewCheckKind::Correctness {
                    ReviewCheckStatus::Fail
                } else {
                    ReviewCheckStatus::Pass
                },
                evidence_ids: vec!["execution.summary".to_string()],
                finding: "Bounded evidence inspected.".to_string(),
            })
            .collect(),
        blocking_issues: if verdict == ReviewVerdict::Approved {
            Vec::new()
        } else {
            vec!["The boundary behavior needs another revision.".to_string()]
        },
        feedback: if verdict == ReviewVerdict::Approved {
            String::new()
        } else {
            "Correct the boundary behavior and rerun verification.".to_string()
        },
    })
    .unwrap()
}

/// Run one agent review stage: start it, dispatch the reviewer run through the
/// fake Ollama transport with a review-result payload, and complete it.
fn agent_review_stage(
    repository: &mut StateRepository,
    task_id: i64,
    request_id: &str,
    verdict: ReviewVerdict,
    workspace: &Path,
) -> i64 {
    let snapshot = repository.review_orchestration_snapshot().unwrap();
    let start = repository
        .start_review_stage(
            StartReviewStageRequest {
                expected_revision: snapshot.revision,
                task_owner_agent_id: 2,
                task_id,
            },
            &crate::orchestration_acceptance::provider_snapshot(),
        )
        .unwrap();
    let context = start
        .context
        .clone()
        .expect("agent review stage should be pending");
    let reviewer_agent_id = start.stage.as_ref().unwrap().reviewer_agent_id.unwrap();
    let intent = ActionIntent::RunTask {
        agent_id: reviewer_agent_id,
        task_owner_agent_id: 2,
        task_id,
        run_mode: RunMode::Review,
        review_context: Some(context),
    };
    approve(repository, &intent);
    let admitted = repository.admit_run(request_id, &intent).unwrap();
    let attempt_id = admitted.attempt.id;
    let payload = review_result_json(admitted.review_request_json.as_deref().unwrap(), verdict);

    repository
        .prepare_run_attempt(attempt_id, "Ollama", MODEL, Some(WORKSPACE_ID))
        .unwrap();
    repository.mark_run_dispatching(attempt_id).unwrap();
    repository.mark_run_started(attempt_id).unwrap();

    let mut server = FakeOllama::start(OllamaScript::FinalTextOnly, payload);
    let request = provider_request(
        RuntimeProviderId::Ollama,
        workspace,
        ProviderRunMode::Review,
    );
    let session = OllamaSession::for_test_endpoint(&server.endpoint).unwrap();
    let execution = crate::run_ollama_task_with_session(
        context_with_cancel(Arc::new(AtomicBool::new(false))),
        request,
        session,
    );
    server.join();
    let finalized = finalize_dispatch(ProviderRunMode::Review, None, execution, None);
    let completed = complete_from_dispatch(repository, attempt_id, false, finalized);
    assert_eq!(completed.status, RunAttemptStatus::Succeeded);
    reviewer_agent_id
}

fn queue_sequence(repository: &mut StateRepository, task_id: i64) -> Option<i64> {
    let snapshot = repository.task_orchestration_snapshot().unwrap();
    snapshot
        .execute_queue
        .iter()
        .chain(snapshot.held_tasks.iter())
        .find(|entry| entry.task_id == task_id)
        .map(|entry| entry.enqueue_sequence)
}

// ===========================================================================
// S5.1 — Ollama execute dispatch: bounded workspace evidence + specialist finalize
// ===========================================================================

#[test]
fn s5_ollama_execute_dispatch_attaches_bounded_evidence_and_opens_review() {
    let directory = TestDirectory::new("ollama-execute");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Create fixture file");

    let mut server = FakeOllama::start(
        OllamaScript::CreateThenText,
        coding_result_json("Created the bounded fixture file and verified it exists."),
    );
    let completed = execute_via_ollama(
        &mut repository,
        task_id,
        "s5-ollama-execute",
        &workspace,
        &server,
    );
    server.join();

    assert_eq!(completed.status, RunAttemptStatus::Succeeded);
    assert_eq!(completed.changed_files, ["created.txt"]);
    assert!(completed.specialist_result.is_some());
    assert_eq!(
        std::fs::read_to_string(workspace.join("created.txt")).unwrap(),
        "bounded fixture change\n"
    );

    let run = repository.run_snapshot().unwrap();
    assert!(run.active_attempt.is_none());
    assert_eq!(run.recent_attempts[0].id, completed.id);
    assert_eq!(run.recent_attempts[0].changed_files, ["created.txt"]);

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows.len(), 1);
    assert_eq!(review.flows[0].state, "awaiting_review");
    assert_eq!(
        review.flows[0].latest_execution_attempt_id,
        Some(completed.id)
    );
}

// ===========================================================================
// S5.2 — Ollama execute cancellation: terminal `cancelled`, evidence retained
// ===========================================================================

#[test]
fn s5_ollama_execute_cancellation_is_terminal_with_retained_evidence() {
    let directory = TestDirectory::new("ollama-cancel");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Cancelled Ollama run");

    let intent = execute_intent(task_id);
    approve(&mut repository, &intent);
    let admitted = repository.admit_run("s5-ollama-cancel", &intent).unwrap();
    let attempt_id = admitted.attempt.id;
    repository
        .prepare_run_attempt(attempt_id, "Ollama", MODEL, Some(WORKSPACE_ID))
        .unwrap();
    repository.mark_run_dispatching(attempt_id).unwrap();
    repository.request_run_cancellation(attempt_id).unwrap();

    // The cancel flag is already set, so the Ollama transport returns before it
    // opens a socket — the unreachable endpoint is never contacted.
    let request = provider_request(
        RuntimeProviderId::Ollama,
        &workspace,
        ProviderRunMode::Execute,
    );
    let specialist = request.specialist_request.clone();
    let session = OllamaSession::for_test_endpoint("http://127.0.0.1:9/").unwrap();
    let execution = crate::run_ollama_task_with_session(
        context_with_cancel(Arc::new(AtomicBool::new(true))),
        request,
        session,
    );
    assert_eq!(
        execution.as_ref().unwrap_err().code,
        ProviderErrorCode::Cancelled
    );

    let finalized = finalize_dispatch(
        ProviderRunMode::Execute,
        specialist.as_ref(),
        execution,
        Some(WorkspaceEvidenceBaseline::begin(&workspace)),
    );
    let completed = complete_from_dispatch(&mut repository, attempt_id, true, finalized);

    assert_eq!(completed.status, RunAttemptStatus::Cancelled);
    assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
    assert!(repository
        .review_orchestration_snapshot()
        .unwrap()
        .flows
        .is_empty());
}

// ===========================================================================
// S5.3 — Codex execute cancellation: no owned process, truthful terminal evidence
// ===========================================================================

#[test]
fn s5_codex_execute_cancellation_leaves_no_owned_process() {
    let directory = TestDirectory::new("codex-cancel");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Cancelled Codex run");

    let marker = codex_marker("cancel");
    let completed = execute_via_codex(
        &mut repository,
        task_id,
        "s5-codex-cancel",
        &workspace,
        &CodexDispatch {
            scenario: "cancel",
            marker: Some(marker.clone()),
            timeout: Duration::from_secs(4),
            cancel_on_turn: true,
        },
    );

    assert_eq!(completed.status, RunAttemptStatus::Cancelled);
    assert!(
        !codex_marker_process_exists(&marker),
        "no owned process may survive cancellation"
    );
    assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
}

// ===========================================================================
// S5.4 — Codex execute timeout: terminal `timed_out` after containment cleanup
// ===========================================================================

#[test]
fn s5_codex_execute_timeout_is_terminal_after_cleanup() {
    let directory = TestDirectory::new("codex-timeout");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Timed-out Codex run");

    let marker = codex_marker("timeout");
    let completed = execute_via_codex(
        &mut repository,
        task_id,
        "s5-codex-timeout",
        &workspace,
        &CodexDispatch {
            scenario: "timeout",
            marker: Some(marker.clone()),
            timeout: Duration::from_millis(250),
            cancel_on_turn: false,
        },
    );

    assert_eq!(completed.status, RunAttemptStatus::TimedOut);
    assert!(!codex_marker_process_exists(&marker));
    assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
}

// ===========================================================================
// S5.5 — Codex execute success composes through evidence + specialist finalize
// ===========================================================================

#[test]
fn s5_codex_execute_success_records_usage_and_opens_review() {
    let directory = TestDirectory::new("codex-success");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Successful Codex run");

    let completed = execute_via_codex(
        &mut repository,
        task_id,
        "s5-codex-success",
        &workspace,
        &CodexDispatch {
            scenario: "success_coding",
            marker: None,
            timeout: Duration::from_secs(4),
            cancel_on_turn: false,
        },
    );

    assert_eq!(completed.status, RunAttemptStatus::Succeeded);
    assert!(completed.specialist_result.is_some());
    assert_eq!(completed.response_id.as_deref(), Some("thread-fixture"));
    assert_eq!(completed.usage.total_tokens, Some(18));

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "awaiting_review");
}

// ===========================================================================
// S5.6 — review verdict parsed from provider output requeues for revision
// ===========================================================================

#[test]
fn s5_agent_review_changes_verdict_requeues_with_fresh_sequence_and_approval() {
    let directory = TestDirectory::new("review-revision");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Reviewed change");
    let original_sequence = queue_sequence(&mut repository, task_id).expect("task is queued");

    let mut server = FakeOllama::start(
        OllamaScript::CreateThenText,
        coding_result_json("Implemented the bounded change."),
    );
    let execution = execute_via_ollama(
        &mut repository,
        task_id,
        "s5-review-execute",
        &workspace,
        &server,
    );
    server.join();
    assert_eq!(execution.status, RunAttemptStatus::Succeeded);

    let reviewer = agent_review_stage(
        &mut repository,
        task_id,
        "s5-review-changes",
        ReviewVerdict::ChangesRequested,
        &workspace,
    );
    assert_eq!(
        reviewer, 3,
        "a Specialist result is reviewed by the Senior first"
    );

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "revision_queued");
    assert_eq!(review.flows[0].revision_round, 1);

    let revision_sequence = queue_sequence(&mut repository, task_id).expect("revision is requeued");
    assert!(
        revision_sequence > original_sequence,
        "a changes verdict must requeue with a fresh sequence"
    );
    let revision_intent = execute_intent(task_id);
    assert!(
        repository
            .admit_run("s5-review-revision-noapproval", &revision_intent)
            .is_err(),
        "the revision execution must require a fresh approval"
    );
}

// ===========================================================================
// S5.7 — the revision requeue cycle: re-execute returns the flow to review
// ===========================================================================

#[test]
fn s5_revision_reexecute_returns_flow_to_awaiting_review() {
    let directory = TestDirectory::new("review-cycle");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) = create_task(&mut repository, revision, "Revision cycle");

    let mut server = FakeOllama::start(
        OllamaScript::CreateThenText,
        coding_result_json("Implemented the bounded change."),
    );
    execute_via_ollama(
        &mut repository,
        task_id,
        "s5-cycle-execute-0",
        &workspace,
        &server,
    );
    server.join();

    agent_review_stage(
        &mut repository,
        task_id,
        "s5-cycle-review-0",
        ReviewVerdict::ChangesRequested,
        &workspace,
    );
    assert_eq!(
        repository.review_orchestration_snapshot().unwrap().flows[0].state,
        "revision_queued"
    );

    let mut server = FakeOllama::start(
        OllamaScript::CreateThenText,
        coding_result_json("Corrected the boundary behavior."),
    );
    let revised = execute_via_ollama(
        &mut repository,
        task_id,
        "s5-cycle-execute-1",
        &workspace,
        &server,
    );
    server.join();
    assert_eq!(revised.status, RunAttemptStatus::Succeeded);

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "awaiting_review");
    assert_eq!(review.flows[0].revision_round, 1);
    assert_eq!(
        review.flows[0].latest_execution_attempt_id,
        Some(revised.id)
    );
}

// ===========================================================================
// S5.8 — specialist result kind mismatch fails the run, evidence retained,
//         no review flow opened
// ===========================================================================

#[test]
fn s5_specialist_result_mismatch_fails_run_and_retains_evidence() {
    let directory = TestDirectory::new("specialist-mismatch");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (task_id, _revision) =
        create_task(&mut repository, revision, "Mismatched specialist result");

    // The model creates the file but returns a plain sentence, not a CodingResultV1.
    let mut server = FakeOllama::start(
        OllamaScript::CreateThenText,
        "All done — the file is there.",
    );
    let completed = execute_via_ollama(
        &mut repository,
        task_id,
        "s5-specialist-mismatch",
        &workspace,
        &server,
    );
    server.join();

    assert_eq!(completed.status, RunAttemptStatus::Failed);
    assert_eq!(
        completed.error_code.as_deref(),
        Some("PROVIDER_PROTOCOL_ERROR")
    );
    assert!(completed.workspace_changes.summary.total_changes >= 1);
    assert!(
        repository
            .review_orchestration_snapshot()
            .unwrap()
            .flows
            .is_empty(),
        "a failed execution does not open a review flow"
    );
}

// ===========================================================================
// S5.9 — one active run blocks a second dispatch across execute and review
// ===========================================================================

#[test]
fn s5_one_active_run_blocks_second_dispatch_after_real_prepare() {
    let directory = TestDirectory::new("one-active");
    let workspace = directory.workspace();
    let (mut repository, revision) = base_repository(&workspace);
    let (first_id, revision) = create_task(&mut repository, revision, "First task");
    let (second_id, _revision) = create_task(&mut repository, revision, "Second task");

    let first_intent = execute_intent(first_id);
    approve(&mut repository, &first_intent);
    let admitted = repository
        .admit_run("s5-active-first", &first_intent)
        .unwrap();
    repository
        .prepare_run_attempt(admitted.attempt.id, "Ollama", MODEL, Some(WORKSPACE_ID))
        .unwrap();
    repository
        .mark_run_dispatching(admitted.attempt.id)
        .unwrap();

    let second_intent = execute_intent(second_id);
    approve(&mut repository, &second_intent);
    assert_eq!(
        repository
            .admit_run("s5-active-second", &second_intent)
            .unwrap_err()
            .code,
        "RUN_BUSY"
    );
}

// ===========================================================================
// S5.10 — restart after a real dispatch interruption escalates to manual review
// ===========================================================================

#[test]
fn s5_restart_after_dispatch_interruption_requires_manual_review() {
    let directory = TestDirectory::new("restart");
    let database = directory.database_path();
    let workspace = directory.workspace();

    let head_sequence;
    {
        let (mut repository, revision) = base_repository_on_disk(&database, &workspace);
        let (head_id, revision) = create_task(&mut repository, revision, "Interrupted head");
        let (_later, _revision) = create_task(&mut repository, revision, "Waiting task");
        head_sequence = queue_sequence(&mut repository, head_id).unwrap();

        let intent = execute_intent(head_id);
        approve(&mut repository, &intent);
        let admitted = repository.admit_run("s5-restart-head", &intent).unwrap();
        repository
            .prepare_run_attempt(admitted.attempt.id, "Ollama", MODEL, Some(WORKSPACE_ID))
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        // process exits mid-dispatch — outcome uncertain
    }

    let mut reopened = StateRepository::open(&database).unwrap();
    let run = reopened.run_snapshot().unwrap();
    assert_eq!(run.recent_attempts[0].status, RunAttemptStatus::Interrupted);
    assert_eq!(
        run.recent_attempts[0].recovery_disposition.as_deref(),
        Some("manual_review_required")
    );
    assert!(run.active_attempt.is_none());

    let orchestration = reopened.task_orchestration_snapshot().unwrap();
    assert!(orchestration
        .execute_queue
        .iter()
        .chain(orchestration.held_tasks.iter())
        .any(|entry| entry.enqueue_sequence == head_sequence));
}

// ===========================================================================
// Live provider / review acceptance (S5) — `#[ignore]`d.
//
// Run against the real Codex CLI and the real local Ollama service:
//   cargo test --manifest-path src-tauri/Cargo.toml --lib \
//     provider_review_acceptance::live -- --ignored --test-threads=1 --nocapture
//
// Every scenario uses a throwaway `/tmp` workspace; nothing touches the real
// application state. These are excluded from the deterministic gate.
// ===========================================================================

#[cfg(test)]
mod live {
    use super::*;
    use std::process::Command;

    const CODEX_MODEL: &str = "gpt-5.6-sol";

    struct RecordingObserver {
        events: Mutex<Vec<String>>,
        cancel_on_turn: Option<Arc<AtomicBool>>,
    }

    impl RecordingObserver {
        fn new(cancel_on_turn: Option<Arc<AtomicBool>>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                cancel_on_turn,
            }
        }

        fn log(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ProviderRunObserver for RecordingObserver {
        fn emit(&self, event: ProviderRunEvent) -> Result<(), ProviderError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}: {}", event.kind.as_str(), event.message));
            if event.message == "Codex turn started." {
                if let Some(flag) = &self.cancel_on_turn {
                    flag.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        }

        fn mark_started(&self) -> Result<(), ProviderError> {
            self.events.lock().unwrap().push("started".to_string());
            Ok(())
        }
    }

    fn live_workspace(label: &str) -> (TestDirectory, PathBuf) {
        let directory = TestDirectory::new(&format!("live-{label}"));
        let workspace = directory.workspace();
        std::fs::write(
            workspace.join("main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        (directory, workspace)
    }

    fn bwrap_count() -> usize {
        Command::new("pgrep")
            .args(["-x", "bwrap"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).lines().count())
            .unwrap_or(0)
    }

    #[test]
    #[ignore = "live: requires the real Codex CLI + local Ollama"]
    fn live_codex_identity_and_ollama_model_are_available() {
        let codex = crate::codex_runtime::inspect_codex_runtime();
        eprintln!(
            "codex: installed={} authenticated={} compatible={} version={:?}\n  message: {}",
            codex.installed, codex.authenticated, codex.compatible, codex.version, codex.message
        );
        assert!(
            codex.is_ready(),
            "the Codex runtime must be ready for the live batch"
        );

        let ollama = crate::ollama_runtime::inspect_ollama_runtime();
        eprintln!(
            "ollama: connected={} version={:?} catalog_ready={}\n  message: {}",
            ollama.connected, ollama.version, ollama.catalog_ready, ollama.message
        );
        assert!(
            ollama.connected,
            "the local Ollama service must be reachable"
        );
        let model = ollama
            .models
            .iter()
            .find(|model| model.name == MODEL)
            .unwrap_or_else(|| panic!("{MODEL} must be installed locally for the live batch"));
        eprintln!(
            "  {MODEL}: availability={:?} capabilities={:?} context={:?}",
            model.availability, model.capabilities, model.context_length
        );
        assert!(
            model
                .capabilities
                .iter()
                .any(|capability| capability == "tools"),
            "{MODEL} must report tool support"
        );
    }

    #[test]
    #[ignore = "live: requires the real Codex CLI"]
    fn live_codex_bounded_read_run_reports_truthful_zero_change_evidence() {
        let (_directory, workspace) = live_workspace("codex-read");
        let mut request = provider_request_with_model(
            RuntimeProviderId::Codex,
            CODEX_MODEL,
            &workspace,
            ProviderRunMode::Execute,
        );
        // A plain, read-only inspection run — no specialist contract, no writes.
        request.specialist_request = None;
        request.specialist_contract = None;
        request.file_access = "read".to_string();
        request.terminal_access = "none".to_string();
        request.authorized_scopes = vec!["files".to_string()];
        request.task_title =
            "List the files in this workspace and describe main.rs in one sentence.".to_string();
        request.timeout_seconds = 120;

        let observer = Arc::new(RecordingObserver::new(None));
        let context = ProviderRunContext::new(
            observer.clone(),
            ProviderCancellation::new(Arc::new(AtomicBool::new(false))),
        );
        let baseline = WorkspaceEvidenceBaseline::begin(&workspace);
        let result = crate::run_codex_task(context, request);
        let evidence = baseline.finish();

        eprintln!("codex events:\n  {}", observer.log().join("\n  "));
        match result {
            Ok(runtime) => {
                eprintln!(
                    "codex ok: model={} response_id={:?} usage={:?}\n  output: {}",
                    runtime.model,
                    runtime.response_id,
                    runtime.usage,
                    runtime.output.trim()
                );
                assert!(
                    !runtime.output.trim().is_empty(),
                    "a completed run has output"
                );
            }
            Err(error) => panic!(
                "live Codex read run failed: {} / {}",
                error.code.as_str(),
                error.message
            ),
        }
        eprintln!(
            "workspace evidence: mode={:?} status={:?} reviewability={:?} total_changes={}",
            evidence.mode, evidence.status, evidence.reviewability, evidence.summary.total_changes
        );
        assert_eq!(
            evidence.summary.total_changes, 0,
            "a read-only Codex run must not change the workspace"
        );
    }

    #[test]
    #[ignore = "live: requires the real Codex CLI"]
    fn live_codex_cancellation_leaves_no_owned_process() {
        let (_directory, workspace) = live_workspace("codex-cancel");
        let mut request = provider_request_with_model(
            RuntimeProviderId::Codex,
            CODEX_MODEL,
            &workspace,
            ProviderRunMode::Execute,
        );
        request.specialist_request = None;
        request.specialist_contract = None;
        request.file_access = "read".to_string();
        request.terminal_access = "none".to_string();
        request.authorized_scopes = vec!["files".to_string()];
        request.task_title =
            "Carefully audit every file in this workspace line by line and write a long report."
                .to_string();
        request.timeout_seconds = 120;

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let observer = Arc::new(RecordingObserver::new(Some(cancel_flag.clone())));
        let context =
            ProviderRunContext::new(observer.clone(), ProviderCancellation::new(cancel_flag));

        let before = bwrap_count();
        let started = std::time::Instant::now();
        let result = crate::run_codex_task(context, request);
        let elapsed = started.elapsed();
        eprintln!("codex events:\n  {}", observer.log().join("\n  "));

        let error = result.expect_err("a cancelled Codex run must return an error");
        assert_eq!(
            error.code,
            ProviderErrorCode::Cancelled,
            "cancellation must be a typed Cancelled error, got {}",
            error.message
        );
        assert!(
            elapsed < Duration::from_secs(30),
            "cancellation must be prompt (took {elapsed:?})"
        );

        // The run kills its own process group and requires namespace cleanup
        // before reporting cancellation, so the bwrap count returns to baseline.
        let mut settled = false;
        for _ in 0..20 {
            if bwrap_count() <= before {
                settled = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            settled,
            "no bwrap process may survive a cancelled Codex run (before={before}, after={})",
            bwrap_count()
        );
    }

    #[test]
    #[ignore = "live: requires local Ollama + qwen2.5-coder:7b"]
    fn live_ollama_bounded_coding_run_terminates_with_typed_evidence() {
        let (_directory, workspace) = live_workspace("ollama-coding");
        let mut request = provider_request_with_model(
            RuntimeProviderId::Ollama,
            MODEL,
            &workspace,
            ProviderRunMode::Execute,
        );
        request.task_title =
            "Create a new file named greeting.txt whose only content is the word hello, then stop."
                .to_string();
        request.timeout_seconds = 300;

        let specialist = request.specialist_request.clone();
        let observer = Arc::new(RecordingObserver::new(None));
        let context = ProviderRunContext::new(
            observer.clone(),
            ProviderCancellation::new(Arc::new(AtomicBool::new(false))),
        );
        let baseline = WorkspaceEvidenceBaseline::begin(&workspace);
        let started = std::time::Instant::now();
        let execution = crate::run_ollama_task(context, request);
        let elapsed = started.elapsed();
        let finalized = finalize_dispatch(
            ProviderRunMode::Execute,
            specialist.as_ref(),
            execution,
            Some(baseline),
        );

        eprintln!(
            "ollama coding run finished in {elapsed:?}\n  events:\n  {}",
            observer.log().join("\n  ")
        );
        match finalized {
            Ok(runtime) => eprintln!(
                "ollama ok: model={} changed_files={:?}\n  evidence: mode={:?} status={:?} total_changes={}\n  output: {}",
                runtime.model,
                runtime.changed_files,
                runtime.evidence.workspace_changes.mode,
                runtime.evidence.workspace_changes.status,
                runtime.evidence.workspace_changes.summary.total_changes,
                runtime.output.trim()
            ),
            Err(error) => eprintln!(
                "ollama typed failure: {} / {}\n  evidence total_changes={}",
                error.code.as_str(),
                error.message,
                error.evidence.workspace_changes.summary.total_changes
            ),
        }
        assert!(
            elapsed < Duration::from_secs(300),
            "the bounded Ollama run must terminate within its deadline"
        );
    }

    #[test]
    #[ignore = "live: requires local Ollama + qwen2.5-coder:7b"]
    fn live_ollama_cancellation_is_prompt_and_typed() {
        let (_directory, workspace) = live_workspace("ollama-cancel");
        let mut request = provider_request_with_model(
            RuntimeProviderId::Ollama,
            MODEL,
            &workspace,
            ProviderRunMode::Execute,
        );
        request.task_title = "Write an extremely detailed multi-section refactor plan.".to_string();
        request.timeout_seconds = 300;

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let observer = Arc::new(RecordingObserver::new(None));
        let context = ProviderRunContext::new(
            observer.clone(),
            ProviderCancellation::new(cancel_flag.clone()),
        );

        let worker = std::thread::spawn(move || crate::run_ollama_task(context, request));
        std::thread::sleep(Duration::from_millis(1500));
        cancel_flag.store(true, Ordering::SeqCst);
        let started = std::time::Instant::now();
        let result = worker
            .join()
            .expect("the Ollama worker thread must not panic");
        let elapsed = started.elapsed();

        eprintln!(
            "ollama cancel: settled in {elapsed:?} after the flag was set\n  events:\n  {}",
            observer.log().join("\n  ")
        );
        let error = result.expect_err("a cancelled Ollama run must return an error");
        assert_eq!(error.code, ProviderErrorCode::Cancelled);
        assert!(
            elapsed < Duration::from_secs(20),
            "the Ollama transport must abort and await the request task promptly"
        );
    }
}
