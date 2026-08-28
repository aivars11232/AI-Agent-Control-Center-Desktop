DROP INDEX IF EXISTS reminders_retention_v8;

ALTER TABLE application_meta RENAME TO application_meta_v10;

CREATE TABLE application_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    initialized INTEGER NOT NULL DEFAULT 0 CHECK (initialized IN (0, 1)),
    state_revision INTEGER NOT NULL DEFAULT 0
        CHECK (state_revision BETWEEN 0 AND 9007199254740991),
    source_kind TEXT CHECK (
        source_kind IN (
            'fresh',
            'legacy_local_storage',
            'legacy_backup',
            'backup_v3',
            'backup_v4',
            'reset'
        )
    ),
    source_version INTEGER,
    migrated_at_unix_ms INTEGER CHECK (migrated_at_unix_ms >= 0),
    legacy_cleanup_ack_at_unix_ms INTEGER CHECK (legacy_cleanup_ack_at_unix_ms >= 0)
);

INSERT INTO application_meta (
    singleton,
    initialized,
    state_revision,
    source_kind,
    source_version,
    migrated_at_unix_ms,
    legacy_cleanup_ack_at_unix_ms
)
SELECT
    singleton,
    initialized,
    state_revision,
    source_kind,
    source_version,
    migrated_at_unix_ms,
    legacy_cleanup_ack_at_unix_ms
FROM application_meta_v10;

DROP TABLE application_meta_v10;

ALTER TABLE reminders RENAME TO reminders_v10;

