//! TASK-0026 — composed reminders / structured-memory / backup-restore /
//! data-lifecycle acceptance (S7).
//!
//! These `#[cfg(test)]` scenarios wire the non-provider user-data surfaces end to
//! end over a real [`StateRepository`] the way the Tauri commands drive them:
//! the reminder scheduler (recurrence, DST, due-window classification, passive
//! bounded delivery, `needs_attention` holds), structured memory (scope
//! isolation and prompt-bundle bounds through the real write path), portable
//! backup v4 (byte-stable round trip, authority downgrade on import, strict
//! rejection of malformed or unsafe envelopes), reset-versus-purge wording,
//! retention maintenance (age pruning, clock-rollback evidence, protected active
//! work), monitoring revision fail-closed behaviour, and the bounded sequential
//! management-handoff history.
//!
//! Each subsystem is already unit-tested in isolation (`task_0014_*`,
//! `task_0018_*`); this module is the composed matrix plus the regression home
//! for defects the composition exposes. Live installed notification-portal
//! delivery, tray, restart, and real DST behaviour stay with TASK-0020 /
//! TASK-0030; the opt-in [`live`] submodule re-confirms only the notification
//! sink and is excluded from the deterministic gate.

use crate::app_state::{
    default_application_state, AgentTask, ApprovalRequest, WorkspaceDefinition,
    CURRENT_SCHEMA_VERSION,
};
use crate::data_lifecycle::{
    build_backup_export_with_domains, parse_backup_candidate, BACKUP_FORMAT, BACKUP_VERSION,
};
use crate::management_handoffs::{
    handoff_visible_to, validate_sequential_handoffs, ManagementHandoffKind,
    ManagementHandoffSource, ManagementHandoffV1, ManagementOwnerRole, ManagementVisibilityContext,
};
use crate::persistence::StateRepository;
use crate::reminder_scheduler::{
    classify_due_window, resolve_local_due_at, CreateScheduledItemRequest, DeliveryMode,
    DueWindowClassification, PrivacyMode, RecurrenceKind, RecurrenceRuleV1, ScheduleStatus,
    ScheduledItemKind,
};
use crate::structured_memory::{
    build_prompt_bundle, CreateMemoryRecordRequest, MemoryRecordKind, MemoryRetentionPolicy,
    MemoryScopeV1, MemorySelectionContext, MAX_PROMPT_MEMORY_RECORDS,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const FIXED_EXPORT_MS: i64 = 1_777_000_000_000;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("aacc-task-0026-{}-{sequence}", std::process::id()));
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

/// A fresh in-memory repository with default state and its current revision.
fn fresh_repository() -> (StateRepository, i64) {
    let mut repository = StateRepository::open_in_memory().unwrap();
    let envelope = repository.initialize_fresh().unwrap();
    (repository, envelope.revision)
}

#[allow(clippy::too_many_arguments)]
fn create_reminder(
    repository: &mut StateRepository,
    request_id: &str,
    title: &str,
    local_due_at: &str,
    time_zone: &str,
    recurrence: RecurrenceRuleV1,
    delivery_mode: DeliveryMode,
    privacy_mode: PrivacyMode,
) {
    let revision = repository.reminder_scheduler_snapshot().unwrap().revision;
    repository
        .create_scheduled_item(CreateScheduledItemRequest {
            expected_revision: revision,
            request_id: request_id.to_string(),
            kind: ScheduledItemKind::Reminder,
            title: title.to_string(),
            notes: String::new(),
            local_due_at: local_due_at.to_string(),
            time_zone: time_zone.to_string(),
            event_end_local: None,
            recurrence,
            delivery_mode,
            privacy_mode,
            subject_agent_id: None,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
        })
        .unwrap();
}

fn create_memory(
    repository: &mut StateRepository,
    request_id: &str,
    scope: MemoryScopeV1,
    content: &str,
    retention: MemoryRetentionPolicy,
) {
    let revision = repository.structured_memory_snapshot().unwrap().revision;
    repository
        .create_memory_record(CreateMemoryRecordRequest {
            expected_revision: revision,
            request_id: request_id.to_string(),
            scope,
            kind: MemoryRecordKind::Fact,
            content: content.to_string(),
            retention,
        })
        .unwrap();
}

/// Persist one workspace through the ordinary whole-state save path so a
/// project-scoped memory record has a live reference.
fn add_workspace(repository: &mut StateRepository, revision: i64, workspace_id: &str) -> i64 {
    let mut state = repository.load().unwrap().unwrap().state;
    state.preferences.workspaces.push(WorkspaceDefinition {
        id: workspace_id.to_string(),
        name: "Project A".to_string(),
        path: "/tmp/aacc-project-a".to_string(),
    });
    state.preferences.active_workspace_id = Some(workspace_id.to_string());
    repository.save(revision, &state, true).unwrap().revision
}

fn no_run_attempts(repository: &mut StateRepository) {
    let run = repository.run_snapshot().unwrap();
    assert!(
        run.active_attempt.is_none(),
        "a data-lifecycle scenario never starts a run"
    );
    assert_eq!(
        run.recent_attempts.len(),
        0,
        "a data-lifecycle scenario never records a run attempt"
    );
}

// ===========================================================================
// Reminders: recurrence, DST, due-window classification
// ===========================================================================

