ALTER TABLE application_meta RENAME TO application_meta_v7;

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
FROM application_meta_v7;

DROP TABLE application_meta_v7;

ALTER TABLE agent_tasks
ADD COLUMN created_at_unix_ms INTEGER CHECK (created_at_unix_ms >= 0);

ALTER TABLE agent_tasks
ADD COLUMN completed_at_unix_ms INTEGER CHECK (completed_at_unix_ms >= 0);

ALTER TABLE agent_activity
ADD COLUMN created_at_unix_ms INTEGER CHECK (created_at_unix_ms >= 0);

ALTER TABLE reminders
ADD COLUMN created_at_unix_ms INTEGER CHECK (created_at_unix_ms >= 0);

ALTER TABLE reminders
ADD COLUMN resolved_at_unix_ms INTEGER CHECK (resolved_at_unix_ms >= 0);

CREATE TABLE data_lifecycle_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    last_observed_at_unix_ms INTEGER CHECK (last_observed_at_unix_ms >= 0),
    last_started_at_unix_ms INTEGER CHECK (last_started_at_unix_ms >= 0),
    last_completed_at_unix_ms INTEGER CHECK (last_completed_at_unix_ms >= 0),
    last_success_at_unix_ms INTEGER CHECK (last_success_at_unix_ms >= 0),
    last_error_code TEXT,
    last_error_message TEXT,
    total_runs INTEGER NOT NULL DEFAULT 0 CHECK (total_runs >= 0),
    total_pruned_tasks INTEGER NOT NULL DEFAULT 0 CHECK (total_pruned_tasks >= 0),
    total_pruned_attempts INTEGER NOT NULL DEFAULT 0 CHECK (total_pruned_attempts >= 0),
    total_pruned_review_flows INTEGER NOT NULL DEFAULT 0
        CHECK (total_pruned_review_flows >= 0),
    total_pruned_activity INTEGER NOT NULL DEFAULT 0 CHECK (total_pruned_activity >= 0),
    total_pruned_approvals INTEGER NOT NULL DEFAULT 0 CHECK (total_pruned_approvals >= 0),
    total_pruned_reminders INTEGER NOT NULL DEFAULT 0 CHECK (total_pruned_reminders >= 0),
    inferred_timestamp_count INTEGER NOT NULL DEFAULT 0
        CHECK (inferred_timestamp_count >= 0)
);

INSERT INTO data_lifecycle_meta (singleton, inferred_timestamp_count)
SELECT 1,
    (SELECT COUNT(*) FROM agent_tasks WHERE strftime('%s', created_at) IS NULL)
    + (SELECT COUNT(*) FROM agent_tasks
       WHERE status IN ('Completed', 'Failed')
         AND (completed_at IS NULL OR strftime('%s', completed_at) IS NULL))
    + (SELECT COUNT(*) FROM agent_activity WHERE strftime('%s', created_at) IS NULL)
    + (SELECT COUNT(*) FROM reminders WHERE strftime('%s', created_at) IS NULL)
    + (SELECT COUNT(*) FROM reminders
       WHERE status IN ('Completed', 'Dismissed'))
    + (SELECT COUNT(*) FROM approval_requests
       WHERE created_at_unix_ms IS NULL AND strftime('%s', created_at) IS NULL)
    + (SELECT COUNT(*) FROM approval_requests
       WHERE (status IN ('Denied', 'Expired') OR consumed_at IS NOT NULL)
         AND resolved_at_unix_ms IS NULL
         AND strftime('%s', resolved_at) IS NULL
         AND strftime('%s', consumed_at) IS NULL)
    + (SELECT COUNT(*) FROM approval_requests
       WHERE expires_at_unix_ms IS NULL AND strftime('%s', expires_at) IS NULL);

