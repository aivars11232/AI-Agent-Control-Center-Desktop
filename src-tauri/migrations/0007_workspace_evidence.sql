ALTER TABLE run_attempts
ADD COLUMN workspace_evidence_json TEXT
    CHECK (workspace_evidence_json IS NULL OR json_valid(workspace_evidence_json));

ALTER TABLE agent_tasks
ADD COLUMN workspace_evidence_json TEXT
    CHECK (workspace_evidence_json IS NULL OR json_valid(workspace_evidence_json));
