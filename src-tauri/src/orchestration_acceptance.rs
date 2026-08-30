//! TASK-0023 — composed live-orchestration/policy acceptance (S4).
//!
//! These `#[cfg(test)]` scenarios wire the whole orchestration/policy path end to
//! end over a real [`StateRepository`] — agent registry, deterministic routing,
//! the one global execute queue, one-active-run admission, authoritative
//! approvals/policy, the sequential review pipeline, human adjudication, and
//! restart reconciliation — and assert that the renderer-facing snapshot
//! projections stay mutually consistent with backend truth at every transition.
//!
//! Each subsystem is already unit-tested in isolation (`task_0009_*`,
//! `task_0010_*`, `task_0011_*`, `task_0005_*`); this module is the composed
//! matrix plus the regression home for defects the composition exposes.
//! Post-dispatch provider execution transport (real Codex/Ollama I/O) is owned by
//! TASK-0024: the dispatch boundary is driven with synthetic completions,
//! exactly as the real provider layer reports them.

use crate::agent_registry::{CreateAgentRequest, DeleteAgentRequest, UpdateAgentRequest};
use crate::app_state::WorkspaceDefinition;
use crate::authorization::{ApprovalResolution, AuthorizationDecision};
use crate::persistence::{StateEnvelope, StateRepository};
use crate::policy::{ActionIntent, RunMode};
use crate::provider_runtime::{
    catalog_provider_bindings, ollama_descriptor, ProviderAvailability, ProviderRegistrySnapshot,
    ProviderRuntimeModel, ProviderRuntimeStatus,
};
use crate::review_orchestration::{
    HumanReviewDecisionRequest, ReviewCheckKind, ReviewCheckResultV1, ReviewCheckStatus,
    ReviewRequestV1, ReviewResultV1, ReviewStageStart, ReviewVerdict, StartReviewStageRequest,
    REQUIRED_REVIEW_CHECKS,
};
use crate::run_coordinator::{
    RunAttemptProjection, RunAttemptStatus, RunCompletion, RunTruncationEvidence, RunUsage,
};
use crate::specialist_capabilities::{
    CodingRequestV1, CodingResultV1, SpecialistResultV1, SpecialistTaskRequestV1,
    WorkspaceMutationClass, SPECIALIST_PROFILE_VERSION, SPECIALIST_SCHEMA_VERSION,
};
use crate::task_orchestration::{
    CreateRoutedTaskRequest, QueueDisposition, RerouteTaskRequest, SetTaskQueueDispositionRequest,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const REVIEW_MODEL: &str = "qwen2.5-coder:7b";
const WORKSPACE_ID: &str = "workspace-1";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("aacc-task-0023-{}-{sequence}", std::process::id()));
        std::fs::create_dir(&path).expect("scenario directory should be created");
        Self { path }
    }

    fn database_path(&self) -> PathBuf {
        self.path.join("application-state.sqlite3")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn provider_snapshot() -> ProviderRegistrySnapshot {
    ProviderRegistrySnapshot {
        providers: vec![ProviderRuntimeStatus {
            provider: ollama_descriptor(),
            availability: ProviderAvailability::Ready,
            version: Some("fixture".to_string()),
            models: vec![ProviderRuntimeModel {
                name: REVIEW_MODEL.to_string(),
                capabilities: vec!["completion".to_string(), "tools".to_string()],
                context_length: Some(32_768),
                availability: ProviderAvailability::Ready,
                message: "Ready".to_string(),
            }],
            message: "Ready".to_string(),
        }],
        catalog_bindings: catalog_provider_bindings(),
    }
}

fn coding_request(objective: &str) -> SpecialistTaskRequestV1 {
    SpecialistTaskRequestV1::Coding(CodingRequestV1 {
        schema_version: SPECIALIST_SCHEMA_VERSION,
        profile_version: SPECIALIST_PROFILE_VERSION.to_string(),
        objective: objective.to_string(),
        acceptance_criteria: vec!["The requested bounded change is verified.".to_string()],
        constraints: vec!["Preserve unrelated workspace state.".to_string()],
        mutation_classes: vec![WorkspaceMutationClass::Modify],
        requested_checks: vec![],
        allow_web_research: false,
    })
}

fn successful_completion(output: &str) -> RunCompletion {
    RunCompletion {
        status: RunAttemptStatus::Succeeded,
        output_summary: Some(output.to_string()),
        stderr_excerpt: None,
        response_id: Some("response-1".to_string()),
        runtime_model: Some(REVIEW_MODEL.to_string()),
        usage: RunUsage {
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
        },
        changed_files: Vec::new(),
        diff: None,
        workspace_changes:
            crate::workspace_evidence::WorkspaceChangeEvidenceV1::complete_without_changes(
                crate::workspace_evidence::WorkspaceEvidenceMode::Filesystem,
            ),
        specialist_result: Some(SpecialistResultV1::Coding(CodingResultV1 {
            summary: "The bounded fixture change completed.".to_string(),
            changes: vec![],
            verification: vec![],
            evidence_refs: vec![],
            limitations: vec![],
        })),
        duration_seconds: 3,
        error_code: None,
        error_message: None,
        truncation: RunTruncationEvidence::default(),
        recovery_disposition: None,
    }
}

fn review_completion(output: &str) -> RunCompletion {
    RunCompletion {
        changed_files: Vec::new(),
        diff: None,
        specialist_result: None,
        ..successful_completion(output)
    }
}

/// Fresh in-memory repository with an active Ollama provider, one workspace,
/// manual review, and eligible reviewers on agent 2's reporting chain
/// (Senior 3, Team Leader 6, Supervisor 1).
fn base_repository() -> (StateRepository, i64) {
    let mut repository = StateRepository::open_in_memory().unwrap();
    let initialized = repository.initialize_fresh().unwrap();
    let revision = configure(&mut repository, initialized);
    (repository, revision)
}

fn base_repository_on_disk(path: &Path) -> (StateRepository, i64) {
    let mut repository = StateRepository::open(path).unwrap();
    let initialized = repository.initialize_fresh().unwrap();
    let revision = configure(&mut repository, initialized);
    (repository, revision)
}

fn configure(repository: &mut StateRepository, initialized: StateEnvelope) -> i64 {
    let mut configured = initialized.state;
    configured.preferences.active_ai_provider = "ollama".to_string();
    configured.preferences.workspaces.push(WorkspaceDefinition {
        id: WORKSPACE_ID.to_string(),
        name: "Fixture".to_string(),
        path: "/tmp/aacc-task-0023-fixture".to_string(),
    });
    configured.preferences.active_workspace_id = Some(WORKSPACE_ID.to_string());
    configured.preferences.workspace_path = "/tmp/aacc-task-0023-fixture".to_string();
    configured.preferences.review_mode = "manual".to_string();
    for agent_id in [1, 3, 6] {
        let reviewer = configured
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .unwrap();
        reviewer.model = REVIEW_MODEL.to_string();
        reviewer.approvals.files = "allow".to_string();
    }
    repository
        .save(initialized.revision, &configured, true)
        .unwrap()
        .revision
}

/// Create an execute task owned and executed by agent 2 (Coding Agent) via an
/// explicit selection, matching the review-pipeline fixtures.
fn create_agent2_task(repository: &mut StateRepository, revision: i64, title: &str) -> (i64, i64) {
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
            &provider_snapshot(),
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

fn approve(repository: &mut StateRepository, intent: &ActionIntent) -> i64 {
    let pending = repository
        .request_authorization(intent)
        .unwrap()
        .approval
        .expect("an elevated run intent should require an approval");
    repository
        .resolve_approval(pending.id, ApprovalResolution::Approve, true)
        .unwrap();
    pending.id
}

/// Drive one full execute attempt through the dispatch boundary to a successful
/// terminal completion, the way the real provider layer reports it.
fn execute(
    repository: &mut StateRepository,
    task_id: i64,
    request_id: &str,
) -> RunAttemptProjection {
    let intent = execute_intent(task_id);
    approve(repository, &intent);
    let admitted = repository.admit_run(request_id, &intent).unwrap();
    repository
        .prepare_run_attempt(
            admitted.attempt.id,
            "Ollama",
            REVIEW_MODEL,
            Some(WORKSPACE_ID),
        )
        .unwrap();
    repository
        .mark_run_dispatching(admitted.attempt.id)
        .unwrap();
    repository.mark_run_started(admitted.attempt.id).unwrap();
    repository
        .complete_run(
            admitted.attempt.id,
            &successful_completion("Implemented and verified the bounded change."),
        )
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
                finding: "Bound evidence inspected.".to_string(),
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

/// Run one agent review stage to a terminal verdict and return the stage start.
fn review_stage(
    repository: &mut StateRepository,
    task_id: i64,
    request_id: &str,
    verdict: ReviewVerdict,
) -> ReviewStageStart {
    let snapshot = repository.review_orchestration_snapshot().unwrap();
    let start = repository
        .start_review_stage(
            StartReviewStageRequest {
                expected_revision: snapshot.revision,
                task_owner_agent_id: 2,
                task_id,
            },
            &provider_snapshot(),
        )
        .unwrap();
    let context = start.context.clone().unwrap();
    let reviewer_agent_id = start.stage.as_ref().unwrap().reviewer_agent_id.unwrap();
    let intent = ActionIntent::RunTask {
        agent_id: reviewer_agent_id,
        task_owner_agent_id: 2,
        task_id,
        run_mode: RunMode::Review,
        review_context: Some(context),
    };
    assert_eq!(
        repository.request_authorization(&intent).unwrap().decision,
        AuthorizationDecision::Allowed
    );
    let admitted = repository.admit_run(request_id, &intent).unwrap();
    let output = review_result_json(admitted.review_request_json.as_deref().unwrap(), verdict);
    repository
        .prepare_run_attempt(
            admitted.attempt.id,
            "Ollama",
            REVIEW_MODEL,
            Some(WORKSPACE_ID),
        )
        .unwrap();
    repository
        .mark_run_dispatching(admitted.attempt.id)
        .unwrap();
    repository.mark_run_started(admitted.attempt.id).unwrap();
    repository
        .complete_run(admitted.attempt.id, &review_completion(&output))
        .unwrap();
    start
}

fn task_status(repository: &mut StateRepository, task_id: i64) -> String {
    repository
        .load()
        .unwrap()
        .unwrap()
        .state
        .agents
        .iter()
        .flat_map(|agent| agent.tasks.clone())
        .find(|task| task.id == task_id)
        .expect("task should exist")
        .status
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

// ---------------------------------------------------------------------------
// A–G — full green path, projections stay mutually consistent
// ---------------------------------------------------------------------------

#[test]
fn s4_happy_path_keeps_every_projection_consistent_with_backend_truth() {
    let (mut repository, revision) = base_repository();
    let (task_id, revision) =
        create_agent2_task(&mut repository, revision, "Implement parser boundary");

    let orchestration = repository.task_orchestration_snapshot().unwrap();
    assert_eq!(orchestration.execute_queue.len(), 1);
    assert_eq!(orchestration.execute_queue[0].task_id, task_id);
    assert_eq!(orchestration.execute_queue[0].queue_position, Some(1));
    assert!(orchestration.active_execute.is_none());

    let registry = repository.agent_registry_snapshot().unwrap();
    assert_eq!(registry.revision, revision);
    assert!(registry
        .templates
        .iter()
        .any(|template| template.template_key == "coding"));

    let execution = execute(&mut repository, task_id, "s4-happy-execute");
    assert_eq!(execution.status, RunAttemptStatus::Succeeded);

    let run = repository.run_snapshot().unwrap();
    assert!(run.active_attempt.is_none());
    assert_eq!(run.recent_attempts[0].id, execution.id);
    assert_eq!(run.recent_attempts[0].status, RunAttemptStatus::Succeeded);

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows.len(), 1);
    assert_eq!(review.flows[0].state, "awaiting_review");
    assert_eq!(
        review.flows[0].latest_execution_attempt_id,
        Some(execution.id)
    );

    for (index, reviewer) in [3, 6, 1].into_iter().enumerate() {
        let start = review_stage(
            &mut repository,
            task_id,
            &format!("s4-happy-review-{index}"),
            ReviewVerdict::Approved,
        );
        assert_eq!(start.stage.unwrap().reviewer_agent_id, Some(reviewer));
        assert!(repository.run_snapshot().unwrap().active_attempt.is_none());
    }

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "completed");
    assert_eq!(task_status(&mut repository, task_id), "Completed");

    let orchestration = repository.task_orchestration_snapshot().unwrap();
    assert!(orchestration.execute_queue.is_empty());
    assert!(orchestration.held_tasks.is_empty());
    assert!(orchestration.active_execute.is_none());
}

// ---------------------------------------------------------------------------
// C/D — one active run is authoritative across execute and review
// ---------------------------------------------------------------------------

#[test]
fn s4_one_active_run_is_authoritative_across_execute_and_review() {
    let (mut repository, revision) = base_repository();
    let (first_id, revision) = create_agent2_task(&mut repository, revision, "First queued task");
    let (second_id, _revision) =
        create_agent2_task(&mut repository, revision, "Second queued task");

    let head_intent = execute_intent(first_id);
    approve(&mut repository, &head_intent);
    let admitted = repository.admit_run("s4-busy-head", &head_intent).unwrap();
    repository
        .prepare_run_attempt(
            admitted.attempt.id,
            "Ollama",
            REVIEW_MODEL,
            Some(WORKSPACE_ID),
        )
        .unwrap();
    repository
        .mark_run_dispatching(admitted.attempt.id)
        .unwrap();

    let second_intent = execute_intent(second_id);
    approve(&mut repository, &second_intent);
    assert_eq!(
        repository
            .admit_run("s4-busy-second", &second_intent)
            .unwrap_err()
            .code,
        "RUN_BUSY"
    );
    assert_eq!(
        repository
            .run_snapshot()
            .unwrap()
            .active_attempt
            .as_ref()
            .unwrap()
            .id,
        admitted.attempt.id
    );

    repository.mark_run_started(admitted.attempt.id).unwrap();
    repository
        .complete_run(
            admitted.attempt.id,
            &successful_completion("First task implemented."),
        )
        .unwrap();

    let snapshot = repository.review_orchestration_snapshot().unwrap();
    let start = repository
        .start_review_stage(
            StartReviewStageRequest {
                expected_revision: snapshot.revision,
                task_owner_agent_id: 2,
                task_id: first_id,
            },
            &provider_snapshot(),
        )
        .unwrap();
    let context = start.context.clone().unwrap();
    let reviewer_agent_id = start.stage.as_ref().unwrap().reviewer_agent_id.unwrap();
    let review_intent = ActionIntent::RunTask {
        agent_id: reviewer_agent_id,
        task_owner_agent_id: 2,
        task_id: first_id,
        run_mode: RunMode::Review,
        review_context: Some(context),
    };
    let review_admitted = repository
        .admit_run("s4-busy-review", &review_intent)
        .unwrap();
    repository
        .prepare_run_attempt(
            review_admitted.attempt.id,
            "Ollama",
            REVIEW_MODEL,
            Some(WORKSPACE_ID),
        )
        .unwrap();
    repository
        .mark_run_dispatching(review_admitted.attempt.id)
        .unwrap();

    assert_eq!(
        repository
            .admit_run("s4-busy-second-during-review", &second_intent)
            .unwrap_err()
            .code,
        "RUN_BUSY"
    );
    assert_eq!(
        repository
            .run_snapshot()
            .unwrap()
            .active_attempt
            .as_ref()
            .unwrap()
            .id,
        review_admitted.attempt.id
    );
}

// ---------------------------------------------------------------------------
// D — replay / consumed / expired approvals fail closed
// ---------------------------------------------------------------------------

#[test]
fn s4_replay_consumed_and_expired_approvals_fail_closed() {
    let (mut repository, revision) = base_repository();
    let (task_id, _revision) =
        create_agent2_task(&mut repository, revision, "Approval boundary task");
    let intent = execute_intent(task_id);

    let execution = execute(&mut repository, task_id, "s4-approval-consume");
    assert_eq!(execution.status, RunAttemptStatus::Succeeded);

    // The consumed approval cannot authorize a second admission of this task.
    let replay = repository
        .admit_run("s4-approval-replay", &intent)
        .unwrap_err();
    assert!(
        [
            "QUEUE_HEAD_REQUIRED",
            "TASK_NOT_QUEUED",
            "APPROVAL_ALREADY_CONSUMED",
            "RUN_BUSY"
        ]
        .contains(&replay.code.as_str()),
        "consumed-approval replay should fail closed, got {}",
        replay.code
    );

    // Reusing a bound request id for a different action is rejected outright.
    let revision = repository.load().unwrap().unwrap().revision;
    let (other_id, _revision) =
        create_agent2_task(&mut repository, revision, "Second approval boundary task");
    let other_intent = execute_intent(other_id);
    let other_approval = approve(&mut repository, &other_intent);
    assert_eq!(
        repository
            .admit_run("s4-approval-consume", &other_intent)
            .unwrap_err()
            .code,
        "RUN_IDEMPOTENCY_CONFLICT"
    );

    // Once that approval has expired on the authoritative clock, admission of
    // the same task fails closed.
    repository
        .force_expire_approval_for_tests(other_approval)
        .unwrap();
    let expired = repository
        .admit_run("s4-approval-expired", &other_intent)
        .unwrap_err();
    assert!(
        [
            "APPROVAL_EXPIRED",
            "AUTHORIZATION_REQUIRED",
            "STALE_APPROVAL"
        ]
        .contains(&expired.code.as_str()),
        "expired approval should fail closed, got {}",
        expired.code
    );
}

// ---------------------------------------------------------------------------
// A/B/C — renderer forgery is rejected across registry, routing, queue, review
// ---------------------------------------------------------------------------

#[test]
fn s4_renderer_forgery_is_rejected_across_subsystems() {
    let (mut repository, revision) = base_repository();
    let (task_id, _revision) = create_agent2_task(&mut repository, revision, "Forgery target task");

    // Hierarchy / identity forgery through an untrusted generic whole-state save.
    let envelope = repository.load().unwrap().unwrap();
    let mut forged = envelope.state.clone();
    forged.agents[1].name = "Renderer Renamed Agent".to_string();
    assert_eq!(
        repository
            .save(envelope.revision, &forged, false)
            .unwrap_err()
            .code,
        "AGENT_REGISTRY_MUTATION_REQUIRED"
    );

    // Queue / lifecycle / routing-evidence forgery through a trusted generic
    // save must not take effect: the backend-owned fields are preserved.
    let envelope = repository.load().unwrap().unwrap();
    let mut forged = envelope.state.clone();
    for agent in &mut forged.agents {
        for task in &mut agent.tasks {
            if task.id == task_id {
                task.queue_state = "running".to_string();
                task.enqueue_sequence = Some(9_999);
                task.status = "Completed".to_string();
                task.phase = "Finished".to_string();
                task.routing_evidence = None;
                task.review_status = "Approved".to_string();
            }
        }
    }
    let _ = repository.save(envelope.revision, &forged, true);

    let reloaded = repository.load().unwrap().unwrap().state;
    let task = reloaded
        .agents
        .iter()
        .flat_map(|agent| &agent.tasks)
        .find(|task| task.id == task_id)
        .unwrap();
    assert_eq!(task.queue_state, "queued");
    assert_eq!(task.enqueue_sequence, Some(1));
    assert_eq!(task.status, "Pending");
    assert!(task.routing_evidence.is_some());
    assert_ne!(task.review_status, "Approved");

    let orchestration = repository.task_orchestration_snapshot().unwrap();
    assert_eq!(orchestration.execute_queue.len(), 1);
    assert_eq!(orchestration.execute_queue[0].task_id, task_id);
    assert!(orchestration.active_execute.is_none());
}

// ---------------------------------------------------------------------------
// B — a manual routing override cannot bypass a hard eligibility filter
// ---------------------------------------------------------------------------

#[test]
fn s4_manual_routing_override_cannot_bypass_a_hard_filter() {
    let (mut repository, revision) = base_repository();

    // Agent 4 (Browser Agent) runs a hosted OpenAI/Codex model; under an
    // Ollama-only active provider it is hard-ineligible. An explicit selection
    // must not route to it — routing fails closed and no task is queued.
    let error = repository
        .create_routed_task(
            CreateRoutedTaskRequest {
                expected_revision: revision,
                task_owner_agent_id: 1,
                title: "Override attempt".to_string(),
                category: "Development".to_string(),
                priority: "Normal".to_string(),
                workspace_id: WORKSPACE_ID.to_string(),
                routing_mode: "selected".to_string(),
                preferred_agent_id: Some(4),
                selected_agent_id: Some(4),
                specialist_request: Some(coding_request("Override attempt")),
            },
            &provider_snapshot(),
        )
        .unwrap_err();
    assert!(
        error.code.starts_with("SELECTED_AGENT"),
        "a manual override onto an ineligible agent must fail closed, got {}",
        error.code
    );

    let orchestration = repository.task_orchestration_snapshot().unwrap();
    assert!(
        orchestration.execute_queue.is_empty(),
        "the rejected routing must not queue a partial task"
    );
}

// ---------------------------------------------------------------------------
// E — a changes verdict requeues with a fresh policy/approval evaluation
// ---------------------------------------------------------------------------

#[test]
fn s4_changes_requested_requeues_with_a_fresh_policy_evaluation() {
    let (mut repository, revision) = base_repository();
    let (task_id, _revision) = create_agent2_task(&mut repository, revision, "Revision cycle task");
    let original_sequence = queue_sequence(&mut repository, task_id).unwrap();

    execute(&mut repository, task_id, "s4-revision-execute-0");
    review_stage(
        &mut repository,
        task_id,
        "s4-revision-review-0",
        ReviewVerdict::ChangesRequested,
    );

    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "revision_queued");
    assert_eq!(review.flows[0].revision_round, 1);

    // The task is queued again with a fresh enqueue sequence.
    let fresh_sequence = queue_sequence(&mut repository, task_id).unwrap();
    assert!(
        fresh_sequence > original_sequence,
        "a requeued revision must take a fresh queue age"
    );

    // The prior consumed approval cannot authorize the revision execution.
    let stale = repository
        .admit_run("s4-revision-stale-approval", &execute_intent(task_id))
        .unwrap_err();
    assert!(
        [
            "APPROVAL_ALREADY_CONSUMED",
            "AUTHORIZATION_REQUIRED",
            "STALE_APPROVAL",
            "QUEUE_HEAD_REQUIRED"
        ]
        .contains(&stale.code.as_str()),
        "the revision execution must require a fresh approval, got {}",
        stale.code
    );

    // A fresh approval admits the revision execution normally.
    let execution = execute(&mut repository, task_id, "s4-revision-execute-1");
    assert_eq!(execution.status, RunAttemptStatus::Succeeded);
    let review = repository.review_orchestration_snapshot().unwrap();
    assert_eq!(review.flows[0].state, "awaiting_review");
    assert_eq!(review.flows[0].revision_round, 1);
}

// ---------------------------------------------------------------------------
// E/F — the revision cap escalates to human adjudication
// ---------------------------------------------------------------------------

#[test]
fn s4_revision_cap_escalates_to_human_and_cannot_queue_a_fourth() {
    let (mut repository, revision) = base_repository();
    let (task_id, _revision) = create_agent2_task(&mut repository, revision, "Revision cap task");

    execute(&mut repository, task_id, "s4-cap-execute-0");
    for round in 0..=3 {
        review_stage(
            &mut repository,
            task_id,
            &format!("s4-cap-review-{round}"),
            ReviewVerdict::ChangesRequested,
        );
        let review = repository.review_orchestration_snapshot().unwrap();
        if round < 3 {
            assert_eq!(review.flows[0].state, "revision_queued");
            assert_eq!(review.flows[0].revision_round, round + 1);
            execute(
                &mut repository,
                task_id,
                &format!("s4-cap-execute-{}", round + 1),
            );
        } else {
            assert_eq!(review.flows[0].state, "awaiting_human");
            assert_eq!(review.flows[0].revision_round, 3);
        }
    }

    // A trusted human "request changes" past the cap fails the flow closed.
    let review = repository.review_orchestration_snapshot().unwrap();
    let flow_id = review.flows[0].id;
    let decision = repository
        .record_human_review_decision(HumanReviewDecisionRequest {
            expected_revision: review.revision,
            task_owner_agent_id: 2,
            task_id,
            flow_id,
            verdict: ReviewVerdict::ChangesRequested,
            feedback: "The bounded change is still not acceptable.".to_string(),
        })
        .unwrap();
    assert_eq!(
        decision
            .flows
            .iter()
            .find(|flow| flow.task_id == task_id)
            .unwrap()
            .state,
        "failed"
    );
    assert_eq!(task_status(&mut repository, task_id), "Failed");
}

// ---------------------------------------------------------------------------
// F — restart reconciliation preserves queue age and escalates uncertain dispatch
// ---------------------------------------------------------------------------

#[test]
fn s4_restart_preserves_queue_age_and_escalates_uncertain_dispatch() {
    let directory = TestDirectory::new();
    let path = directory.database_path();

    let head_sequence;
    {
        let (mut repository, revision) = base_repository_on_disk(&path);
        let (head_id, revision) = create_agent2_task(&mut repository, revision, "Head task");
        let (_later_id, _revision) = create_agent2_task(&mut repository, revision, "Later task");
        head_sequence = queue_sequence(&mut repository, head_id).unwrap();

        let intent = execute_intent(head_id);
        approve(&mut repository, &intent);
        let admitted = repository.admit_run("s4-restart-head", &intent).unwrap();
        repository
            .prepare_run_attempt(
                admitted.attempt.id,
                "Ollama",
                REVIEW_MODEL,
                Some(WORKSPACE_ID),
            )
            .unwrap();
        repository
            .mark_run_dispatching(admitted.attempt.id)
            .unwrap();
        // the process exits here — the dispatch outcome is uncertain
    }

    let mut reopened = StateRepository::open(&path).unwrap();
    let run = reopened.run_snapshot().unwrap();
    let recovered = &run.recent_attempts[0];
    assert_eq!(recovered.status, RunAttemptStatus::Interrupted);
    assert_eq!(
        recovered.recovery_disposition.as_deref(),
        Some("manual_review_required")
    );
    assert!(run.active_attempt.is_none());

    let orchestration = reopened.task_orchestration_snapshot().unwrap();
    assert!(
        orchestration
            .execute_queue
            .iter()
            .chain(orchestration.held_tasks.iter())
            .any(|entry| entry.enqueue_sequence == head_sequence),
        "the interrupted head must retain its enqueue sequence"
    );
}

// ---------------------------------------------------------------------------
// A — a freshly created custom agent can be edited (role / category / manager)
// ---------------------------------------------------------------------------

#[test]
fn s4_custom_agent_can_be_edited_after_creation() {
    let (mut repository, revision) = base_repository();

    let created = repository
        .create_agent(CreateAgentRequest {
            expected_revision: revision,
            name: "Linguistics".to_string(),
            description: "Research, create, work with all sorts of languages.".to_string(),
            role: "Specialist".to_string(),
            category: "Research".to_string(),
            reports_to: Some(9),
        })
        .unwrap();
    let custom_id = created
        .state
        .agents
        .iter()
        .find(|agent| agent.name == "Linguistics")
        .unwrap()
        .id;

    // Change its name, category, role, and reporting line in one update.
    let updated = repository
        .update_agent(UpdateAgentRequest {
            expected_revision: created.revision,
            agent_id: custom_id,
            name: "Applied Linguistics".to_string(),
            description: "Research and tooling for natural language.".to_string(),
            role: "Senior Agent".to_string(),
            category: "Research".to_string(),
            reports_to: Some(6),
        })
        .unwrap();
    let agent = updated
        .state
        .agents
        .iter()
        .find(|agent| agent.id == custom_id)
        .unwrap();
    assert_eq!(agent.name, "Applied Linguistics");
    assert_eq!(agent.role, "Senior Agent");
    assert_eq!(agent.reports_to, Some(6));
    assert_eq!(agent.authority_level, 2);
    assert_eq!(agent.registry_state, "active");

    // The change survives a database reload and the registry projection agrees.
    let reloaded = repository.load().unwrap().unwrap().state;
    assert_eq!(
        reloaded
            .agents
            .iter()
            .find(|agent| agent.id == custom_id)
            .unwrap()
            .reports_to,
        Some(6)
    );

    // Self-parenting is still rejected without changing state.
    let before = repository.load().unwrap().unwrap().revision;
    let error = repository
        .update_agent(UpdateAgentRequest {
            expected_revision: before,
            agent_id: custom_id,
            name: "Applied Linguistics".to_string(),
            description: "Research and tooling for natural language.".to_string(),
            role: "Senior Agent".to_string(),
            category: "Research".to_string(),
            reports_to: Some(custom_id),
        })
        .unwrap_err();
    assert_eq!(error.code, "STATE_VALIDATION_FAILED");
    assert_eq!(repository.load().unwrap().unwrap().revision, before);
}

// ---------------------------------------------------------------------------
// A — deletion is a durable tombstone; reroute and hold/resume preserve age
// ---------------------------------------------------------------------------

#[test]
fn s4_agent_deletion_tombstones_and_queue_moves_preserve_age() {
    let (mut repository, revision) = base_repository();

    let created = repository
        .create_agent(CreateAgentRequest {
            expected_revision: revision,
            name: "Disposable Builder".to_string(),
            description: "A throwaway acceptance agent.".to_string(),
            role: "Specialist".to_string(),
            category: "Development".to_string(),
            reports_to: Some(3),
        })
        .unwrap();
    let custom_id = created
        .state
        .agents
        .iter()
        .find(|agent| agent.name == "Disposable Builder")
        .unwrap()
        .id;
    let deleted = repository
        .delete_agent(DeleteAgentRequest {
            expected_revision: created.revision,
            agent_id: custom_id,
            replacement_manager_id: None,
        })
        .unwrap();
    assert_eq!(
        deleted
            .state
            .agents
            .iter()
            .find(|agent| agent.id == custom_id)
            .unwrap()
            .registry_state,
        "deleted"
    );

    // Reopening the registry projection shows no active template bound to it and
    // the tombstone does not resurrect.
    let registry = repository.agent_registry_snapshot().unwrap();
    assert!(registry
        .templates
        .iter()
        .all(|template| template.active_agent_id != Some(custom_id)));
    assert_eq!(
        repository
            .load()
            .unwrap()
            .unwrap()
            .state
            .agents
            .iter()
            .find(|agent| agent.id == custom_id)
            .unwrap()
            .registry_state,
        "deleted"
    );

    // A reroute of a queued task keeps its enqueue sequence.
    let revision = repository.load().unwrap().unwrap().revision;
    let (task_id, revision) = create_agent2_task(&mut repository, revision, "Reroute age task");
    let before = queue_sequence(&mut repository, task_id).unwrap();
    repository
        .reroute_task(
            RerouteTaskRequest {
                expected_revision: revision,
                task_owner_agent_id: 2,
                task_id,
                title: "Reroute age task".to_string(),
                category: "Development".to_string(),
                priority: "High".to_string(),
                workspace_id: WORKSPACE_ID.to_string(),
                routing_mode: "selected".to_string(),
                preferred_agent_id: Some(2),
                selected_agent_id: Some(2),
                specialist_request: Some(coding_request("Reroute age task")),
            },
            &provider_snapshot(),
        )
        .unwrap();
    let after = queue_sequence(&mut repository, task_id).unwrap();
    assert_eq!(before, after, "a reroute must preserve queue age");

    // Hold then resume also preserves the queue age.
    let revision = repository.load().unwrap().unwrap().revision;
    repository
        .set_task_queue_disposition(SetTaskQueueDispositionRequest {
            expected_revision: revision,
            task_owner_agent_id: 2,
            task_id,
            disposition: QueueDisposition::Hold,
        })
        .unwrap();
    let revision = repository.load().unwrap().unwrap().revision;
    repository
        .set_task_queue_disposition(SetTaskQueueDispositionRequest {
            expected_revision: revision,
            task_owner_agent_id: 2,
            task_id,
            disposition: QueueDisposition::Resume,
        })
        .unwrap();
    assert_eq!(
        queue_sequence(&mut repository, task_id).unwrap(),
        after,
        "hold/resume must preserve queue age"
    );
}