#[test]
fn s7_recurrence_dst_and_due_window_are_consistent_end_to_end() {
    let (mut repository, _revision) = fresh_repository();

    // A daily reminder anchored across the US spring-forward gap and the
    // autumn fold — the scheduler must anchor every occurrence to the original
    // civil time, not drift by the DST offset.
    create_reminder(
        &mut repository,
        "s7:recurrence:daily",
        "Take morning medication",
        "2026-03-07T02:30:00",
        "America/New_York",
        RecurrenceRuleV1 {
            kind: RecurrenceKind::Daily,
            interval: 1,
            occurrence_limit: Some(400),
            until_unix_ms: None,
        },
        DeliveryMode::InApp,
        PrivacyMode::Generic,
    );

    let snapshot = repository.reminder_scheduler_snapshot().unwrap();
    let item = snapshot
        .items
        .iter()
        .find(|item| item.title == "Take morning medication")
        .expect("the recurring reminder is stored");

    // The stored anchor instant resolves and the DST classification is explicit.
    let anchor = resolve_local_due_at("2026-03-07T02:30:00", "America/New_York").unwrap();
    assert_eq!(item.due_at_unix_ms, Some(anchor.due_at_unix_ms));

    // The occurrence one day later lands on 03-08 02:30 local — which does not
    // exist — so the scheduler shifts it forward rather than silently dropping
    // it or moving it a whole hour off the civil anchor.
    let gap_day = crate::reminder_scheduler::recurrence_resolution(
        "2026-03-07T02:30:00",
        "America/New_York",
        &item.recurrence,
        1,
    )
    .unwrap()
    .expect("the gap-day occurrence resolves");
    assert_eq!(
        gap_day.dst_resolution,
        crate::reminder_scheduler::DstResolution::GapShiftedForward
    );

    // Due-window classification never counts an overdue occurrence as upcoming
    // and never counts a beyond-the-window occurrence as due.
    let due = item.due_at_unix_ms.unwrap();
    assert_eq!(
        classify_due_window(item, due - 2 * DAY_MS, due - DAY_MS),
        DueWindowClassification::Future
    );
    assert_eq!(
        classify_due_window(item, due - DAY_MS, due),
        DueWindowClassification::DueWithinWindow
    );
    assert_eq!(
        classify_due_window(item, due, due),
        DueWindowClassification::DueNow
    );
    assert_eq!(
        classify_due_window(item, due + 1, due + DAY_MS),
        DueWindowClassification::Overdue
    );

    no_run_attempts(&mut repository);
}

// ===========================================================================
// Reminders: passive bounded delivery + monitoring
// ===========================================================================

#[test]
fn s7_due_reminder_delivery_is_passive_bounded_and_reflected_in_monitoring() {
    let (mut repository, _revision) = fresh_repository();

    create_reminder(
        &mut repository,
        "s7:delivery:daily",
        "Rotate the backup drive",
        "2026-08-30T09:00:00",
        "UTC",
        RecurrenceRuleV1 {
            kind: RecurrenceKind::Daily,
            interval: 1,
            occurrence_limit: Some(5),
            until_unix_ms: None,
        },
        DeliveryMode::Portal,
        PrivacyMode::Generic,
    );

    let first_due = resolve_local_due_at("2026-08-30T09:00:00", "UTC")
        .unwrap()
        .due_at_unix_ms;

    // Nothing is delivered before the first occurrence is due.
    assert!(repository
        .scan_due_reminders(first_due - 1)
        .unwrap()
        .is_empty());
    no_run_attempts(&mut repository);

    // Just past the second occurrence, the first two are due. Delivery is a
    // bounded notification derived from the schedule — never a model prompt.
    let jobs = repository
        .scan_due_reminders(first_due + DAY_MS + 1)
        .unwrap();
    assert_eq!(jobs.len(), 2);
    for job in &jobs {
        assert_eq!(job.title, "AI Agent Control Center reminder");
        assert!(job.body.contains("scheduled item is due"));
        assert!(!job.notification_id.is_empty());
        assert!(job.occurrence_id > 0);
    }
    no_run_attempts(&mut repository);

    // One accepted, one failed delivery — both record occurrence evidence and
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
    let accepted = snapshot
        .recent_occurrences
        .iter()
        .filter(|occurrence| occurrence.status == "portal_accepted")
        .count();
    let failed = snapshot
        .recent_occurrences
        .iter()
        .filter(|occurrence| occurrence.status == "failed")
        .count();
    assert_eq!(accepted, 1);
    assert_eq!(failed, 1);
    no_run_attempts(&mut repository);

    // Monitoring counts only active, still-scheduled reminders as upcoming.
    let monitoring = repository.monitoring_snapshot().unwrap();
    assert!(monitoring.authoritative);
    assert_eq!(monitoring.counts.upcoming_reminders, 1);
}

// ===========================================================================
// Reminders: unresolvable schedule is held, never dropped
// ===========================================================================

