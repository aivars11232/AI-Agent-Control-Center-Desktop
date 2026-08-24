ALTER TABLE agents
ADD COLUMN template_key TEXT;

ALTER TABLE agents
ADD COLUMN registry_state TEXT NOT NULL DEFAULT 'active'
    CHECK (registry_state IN ('active', 'unassigned', 'deleted'));

ALTER TABLE agents
ADD COLUMN registry_issue TEXT
    CHECK (
        registry_issue IS NULL OR registry_issue IN (
            'self-parent',
            'missing-manager',
            'manager-not-active',
            'manager-authority',
            'cycle'
        )
    );

ALTER TABLE agents
ADD COLUMN deleted_at_unix_ms INTEGER
    CHECK (deleted_at_unix_ms IS NULL OR deleted_at_unix_ms >= 0);

UPDATE agents
SET template_key = CASE
    WHEN id = 1 AND role = 'Supervisor' AND category = 'Management' THEN 'supervisor'
    WHEN id = 2 AND role = 'Specialist' AND category = 'Development' THEN 'coding'
    WHEN id = 3 AND role = 'Senior Agent' AND category = 'Development' THEN 'debugging'
    WHEN id = 4 AND role = 'Specialist' AND category = 'Browsing' THEN 'browser'
    WHEN id = 5 AND role = 'Specialist' AND category = 'Finance' THEN 'financial'
    WHEN id = 6 AND role = 'Team Leader' AND category = 'Management' THEN 'development-team-leader'
    WHEN id = 7 AND role = 'Specialist' AND category = 'System Control' THEN 'pc-control'
    WHEN id = 8 AND role = 'Specialist' AND category = 'Business' THEN 'event-reminder'
    WHEN id = 9 AND role = 'Senior Agent' AND category = 'Browsing' THEN 'research-web-senior'
    WHEN id = 10 AND role = 'Senior Agent' AND category = 'Finance' THEN 'finance-senior'
    WHEN id = 11 AND role = 'Senior Agent' AND category = 'Business' THEN 'operations-senior'
    ELSE NULL
END;

UPDATE agents
SET authority_level = CASE role
    WHEN 'Supervisor' THEN 4
    WHEN 'Team Leader' THEN 3
    WHEN 'Senior Agent' THEN 2
    ELSE 1
END;

UPDATE agents
SET reports_to = NULL
WHERE role = 'Supervisor';

UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'self-parent',
    status = 'Paused', reports_to = NULL
WHERE role <> 'Supervisor' AND reports_to = id;

UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'missing-manager',
    status = 'Paused', reports_to = NULL
WHERE role <> 'Supervisor' AND reports_to IS NULL AND registry_state = 'active';

UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'missing-manager',
    status = 'Paused', reports_to = NULL
WHERE role <> 'Supervisor'
  AND registry_state = 'active'
  AND NOT EXISTS (SELECT 1 FROM agents AS manager WHERE manager.id = agents.reports_to);

UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'manager-authority',
    status = 'Paused', reports_to = NULL
WHERE role <> 'Supervisor'
  AND registry_state = 'active'
  AND EXISTS (
      SELECT 1 FROM agents AS manager
      WHERE manager.id = agents.reports_to
        AND manager.authority_level <= agents.authority_level
  );

-- Propagate quarantine through the maximum supported authority depth.
UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'manager-not-active',
    status = 'Paused', reports_to = NULL
WHERE registry_state = 'active' AND role <> 'Supervisor'
  AND EXISTS (SELECT 1 FROM agents AS manager WHERE manager.id = agents.reports_to AND manager.registry_state <> 'active');
UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'manager-not-active',
    status = 'Paused', reports_to = NULL
WHERE registry_state = 'active' AND role <> 'Supervisor'
  AND EXISTS (SELECT 1 FROM agents AS manager WHERE manager.id = agents.reports_to AND manager.registry_state <> 'active');
UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'manager-not-active',
    status = 'Paused', reports_to = NULL
WHERE registry_state = 'active' AND role <> 'Supervisor'
  AND EXISTS (SELECT 1 FROM agents AS manager WHERE manager.id = agents.reports_to AND manager.registry_state <> 'active');
UPDATE agents
SET registry_state = 'unassigned', registry_issue = 'manager-not-active',
    status = 'Paused', reports_to = NULL
WHERE registry_state = 'active' AND role <> 'Supervisor'
  AND EXISTS (SELECT 1 FROM agents AS manager WHERE manager.id = agents.reports_to AND manager.registry_state <> 'active');

CREATE UNIQUE INDEX agents_template_key_unique
ON agents (template_key)
WHERE template_key IS NOT NULL;

CREATE TABLE agent_registry_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    next_agent_id INTEGER NOT NULL
        CHECK (next_agent_id BETWEEN 1 AND 9007199254740991)
);

INSERT INTO agent_registry_meta (singleton, next_agent_id)
SELECT 1,
       CASE
           WHEN COALESCE(MAX(id), 0) < 9007199254740991
               THEN COALESCE(MAX(id), 0) + 1
           ELSE 9007199254740991
       END
FROM agents;
