CREATE TABLE system_action_audits (
    id INTEGER PRIMARY KEY AUTOINCREMENT
        CHECK (id BETWEEN 1 AND 9007199254740991),
    request_id TEXT NOT NULL UNIQUE,
    request_fingerprint TEXT NOT NULL,
    intent_kind TEXT NOT NULL,
    risk_class TEXT NOT NULL CHECK (
        risk_class IN ('reversible', 'meaningful', 'destructive')
    ),
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    agent_id INTEGER NOT NULL
        CHECK (agent_id BETWEEN 1 AND 9007199254740991),
    task_owner_agent_id INTEGER CHECK (
        task_owner_agent_id BETWEEN 1 AND 9007199254740991
    ),
    task_id INTEGER CHECK (task_id BETWEEN 1 AND 9007199254740991),
    approval_id INTEGER CHECK (approval_id BETWEEN 1 AND 9007199254740991),
    authorization_kind TEXT NOT NULL CHECK (
        authorization_kind IN (
            'policyAllowed',
            'approvalRequired',
            'approvalConsumed',
            'policyRejected'
        )
    ),
    intent_fingerprint_sha256 TEXT NOT NULL,
    policy_fingerprint_sha256 TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN (
            'approvalRequired',
            'taskCreated',
            'applied',
            'dispatched',
            'rejected',
            'failed',
            'uncertain'
        )
    ),
    detail_code TEXT,
    detail_message TEXT,
    content_sha256 TEXT,
    content_length INTEGER CHECK (content_length BETWEEN 1 AND 4096),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0),
    CHECK (
        (content_sha256 IS NULL AND content_length IS NULL)
        OR (content_sha256 IS NOT NULL AND content_length IS NOT NULL)
    )
);

CREATE INDEX system_action_audits_recent_v9
ON system_action_audits (id DESC);

CREATE INDEX system_action_audits_retention_v9
ON system_action_audits (updated_at_unix_ms, id);

ALTER TABLE data_lifecycle_meta
ADD COLUMN total_pruned_system_action_audits INTEGER NOT NULL DEFAULT 0
    CHECK (total_pruned_system_action_audits >= 0);

ALTER TABLE data_lifecycle_runs
ADD COLUMN pruned_system_action_audits INTEGER NOT NULL DEFAULT 0
    CHECK (pruned_system_action_audits >= 0);

UPDATE approval_requests
SET authoritative = 0,
    status = 'Expired',
    resolved_at = COALESCE(resolved_at, expires_at),
    resolved_at_unix_ms = COALESCE(resolved_at_unix_ms, expires_at_unix_ms),
    intent_json = '{"kind":"typeDesktopText","agentId":0,"textSha256":"legacy-redacted","textLength":0}',
    intent_fingerprint = 'intent-v3|legacy-redacted'
WHERE intent_kind = 'typeDesktopText';
