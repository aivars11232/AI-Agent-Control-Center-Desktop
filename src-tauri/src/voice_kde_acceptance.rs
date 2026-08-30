//! TASK-0025 — composed voice / KDE portal / PC-control / notification
//! acceptance (S6).
//!
//! These `#[cfg(test)]` scenarios wire the voice and system-action path end to
//! end over a real [`StateRepository`] — canonical voice-intent normalization,
//! the authoritative capability / approval / forced-one-use policy, the redacted
//! restart-safe system-action audit lifecycle, the offline listener state
//! machine, the KDE RemoteDesktop lifecycle guards, pressed-input release, the
//! private restore-token contract, and the passive reminder → XDG notification
//! delivery path — and assert that every backend invariant already unit-tested
//! in isolation (`task_0015_*`, `task_0016_*`, `task_0018_*`) still holds when
//! the pieces are composed the way
//! [`submit_voice_intent`](crate::submit_voice_intent) drives them.
//!
//! Real microphone capture, a real KDE portal grant, real compositor input, and
//! real notification rendering are owned by the `live` submodule below and are
//! excluded from the deterministic gate.

use crate::app_state::ApplicationState;
use crate::authorization::{ApprovalResolution, AuthorizationDecision};
use crate::desktop_control::{state_retains_desktop_control, PressedInputTracker};
use crate::persistence::StateRepository;
use crate::policy::ActionIntent;
use crate::reminder_scheduler::{
    CreateScheduledItemRequest, DeliveryMode, PrivacyMode, RecurrenceKind, RecurrenceRuleV1,
    ScheduledItemKind,
};
use crate::system_actions::{
    sha256_hex, AuditWrite, AuthorizedSystemAction, KeyboardAction, PointerAction, StandardFolder,
    SubmitVoiceIntentRequest, VoiceIntent, WindowAction,
};
use crate::voice_runtime::{InstallKind, VoiceRuntime};
use crate::{
    apply_voice_listener_event, ensure_gateway_retry_target, parse_voice_transcript_event,
    save_desktop_control_token, unresolved_voice_target, DesktopControl, VoiceTranscriptEvent,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const PC_TEMPLATE: &str = "pc-control";
const WINDOW_ID: &str = "kwin-internal-4211";
const DESKTOP_ID: &str = "org.kde.dolphin.desktop";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("aacc-task-0025-{}-{sequence}", std::process::id()));
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

/// The exact active Full PC Control agent id from a fresh install.
fn pc_agent_id(state: &ApplicationState) -> i64 {
    state
        .agents
        .iter()
        .find(|agent| agent.template_key.as_deref() == Some(PC_TEMPLATE))
        .expect("a fresh install has a PC Control agent")
        .id
}

/// A fresh in-memory repository whose PC Control agent has the requested `system`
/// capability and approval mode. Returns `(repository, revision, pc_agent_id)`.
fn base_repository(system_capability: &str, system_approval: &str) -> (StateRepository, i64, i64) {
    let mut repository = StateRepository::open_in_memory().unwrap();
    let initialized = repository.initialize_fresh().unwrap();
    let mut configured = initialized.state;
    let agent_id = pc_agent_id(&configured);
    let pc_agent = configured
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
        .unwrap();
    // A fresh install ships the PC Control agent paused; an operator who turns on
    // voice PC control activates it. Model that starting point.
    pc_agent.status = "Waiting".to_string();
    pc_agent.capabilities.system = system_capability.to_string();
    pc_agent.approvals.system = system_approval.to_string();
    let revision = repository
        .save(initialized.revision, &configured, true)
        .unwrap()
        .revision;
    (repository, revision, agent_id)
}

fn system_intent(agent_id: i64, action: AuthorizedSystemAction) -> ActionIntent {
    ActionIntent::SystemAction { agent_id, action }
}

fn close_window() -> AuthorizedSystemAction {
    AuthorizedSystemAction::CloseWindow {
        window_id: WINDOW_ID.to_string(),
        desktop_id: DESKTOP_ID.to_string(),
    }
}