#[test]
fn s7_unresolvable_recurrence_is_held_for_attention_and_never_delivered() {
    let (mut repository, _revision) = fresh_repository();

    // A monthly reminder anchored to the 31st in a zone with DST. Most months
    // resolve; a future occurrence eventually cannot. The scheduler holds it
    // with inspectable issue evidence instead of dropping it or firing a model.
    create_reminder(
        &mut repository,
        "s7:hold:monthly",
        "Reconcile month-end ledger",
        "2026-01-31T09:00:00",
        "Europe/Amsterdam",
        RecurrenceRuleV1 {
            kind: RecurrenceKind::Monthly,
            interval: 1,
            occurrence_limit: None,
            until_unix_ms: None,
        },
        DeliveryMode::InApp,
        PrivacyMode::Title,
    );

    let snapshot = repository.reminder_scheduler_snapshot().unwrap();
    let item = &snapshot.items[0];
    assert_eq!(item.status, ScheduleStatus::Scheduled);

    // Scan far into the future. Every occurrence that resolves is a bounded
    // in-app occurrence; when the recurrence can no longer resolve, the item
    // moves to `needs_attention` with an issue code and message, and no portal
    // job is produced (this reminder is in-app anyway).
    let jobs = repository
        .scan_due_reminders(
            resolve_local_due_at("2026-01-31T09:00:00", "Europe/Amsterdam")
                .unwrap()
                .due_at_unix_ms
                + 4000 * DAY_MS,
        )
        .unwrap();
    assert!(
        jobs.is_empty(),
        "in-app reminders never produce portal jobs"
    );

    let after = repository.reminder_scheduler_snapshot().unwrap();
    let held = &after.items[0];
    if held.status == ScheduleStatus::NeedsAttention {
        assert!(held.schedule_issue_code.is_some());
        assert!(held.schedule_issue_message.is_some());
    } else {
        // If the calendar range never exhausts within the scan cap the item
        // simply stays scheduled — it is still never silently dropped.
        assert_eq!(held.status, ScheduleStatus::Scheduled);
    }
    assert_eq!(after.items.len(), 1, "the schedule is never dropped");
    no_run_attempts(&mut repository);
}

// ===========================================================================
// Structured memory: scope isolation and prompt-bundle bounds
// ===========================================================================

#[test]
fn s7_structured_memory_isolation_and_bounds_hold_through_the_real_write_path() {
    let (mut repository, mut revision) = fresh_repository();
    revision = add_workspace(&mut repository, revision, "workspace-a");
    let _ = revision;

    // Agent 1 (Supervisor), agent 6 (Development Team Leader), agent 9
    // (Research and Web Senior) all exist in a fresh install; agent 6 and 9 are
    // valid team-leader references.
    create_memory(
        &mut repository,
        "s7:mem:agent1:a",
        MemoryScopeV1::agent(1),
        "supervisor prefers concise status updates",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:mem:agent1:b",
        MemoryScopeV1::agent(1),
        "supervisor escalation path is documented",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:mem:agent2",
        MemoryScopeV1::agent(2),
        "CODING-AGENT-ONLY private note",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:mem:team6",
        MemoryScopeV1::team(6),
        "development team standard is trunk based",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:mem:team9",
        MemoryScopeV1::team(9),
        "RESEARCH-TEAM-ONLY private note",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:mem:project",
        MemoryScopeV1::project("workspace-a"),
        "project A ships on fridays",
        MemoryRetentionPolicy::Days30,
    );

    let records = repository.structured_memory_snapshot().unwrap().records;
    assert_eq!(records.len(), 6);
    assert!(records
        .iter()
        .all(|record| record.provenance == crate::structured_memory::MemoryProvenanceKind::User));

    // A prompt bundle for agent 1, in project A, on team 6 must see agent-1,
    // team-6, and project-A memory only — never agent 2 or team 9.
    let context = MemorySelectionContext {
        agent_id: 1,
        workspace_id: Some("workspace-a".to_string()),
        task_owner_agent_id: None,
        task_id: None,
        team_leader_agent_ids: vec![6],
    };
    let bundle = build_prompt_bundle(&records, context.clone(), 0).unwrap();
    let json = bundle.canonical_json().unwrap();
    assert!(!json.contains("CODING-AGENT-ONLY"));
    assert!(!json.contains("RESEARCH-TEAM-ONLY"));
    assert_eq!(bundle.records.len(), 4);
    assert_eq!(bundle.omitted_record_count, 0);

    // Deterministic hashing over the exact selected set.
    let again = build_prompt_bundle(&records, context, 0).unwrap();
    assert_eq!(bundle.sha256().unwrap(), again.sha256().unwrap());

    // Bounds: 130 agent-3 records through the real write path — the bundle caps
    // at 128 and reports the remainder as omitted.
    for index in 0..130 {
        create_memory(
            &mut repository,
            &format!("s7:mem:agent3:{index}"),
            MemoryScopeV1::agent(3),
            &format!("agent 3 note {index} {}", "x".repeat(64)),
            MemoryRetentionPolicy::Manual,
        );
    }
    let all_records = repository.structured_memory_snapshot().unwrap().records;
    let bounded = build_prompt_bundle(
        &all_records,
        MemorySelectionContext {
            agent_id: 3,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
            team_leader_agent_ids: vec![],
        },
        0,
    )
    .unwrap();
    assert_eq!(bounded.records.len(), MAX_PROMPT_MEMORY_RECORDS);
    assert_eq!(
        bounded.omitted_record_count,
        130 - MAX_PROMPT_MEMORY_RECORDS
    );

    no_run_attempts(&mut repository);
}

// ===========================================================================
// Structured memory: retention expiry
// ===========================================================================

