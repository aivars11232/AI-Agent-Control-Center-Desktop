CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE,
    applied_at_unix_ms INTEGER NOT NULL CHECK (applied_at_unix_ms >= 0)
);

CREATE TABLE application_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    initialized INTEGER NOT NULL DEFAULT 0 CHECK (initialized IN (0, 1)),
    state_revision INTEGER NOT NULL DEFAULT 0 CHECK (state_revision BETWEEN 0 AND 9007199254740991),
    source_kind TEXT CHECK (source_kind IN ('fresh', 'legacy_local_storage', 'legacy_backup', 'reset')),
    source_version INTEGER,
    migrated_at_unix_ms INTEGER CHECK (migrated_at_unix_ms >= 0),
    legacy_cleanup_ack_at_unix_ms INTEGER CHECK (legacy_cleanup_ack_at_unix_ms >= 0)
);

INSERT INTO application_meta (singleton) VALUES (1);

CREATE TABLE preferences (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    theme TEXT NOT NULL CHECK (theme IN ('dark', 'light', 'system')),
    accent_color TEXT NOT NULL CHECK (accent_color IN ('violet', 'blue', 'cyan', 'green')),
    density TEXT NOT NULL CHECK (density IN ('comfortable', 'compact')),
    reduced_motion INTEGER NOT NULL CHECK (reduced_motion IN (0, 1)),
    default_model TEXT NOT NULL,
    active_ai_provider TEXT NOT NULL CHECK (active_ai_provider IN ('codex', 'ollama')),
    default_agent_status TEXT NOT NULL CHECK (default_agent_status IN ('Working', 'Waiting', 'Paused')),
    default_task_category TEXT NOT NULL CHECK (default_task_category IN ('Development', 'Research', 'Browsing', 'Finance', 'Business', 'Communication', 'System Control', 'General')),
    default_task_priority TEXT NOT NULL CHECK (default_task_priority IN ('Low', 'Normal', 'High', 'Critical')),
    default_strength INTEGER NOT NULL CHECK (default_strength BETWEEN 1 AND 10),
    default_focus TEXT NOT NULL CHECK (default_focus IN ('speed', 'balanced', 'strength')),
    default_cpu_limit INTEGER NOT NULL CHECK (default_cpu_limit BETWEEN 10 AND 100),
    default_gpu_limit INTEGER NOT NULL CHECK (default_gpu_limit BETWEEN 0 AND 100),
    default_overflow_action TEXT NOT NULL CHECK (default_overflow_action IN ('queue', 'redirect')),
    default_redirect_agent_id INTEGER CHECK (default_redirect_agent_id BETWEEN 1 AND 9007199254740991),
    workspace_path TEXT NOT NULL,
    active_workspace_id TEXT,
    agent_timeout_minutes INTEGER NOT NULL CHECK (agent_timeout_minutes BETWEEN 1 AND 120),
    safety_mode TEXT NOT NULL CHECK (safety_mode IN ('balanced', 'strict', 'locked')),
    approval_expiry_minutes INTEGER NOT NULL CHECK (approval_expiry_minutes BETWEEN 5 AND 120),
    default_routing_mode TEXT NOT NULL CHECK (default_routing_mode IN ('selected', 'automatic')),
    review_mode TEXT NOT NULL CHECK (review_mode IN ('off', 'manual', 'automatic')),
    background_voice_enabled INTEGER NOT NULL CHECK (background_voice_enabled IN (0, 1)),
    voice_control_master_enabled INTEGER NOT NULL CHECK (voice_control_master_enabled IN (0, 1)),
    voice_wake_phrase TEXT NOT NULL,
    voice_deactivate_phrase TEXT NOT NULL,
    voice_open_phrases TEXT NOT NULL,
    voice_close_phrases TEXT NOT NULL,
    voice_command_replacements TEXT NOT NULL,
    voice_state TEXT NOT NULL CHECK (voice_state IN ('VOICE_OFF', 'VOICE_PASSIVE', 'VOICE_ACTIVE'))
);

CREATE TABLE retention_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    task_retention TEXT NOT NULL CHECK (task_retention IN ('7', '30', '90', 'never')),
    activity_retention TEXT NOT NULL CHECK (activity_retention IN ('7', '30', '90', 'never'))
);

CREATE TABLE workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    name TEXT NOT NULL,
    path TEXT NOT NULL
);