fn launch_app() -> AuthorizedSystemAction {
    AuthorizedSystemAction::LaunchApplication {
        desktop_id: DESKTOP_ID.to_string(),
    }
}

fn transcript_event(kind: &str, transcript: &str) -> VoiceTranscriptEvent {
    VoiceTranscriptEvent {
        kind: kind.to_string(),
        transcript: transcript.to_string(),
    }
}

fn audit_write(request_id: &str, agent_id: i64, action: &AuthorizedSystemAction) -> AuditWrite {
    let (target_kind, target_id) = action.target();
    AuditWrite {
        request_id: request_id.to_string(),
        request_fingerprint: format!("voice-intent-v1|{}", sha256_hex(request_id.as_bytes())),
        intent_kind: "closeApplication".to_string(),
        risk_class: action.risk_class().to_string(),
        target_kind,
        target_id,
        agent_id,
        task_owner_agent_id: None,
        task_id: None,
        approval_id: None,
        authorization_kind: "approvalRequired".to_string(),
        intent_fingerprint_sha256: sha256_hex(b"intent"),
        policy_fingerprint_sha256: sha256_hex(b"policy"),
        status: "approvalRequired".to_string(),
        detail_code: Some("APPROVAL_REQUIRED".to_string()),
        detail_message: Some("Waiting for one-use backend authorization.".to_string()),
        content_sha256: None,
        content_length: None,
    }
}

fn no_run_attempts(repository: &mut StateRepository) {
    let snapshot = repository.run_snapshot().unwrap();
    assert!(
        snapshot.active_attempt.is_none(),
        "no run attempt is active"
    );
    assert!(
        snapshot.recent_attempts.is_empty(),
        "no run attempt was recorded"
    );
    assert_eq!(snapshot.retained_attempt_count, 0);
}

// ===========================================================================
// Voice-intent normalization and the redacted authorization contract
// ===========================================================================

#[test]
fn s6_canonical_voice_intents_normalize_validate_and_bind_redacted_evidence() {
    // The renderer submission shape validates and binds a redacted digest.
    let request = SubmitVoiceIntentRequest {
        request_id: "voice:s6:type".to_string(),
        intent: VoiceIntent::TypeText {
            text: "deploy staging build 42".to_string(),
        },
    };
    request
        .validate()
        .expect("dictated text is bounded and safe");
    let fingerprint = request.fingerprint().unwrap();
    let (digest, length) = request.intent.content_digest().unwrap();
    assert!(fingerprint.starts_with("voice-intent-v1|"));
    assert!(!fingerprint.contains("deploy staging build 42"));
    assert_eq!(digest.len(), 64);
    assert_eq!(length, 23);

    // Every canonical intent classifies to a stable kind and an unresolved
    // target that never carries raw application text before resolution.
    for (intent, kind, risk) in [
        (
            VoiceIntent::LaunchApplication {
                application: "Dolphin".to_string(),
            },
            "launchApplication",
            "reversible",
        ),
        (
            VoiceIntent::OpenStandardFolder {
                folder: StandardFolder::Downloads,
            },
            "openStandardFolder",
            "reversible",
        ),
        (
            VoiceIntent::CloseApplication {
                application: "Dolphin".to_string(),
            },
            "closeApplication",
            "destructive",
        ),
        (
            VoiceIntent::CloseActiveWindow,
            "closeActiveWindow",
            "destructive",
        ),
        (
            VoiceIntent::PointerAction {
                action: PointerAction::Click,
            },
            "pointerAction",
            "meaningful",
        ),
        (
            VoiceIntent::KeyboardAction {
                action: KeyboardAction::Cut,
            },
            "keyboardAction",
            "destructive",
        ),
        (
            VoiceIntent::ActiveWindowAction {
                action: WindowAction::Maximize,
            },
            "activeWindowAction",
            "meaningful",
        ),
    ] {
        intent.validate().expect("canonical intent validates");
        assert_eq!(intent.kind_name(), kind);
        let (target_kind, target_id, target_risk) = unresolved_voice_target(&intent);
        assert_eq!(target_kind, "unresolvedTarget");
        assert_eq!(target_risk, risk);
        assert!(target_id.starts_with("sha256:"));
        assert!(!target_id.contains("Dolphin"));
    }

    // Malformed intents fail closed before any authorization.
    assert!(VoiceIntent::TypeText {
        text: "\u{1b}rm -rf".to_string(),
    }
    .validate()
    .is_err());
    assert!(SubmitVoiceIntentRequest {
        request_id: "bad id".to_string(),
        intent: VoiceIntent::CloseActiveWindow,
    }
    .validate()
    .is_err());
}