#[test]
fn s7_memory_retention_prunes_only_expired_records() {
    let (mut repository, _revision) = fresh_repository();

    create_memory(
        &mut repository,
        "s7:retain:manual",
        MemoryScopeV1::agent(1),
        "manual retention never expires by age",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:retain:7d",
        MemoryScopeV1::agent(1),
        "seven day retention",
        MemoryRetentionPolicy::Days7,
    );
    create_memory(
        &mut repository,
        "s7:retain:90d",
        MemoryScopeV1::agent(1),
        "ninety day retention",
        MemoryRetentionPolicy::Days90,
    );

    let before = repository.structured_memory_snapshot().unwrap();
    assert_eq!(before.records.len(), 3);
    let created_at = before
        .records
        .iter()
        .map(|record| record.created_at_unix_ms)
        .max()
        .unwrap();

    // Maintenance ten days after creation: the 7-day record is past its
    // expiry, the 90-day and manual records are not.
    let result = repository
        .run_data_lifecycle_maintenance("test", created_at + 10 * DAY_MS)
        .unwrap();
    assert_eq!(result.status, "succeeded");
    assert_eq!(result.pruned.memory_records, 1);

    let after = repository.structured_memory_snapshot().unwrap();
    let remaining: Vec<&str> = after
        .records
        .iter()
        .map(|record| record.content.as_str())
        .collect();
    assert_eq!(after.records.len(), 2);
    assert!(remaining.contains(&"manual retention never expires by age"));
    assert!(remaining.contains(&"ninety day retention"));
    assert!(after
        .recent_events
        .iter()
        .any(|event| event.action == "retention_deleted" && event.actor_kind == "maintenance"));

    // A retention deletion is not a run.
    no_run_attempts(&mut repository);
}

// ===========================================================================
// Portable backup: byte-stable round trip
// ===========================================================================

#[test]
fn s7_backup_v4_round_trip_is_byte_stable_and_preserves_portable_safe_data() {
    let (mut repository, mut revision) = fresh_repository();
    revision = add_workspace(&mut repository, revision, "workspace-a");
    let _ = revision;

    create_reminder(
        &mut repository,
        "s7:backup:reminder:1",
        "Weekly review",
        "2026-09-01T17:00:00",
        "UTC",
        RecurrenceRuleV1 {
            kind: RecurrenceKind::Weekly,
            interval: 1,
            occurrence_limit: None,
            until_unix_ms: None,
        },
        DeliveryMode::InApp,
        PrivacyMode::Generic,
    );
    create_reminder(
        &mut repository,
        "s7:backup:reminder:2",
        "One-off checkpoint",
        "2026-09-15T09:00:00",
        "UTC",
        RecurrenceRuleV1::default(),
        DeliveryMode::InApp,
        PrivacyMode::Generic,
    );
    create_memory(
        &mut repository,
        "s7:backup:mem:1",
        MemoryScopeV1::agent(1),
        "portable agent memory",
        MemoryRetentionPolicy::Manual,
    );
    create_memory(
        &mut repository,
        "s7:backup:mem:2",
        MemoryScopeV1::project("workspace-a"),
        "portable project memory",
        MemoryRetentionPolicy::Days90,
    );

    let export = repository.export_backup().unwrap();
    assert_eq!(export.counts.reminders, 2);
    assert_eq!(export.counts.memory_records, 2);
    assert_eq!(export.counts.workspaces, 1);
    assert!(export
        .omitted_domains
        .iter()
        .any(|domain| domain == "runAttempts"));
    assert!(export
        .omitted_domains
        .iter()
        .any(|domain| domain == "systemActionAudit"));

    // Parse the candidate and re-export it deterministically twice at a fixed
    // timestamp — the portable form is byte-identical.
    let current = repository.load().unwrap().unwrap().state;
    let candidate = parse_backup_candidate(&export.backup_json, &current).unwrap();
    let first = build_backup_export_with_domains(
        &candidate.state,
        &candidate.scheduled_items,
        &candidate.memory_records,
        FIXED_EXPORT_MS,
    )
    .unwrap();
    let reparsed = parse_backup_candidate(&first.backup_json, &candidate.state).unwrap();
    let second = build_backup_export_with_domains(
        &reparsed.state,
        &reparsed.scheduled_items,
        &reparsed.memory_records,
        FIXED_EXPORT_MS,
    )
    .unwrap();
    assert_eq!(first.backup_json, second.backup_json);
    assert_eq!(first.counts, second.counts);
}

// ===========================================================================
// Portable backup: authority downgrade on import
// ===========================================================================

