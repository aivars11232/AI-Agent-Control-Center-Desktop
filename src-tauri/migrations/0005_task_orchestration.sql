ALTER TABLE preferences
ADD COLUMN default_queue_threshold INTEGER NOT NULL DEFAULT 10
    CHECK (default_queue_threshold BETWEEN 1 AND 100);

ALTER TABLE agents
ADD COLUMN queue_threshold INTEGER NOT NULL DEFAULT 10
    CHECK (queue_threshold BETWEEN 1 AND 100);

ALTER TABLE agent_tasks
ADD COLUMN queue_state TEXT NOT NULL DEFAULT 'notQueued'
    CHECK (queue_state IN ('queued', 'held', 'admitted', 'running', 'notQueued'));

ALTER TABLE agent_tasks
ADD COLUMN enqueue_sequence INTEGER
    CHECK (enqueue_sequence IS NULL OR enqueue_sequence BETWEEN 1 AND 9007199254740991);

ALTER TABLE agent_tasks
ADD COLUMN routing_evidence_json TEXT
    CHECK (routing_evidence_json IS NULL OR json_valid(routing_evidence_json));

UPDATE agent_tasks
SET queue_state = CASE status
    WHEN 'Pending' THEN 'queued'
    WHEN 'Blocked' THEN 'held'
    WHEN 'Running' THEN 'running'
    ELSE 'notQueued'
END;

WITH ordered AS (
    SELECT owner_agent_id, id,
           ROW_NUMBER() OVER (
               ORDER BY created_at, owner_agent_id, id
           ) AS sequence
    FROM agent_tasks
    WHERE queue_state IN ('queued', 'held', 'admitted', 'running')
)
UPDATE agent_tasks
SET enqueue_sequence = (
    SELECT sequence
    FROM ordered
    WHERE ordered.owner_agent_id = agent_tasks.owner_agent_id
      AND ordered.id = agent_tasks.id
)
WHERE queue_state IN ('queued', 'held', 'admitted', 'running');

CREATE UNIQUE INDEX agent_tasks_enqueue_sequence_unique
ON agent_tasks (enqueue_sequence)
WHERE enqueue_sequence IS NOT NULL;

CREATE INDEX agent_tasks_execute_queue
ON agent_tasks (
    queue_state,
    CASE priority
        WHEN 'Critical' THEN 0
        WHEN 'High' THEN 1
        WHEN 'Normal' THEN 2
        ELSE 3
    END,
    enqueue_sequence,
    owner_agent_id,
    id
);

CREATE TABLE task_orchestration_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL DEFAULT 0
        CHECK (revision BETWEEN 0 AND 9007199254740991),
    next_task_id INTEGER NOT NULL
        CHECK (next_task_id BETWEEN 1 AND 9007199254740991),
    next_enqueue_sequence INTEGER NOT NULL
        CHECK (next_enqueue_sequence BETWEEN 1 AND 9007199254740991)
);

INSERT INTO task_orchestration_meta (
    singleton,
    revision,
    next_task_id,
    next_enqueue_sequence
)
SELECT
    1,
    0,
    CASE
        WHEN COALESCE(MAX(id), 0) < 9007199254740991
            THEN COALESCE(MAX(id), 0) + 1
        ELSE 9007199254740991
    END,
    CASE
        WHEN COALESCE(MAX(enqueue_sequence), 0) < 9007199254740991
            THEN COALESCE(MAX(enqueue_sequence), 0) + 1
        ELSE 9007199254740991
    END
FROM agent_tasks;

CREATE TRIGGER agent_tasks_queue_insert_consistent
BEFORE INSERT ON agent_tasks
WHEN (
    NEW.queue_state IN ('queued', 'held', 'admitted', 'running')
    AND NEW.enqueue_sequence IS NULL
) OR (
    NEW.queue_state = 'notQueued'
    AND NEW.enqueue_sequence IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'task queue state and enqueue sequence are inconsistent');
END;

CREATE TRIGGER agent_tasks_queue_update_consistent
BEFORE UPDATE OF queue_state, enqueue_sequence ON agent_tasks
WHEN (
    NEW.queue_state IN ('queued', 'held', 'admitted', 'running')
    AND NEW.enqueue_sequence IS NULL
) OR (
    NEW.queue_state = 'notQueued'
    AND NEW.enqueue_sequence IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'task queue state and enqueue sequence are inconsistent');
END;
