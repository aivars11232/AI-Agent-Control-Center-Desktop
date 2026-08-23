CREATE TABLE run_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT
        CHECK (id BETWEEN 1 AND 9007199254740991),
    request_id TEXT NOT NULL UNIQUE,
    intent_json TEXT NOT NULL,
    intent_fingerprint TEXT NOT NULL,
    policy_fingerprint TEXT NOT NULL,
    workspace_fingerprint TEXT NOT NULL,
    agent_id INTEGER NOT NULL CHECK (agent_id BETWEEN 1 AND 9007199254740991),
    task_owner_agent_id INTEGER NOT NULL
        CHECK (task_owner_agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER NOT NULL CHECK (task_id BETWEEN 1 AND 9007199254740991),
    task_title TEXT NOT NULL,
    run_mode TEXT NOT NULL CHECK (run_mode IN ('execute', 'review')),
    status TEXT NOT NULL CHECK (
        status IN (
            'admitted',
            'starting',
            'dispatching',
            'running',
            'cancel_requested',
            'succeeded',
            'cancelled',
            'timed_out',
            'startup_failed',
            'failed',
            'interrupted'
        )
    ),
    provider TEXT,
    model TEXT,
    workspace_id TEXT,
    approval_id INTEGER,
    task_status_before TEXT NOT NULL,
    task_phase_before TEXT NOT NULL,
    review_status_before TEXT NOT NULL,
    admitted_at_unix_ms INTEGER NOT NULL CHECK (admitted_at_unix_ms >= 0),
    started_at_unix_ms INTEGER CHECK (started_at_unix_ms >= 0),
    cancel_requested_at_unix_ms INTEGER CHECK (cancel_requested_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (completed_at_unix_ms >= 0),
    duration_seconds INTEGER CHECK (duration_seconds >= 0),
    output_summary TEXT,
    stderr_excerpt TEXT,
    response_id TEXT,
    input_tokens INTEGER CHECK (input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens >= 0),
    total_tokens INTEGER CHECK (total_tokens >= 0),
    changed_files_json TEXT NOT NULL DEFAULT '[]',
    diff TEXT,
    error_code TEXT,
    error_message TEXT,
    progress_event_count INTEGER NOT NULL DEFAULT 0
        CHECK (progress_event_count >= 0),
    progress_bytes INTEGER NOT NULL DEFAULT 0 CHECK (progress_bytes >= 0),
    payload_bytes INTEGER NOT NULL DEFAULT 0 CHECK (payload_bytes >= 0),
    stdout_truncated INTEGER NOT NULL DEFAULT 0 CHECK (stdout_truncated IN (0, 1)),
    stderr_truncated INTEGER NOT NULL DEFAULT 0 CHECK (stderr_truncated IN (0, 1)),
    summary_truncated INTEGER NOT NULL DEFAULT 0 CHECK (summary_truncated IN (0, 1)),
    diff_truncated INTEGER NOT NULL DEFAULT 0 CHECK (diff_truncated IN (0, 1)),
    changed_files_truncated INTEGER NOT NULL DEFAULT 0
        CHECK (changed_files_truncated IN (0, 1)),
    progress_truncated INTEGER NOT NULL DEFAULT 0
        CHECK (progress_truncated IN (0, 1)),
    before_snapshot_truncated INTEGER NOT NULL DEFAULT 0
        CHECK (before_snapshot_truncated IN (0, 1)),
    after_snapshot_truncated INTEGER NOT NULL DEFAULT 0
        CHECK (after_snapshot_truncated IN (0, 1)),
    original_stdout_bytes INTEGER NOT NULL DEFAULT 0 CHECK (original_stdout_bytes >= 0),
    original_stderr_bytes INTEGER NOT NULL DEFAULT 0 CHECK (original_stderr_bytes >= 0),
    original_summary_bytes INTEGER NOT NULL DEFAULT 0 CHECK (original_summary_bytes >= 0),
    original_diff_bytes INTEGER NOT NULL DEFAULT 0 CHECK (original_diff_bytes >= 0),
    original_changed_file_count INTEGER NOT NULL DEFAULT 0
        CHECK (original_changed_file_count >= 0),
    omitted_progress_event_count INTEGER NOT NULL DEFAULT 0
        CHECK (omitted_progress_event_count >= 0),
    recovery_disposition TEXT,
    CHECK (
        (status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed', 'failed', 'interrupted')
            AND completed_at_unix_ms IS NOT NULL)
        OR
        (status NOT IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed', 'failed', 'interrupted')
            AND completed_at_unix_ms IS NULL)
    )
);

CREATE TABLE run_coordinator_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    active_attempt_id INTEGER UNIQUE,
    retained_attempt_count INTEGER NOT NULL DEFAULT 0
        CHECK (retained_attempt_count >= 0),
    retained_payload_bytes INTEGER NOT NULL DEFAULT 0
        CHECK (retained_payload_bytes >= 0),
    pruned_attempt_count INTEGER NOT NULL DEFAULT 0
        CHECK (pruned_attempt_count >= 0),
    last_pruned_at_unix_ms INTEGER CHECK (last_pruned_at_unix_ms >= 0),
    FOREIGN KEY (active_attempt_id) REFERENCES run_attempts(id) ON DELETE SET NULL
);

INSERT INTO run_coordinator_meta (singleton) VALUES (1);

CREATE TABLE run_events (
    attempt_id INTEGER NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    kind TEXT NOT NULL CHECK (kind IN ('status', 'progress', 'complete', 'error')),
    message TEXT NOT NULL,
    message_truncated INTEGER NOT NULL DEFAULT 0
        CHECK (message_truncated IN (0, 1)),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    PRIMARY KEY (attempt_id, sequence),
    FOREIGN KEY (attempt_id) REFERENCES run_attempts(id) ON DELETE CASCADE
);

CREATE TABLE run_approval_reservations (
    attempt_id INTEGER PRIMARY KEY,
    approval_id INTEGER NOT NULL UNIQUE,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    FOREIGN KEY (attempt_id) REFERENCES run_attempts(id) ON DELETE CASCADE,
    FOREIGN KEY (approval_id) REFERENCES approval_requests(id) ON DELETE RESTRICT
);

CREATE INDEX run_attempts_task_history
ON run_attempts (task_owner_agent_id, task_id, id DESC);

CREATE INDEX run_attempts_terminal_retention
ON run_attempts (completed_at_unix_ms, id)
WHERE status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed', 'failed', 'interrupted');

CREATE TRIGGER run_attempts_terminal_immutable
BEFORE UPDATE ON run_attempts
WHEN OLD.status IN ('succeeded', 'cancelled', 'timed_out', 'startup_failed', 'failed', 'interrupted')
BEGIN
    SELECT RAISE(ABORT, 'terminal run attempts are immutable');
END;