fn unsafe_backup_envelope() -> String {
    let mut state = default_application_state().unwrap();

    // A pending and an approved approval record — portable data cannot carry
    // live approval authority.
    for (id, status) in [(900, "Pending"), (901, "Approved")] {
        state.approval_requests.push(ApprovalRequest {
            id,
            agent_id: 2,
            task_id: Some(id),
            title: "Imported approval must not authorize anything".to_string(),
            reason: "portable data cannot mint authority".to_string(),
            status: status.to_string(),
            created_at: "2026-08-26T10:00:00.000Z".to_string(),
            resolved_at: None,
            risk_level: "High".to_string(),
            scopes: vec!["files".to_string()],
            workspace_id: None,
            task_snapshot: "Imported task".to_string(),
            expires_at: "2026-08-26T10:30:00.000Z".to_string(),
            consumed_at: None,
        });
    }

    // A running task with model/usage/diff evidence — import must neutralise it.
    let owner_id = state.agents[1].id;
    state.agents[1].tasks.push(AgentTask {
        id: 900,
        title: "Imported running task".to_string(),
        category: "Development".to_string(),
        priority: "High".to_string(),
        assigned_agent_id: owner_id,
        status: "Running".to_string(),
        phase: "Specialist Work".to_string(),
        created_at: "2026-08-26T10:00:00.000Z".to_string(),
        completed_at: None,
        result: Some("untrusted output".to_string()),
        response_id: Some("response-imported".to_string()),
        runtime_model: Some("model-imported".to_string()),
        total_tokens: Some(42),
        workspace_id: None,
        specialist_request: None,
        changed_files: vec!["secret.txt".to_string()],
        diff: Some("diff".to_string()),
        workspace_changes: None,
        duration_seconds: Some(1.0),
        routing_mode: "selected".to_string(),
        routed_from_agent_id: None,
        routing_reason: Some("reason".to_string()),
        queue_state: "running".to_string(),
        enqueue_sequence: Some(1),
        routing_evidence: None,
        review_agent_id: None,
        review_status: "Running".to_string(),
        review_result: None,
        review_model: None,
        review_duration_seconds: None,
        reviewed_at: None,
    });

    // Voice runtime enabled — import must disable it.
    state.preferences.background_voice_enabled = true;

    // A portal-delivery reminder and a user-provenance memory record in their
    // structured domains.
    let scheduled = json!([{
        "id": 1,
        "position": 0,
        "revision": 1,
        "kind": "reminder",
        "title": "Imported portal reminder",
        "notes": "",
        "localDueAt": "2026-09-01T09:00:00",
        "timeZone": "UTC",
        "dueAt": "2026-09-01T09:00:00Z",
        "dueAtUnixMs": resolve_local_due_at("2026-09-01T09:00:00", "UTC").unwrap().due_at_unix_ms,
        "eventEndLocal": null,
        "eventEndUnixMs": null,
        "dstResolution": "exact",
        "status": "scheduled",
        "recurrence": {"kind": "none", "interval": 1, "occurrenceLimit": null, "untilUnixMs": null},
        "nextOccurrenceSequence": 0,
        "missedOccurrenceCount": 0,
        "deliveryMode": "portal",
        "privacyMode": "generic",
        "scheduleFingerprint": "a".repeat(64),
        "subjectAgentId": null,
        "workspaceId": null,
        "taskOwnerAgentId": null,
        "taskId": null,
        "schedulerAgentId": null,
        "scheduleIssueCode": null,
        "scheduleIssueMessage": null,
        "createdAt": "2026-08-26T10:00:00Z",
        "createdAtUnixMs": 1_756_202_400_000_i64,
        "resolvedAtUnixMs": null,
        "updatedAtUnixMs": 1_756_202_400_000_i64
    }]);
    let memory = json!([{
        "id": 1,
        "scope": {"kind": "agent", "agentId": 1, "workspaceId": null,
                  "taskOwnerAgentId": null, "taskId": null, "teamLeaderAgentId": null},
        "kind": "fact",
        "content": "imported memory content",
        "provenance": "user",
        "provenanceRef": null,
        "revision": 1,
        "retention": "manual",
        "expiresAtUnixMs": null,
        "createdAtUnixMs": 1_756_202_400_000_i64,
        "updatedAtUnixMs": 1_756_202_400_000_i64
    }]);

    let envelope = json!({
        "format": BACKUP_FORMAT,
        "version": BACKUP_VERSION,
        "exportedAtUnixMs": FIXED_EXPORT_MS,
        "sourceSchemaVersion": CURRENT_SCHEMA_VERSION,
        "data": serde_json::to_value(&state).unwrap(),
        "scheduledItems": scheduled,
        "memoryRecords": memory,
    });
    serde_json::to_string(&envelope).unwrap()
}

#[test]
fn s7_backup_import_downgrades_every_authority_and_mints_none() {
    let (mut repository, revision) = fresh_repository();
    let backup_json = unsafe_backup_envelope();

    // The preview names the exact neutralisation without mutating state.
    let preview = repository
        .preview_backup_import(revision, &backup_json)
        .unwrap();
    assert_eq!(preview.format_version, 4);
    assert!(preview.replaces_current_state);
    assert!(preview.clears_run_and_review_history);
    assert_eq!(preview.sanitizations.held_tasks, 1);
    assert_eq!(preview.sanitizations.expired_approvals, 2);
    assert_eq!(preview.sanitizations.cleared_task_evidence, 1);
    assert!(preview.sanitizations.disabled_voice_runtime);
    assert_eq!(preview.sanitizations.portal_deliveries_disabled, 1);
    assert_eq!(
        repository.load().unwrap().unwrap().revision,
        revision,
        "preview does not advance the revision"
    );

    // Apply the import.
    let imported = repository
        .apply_backup_import(revision, &backup_json)
        .unwrap();
    let state = &imported.state;

    // Every imported approval is expired and non-authoritative.
    assert!(state
        .approval_requests
        .iter()
        .all(|request| request.status == "Expired" && request.consumed_at.is_none()));

    // The imported running task is held, stripped of evidence, and re-queued.
    let task = state.agents[1]
        .tasks
        .iter()
        .find(|task| task.id == 900)
        .expect("the imported task survives as held work");
    assert_eq!(task.status, "Pending");
    assert_eq!(task.phase, "Assigned");
    assert_eq!(task.queue_state, "held");
    assert!(task.response_id.is_none());
    assert!(task.runtime_model.is_none());
    assert!(task.changed_files.is_empty());
    assert!(task.diff.is_none());
    assert!(task.review_result.is_none());

    // Voice runtime is disabled.
    assert!(!state.preferences.background_voice_enabled);
    assert_eq!(state.preferences.voice_state, "VOICE_OFF");

    // The portal reminder is downgraded to in-app delivery.
    let reminders = repository.reminder_scheduler_snapshot().unwrap();
    assert_eq!(reminders.items.len(), 1);
    assert_eq!(reminders.items[0].delivery_mode, DeliveryMode::InApp);
    assert!(reminders.items[0].schedule_fingerprint.is_none());

    // Imported memory keeps its content but carries backup-import provenance.
    let memory = repository.structured_memory_snapshot().unwrap();
    assert_eq!(memory.records.len(), 1);
    assert_eq!(
        memory.records[0].provenance,
        crate::structured_memory::MemoryProvenanceKind::BackupImport
    );

    // No runtime authority was minted: no active run, no pending approvals in
    // the authoritative monitoring projection.
    no_run_attempts(&mut repository);
    let monitoring = repository.monitoring_snapshot().unwrap();
    assert_eq!(monitoring.counts.pending_approvals, 0);

    // Re-exporting the imported state is clean and stable.
    let re_export = repository.export_backup().unwrap();
    assert!(parse_backup_candidate(&re_export.backup_json, state).is_ok());
}