UPDATE agent_tasks
SET created_at_unix_ms = COALESCE(
    CAST(strftime('%s', created_at) AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);

UPDATE agent_tasks
SET completed_at_unix_ms = CASE
    WHEN status IN ('Completed', 'Failed') THEN COALESCE(
        CAST(strftime('%s', completed_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    )
    ELSE NULL
END;

UPDATE agent_activity
SET created_at_unix_ms = COALESCE(
    CAST(strftime('%s', created_at) AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);

UPDATE reminders
SET created_at_unix_ms = COALESCE(
    CAST(strftime('%s', created_at) AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
),
resolved_at_unix_ms = CASE
    WHEN status IN ('Completed', 'Dismissed')
        THEN CAST(strftime('%s', 'now') AS INTEGER) * 1000
    ELSE NULL
END;

UPDATE approval_requests
SET created_at_unix_ms = COALESCE(
        created_at_unix_ms,
        CAST(strftime('%s', created_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    ),
    resolved_at_unix_ms = CASE
        WHEN status IN ('Denied', 'Expired') OR consumed_at IS NOT NULL
            THEN COALESCE(
                resolved_at_unix_ms,
                CAST(strftime('%s', resolved_at) AS INTEGER) * 1000,
                CAST(strftime('%s', 'now') AS INTEGER) * 1000
            )
        ELSE resolved_at_unix_ms
    END,
    expires_at_unix_ms = COALESCE(
        expires_at_unix_ms,
        CAST(strftime('%s', expires_at) AS INTEGER) * 1000,
        CAST(strftime('%s', 'now') AS INTEGER) * 1000
    ),
    consumed_at_unix_ms = COALESCE(
        consumed_at_unix_ms,
        CAST(strftime('%s', consumed_at) AS INTEGER) * 1000
    );

CREATE INDEX agent_tasks_retention_v8
ON agent_tasks (status, completed_at_unix_ms, owner_agent_id, id);

CREATE INDEX agent_activity_retention_v8
ON agent_activity (created_at_unix_ms, owner_agent_id, id);

CREATE INDEX reminders_retention_v8
ON reminders (status, resolved_at_unix_ms, id);

CREATE INDEX approval_requests_retention_v8
ON approval_requests (
    authoritative,
    status,
    consumed_at_unix_ms,
    resolved_at_unix_ms,
    expires_at_unix_ms,
    id
);

CREATE TABLE data_lifecycle_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT
        CHECK (id BETWEEN 1 AND 9007199254740991),
    lifecycle_revision INTEGER NOT NULL
        CHECK (lifecycle_revision BETWEEN 1 AND 9007199254740991),
    application_state_revision INTEGER NOT NULL
        CHECK (application_state_revision BETWEEN 0 AND 9007199254740991),
    trigger_kind TEXT NOT NULL CHECK (
        trigger_kind IN ('startup', 'interval', 'settings', 'import', 'test')
    ),
    status TEXT NOT NULL CHECK (
        status IN ('succeeded', 'failed', 'clock_rollback')
    ),
    started_at_unix_ms INTEGER NOT NULL CHECK (started_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER NOT NULL CHECK (completed_at_unix_ms >= 0),
    task_cutoff_unix_ms INTEGER CHECK (task_cutoff_unix_ms >= 0),
    activity_cutoff_unix_ms INTEGER CHECK (activity_cutoff_unix_ms >= 0),
    pruned_tasks INTEGER NOT NULL DEFAULT 0 CHECK (pruned_tasks >= 0),
    pruned_attempts INTEGER NOT NULL DEFAULT 0 CHECK (pruned_attempts >= 0),
    pruned_review_flows INTEGER NOT NULL DEFAULT 0 CHECK (pruned_review_flows >= 0),
    pruned_activity INTEGER NOT NULL DEFAULT 0 CHECK (pruned_activity >= 0),
    pruned_approvals INTEGER NOT NULL DEFAULT 0 CHECK (pruned_approvals >= 0),
    pruned_reminders INTEGER NOT NULL DEFAULT 0 CHECK (pruned_reminders >= 0),
    skipped_protected INTEGER NOT NULL DEFAULT 0 CHECK (skipped_protected >= 0),
    backlog_remaining INTEGER NOT NULL DEFAULT 0 CHECK (backlog_remaining IN (0, 1)),
    error_code TEXT,
    error_message TEXT
);

CREATE INDEX data_lifecycle_runs_recent_v8
ON data_lifecycle_runs (id DESC);