#[test]
fn s6_destructive_close_forces_one_use_approval_even_when_policy_allows() {
    // Policy is the most permissive it can be: full system capability, allow.
    let (mut repository, _revision, agent_id) = base_repository("full", "allow");
    let action = close_window();
    assert_eq!(action.risk_class(), "destructive");
    assert!(action.force_approval());
    let (target_kind, _target_id) = action.target();
    assert_eq!(target_kind, "kwinWindow");

    let intent = system_intent(agent_id, action.clone());
    let outcome = repository.request_authorization(&intent).unwrap();
    assert_eq!(outcome.decision, AuthorizationDecision::ApprovalRequired);
    let approval = outcome
        .approval
        .expect("a destructive close always yields a one-use approval record");
    assert_eq!(approval.status, "Pending");

    // Approve once, then the grant consumes it and returns policy evidence.
    repository
        .resolve_approval(approval.id, ApprovalResolution::Approve, true)
        .unwrap();
    let grant = repository.authorize_intent(&intent).unwrap();
    let evidence = grant.evidence.expect("a consumed grant returns evidence");
    assert!(!evidence.intent_fingerprint.is_empty());

    // The audit lifecycle is written the way the gateway writes it.
    let write = AuditWrite {
        approval_id: Some(approval.id),
        intent_fingerprint_sha256: sha256_hex(evidence.intent_fingerprint.as_bytes()),
        policy_fingerprint_sha256: sha256_hex(evidence.policy_fingerprint.as_bytes()),
        ..audit_write("voice:s6:close", agent_id, &action)
    };
    let pending = repository.write_system_action_audit(&write).unwrap();
    assert_eq!(pending.status, "approvalRequired");

    let dispatched = repository
        .write_system_action_audit(&AuditWrite {
            status: "dispatched".to_string(),
            authorization_kind: "approvalConsumed".to_string(),
            detail_code: Some("SYSTEM_ACTION_DISPATCHED".to_string()),
            ..write.clone()
        })
        .unwrap();
    assert_eq!(dispatched.status, "dispatched");

    let applied = repository
        .write_system_action_audit(&AuditWrite {
            status: "applied".to_string(),
            authorization_kind: "approvalConsumed".to_string(),
            detail_code: Some("SYSTEM_ACTION_APPLIED".to_string()),
            ..write.clone()
        })
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert!(applied.content_sha256.is_none());

    // A second approve of the same one-use approval is refused (fail closed).
    assert!(repository
        .resolve_approval(approval.id, ApprovalResolution::Approve, true)
        .is_err());
}

#[test]
fn s6_reversible_launch_with_allow_policy_needs_no_approval() {
    let (mut repository, _revision, agent_id) = base_repository("full", "allow");
    let action = launch_app();
    assert_eq!(action.risk_class(), "reversible");
    assert!(!action.force_approval());

    let outcome = repository
        .request_authorization(&system_intent(agent_id, action))
        .unwrap();
    assert_eq!(outcome.decision, AuthorizationDecision::Allowed);
    assert!(outcome.approval.is_none());
}