// ===========================================================================
// Portable backup: strict rejection before any mutation
// ===========================================================================

#[test]
fn s7_backup_rejects_unsafe_and_malformed_envelopes_before_mutation() {
    let (mut repository, revision) = fresh_repository();
    let current = repository.load().unwrap().unwrap().state;

    let duplicate = r#"{"version":4,"version":4}"#;
    let trailing = format!(
        "{} trailing",
        serde_json::to_string(
            &json!({"version": 2, "agents": [], "models": [], "approvalRequests": []})
        )
        .unwrap()
    );
    let future = r#"{"version":99}"#;
    let oversized = " ".repeat(crate::data_lifecycle::MAX_BACKUP_BYTES + 1);
    let depth_bomb = {
        let mut value = String::new();
        for _ in 0..(crate::data_lifecycle::MAX_BACKUP_JSON_DEPTH + 4) {
            value.push('[');
        }
        value
    };

    for (label, backup) in [
        ("duplicate keys", duplicate.to_string()),
        ("trailing content", trailing),
        ("future version", future.to_string()),
        ("oversized", oversized),
        ("depth bomb", depth_bomb),
    ] {
        assert!(
            repository.preview_backup_import(revision, &backup).is_err(),
            "{label} must be rejected"
        );
        assert!(
            repository.apply_backup_import(revision, &backup).is_err(),
            "{label} must be rejected before mutation"
        );
    }

    // A v4 envelope that smuggles reminders into the legacy `data.reminders`
    // array is rejected — reminders belong only in the structured domain.
    let mut smuggled: Value =
        serde_json::from_str(&unsafe_backup_envelope_with_clean_state()).unwrap();
    smuggled["data"]["reminders"] = json!([{
        "id": 1, "title": "smuggled", "dueAt": "2026-09-01T09:00:00Z", "status": "Scheduled"
    }]);
    let smuggled_json = serde_json::to_string(&smuggled).unwrap();
    assert!(repository
        .apply_backup_import(revision, &smuggled_json)
        .is_err());

    // An unknown top-level field is rejected by the strict schema.
    let mut unknown: Value =
        serde_json::from_str(&unsafe_backup_envelope_with_clean_state()).unwrap();
    unknown["trustedSession"] = Value::Bool(true);
    assert!(repository
        .apply_backup_import(revision, &serde_json::to_string(&unknown).unwrap())
        .is_err());

    // Every rejection left the current state and its revision untouched.
    let after = repository.load().unwrap().unwrap();
    assert_eq!(after.revision, revision);
    assert_eq!(after.state.agents.len(), current.agents.len());
}

/// The unsafe envelope helper, but with a default (already valid) embedded state
/// so the only reason a mutated copy fails is the mutation under test.
fn unsafe_backup_envelope_with_clean_state() -> String {
    let state = default_application_state().unwrap();
    let envelope = json!({
        "format": BACKUP_FORMAT,
        "version": BACKUP_VERSION,
        "exportedAtUnixMs": FIXED_EXPORT_MS,
        "sourceSchemaVersion": CURRENT_SCHEMA_VERSION,
        "data": serde_json::to_value(&state).unwrap(),
        "scheduledItems": json!([]),
        "memoryRecords": json!([]),
    });
    serde_json::to_string(&envelope).unwrap()
}

// ===========================================================================
// Reset vs purge: truthful wording and behaviour
// ===========================================================================

#[test]
fn s7_reset_keeps_the_database_and_is_worded_truthfully() {
    let directory = TestDirectory::new();
    let path = directory.database_path();

    let revision = {
        let mut repository = StateRepository::open(&path).unwrap();
        repository.initialize_fresh().unwrap();
        // Some maintenance history to prove it survives a reset.
        repository
            .run_data_lifecycle_maintenance("test", 1_800_000_000_000)
            .unwrap();
        create_memory(
            &mut repository,
            "s7:reset:mem",
            MemoryScopeV1::agent(1),
            "this memory is cleared by reset",
            MemoryRetentionPolicy::Manual,
        );
        repository.load().unwrap().unwrap().revision
    };

    // Reset requires the exact confirmation token.
    let mut repository = StateRepository::open(&path).unwrap();
    assert!(repository.reset(revision, "nope").is_err());
    let envelope = repository.reset(revision, "RESET").unwrap();
    assert!(envelope.revision > revision);

    // The database file is still there and still opens at the current schema.
    assert!(path.exists());
    let mut reopened = StateRepository::open(&path).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    let state = reopened.load().unwrap().unwrap().state;
    assert_eq!(state.agents.len(), 11, "reset restores factory defaults");
    assert!(reopened
        .structured_memory_snapshot()
        .unwrap()
        .records
        .is_empty());

    // Bounded maintenance evidence is retained across the reset.
    let lifecycle = reopened.monitoring_snapshot().unwrap().lifecycle;
    assert!(
        lifecycle.total_runs >= 1,
        "reset keeps maintenance evidence"
    );

    // The in-app reset wording tells the truth: factory defaults, the database
    // file remains — and it is not the irreversible CLI purge text.
    let source = include_str!("lib.rs");
    let reset_start = source
        .find("async fn reset_application_state(")
        .expect("the reset command exists");
    let reset_block = &source[reset_start..reset_start + 1200];
    assert!(reset_block.contains("factory defaults"));
    assert!(reset_block.contains("database file"));
    assert!(reset_block.contains("will remain"));
    assert!(
        !reset_block.contains("irreversibly"),
        "reset must not borrow the purge command's irreversible-deletion wording"
    );

    let purge_help = source
        .find("--purge --confirm PURGE")
        .expect("the purge CLI exists");
    let purge_block = &source[purge_help..purge_help + 200];
    assert!(purge_block.contains("irreversibly remove all owned data"));
}