CREATE TABLE reminders (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    revision INTEGER NOT NULL DEFAULT 1
        CHECK (revision BETWEEN 1 AND 9007199254740991),
    kind TEXT NOT NULL CHECK (kind IN ('reminder', 'event')),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 4096),
    notes TEXT NOT NULL CHECK (length(notes) <= 2097152),
    local_due_at TEXT NOT NULL CHECK (length(local_due_at) BETWEEN 1 AND 128),
    time_zone TEXT NOT NULL CHECK (length(time_zone) BETWEEN 1 AND 256),
    due_at TEXT NOT NULL CHECK (length(due_at) BETWEEN 1 AND 128),
    due_at_unix_ms INTEGER CHECK (due_at_unix_ms >= 0),
    event_end_local TEXT CHECK (length(event_end_local) BETWEEN 1 AND 128),
    event_end_unix_ms INTEGER CHECK (event_end_unix_ms >= 0),
    dst_resolution TEXT NOT NULL
        CHECK (dst_resolution IN ('exact', 'fold_earlier', 'gap_shifted_forward', 'unresolved')),
    status TEXT NOT NULL
        CHECK (status IN ('scheduled', 'due', 'completed', 'dismissed', 'needs_attention')),
    recurrence_kind TEXT NOT NULL
        CHECK (recurrence_kind IN ('none', 'daily', 'weekly', 'monthly')),
    recurrence_interval INTEGER NOT NULL DEFAULT 1
        CHECK (recurrence_interval BETWEEN 1 AND 366),
    recurrence_limit INTEGER CHECK (recurrence_limit BETWEEN 1 AND 10000),
    recurrence_until_unix_ms INTEGER CHECK (recurrence_until_unix_ms >= 0),
    next_occurrence_sequence INTEGER NOT NULL DEFAULT 0
        CHECK (next_occurrence_sequence BETWEEN 0 AND 9007199254740991),
    missed_occurrence_count INTEGER NOT NULL DEFAULT 0 CHECK (missed_occurrence_count >= 0),
    delivery_mode TEXT NOT NULL CHECK (delivery_mode IN ('in_app', 'portal')),
    privacy_mode TEXT NOT NULL CHECK (privacy_mode IN ('generic', 'title')),
    schedule_fingerprint TEXT,
    authorization_kind TEXT CHECK (authorization_kind IN ('policy_allow', 'one_use_approval')),
    approval_id INTEGER CHECK (approval_id BETWEEN 1 AND 9007199254740991),
    authorization_policy_fingerprint TEXT,
    subject_agent_id INTEGER CHECK (subject_agent_id BETWEEN 1 AND 9007199254740991),
    workspace_id TEXT,
    task_owner_agent_id INTEGER CHECK (task_owner_agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER CHECK (task_id BETWEEN 1 AND 9007199254740991),
    scheduler_agent_id INTEGER CHECK (scheduler_agent_id BETWEEN 1 AND 9007199254740991),
    schedule_issue_code TEXT,
    schedule_issue_message TEXT,
    created_at TEXT NOT NULL,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    resolved_at_unix_ms INTEGER CHECK (resolved_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    CHECK ((task_owner_agent_id IS NULL) = (task_id IS NULL)),
    CHECK (kind = 'event' OR (event_end_local IS NULL AND event_end_unix_ms IS NULL)),
    CHECK (status = 'needs_attention' OR due_at_unix_ms IS NOT NULL),
    CHECK (
        delivery_mode = 'in_app'
        OR (
            schedule_fingerprint IS NOT NULL
            AND authorization_kind IS NOT NULL
            AND authorization_policy_fingerprint IS NOT NULL
        )
    )
);

INSERT INTO reminders (
    id,
    position,
    revision,
    kind,
    title,
    notes,
    local_due_at,
    time_zone,
    due_at,
    due_at_unix_ms,
    dst_resolution,
    status,
    recurrence_kind,
    recurrence_interval,
    next_occurrence_sequence,
    missed_occurrence_count,
    delivery_mode,
    privacy_mode,
    subject_agent_id,
    task_owner_agent_id,
    task_id,
    scheduler_agent_id,
    schedule_issue_code,
    schedule_issue_message,
    created_at,
    created_at_unix_ms,
    resolved_at_unix_ms,
    updated_at_unix_ms
)
SELECT
    legacy.id,
    legacy.position,
    1,
    'reminder',
    legacy.title,
    legacy.notes,
    CASE
        WHEN strftime('%s', legacy.due_at) IS NULL THEN legacy.due_at
        ELSE strftime('%Y-%m-%dT%H:%M:%S', legacy.due_at)
    END,
    'UTC',
    legacy.due_at,
    CASE
        WHEN strftime('%s', legacy.due_at) IS NULL THEN NULL
        ELSE CAST(strftime('%s', legacy.due_at) AS INTEGER) * 1000
    END,
    CASE WHEN strftime('%s', legacy.due_at) IS NULL THEN 'unresolved' ELSE 'exact' END,
    CASE
        WHEN legacy.status = 'Completed' THEN 'completed'
        WHEN legacy.status = 'Dismissed' THEN 'dismissed'
        WHEN strftime('%s', legacy.due_at) IS NULL THEN 'needs_attention'
        ELSE 'scheduled'
    END,
    'none',
    1,
    0,
    0,
    'in_app',
    'generic',
    legacy.agent_id,
    CASE WHEN legacy.agent_id IS NOT NULL AND legacy.task_id IS NOT NULL THEN legacy.agent_id END,
    CASE WHEN legacy.agent_id IS NOT NULL THEN legacy.task_id END,
    (SELECT id FROM agents WHERE template_key = 'event-reminder' ORDER BY id LIMIT 1),
    CASE WHEN strftime('%s', legacy.due_at) IS NULL THEN 'LEGACY_DUE_AT_INVALID' END,
    CASE WHEN strftime('%s', legacy.due_at) IS NULL
        THEN 'The legacy due time is invalid and must be corrected before scheduling.' END,
    legacy.created_at,
    COALESCE(
        legacy.created_at_unix_ms,
        CAST(strftime('%s', legacy.created_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    ),
    legacy.resolved_at_unix_ms,
    COALESCE(
        legacy.resolved_at_unix_ms,
        legacy.created_at_unix_ms,
        CAST(strftime('%s', legacy.created_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    )
FROM reminders_v10 AS legacy;

DROP TABLE reminders_v10;

CREATE INDEX reminders_due_v11
ON reminders (status, due_at_unix_ms, id);

CREATE INDEX reminders_retention_v11
ON reminders (status, resolved_at_unix_ms, id);

CREATE TABLE reminder_scheduler_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    next_reminder_id INTEGER NOT NULL DEFAULT 1
        CHECK (next_reminder_id BETWEEN 1 AND 9007199254740991),
    next_occurrence_id INTEGER NOT NULL DEFAULT 1
        CHECK (next_occurrence_id BETWEEN 1 AND 9007199254740991),
    last_scan_at_unix_ms INTEGER CHECK (last_scan_at_unix_ms >= 0),
    last_error_code TEXT,
    last_error_message TEXT
);

INSERT INTO reminder_scheduler_meta (
    singleton,
    next_reminder_id,
    next_occurrence_id
)
SELECT
    1,
    COALESCE((SELECT MAX(id) + 1 FROM reminders), 1),
    1;

CREATE TABLE reminder_occurrences (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    reminder_id INTEGER NOT NULL CHECK (reminder_id BETWEEN 1 AND 9007199254740991),
    schedule_revision INTEGER NOT NULL CHECK (schedule_revision >= 1),
    occurrence_sequence INTEGER NOT NULL CHECK (occurrence_sequence >= 0),
    occurrence_key TEXT NOT NULL UNIQUE,
    due_at_unix_ms INTEGER NOT NULL CHECK (due_at_unix_ms >= 0),
    status TEXT NOT NULL
        CHECK (status IN ('reserved', 'in_app_due', 'portal_accepted', 'failed', 'uncertain')),
    missed_count INTEGER NOT NULL DEFAULT 0 CHECK (missed_count >= 0),
    first_missed_at_unix_ms INTEGER CHECK (first_missed_at_unix_ms >= 0),
    last_missed_at_unix_ms INTEGER CHECK (last_missed_at_unix_ms >= 0),
    portal_notification_id TEXT,
    detail_code TEXT,
    detail_message TEXT,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    FOREIGN KEY (reminder_id) REFERENCES reminders(id) ON DELETE CASCADE,
    UNIQUE (reminder_id, schedule_revision, occurrence_sequence)
);

CREATE INDEX reminder_occurrences_recent_v11
ON reminder_occurrences (updated_at_unix_ms DESC, id DESC);

CREATE TABLE reminder_mutation_requests (
    request_id TEXT PRIMARY KEY CHECK (length(request_id) BETWEEN 1 AND 128),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) = 64),
    resulting_revision INTEGER NOT NULL
        CHECK (resulting_revision BETWEEN 1 AND 9007199254740991),
    item_id INTEGER CHECK (item_id BETWEEN 1 AND 9007199254740991),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
);

CREATE TABLE structured_memory_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    next_record_id INTEGER NOT NULL DEFAULT 1
        CHECK (next_record_id BETWEEN 1 AND 9007199254740991),
    next_event_id INTEGER NOT NULL DEFAULT 1
        CHECK (next_event_id BETWEEN 1 AND 9007199254740991)
);

CREATE TABLE memory_records (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('agent', 'project', 'task', 'team')),
    agent_id INTEGER CHECK (agent_id BETWEEN 1 AND 9007199254740991),
    workspace_id TEXT,
    task_owner_agent_id INTEGER CHECK (task_owner_agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER CHECK (task_id BETWEEN 1 AND 9007199254740991),
    team_leader_agent_id INTEGER CHECK (team_leader_agent_id BETWEEN 1 AND 9007199254740991),
    record_kind TEXT NOT NULL CHECK (record_kind IN ('instruction', 'fact', 'decision', 'summary')),
    content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 2097152),
    provenance_kind TEXT NOT NULL
        CHECK (provenance_kind IN ('user', 'legacy_agent_memory', 'handoff_promotion', 'backup_import')),
    provenance_ref TEXT,
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    retention_policy TEXT NOT NULL
        CHECK (retention_policy IN ('manual', '7d', '30d', '90d', 'task_lifetime')),
    expires_at_unix_ms INTEGER CHECK (expires_at_unix_ms >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    CHECK (
        (scope_kind = 'agent' AND agent_id IS NOT NULL AND workspace_id IS NULL
            AND task_owner_agent_id IS NULL AND task_id IS NULL AND team_leader_agent_id IS NULL)
        OR (scope_kind = 'project' AND agent_id IS NULL AND workspace_id IS NOT NULL
            AND task_owner_agent_id IS NULL AND task_id IS NULL AND team_leader_agent_id IS NULL)
        OR (scope_kind = 'task' AND agent_id IS NULL AND workspace_id IS NULL
            AND task_owner_agent_id IS NOT NULL AND task_id IS NOT NULL
            AND team_leader_agent_id IS NULL)
        OR (scope_kind = 'team' AND agent_id IS NULL AND workspace_id IS NULL
            AND task_owner_agent_id IS NULL AND task_id IS NULL
            AND team_leader_agent_id IS NOT NULL)
    )
);

CREATE INDEX memory_records_scope_v11
ON memory_records (
    scope_kind,
    agent_id,
    workspace_id,
    task_owner_agent_id,
    task_id,
    team_leader_agent_id,
    updated_at_unix_ms DESC,
    id DESC
);

CREATE INDEX memory_records_expiry_v11
ON memory_records (expires_at_unix_ms, id);

CREATE TABLE memory_mutation_requests (
    request_id TEXT PRIMARY KEY CHECK (length(request_id) BETWEEN 1 AND 128),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) = 64),
    resulting_revision INTEGER NOT NULL
        CHECK (resulting_revision BETWEEN 1 AND 9007199254740991),
    record_id INTEGER CHECK (record_id BETWEEN 1 AND 9007199254740991),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
);