#[test]
fn s6_capability_denied_when_the_pc_agent_lacks_system_permission() {
    let (mut repository, _revision, agent_id) = base_repository("none", "deny");
    let error = repository
        .request_authorization(&system_intent(agent_id, launch_app()))
        .unwrap_err();
    assert!(
        error.code == "CAPABILITY_DENIED" || error.code == "APPROVAL_POLICY_DENIED",
        "unexpected denial code: {}",
        error.code
    );

    // The gateway records a rejection audit against the unresolved target.
    let (target_kind, target_id, risk) = unresolved_voice_target(&VoiceIntent::LaunchApplication {
        application: "Dolphin".to_string(),
    });
    let rejected = repository
        .write_system_action_audit(&AuditWrite {
            request_id: "voice:s6:denied".to_string(),
            request_fingerprint: "voice-intent-v1|s6denied".to_string(),
            intent_kind: "launchApplication".to_string(),
            risk_class: risk.to_string(),
            target_kind: target_kind.to_string(),
            target_id,
            agent_id,
            task_owner_agent_id: None,
            task_id: None,
            approval_id: None,
            authorization_kind: "policyRejected".to_string(),
            intent_fingerprint_sha256: sha256_hex(b"intent"),
            policy_fingerprint_sha256: sha256_hex(b"policy"),
            status: "rejected".to_string(),
            detail_code: Some(error.code.clone()),
            detail_message: Some(error.message.clone()),
            content_sha256: None,
            content_length: None,
        })
        .unwrap();
    assert_eq!(rejected.status, "rejected");
}

#[test]
fn s6_approval_retry_refuses_a_changed_exact_target_and_binds_the_request_id() {
    let (mut repository, _revision, agent_id) = base_repository("full", "ask");
    let action = close_window();
    let (target_kind, target_id) = action.target();
    let base = audit_write("voice:s6:retry", agent_id, &action);
    let existing = repository.write_system_action_audit(&base).unwrap();

    // Same exact target: the retry is accepted.
    assert!(ensure_gateway_retry_target(
        &existing,
        "destructive",
        &target_kind,
        &target_id,
        agent_id,
    )
    .is_ok());

    // Changed exact target: refused with the exact code.
    assert_eq!(
        ensure_gateway_retry_target(
            &existing,
            "destructive",
            &target_kind,
            "kwin-internal-9999:desktop:org.kde.kate.desktop",
            agent_id,
        )
        .unwrap_err()
        .code,
        "SYSTEM_ACTION_TARGET_CHANGED"
    );

    // Reusing the request id with a different exact action is refused by the
    // persistence idempotency binding itself.
    let conflict = repository
        .write_system_action_audit(&AuditWrite {
            target_id: "kwin-internal-9999:desktop:org.kde.kate.desktop".to_string(),
            ..base.clone()
        })
        .unwrap_err();
    assert_eq!(conflict.code, "SYSTEM_ACTION_IDEMPOTENCY_CONFLICT");
}

#[test]
fn s6_idempotent_replay_is_restart_safe_and_never_repeats_a_terminal_action() {
    let directory = TestDirectory::new();
    let path = directory.database_path();
    let base = {
        let mut repository = StateRepository::open(&path).unwrap();
        repository.initialize_fresh().unwrap();
        let write = AuditWrite {
            authorization_kind: "policyAllowed".to_string(),
            ..audit_write("voice:s6:replay", 7, &close_window())
        };
        repository.write_system_action_audit(&write).unwrap();
        repository
            .write_system_action_audit(&AuditWrite {
                status: "dispatched".to_string(),
                ..write.clone()
            })
            .unwrap();
        repository
            .write_system_action_audit(&AuditWrite {
                status: "applied".to_string(),
                ..write.clone()
            })
            .unwrap();
        write
    };

    // Reopen the database — a fresh process observes the same terminal record.
    let mut reopened = StateRepository::open(&path).unwrap();
    let stored = reopened
        .system_action_audit("voice:s6:replay")
        .unwrap()
        .expect("the terminal audit survives a restart");
    assert_eq!(stored.status, "applied");

    // Replaying a terminal request is refused; the action is not repeated.
    let terminal = reopened
        .write_system_action_audit(&AuditWrite {
            status: "dispatched".to_string(),
            ..base.clone()
        })
        .unwrap_err();
    assert_eq!(terminal.code, "SYSTEM_ACTION_TERMINAL");
}