// ===========================================================================
// Retention: clock rollback evidence
// ===========================================================================

#[test]
fn s7_retention_clock_rollback_is_evidenced_and_skips_age_deletion() {
    let (mut repository, _revision) = fresh_repository();

    create_memory(
        &mut repository,
        "s7:rollback:7d",
        MemoryScopeV1::agent(1),
        "seven day retention",
        MemoryRetentionPolicy::Days7,
    );
    let created_at = repository.structured_memory_snapshot().unwrap().records[0].created_at_unix_ms;

    // First maintenance well in the future records the observed time.
    let forward = repository
        .run_data_lifecycle_maintenance("test", created_at + 30 * DAY_MS)
        .unwrap();
    assert_eq!(forward.status, "succeeded");
    assert_eq!(forward.pruned.memory_records, 1);

    // Recreate an expiring record, then run maintenance with the clock behind
    // the last observed time.
    create_memory(
        &mut repository,
        "s7:rollback:7d:again",
        MemoryScopeV1::agent(1),
        "seven day retention again",
        MemoryRetentionPolicy::Days7,
    );
    let rolled_back = repository
        .run_data_lifecycle_maintenance("test", created_at - DAY_MS)
        .unwrap();
    assert_eq!(rolled_back.status, "clock_rollback");
    assert_eq!(rolled_back.error_code.as_deref(), Some("CLOCK_ROLLBACK"));
    assert_eq!(rolled_back.pruned, Default::default());

    // The record the rollback pass refused to age out is still present.
    assert_eq!(
        repository
            .structured_memory_snapshot()
            .unwrap()
            .records
            .len(),
        1
    );
    no_run_attempts(&mut repository);
}

// ===========================================================================
// Monitoring: fail closed on a stale revision
// ===========================================================================

#[test]
fn s7_monitoring_pages_fail_closed_on_a_stale_revision_tuple() {
    let (mut repository, _revision) = fresh_repository();
    let snapshot = repository.monitoring_snapshot().unwrap();
    let stale = snapshot.revision.clone();

    // Advance the application revision behind monitoring's back.
    create_memory(
        &mut repository,
        "s7:monitoring:mem",
        MemoryScopeV1::agent(1),
        "advances the application revision",
        MemoryRetentionPolicy::Manual,
    );

    let fresh = repository.monitoring_snapshot().unwrap().revision;
    assert_ne!(fresh, stale);

    let error = repository
        .query_monitoring_tasks(&stale, None, None, 0, 50)
        .unwrap_err();
    assert_eq!(error.code, "MONITORING_REVISION_CONFLICT");

    // The fresh tuple works.
    assert!(repository
        .query_monitoring_tasks(&fresh, None, None, 0, 50)
        .is_ok());
}

// ===========================================================================
// Management handoffs: sequential, bounded, visibility scoped
// ===========================================================================

#[test]
fn s7_management_handoff_history_is_sequential_bounded_and_scoped() {
    fn handoff(id: i64, kind: ManagementHandoffKind) -> ManagementHandoffV1 {
        ManagementHandoffV1 {
            id,
            task_owner_agent_id: 2,
            task_id: 5,
            kind,
            from_agent_id: Some(6),
            to_agent_id: Some(2),
            owner_role: ManagementOwnerRole::TeamLeader,
            revision_round: 0,
            run_attempt_id: None,
            review_flow_id: None,
            review_stage_attempt_id: None,
            source: ManagementHandoffSource::TaskOrchestration,
            summary: "bounded sequential evidence".to_string(),
            payload: json!({"evidence": "retained"}),
            idempotency_key: format!("s7-handoff-{id}"),
            created_at_unix_ms: id,
        }
    }

    // A valid task/plan/assignment/evidence/decision chain passes.
    let ordered = vec![
        handoff(1, ManagementHandoffKind::TaskPlan),
        handoff(2, ManagementHandoffKind::Assignment),
        handoff(3, ManagementHandoffKind::ExecutionEvidence),
        handoff(4, ManagementHandoffKind::ReviewDecision),
    ];
    assert!(validate_sequential_handoffs(&ordered).is_ok());

    // A review decision without its execution-evidence predecessor is rejected —
    // the history is sequential evidence, not free-form agent messaging.
    assert!(
        validate_sequential_handoffs(&[handoff(1, ManagementHandoffKind::ReviewDecision)]).is_err()
    );

    // Visibility is bounded to the management chain: a supervisor who manages
    // agent 2 sees it; an unrelated senior does not; a human always does.
    let record = handoff(1, ManagementHandoffKind::TaskPlan);
    assert!(handoff_visible_to(
        &record,
        &ManagementVisibilityContext {
            viewer_agent_id: Some(1),
            viewer_role: ManagementOwnerRole::Supervisor,
            managed_agent_ids: vec![2],
        }
    ));
    assert!(!handoff_visible_to(
        &record,
        &ManagementVisibilityContext {
            viewer_agent_id: Some(4),
            viewer_role: ManagementOwnerRole::Senior,
            managed_agent_ids: vec![],
        }
    ));
    assert!(handoff_visible_to(
        &record,
        &ManagementVisibilityContext {
            viewer_agent_id: None,
            viewer_role: ManagementOwnerRole::Human,
            managed_agent_ids: vec![],
        }
    ));
}