CREATE TABLE memory_events (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    record_id INTEGER CHECK (record_id BETWEEN 1 AND 9007199254740991),
    action TEXT NOT NULL CHECK (action IN ('created', 'updated', 'deleted', 'retention_deleted')),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('human', 'migration', 'import', 'maintenance')),
    record_revision INTEGER NOT NULL CHECK (record_revision >= 1),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
);

INSERT INTO memory_records (
    id,
    scope_kind,
    agent_id,
    record_kind,
    content,
    provenance_kind,
    provenance_ref,
    revision,
    retention_policy,
    created_at_unix_ms,
    updated_at_unix_ms
)
SELECT
    agent.id,
    'agent',
    agent.id,
    'instruction',
    agent.memory,
    'legacy_agent_memory',
    'schema-v10-agent-memory',
    1,
    'manual',
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
FROM agents AS agent
WHERE length(trim(agent.memory)) > 0;

INSERT INTO memory_events (
    id,
    record_id,
    action,
    actor_kind,
    record_revision,
    created_at_unix_ms
)
SELECT
    record.id,
    record.id,
    'created',
    'migration',
    1,
    record.created_at_unix_ms
FROM memory_records AS record;

INSERT INTO structured_memory_meta (
    singleton,
    next_record_id,
    next_event_id
)
SELECT
    1,
    COALESCE((SELECT MAX(id) + 1 FROM memory_records), 1),
    COALESCE((SELECT MAX(id) + 1 FROM memory_events), 1);

