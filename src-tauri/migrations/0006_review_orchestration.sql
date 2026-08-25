CREATE TABLE review_orchestration_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991)
);

INSERT INTO review_orchestration_meta (singleton) VALUES (1);

CREATE TABLE review_flows (
    id INTEGER PRIMARY KEY AUTOINCREMENT
        CHECK (id BETWEEN 1 AND 9007199254740991),
    task_owner_agent_id INTEGER NOT NULL
        CHECK (task_owner_agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER NOT NULL CHECK (task_id BETWEEN 1 AND 9007199254740991),
    executor_agent_id INTEGER NOT NULL
        CHECK (executor_agent_id BETWEEN 1 AND 9007199254740991),
    pipeline_version TEXT NOT NULL CHECK (pipeline_version = 'review-pipeline-v1'),
    state TEXT NOT NULL CHECK (
        state IN (
            'awaiting_execution',
            'awaiting_review',
            'review_pending',
            'reviewing',
            'awaiting_human',
            'revision_queued',
            'completed',
            'failed',
            'cancelled'
        )
    ),
    revision_round INTEGER NOT NULL DEFAULT 0 CHECK (revision_round BETWEEN 0 AND 3),
    max_revisions INTEGER NOT NULL DEFAULT 3 CHECK (max_revisions = 3),
    required_levels_json TEXT NOT NULL CHECK (
        json_valid(required_levels_json)
        AND json_type(required_levels_json) = 'array'
    ),
    current_level TEXT CHECK (current_level IN ('senior', 'team_leader', 'supervisor')),
    latest_execution_attempt_id INTEGER,
    review_mode TEXT NOT NULL CHECK (review_mode IN ('manual', 'automatic')),
    last_error_code TEXT,
    last_error_message TEXT CHECK (
        last_error_message IS NULL OR length(CAST(last_error_message AS BLOB)) <= 65536
    ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (completed_at_unix_ms >= 0),
    FOREIGN KEY (latest_execution_attempt_id) REFERENCES run_attempts(id) ON DELETE SET NULL,
    CHECK (
        (state IN ('completed', 'failed', 'cancelled') AND completed_at_unix_ms IS NOT NULL)
        OR
        (state NOT IN ('completed', 'failed', 'cancelled') AND completed_at_unix_ms IS NULL)
    )
);

CREATE UNIQUE INDEX review_flows_one_active_task
ON review_flows (task_owner_agent_id, task_id)
WHERE state NOT IN ('completed', 'failed', 'cancelled');

CREATE INDEX review_flows_task_history
ON review_flows (task_owner_agent_id, task_id, id DESC);

CREATE TABLE review_stage_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT
        CHECK (id BETWEEN 1 AND 9007199254740991),
    flow_id INTEGER NOT NULL CHECK (flow_id BETWEEN 1 AND 9007199254740991),
    revision_round INTEGER NOT NULL CHECK (revision_round BETWEEN 0 AND 3),
    level TEXT NOT NULL CHECK (level IN ('senior', 'team_leader', 'supervisor')),
    attempt_number INTEGER NOT NULL CHECK (attempt_number BETWEEN 1 AND 4),
    actor TEXT NOT NULL CHECK (actor IN ('agent', 'human')),
    reviewer_agent_id INTEGER CHECK (reviewer_agent_id BETWEEN 1 AND 9007199254740991),
    state TEXT NOT NULL CHECK (
        state IN (
            'pending',
            'admitted',
            'running',
            'approved',
            'changes_requested',
            'invalid',
            'cancelled',
            'failed',
            'interrupted'
        )
    ),
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    request_fingerprint TEXT NOT NULL CHECK (
        substr(request_fingerprint, 1, 18) = 'review-request-v1:'
        AND substr(request_fingerprint, 19) NOT GLOB '*[^0-9a-f]*'
        AND length(request_fingerprint) = 82
    ),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    verdict TEXT CHECK (verdict IN ('approved', 'changes_requested')),
    feedback TEXT CHECK (feedback IS NULL OR length(CAST(feedback AS BLOB)) <= 32768),
    run_attempt_id INTEGER UNIQUE,
    error_code TEXT,
    error_message TEXT CHECK (
        error_message IS NULL OR length(CAST(error_message AS BLOB)) <= 65536
    ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    started_at_unix_ms INTEGER CHECK (started_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (completed_at_unix_ms >= 0),
    UNIQUE (flow_id, revision_round, level, attempt_number),
    FOREIGN KEY (flow_id) REFERENCES review_flows(id) ON DELETE CASCADE,
    CHECK (
        (actor = 'agent' AND reviewer_agent_id IS NOT NULL)
        OR (actor = 'human' AND reviewer_agent_id IS NULL)
    ),
    CHECK (
        (state = 'approved' AND verdict = 'approved' AND completed_at_unix_ms IS NOT NULL)
        OR (state = 'changes_requested' AND verdict = 'changes_requested'
            AND completed_at_unix_ms IS NOT NULL)
        OR (state IN ('invalid', 'cancelled', 'failed', 'interrupted')
            AND verdict IS NULL AND completed_at_unix_ms IS NOT NULL)
        OR (state IN ('pending', 'admitted', 'running')
            AND verdict IS NULL AND completed_at_unix_ms IS NULL)
    )
);

CREATE INDEX review_stage_attempts_flow_history
ON review_stage_attempts (flow_id, revision_round, id);

CREATE INDEX review_stage_attempts_active
ON review_stage_attempts (flow_id, state)
WHERE state IN ('pending', 'admitted', 'running');

ALTER TABLE run_attempts
ADD COLUMN review_flow_id INTEGER;

ALTER TABLE run_attempts
ADD COLUMN review_stage_attempt_id INTEGER;

ALTER TABLE run_attempts
ADD COLUMN review_revision_round INTEGER
    CHECK (review_revision_round IS NULL OR review_revision_round BETWEEN 0 AND 3);

CREATE UNIQUE INDEX run_attempts_review_stage_unique
ON run_attempts (review_stage_attempt_id)
WHERE review_stage_attempt_id IS NOT NULL;

CREATE TRIGGER run_attempts_review_binding_insert
BEFORE INSERT ON run_attempts
WHEN (
    NEW.run_mode = 'execute'
    AND (NEW.review_flow_id IS NOT NULL
         OR NEW.review_stage_attempt_id IS NOT NULL
         OR NEW.review_revision_round IS NOT NULL)
) OR (
    NEW.run_mode = 'review'
    AND NEW.intent_fingerprint <> 'legacy-reconciliation'
    AND (NEW.review_flow_id IS NULL
         OR NEW.review_stage_attempt_id IS NULL
         OR NEW.review_revision_round IS NULL)
)
BEGIN
    SELECT RAISE(ABORT, 'run mode and review binding are inconsistent');
END;

CREATE TRIGGER run_attempts_review_binding_update
BEFORE UPDATE OF run_mode, review_flow_id, review_stage_attempt_id, review_revision_round
ON run_attempts
WHEN (
    NEW.run_mode = 'execute'
    AND (NEW.review_flow_id IS NOT NULL
         OR NEW.review_stage_attempt_id IS NOT NULL
         OR NEW.review_revision_round IS NOT NULL)
) OR (
    NEW.run_mode = 'review'
    AND NEW.intent_fingerprint <> 'legacy-reconciliation'
    AND (NEW.review_flow_id IS NULL
         OR NEW.review_stage_attempt_id IS NULL
         OR NEW.review_revision_round IS NULL)
)
BEGIN
    SELECT RAISE(ABORT, 'run mode and review binding are inconsistent');
END;

CREATE TRIGGER review_stage_attempts_terminal_immutable
BEFORE UPDATE ON review_stage_attempts
WHEN OLD.state IN ('approved', 'changes_requested', 'invalid', 'cancelled', 'failed', 'interrupted')
BEGIN
    SELECT RAISE(ABORT, 'terminal review stage attempts are immutable');
END;

INSERT INTO review_flows (
    task_owner_agent_id,
    task_id,
    executor_agent_id,
    pipeline_version,
    state,
    revision_round,
    max_revisions,
    required_levels_json,
    current_level,
    latest_execution_attempt_id,
    review_mode,
    last_error_code,
    last_error_message,
    created_at_unix_ms,
    updated_at_unix_ms
)
SELECT
    task.owner_agent_id,
    task.id,
    task.assigned_agent_id,
    'review-pipeline-v1',
    'awaiting_human',
    0,
    3,
    CASE executor.role
        WHEN 'Specialist' THEN '["senior","team_leader","supervisor"]'
        WHEN 'Senior Agent' THEN '["team_leader","supervisor"]'
        WHEN 'Team Leader' THEN '["supervisor"]'
        ELSE '[]'
    END,
    CASE task.phase
        WHEN 'Senior Review' THEN 'senior'
        WHEN 'Team Leader Review' THEN 'team_leader'
        WHEN 'Supervisor Approval' THEN 'supervisor'
        WHEN 'Assigned' THEN CASE executor.role
            WHEN 'Specialist' THEN 'senior'
            WHEN 'Senior Agent' THEN 'team_leader'
            WHEN 'Team Leader' THEN 'supervisor'
            ELSE NULL
        END
        ELSE NULL
    END,
    (
        SELECT MAX(attempt.id)
        FROM run_attempts AS attempt
        WHERE attempt.task_owner_agent_id = task.owner_agent_id
          AND attempt.task_id = task.id
          AND attempt.run_mode = 'execute'
          AND attempt.status = 'succeeded'
    ),
    CASE preferences.review_mode WHEN 'automatic' THEN 'automatic' ELSE 'manual' END,
    'LEGACY_REVIEW_UNBOUND',
    'Legacy review state had no exact flow, round, stage, or request binding. Human review is required before transition.',
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
FROM agent_tasks AS task
JOIN agents AS executor ON executor.id = task.assigned_agent_id
JOIN preferences ON preferences.singleton = 1
WHERE task.status = 'Under Review'
   OR task.review_status IN ('Pending', 'Running', 'Failed', 'Changes Requested');
