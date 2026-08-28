ALTER TABLE agent_tasks
ADD COLUMN specialist_request_json TEXT
    CHECK (
        specialist_request_json IS NULL
        OR (
            json_valid(specialist_request_json)
            AND length(CAST(specialist_request_json AS BLOB)) BETWEEN 1 AND 65536
        )
    );

ALTER TABLE run_attempts
ADD COLUMN specialist_contract_json TEXT
    CHECK (
        specialist_contract_json IS NULL
        OR (
            json_valid(specialist_contract_json)
            AND length(CAST(specialist_contract_json AS BLOB)) BETWEEN 1 AND 65536
        )
    );

ALTER TABLE run_attempts
ADD COLUMN specialist_result_json TEXT
    CHECK (
        specialist_result_json IS NULL
        OR (
            json_valid(specialist_result_json)
            AND length(CAST(specialist_result_json AS BLOB)) BETWEEN 1 AND 262144
        )
    );

-- Narrow untouched v1 seed profiles to their enforced v1 specialist ceilings.
-- User-customized descriptions or capability/approval rows are preserved; the
-- backend specialist contract remains authoritative for every admitted run.
UPDATE agents
SET description = 'Implements approved, typed changes inside one selected workspace',
    capability_clipboard = 'none',
    approval_files = 'ask',
    approval_clipboard = 'deny'
WHERE template_key = 'coding'
  AND description = 'Builds and edits project files'
  AND capability_files = 'write'
  AND capability_internet = 'read'
  AND capability_clipboard = 'write'
  AND capability_terminal = 'safe'
  AND capability_system = 'none'
  AND approval_files = 'allow'
  AND approval_internet = 'ask'
  AND approval_clipboard = 'allow'
  AND approval_terminal = 'ask'
  AND approval_system = 'deny';

UPDATE agents
SET description = 'Diagnoses errors and runs requested checks without changing the workspace',
    capability_clipboard = 'none',
    approval_clipboard = 'deny'
WHERE template_key = 'debugging'
  AND description = 'Finds errors and verifies fixes'
  AND capability_files = 'read'
  AND capability_internet = 'none'
  AND capability_clipboard = 'read'
  AND capability_terminal = 'safe'
  AND capability_system = 'none'
  AND approval_files = 'ask'
  AND approval_internet = 'deny'
  AND approval_clipboard = 'ask'
  AND approval_terminal = 'ask'
  AND approval_system = 'deny';

UPDATE agents
SET description = 'Performs hosted read-only research with bounded HTTPS sources',
    capability_clipboard = 'none',
    approval_clipboard = 'deny'
WHERE template_key = 'browser'
  AND description = 'Uses websites when permission is granted'
  AND capability_files = 'none'
  AND capability_internet = 'read'
  AND capability_clipboard = 'read'
  AND capability_terminal = 'none'
  AND capability_system = 'none'
  AND approval_files = 'deny'
  AND approval_internet = 'ask'
  AND approval_clipboard = 'ask'
  AND approval_terminal = 'deny'
  AND approval_system = 'deny';

UPDATE agents
SET description = 'Produces local reports with bounded fixed-point calculations and no transactions',
    capability_files = 'none',
    approval_files = 'deny'
WHERE template_key = 'financial'
  AND description = 'Tracks financial tasks and reports'
  AND capability_files = 'read'
  AND capability_internet = 'none'
  AND capability_clipboard = 'none'
  AND capability_terminal = 'none'
  AND capability_system = 'none'
  AND approval_files = 'ask'
  AND approval_internet = 'deny'
  AND approval_clipboard = 'deny'
  AND approval_terminal = 'deny'
  AND approval_system = 'deny';
