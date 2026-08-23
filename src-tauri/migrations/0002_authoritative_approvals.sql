CREATE TABLE approval_requests_v2 (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    agent_id INTEGER NOT NULL CHECK (agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER CHECK (task_id BETWEEN 1 AND 9007199254740991),
    title TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('Pending', 'Approved', 'Denied', 'Expired')),
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    risk_level TEXT NOT NULL CHECK (risk_level IN ('Low', 'Medium', 'High', 'Critical')),
    workspace_id TEXT,
    task_snapshot TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    origin TEXT NOT NULL CHECK (
        origin IN (
            'fresh',
            'legacy_local_storage',
            'legacy_backup',
            'renderer_prototype',
            'backend_authority'
        )
    ),
    authoritative INTEGER NOT NULL DEFAULT 0 CHECK (authoritative IN (0, 1)),
    intent_kind TEXT NOT NULL DEFAULT '',
    intent_json TEXT NOT NULL DEFAULT '',
    intent_fingerprint TEXT NOT NULL DEFAULT '',
    policy_fingerprint TEXT NOT NULL DEFAULT '',
    workspace_fingerprint TEXT NOT NULL DEFAULT '',
    created_at_unix_ms INTEGER,
    resolved_at_unix_ms INTEGER,
    expires_at_unix_ms INTEGER,
    consumed_at_unix_ms INTEGER,
    CHECK (
        authoritative = 0 OR (
            origin = 'backend_authority'
            AND intent_kind <> ''
            AND intent_json <> ''
            AND intent_fingerprint <> ''
            AND policy_fingerprint <> ''
            AND workspace_fingerprint <> ''
            AND created_at_unix_ms IS NOT NULL
            AND expires_at_unix_ms IS NOT NULL
            AND expires_at_unix_ms > created_at_unix_ms
        )
    )
);

INSERT INTO approval_requests_v2 (
    id, position, agent_id, task_id, title, reason, status, created_at,
    resolved_at, risk_level, workspace_id, task_snapshot, expires_at,
    consumed_at, origin, authoritative
)
SELECT
    id,
    position,
    agent_id,
    task_id,
    title,
    reason,
    CASE WHEN status IN ('Pending', 'Approved') THEN 'Expired' ELSE status END,
    created_at,
    resolved_at,
    risk_level,
    workspace_id,
    task_snapshot,
    expires_at,
    consumed_at,
    origin,
    0
FROM approval_requests;

CREATE TABLE approval_scopes_v2 (
    approval_id INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    scope TEXT NOT NULL CHECK (scope IN ('files', 'internet', 'clipboard', 'terminal', 'system')),
    PRIMARY KEY (approval_id, position),
    UNIQUE (approval_id, scope),
    FOREIGN KEY (approval_id) REFERENCES approval_requests_v2(id) ON DELETE CASCADE
);

INSERT INTO approval_scopes_v2 (approval_id, position, scope)
SELECT approval_id, position, scope FROM approval_scopes;

DROP TABLE approval_scopes;
DROP TABLE approval_requests;
ALTER TABLE approval_requests_v2 RENAME TO approval_requests;
ALTER TABLE approval_scopes_v2 RENAME TO approval_scopes;

CREATE INDEX approval_requests_active_authority
ON approval_requests (
    authoritative,
    status,
    consumed_at_unix_ms,
    expires_at_unix_ms,
    intent_fingerprint,
    policy_fingerprint,
    workspace_fingerprint
);