UPDATE agents SET memory = '';

CREATE TABLE management_handoff_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    next_handoff_id INTEGER NOT NULL DEFAULT 1
        CHECK (next_handoff_id BETWEEN 1 AND 9007199254740991)
);

CREATE TABLE management_handoffs (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    task_owner_agent_id INTEGER NOT NULL CHECK (task_owner_agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER NOT NULL CHECK (task_id BETWEEN 1 AND 9007199254740991),
    kind TEXT NOT NULL
        CHECK (kind IN ('task_plan', 'assignment', 'execution_evidence', 'review_decision',
                        'revision_request', 'human_override', 'failure', 'recovery')),
    from_agent_id INTEGER CHECK (from_agent_id BETWEEN 1 AND 9007199254740991),
    to_agent_id INTEGER CHECK (to_agent_id BETWEEN 1 AND 9007199254740991),
    owner_role TEXT NOT NULL CHECK (owner_role IN ('senior', 'team_leader', 'supervisor', 'human')),
    revision_round INTEGER NOT NULL DEFAULT 0 CHECK (revision_round BETWEEN 0 AND 10000),
    run_attempt_id INTEGER CHECK (run_attempt_id BETWEEN 1 AND 9007199254740991),
    review_flow_id INTEGER CHECK (review_flow_id BETWEEN 1 AND 9007199254740991),
    review_stage_attempt_id INTEGER CHECK (review_stage_attempt_id BETWEEN 1 AND 9007199254740991),
    source_kind TEXT NOT NULL
        CHECK (source_kind IN ('task_orchestration', 'run_coordinator', 'review_orchestration',
                               'human_decision', 'migration_v11')),
    summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 32768),
    payload_json TEXT NOT NULL CHECK (length(payload_json) BETWEEN 2 AND 131072),
    idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 1 AND 512),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
);

CREATE INDEX management_handoffs_task_v11
ON management_handoffs (task_owner_agent_id, task_id, created_at_unix_ms, id);