// ===========================================================================
// Offline listener lifecycle and bounded transcript projection
// ===========================================================================

#[test]
fn s6_listener_events_project_every_bounded_lifecycle_state() {
    let runtime = VoiceRuntime::default();
    for (kind, expected) in [
        ("ready", "passive"),
        ("activated", "active"),
        ("listening", "listening"),
        ("transcribing", "transcribing"),
        ("heard", "active"),
        ("command", "active"),
        ("deactivated", "passive"),
        ("off_requested", "stopping"),
    ] {
        apply_voice_listener_event(&runtime, &transcript_event(kind, ""));
        assert_eq!(
            runtime.snapshot().unwrap().listener_state,
            expected,
            "event {kind} should project {expected}"
        );
    }

    // A warning does not move the lifecycle state.
    apply_voice_listener_event(
        &runtime,
        &transcript_event("warning", "high accuracy unavailable"),
    );
    assert_eq!(runtime.snapshot().unwrap().listener_state, "stopping");

    // An error fails closed and surfaces the bounded message.
    apply_voice_listener_event(
        &runtime,
        &transcript_event("error", "the bounded listener failed"),
    );
    let snapshot = runtime.snapshot().unwrap();
    assert_eq!(snapshot.listener_state, "failed");
    assert_eq!(snapshot.message, "the bounded listener failed");
}