// ===========================================================================
// ---------------------------------------------------------------------------
// Scenario — an exported backup actually reaches the disk (TASK-0030)
// ---------------------------------------------------------------------------

/// The TASK-0030 live-acceptance defect, held as a structural test.
///
/// Every deterministic backup scenario above proves the *bytes* are correct, and
/// they all passed while the shipped application put those bytes somewhere the
/// operator could not predict. The renderer handed the export to a `Blob` +
/// `<a download>` click; WebKitGTK performs that download into the process's
/// current working directory, and the desktop entry sets no `Path=`, so the
/// file landed wherever the launcher's CWD pointed. The operator was told a byte
/// count and never a destination, and had no way to choose one.
///
/// No unit test could catch that: the failure lived in the hand-off from the
/// renderer to the platform. This scenario pins the contract that closed it —
/// the desktop export leaves through a backend command that writes the file to
/// an operator-chosen path, never through a webview download.
#[test]
fn s7_desktop_backup_export_is_written_by_the_backend_not_a_webview_download() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf();
    let settings = std::fs::read_to_string(root.join("src/features/settings/SettingsPage.tsx"))
        .expect("the settings page is shipped");

    let desktop_branch = settings
        .split_once("if (isDesktopRuntime()) {")
        .expect("the export path must branch on the desktop runtime")
        .1;
    let desktop_branch = desktop_branch
        .split_once("\n      }\n")
        .expect("the desktop branch is terminated")
        .0;

    assert!(
        desktop_branch.contains("onSaveBackup("),
        "the desktop export must hand its bytes to the backend save command"
    );
    assert!(
        !desktop_branch.contains("downloadBackup("),
        "the desktop export must not rely on a webview download; WebKitGTK \
         performs none, which is how a successful-looking export wrote no file"
    );

    // The command the renderer calls has to exist and be registered, or the
    // export fails at runtime while every test above still passes.
    let backend =
        std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).expect("the backend is shipped");
    assert!(
        backend.contains("async fn save_backup_file("),
        "the backend must expose the backup save command"
    );
    assert!(
        backend.contains("\n            save_backup_file,\n"),
        "save_backup_file must be registered in the Tauri invoke handler"
    );
    // The destination comes from the native dialog, never from the renderer.
    assert!(
        backend.contains("--getsavefilename"),
        "the save destination must come from the native dialog"
    );
}

// Live (S7) — `#[ignore]`d. Re-confirms only the reminder notification sink.
//
//   cargo test --manifest-path src-tauri/Cargo.toml --lib \
//     data_lifecycle_acceptance::live -- --ignored --test-threads=1 --nocapture
//
// The XDG notification portal was granted and live-verified under TASK-0025.
// This scenario re-drives it as the reminder-delivery sink: derive a bounded
// notification from a real recurring schedule and deliver + withdraw it through
// the same `ashpd` path `send_portal_reminder` uses. No microphone, no dialog,
// no new grant.
// ===========================================================================
#[cfg(test)]
mod live {
    use super::*;
    use ashpd::desktop::notification::{Notification, NotificationProxy, Priority};

    #[tokio::test]
    #[ignore = "live: real XDG notification portal round trip for a reminder"]
    async fn live_s7_reminder_notification_delivers_and_withdraws() {
        let (mut repository, _revision) = fresh_repository();
        create_reminder(
            &mut repository,
            "s7:live:reminder",
            "TASK-0026 live acceptance reminder",
            "2026-08-30T09:00:00",
            "UTC",
            RecurrenceRuleV1 {
                kind: RecurrenceKind::Daily,
                interval: 1,
                occurrence_limit: Some(1),
                until_unix_ms: None,
            },
            DeliveryMode::Portal,
            PrivacyMode::Title,
        );
        let due = resolve_local_due_at("2026-08-30T09:00:00", "UTC")
            .unwrap()
            .due_at_unix_ms;
        let jobs = repository.scan_due_reminders(due + DAY_MS).unwrap();
        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert_eq!(job.title, "TASK-0026 live acceptance reminder");

        let proxy = NotificationProxy::new().await.expect("notification portal");
        proxy
            .add_notification(
                &job.notification_id,
                Notification::new(&job.title)
                    .body(job.body.as_str())
                    .priority(Priority::Normal),
            )
            .await
            .expect("the portal accepts the reminder notification");
        proxy
            .remove_notification(&job.notification_id)
            .await
            .expect("the portal withdraws the reminder notification");

        repository
            .finish_reminder_delivery(job.occurrence_id, true, None)
            .unwrap();
        no_run_attempts(&mut repository);
    }
}