CREATE INDEX management_handoffs_role_v11
ON management_handoffs (owner_role, created_at_unix_ms DESC, id DESC);

INSERT INTO management_handoffs (
    id,
    task_owner_agent_id,
    task_id,
    kind,
    from_agent_id,
    to_agent_id,
    owner_role,
    revision_round,
    source_kind,
    summary,
    payload_json,
    idempotency_key,
    created_at_unix_ms
)
SELECT
    ROW_NUMBER() OVER (ORDER BY task.owner_agent_id, task.id),
    task.owner_agent_id,
    task.id,
    'task_plan',
    task.owner_agent_id,
    task.assigned_agent_id,
    CASE owner.role
        WHEN 'Supervisor' THEN 'supervisor'
        WHEN 'Team Leader' THEN 'team_leader'
        WHEN 'Senior Agent' THEN 'senior'
        ELSE 'human'
    END,
    0,
    'migration_v11',
    'Historical task plan retained from schema v10.',
    '{"historical":true,"evidenceCompleteness":"retained"}',
    'migration-v11-task-plan:' || task.owner_agent_id || ':' || task.id,
    COALESCE(
        task.created_at_unix_ms,
        CAST(strftime('%s', task.created_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    )
FROM agent_tasks AS task
JOIN agents AS owner ON owner.id = task.owner_agent_id;

INSERT INTO management_handoffs (
    id,
    task_owner_agent_id,
    task_id,
    kind,
    from_agent_id,
    to_agent_id,
    owner_role,
    revision_round,
    source_kind,
    summary,
    payload_json,
    idempotency_key,
    created_at_unix_ms
)
SELECT
    (SELECT COUNT(*) FROM agent_tasks) +
        ROW_NUMBER() OVER (ORDER BY task.owner_agent_id, task.id),
    task.owner_agent_id,
    task.id,
    'assignment',
    task.owner_agent_id,
    task.assigned_agent_id,
    CASE owner.role
        WHEN 'Supervisor' THEN 'supervisor'
        WHEN 'Team Leader' THEN 'team_leader'
        WHEN 'Senior Agent' THEN 'senior'
        ELSE 'human'
    END,
    0,
    'migration_v11',
    'Historical task assignment retained from schema v10.',
    json_object(
        'historical', json('true'),
        'evidenceCompleteness', 'retained',
        'assignedAgentId', task.assigned_agent_id
    ),
    'migration-v11-assignment:' || task.owner_agent_id || ':' || task.id,
    COALESCE(
        task.created_at_unix_ms,
        CAST(strftime('%s', task.created_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    )
FROM agent_tasks AS task
JOIN agents AS owner ON owner.id = task.owner_agent_id;

INSERT INTO management_handoff_meta (singleton, next_handoff_id)
SELECT 1, COALESCE((SELECT MAX(id) + 1 FROM management_handoffs), 1);

ALTER TABLE run_attempts
ADD COLUMN memory_bundle_json TEXT;

ALTER TABLE run_attempts
ADD COLUMN memory_bundle_sha256 TEXT;

ALTER TABLE data_lifecycle_meta
ADD COLUMN total_pruned_memory_records INTEGER NOT NULL DEFAULT 0
    CHECK (total_pruned_memory_records >= 0);

ALTER TABLE data_lifecycle_meta
ADD COLUMN total_pruned_reminder_occurrences INTEGER NOT NULL DEFAULT 0
    CHECK (total_pruned_reminder_occurrences >= 0);

ALTER TABLE data_lifecycle_meta
ADD COLUMN total_pruned_management_handoffs INTEGER NOT NULL DEFAULT 0
    CHECK (total_pruned_management_handoffs >= 0);

ALTER TABLE data_lifecycle_runs
ADD COLUMN pruned_memory_records INTEGER NOT NULL DEFAULT 0
    CHECK (pruned_memory_records >= 0);

ALTER TABLE data_lifecycle_runs
ADD COLUMN pruned_reminder_occurrences INTEGER NOT NULL DEFAULT 0
    CHECK (pruned_reminder_occurrences >= 0);

ALTER TABLE data_lifecycle_runs
ADD COLUMN pruned_management_handoffs INTEGER NOT NULL DEFAULT 0
    CHECK (pruned_management_handoffs >= 0);