CREATE TABLE agents (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('Working', 'Waiting', 'Paused')),
    role TEXT NOT NULL CHECK (role IN ('Supervisor', 'Team Leader', 'Senior Agent', 'Specialist')),
    category TEXT NOT NULL CHECK (category IN ('Management', 'Development', 'Research', 'Browsing', 'Finance', 'Business', 'Communication', 'System Control', 'General')),
    reports_to INTEGER CHECK (reports_to BETWEEN 1 AND 9007199254740991),
    authority_level INTEGER NOT NULL CHECK (authority_level BETWEEN 1 AND 4),
    model TEXT NOT NULL,
    memory TEXT NOT NULL,
    strength INTEGER NOT NULL CHECK (strength BETWEEN 1 AND 10),
    focus TEXT NOT NULL CHECK (focus IN ('speed', 'balanced', 'strength')),
    cpu_limit INTEGER NOT NULL CHECK (cpu_limit BETWEEN 10 AND 100),
    gpu_limit INTEGER NOT NULL CHECK (gpu_limit BETWEEN 0 AND 100),
    overflow_action TEXT NOT NULL CHECK (overflow_action IN ('queue', 'redirect')),
    redirect_agent_id INTEGER CHECK (redirect_agent_id BETWEEN 1 AND 9007199254740991),
    capability_files TEXT NOT NULL CHECK (capability_files IN ('none', 'read', 'write', 'full')),
    capability_internet TEXT NOT NULL CHECK (capability_internet IN ('none', 'read', 'write', 'full')),
    capability_clipboard TEXT NOT NULL CHECK (capability_clipboard IN ('none', 'read', 'write', 'full')),
    capability_terminal TEXT NOT NULL CHECK (capability_terminal IN ('none', 'safe', 'user', 'admin')),
    capability_system TEXT NOT NULL CHECK (capability_system IN ('none', 'notifications', 'power', 'full')),
    approval_files TEXT NOT NULL CHECK (approval_files IN ('allow', 'ask', 'deny')),
    approval_internet TEXT NOT NULL CHECK (approval_internet IN ('allow', 'ask', 'deny')),
    approval_clipboard TEXT NOT NULL CHECK (approval_clipboard IN ('allow', 'ask', 'deny')),
    approval_terminal TEXT NOT NULL CHECK (approval_terminal IN ('allow', 'ask', 'deny')),
    approval_system TEXT NOT NULL CHECK (approval_system IN ('allow', 'ask', 'deny'))
);

CREATE TABLE agent_tasks (
    owner_agent_id INTEGER NOT NULL,
    id INTEGER NOT NULL CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL CHECK (position >= 0),
    title TEXT NOT NULL,
    category TEXT NOT NULL CHECK (category IN ('Development', 'Research', 'Browsing', 'Finance', 'Business', 'Communication', 'System Control', 'General')),
    priority TEXT NOT NULL CHECK (priority IN ('Low', 'Normal', 'High', 'Critical')),
    assigned_agent_id INTEGER NOT NULL CHECK (assigned_agent_id BETWEEN 1 AND 9007199254740991),
    status TEXT NOT NULL CHECK (status IN ('Pending', 'Running', 'Blocked', 'Under Review', 'Completed', 'Failed')),
    phase TEXT NOT NULL CHECK (phase IN ('Assigned', 'Specialist Work', 'Senior Review', 'Team Leader Review', 'Supervisor Approval', 'Finished', 'Failed')),
    created_at TEXT NOT NULL,
    completed_at TEXT,
    result TEXT,
    response_id TEXT,
    runtime_model TEXT,
    total_tokens INTEGER CHECK (total_tokens >= 0),
    workspace_id TEXT,
    diff TEXT,
    duration_seconds REAL CHECK (duration_seconds >= 0),
    routing_mode TEXT NOT NULL CHECK (routing_mode IN ('selected', 'automatic')),
    routed_from_agent_id INTEGER CHECK (routed_from_agent_id BETWEEN 1 AND 9007199254740991),
    routing_reason TEXT,
    review_agent_id INTEGER CHECK (review_agent_id BETWEEN 1 AND 9007199254740991),
    review_status TEXT NOT NULL CHECK (review_status IN ('Not Requested', 'Pending', 'Running', 'Approved', 'Changes Requested', 'Failed')),
    review_result TEXT,
    review_model TEXT,
    review_duration_seconds REAL CHECK (review_duration_seconds >= 0),
    reviewed_at TEXT,
    PRIMARY KEY (owner_agent_id, id),
    UNIQUE (owner_agent_id, position),
    FOREIGN KEY (owner_agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE TABLE task_changed_files (
    owner_agent_id INTEGER NOT NULL,
    task_id INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    path TEXT NOT NULL,
    PRIMARY KEY (owner_agent_id, task_id, position),
    FOREIGN KEY (owner_agent_id, task_id) REFERENCES agent_tasks(owner_agent_id, id) ON DELETE CASCADE
);

CREATE TABLE agent_activity (
    owner_agent_id INTEGER NOT NULL,
    id INTEGER NOT NULL CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL CHECK (position >= 0),
    message TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (owner_agent_id, id),
    UNIQUE (owner_agent_id, position),
    FOREIGN KEY (owner_agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE TABLE models (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    name TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('OpenAI', 'Anthropic', 'Google', 'Ollama', 'Custom'))
);

CREATE TABLE approval_requests (
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
    origin TEXT NOT NULL CHECK (origin IN ('fresh', 'legacy_local_storage', 'legacy_backup', 'renderer_prototype')),
    authoritative INTEGER NOT NULL DEFAULT 0 CHECK (authoritative = 0)
);

CREATE TABLE approval_scopes (
    approval_id INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    scope TEXT NOT NULL CHECK (scope IN ('files', 'internet', 'clipboard', 'terminal', 'system')),
    PRIMARY KEY (approval_id, position),
    UNIQUE (approval_id, scope),
    FOREIGN KEY (approval_id) REFERENCES approval_requests(id) ON DELETE CASCADE
);

CREATE TABLE reminders (
    id INTEGER PRIMARY KEY CHECK (id BETWEEN 1 AND 9007199254740991),
    position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
    title TEXT NOT NULL,
    notes TEXT NOT NULL,
    due_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('Upcoming', 'Completed', 'Dismissed')),
    agent_id INTEGER CHECK (agent_id BETWEEN 1 AND 9007199254740991),
    task_id INTEGER CHECK (task_id BETWEEN 1 AND 9007199254740991),
    created_at TEXT NOT NULL
);