#[test]
fn s6_listener_transcript_frames_are_bounded_and_fail_closed() {
    assert_eq!(
        parse_voice_transcript_event(br#"{"kind":"command","transcript":"open dolphin"}"#)
            .unwrap()
            .kind,
        "command"
    );
    // Unknown kind, unexpected field, oversized transcript, and control bytes
    // are all rejected.
    assert!(parse_voice_transcript_event(br#"{"kind":"exec","transcript":""}"#).is_none());
    assert!(
        parse_voice_transcript_event(br#"{"kind":"command","transcript":"x","note":1}"#).is_none()
    );
    let oversized = format!(
        r#"{{"kind":"command","transcript":"{}"}}"#,
        "x".repeat(4_096)
    );
    assert!(parse_voice_transcript_event(oversized.as_bytes()).is_none());
    assert!(
        parse_voice_transcript_event(b"{\"kind\":\"command\",\"transcript\":\"a\\u0007b\"}")
            .is_none()
    );
}

#[test]
fn s6_voice_runtime_rejects_overlapping_install_and_listener_start() {
    let runtime = VoiceRuntime::default();

    let reservation = runtime.begin_install(InstallKind::Base).unwrap();
    assert_eq!(
        runtime.begin_install(InstallKind::High).err().unwrap(),
        "VOICE_INSTALL_BUSY: Another voice runtime installation is already active."
    );
    let cancelling = runtime.cancel_install(&reservation.operation_id).unwrap();
    assert_eq!(cancelling.install_state, "cancelling");
    runtime.finish_install(
        &reservation.operation_id,
        "failed",
        "The staged install was cancelled.",
    );
    assert_eq!(runtime.snapshot().unwrap().install_state, "failed");

    runtime.begin_listener_start().unwrap();
    assert_eq!(
        runtime.begin_listener_start().unwrap_err(),
        "VOICE_LISTENER_BUSY: The offline listener is already starting."
    );
    runtime.cancel_listener_start("stopped", "The listener start was cancelled.");
    assert_eq!(runtime.snapshot().unwrap().listener_state, "stopped");
}

// ===========================================================================
// KDE RemoteDesktop lifecycle, input release, and the exact-agent binding
// ===========================================================================

#[test]
fn s6_desktop_control_lifecycle_rejects_overlap_and_preserves_explicit_disable() {
    let desktop_control = DesktopControl::default();
    assert!(desktop_control.begin_start().unwrap());
    assert_eq!(desktop_control.status().unwrap().state, "starting");
    assert!(!desktop_control.status().unwrap().enabled);
    assert_eq!(
        desktop_control.begin_start().unwrap_err(),
        "DESKTOP_CONTROL_BUSY: A KDE desktop-input lifecycle change is already active."
    );
    assert!(desktop_control
        .take_session("disabled", "Desktop input was explicitly disabled.")
        .is_none());
    desktop_control.fail_start("A stale start failed.");
    let disabled = desktop_control.status().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.state, "disabled");
}

#[test]
fn s6_desktop_control_requires_the_exact_active_full_pc_agent() {
    let (mut repository, revision, agent_id) = base_repository("full", "ask");
    let envelope = repository.load().unwrap().unwrap();
    assert!(state_retains_desktop_control(&envelope.state, agent_id));
    assert!(!state_retains_desktop_control(&envelope.state, i64::MAX));

    // Downgrading the capability drops the binding — the reconcile path closes
    // any live session on the next state change.
    let mut downgraded = envelope.state.clone();
    downgraded
        .agents
        .iter_mut()
        .find(|agent| agent.id == agent_id)
        .unwrap()
        .capabilities
        .system = "notifications".to_string();
    repository.save(revision, &downgraded, true).unwrap();
    let reloaded = repository.load().unwrap().unwrap();
    assert!(!state_retains_desktop_control(&reloaded.state, agent_id));
}

#[test]
fn s6_pressed_inputs_release_in_reverse_order_without_duplicates() {
    let mut tracker = PressedInputTracker::default();
    for code in [29, 42, 42, 47] {
        tracker.record_pressed(code);
    }
    assert_eq!(tracker.release_order().collect::<Vec<_>>(), [47, 42, 29]);
    tracker.record_released(42);
    assert_eq!(tracker.release_order().collect::<Vec<_>>(), [47, 29]);
    tracker.record_released(47);
    tracker.record_released(29);
    assert!(tracker.is_empty());
}

#[test]
fn s6_restore_token_contract_rejects_malformed_tokens_before_touching_disk() {
    assert!(save_desktop_control_token("").is_err());
    assert!(save_desktop_control_token("bad\u{0}token").is_err());
    assert!(save_desktop_control_token(&"x".repeat(4_097)).is_err());
}

// ===========================================================================
// Passive reminder → XDG notification delivery (no background model)
// ===========================================================================

#[test]
fn s6_due_reminder_delivers_a_notification_and_never_launches_a_model() {
    let mut repository = StateRepository::open_in_memory().unwrap();
    repository.initialize_fresh().unwrap();
    repository
        .create_scheduled_item(CreateScheduledItemRequest {
            expected_revision: 0,
            request_id: "voice:s6:reminder".to_string(),
            kind: ScheduledItemKind::Reminder,
            title: "Review the deployment checklist".to_string(),
            notes: "No model run is attached to this schedule.".to_string(),
            local_due_at: "2026-08-30T09:00:00".to_string(),
            time_zone: "UTC".to_string(),
            event_end_local: None,
            recurrence: RecurrenceRuleV1 {
                kind: RecurrenceKind::Daily,
                interval: 1,
                occurrence_limit: Some(2),
                until_unix_ms: None,
            },
            delivery_mode: DeliveryMode::Portal,
            privacy_mode: PrivacyMode::Title,
            subject_agent_id: None,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
        })
        .unwrap();

    let due = crate::reminder_scheduler::resolve_local_due_at("2026-08-30T09:00:00", "UTC")
        .unwrap()
        .due_at_unix_ms;
    let jobs = repository
        .scan_due_reminders(due + 24 * 60 * 60 * 1000)
        .unwrap();
    assert_eq!(jobs.len(), 2);

    // The delivery payload is a bounded notification derived from the schedule
    // under its privacy mode — never a model prompt.
    for job in &jobs {
        assert_eq!(job.title, "Review the deployment checklist");
        assert_eq!(job.body, "A scheduled item is due.");
        assert!(!job.notification_id.is_empty());
        assert!(job.occurrence_id > 0);
    }
    no_run_attempts(&mut repository);

    // One accepted delivery, one rejected: both record occurrence evidence and
    // still launch nothing.
    repository
        .finish_reminder_delivery(jobs[0].occurrence_id, true, None)
        .unwrap();
    repository
        .finish_reminder_delivery(
            jobs[1].occurrence_id,
            false,
            Some("The XDG notification portal is unavailable."),
        )
        .unwrap();

    let snapshot = repository.reminder_scheduler_snapshot().unwrap();
    assert_eq!(
        snapshot
            .recent_occurrences
            .iter()
            .filter(|occurrence| occurrence.status == "portal_accepted")
            .count(),
        1
    );
    assert!(snapshot
        .recent_occurrences
        .iter()
        .any(|occurrence| occurrence.status == "failed"));
    no_run_attempts(&mut repository);
}

#[test]
fn s6_reminder_delivery_wiring_is_notification_only() {
    // The production scheduler cycle delivers due reminders through the XDG
    // notification portal and nothing else — no provider, registry, or model.
    let source = include_str!("lib.rs");
    let cycle_start = source
        .find("async fn run_reminder_scheduler_cycle(")
        .expect("the reminder scheduler cycle exists");
    let cycle = &source[cycle_start..cycle_start + 900];
    assert!(cycle.contains("deliver_reminder_jobs_with(persistence, jobs, send_portal_reminder)"));

    let sink_start = source
        .find("async fn send_portal_reminder(")
        .expect("the notification sink exists");
    let sink = &source[sink_start..sink_start + 700];
    assert!(sink.contains("NotificationProxy::new()"));
    assert!(sink.contains("add_notification"));
    for forbidden in [
        "provider_registry",
        "production_provider_registry",
        "run_agent_task",
        "admit_run",
        "registry.run",
    ] {
        assert!(
            !sink.contains(forbidden),
            "the reminder notification sink must not reference {forbidden}"
        );
    }
}

// ===========================================================================
// Live voice / KDE portal / notification acceptance (S6) — `#[ignore]`d.
//
// Run on Arch Linux / KDE Plasma / Wayland, on the real XDG desktop session:
//   cargo test --manifest-path src-tauri/Cargo.toml --lib \
//     voice_kde_acceptance::live -- --ignored --test-threads=1 --nocapture
//
// `live_s6_portals_deliver_and_negotiate` drives the two `ashpd` portal paths
// the production code uses that need no operator input, in one process and one
// connection lifetime: the notification portal accepts and withdraws a real
// notification (the reminder-delivery sink), and the KDE RemoteDesktop portal
// creates a session and negotiates the exact keyboard + pointer devices up to —
// but not through — the `Start()` consent boundary. (ashpd shares a process
// connection whose signal pump does not survive a second `#[tokio::test]`
// runtime, so both portal checks live in one test.)
//
// `live_s6_remote_desktop_grant_and_input` additionally calls `Start()` and
// injects one bounded pointer + keyboard event, then releases and closes. That
// raises the KDE "share input" consent dialog unless a valid restore token
// silently restores the grant, so it only runs when `AACC_LIVE_PORTAL_START=1`
// is set and an operator is present to click "Share" once. The full
// enable → input → disable → restart-reuse lifecycle is exercised through the
// real Tauri app, which holds one stable portal connection.
//
// The offline microphone / Vosk transcription path additionally needs the voice
// runtime installed and an operator to speak; that residual step is recorded in
// the TASK-0025 final report. None of these touch the real application database.
// ===========================================================================

#[cfg(test)]
mod live {
    use ashpd::desktop::{
        notification::{Notification, NotificationProxy, Priority},
        remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions},
        PersistMode,
    };
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "live: real XDG notification + KDE RemoteDesktop session negotiation"]
    async fn live_s6_portals_deliver_and_negotiate() {
        // 1. The reminder notification sink: deliver and withdraw a real
        //    notification through the XDG notification portal.
        let notifications = NotificationProxy::new()
            .await
            .expect("the XDG notification portal is available");
        notifications
            .add_notification(
                "aacc-s6-live-reminder",
                Notification::new("AI Agent Control Center")
                    .body("TASK-0025 live S6 check — reminder notification delivered.")
                    .priority(Priority::Normal),
            )
            .await
            .expect("the notification portal accepts the reminder notification");
        tokio::time::sleep(Duration::from_secs(2)).await;
        notifications
            .remove_notification("aacc-s6-live-reminder")
            .await
            .expect("the notification portal withdraws the notification");

        // 2. The KDE RemoteDesktop portal: create a session and negotiate the
        //    exact keyboard + pointer devices. `Start()` is the operator
        //    consent boundary and is exercised separately.
        let portal = RemoteDesktop::new()
            .await
            .expect("KDE's RemoteDesktop portal is available");
        let session = portal
            .create_session(Default::default())
            .await
            .expect("KDE creates a desktop-input portal session");
        portal
            .select_devices(
                &session,
                SelectDevicesOptions::default()
                    .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked),
            )
            .await
            .expect("KDE negotiates the exact keyboard and pointer devices");
        session
            .close()
            .await
            .expect("the negotiated portal session closes cleanly");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "live: raises the KDE consent dialog and injects bounded input (AACC_LIVE_PORTAL_START=1)"]
    async fn live_s6_remote_desktop_grant_and_input() {
        if std::env::var("AACC_LIVE_PORTAL_START").as_deref() != Ok("1") {
            eprintln!(
                "skipped: set AACC_LIVE_PORTAL_START=1 with an operator present to click \"Share\""
            );
            return;
        }
        let portal = RemoteDesktop::new().await.expect("RemoteDesktop portal");
        let session = portal
            .create_session(Default::default())
            .await
            .expect("session created");
        portal
            .select_devices(
                &session,
                SelectDevicesOptions::default()
                    .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked),
            )
            .await
            .expect("devices negotiated");

        let selected = tokio::time::timeout(
            Duration::from_secs(60),
            portal.start(&session, None, Default::default()),
        )
        .await
        .expect("the operator responded to the KDE consent dialog within 60s")
        .expect("start request completed")
        .response()
        .expect("KDE returned the granted devices");
        assert!(selected.devices().contains(DeviceType::Pointer));
        assert!(selected.devices().contains(DeviceType::Keyboard));

        // One bounded, visible, reversible nudge: move right, move back, tap and
        // release Shift. Every press is matched by a release before close.
        portal
            .notify_pointer_motion(&session, 40.0, 0.0, Default::default())
            .await
            .expect("pointer motion accepted");
        portal
            .notify_pointer_motion(&session, -40.0, 0.0, Default::default())
            .await
            .expect("pointer motion accepted");
        portal
            .notify_pointer_axis_discrete(&session, Axis::Vertical, 0, Default::default())
            .await
            .ok();
        const SHIFT_KEYSYM: i32 = 0xffe1;
        portal
            .notify_keyboard_keysym(
                &session,
                SHIFT_KEYSYM,
                KeyState::Pressed,
                Default::default(),
            )
            .await
            .expect("keysym press accepted");
        portal
            .notify_keyboard_keysym(
                &session,
                SHIFT_KEYSYM,
                KeyState::Released,
                Default::default(),
            )
            .await
            .expect("keysym release accepted");

        session
            .close()
            .await
            .expect("the portal session closes cleanly");
    }
}
