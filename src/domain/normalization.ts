import type {
  AgentPerformance,
  AppPreferences,
  ApprovalRequest,
  SafetyScope,
  WorkspaceDefinition,
} from "../applicationState";

const defaultAgentPerformance: AgentPerformance = {
  strength: 5,
  focus: "balanced",
  cpuLimit: 70,
  gpuLimit: 50,
  queueThreshold: 10,
  overflowAction: "queue",
  redirectAgentId: null,
};

export const defaultAppPreferences: AppPreferences = {
  theme: "dark",
  accentColor: "violet",
  density: "comfortable",
  reducedMotion: false,
  defaultModel: "gpt-5.6-luna",
  activeAiProvider: "codex",
  defaultAgentStatus: "Waiting",
  defaultTaskCategory: "General",
  defaultTaskPriority: "Normal",
  defaultPerformance: defaultAgentPerformance,
  workspacePath: "",
  workspaces: [],
  activeWorkspaceId: null,
  agentTimeoutMinutes: 30,
  safetyMode: "balanced",
  approvalExpiryMinutes: 30,
  defaultRoutingMode: "selected",
  reviewMode: "manual",
  backgroundVoiceEnabled: true,
  voiceControlMasterEnabled: true,
  voiceWakePhrase: "lucy",
  voiceDeactivatePhrase: "lucy deactivate",
  voiceOpenPhrases: "open, launch, start",
  voiceClosePhrases: "close, quit, exit",
  voiceCommandReplacements:
    "fire fox = firefox\nvisual studio = visual studio code",
  voiceState: "VOICE_PASSIVE",
};

function clampNumber(
  value: unknown,
  minimum: number,
  maximum: number,
  fallback: number,
) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(maximum, Math.max(minimum, Math.round(value)))
    : fallback;
}

export function normalizeApprovalRequest(
  request: Partial<ApprovalRequest>,
): ApprovalRequest | null {
  if (
    typeof request.id !== "number" ||
    typeof request.agentId !== "number" ||
    typeof request.title !== "string" ||
    typeof request.reason !== "string"
  ) {
    return null;
  }

  const createdAt =
    typeof request.createdAt === "string"
      ? request.createdAt
      : new Date().toISOString();
  const legacyExpiry = new Date(
    new Date(createdAt).getTime() + 30 * 60 * 1000,
  ).toISOString();

  return {
    id: request.id,
    agentId: request.agentId,
    taskId: typeof request.taskId === "number" ? request.taskId : null,
    title: request.title,
    reason: request.reason,
    status:
      request.status === "Approved" ||
      request.status === "Denied" ||
      request.status === "Expired"
        ? request.status
        : "Pending",
    createdAt,
    resolvedAt:
      typeof request.resolvedAt === "string" ? request.resolvedAt : null,
    riskLevel:
      request.riskLevel === "Medium" ||
      request.riskLevel === "High" ||
      request.riskLevel === "Critical"
        ? request.riskLevel
        : "Low",
    scopes: Array.isArray(request.scopes)
      ? request.scopes.filter(
          (scope): scope is SafetyScope =>
            scope === "files" ||
            scope === "internet" ||
            scope === "clipboard" ||
            scope === "terminal" ||
            scope === "system",
        )
      : [],
    workspaceId:
      typeof request.workspaceId === "string" ? request.workspaceId : null,
    taskSnapshot:
      typeof request.taskSnapshot === "string" ? request.taskSnapshot : "",
    expiresAt:
      typeof request.expiresAt === "string" ? request.expiresAt : legacyExpiry,
    consumedAt:
      typeof request.consumedAt === "string" ? request.consumedAt : null,
  };
}

export function normalizePerformance(
  performance?: Partial<AgentPerformance>,
): AgentPerformance {
  return {
    strength: clampNumber(performance?.strength, 1, 10, 5),
    focus:
      performance?.focus === "speed" ||
      performance?.focus === "balanced" ||
      performance?.focus === "strength"
        ? performance.focus
        : "balanced",
    cpuLimit: clampNumber(performance?.cpuLimit, 10, 100, 70),
    gpuLimit: clampNumber(performance?.gpuLimit, 0, 100, 50),
    queueThreshold: clampNumber(performance?.queueThreshold, 1, 100, 10),
    overflowAction:
      performance?.overflowAction === "redirect" ? "redirect" : "queue",
    redirectAgentId:
      typeof performance?.redirectAgentId === "number"
        ? performance.redirectAgentId
        : null,
  };
}

export function normalizePreferences(
  preferences?: Partial<AppPreferences>,
): AppPreferences {
  const legacyWorkspacePath =
    typeof preferences?.workspacePath === "string"
      ? preferences.workspacePath.trim()
      : "";
  const savedWorkspaces = Array.isArray(preferences?.workspaces)
    ? preferences.workspaces
        .filter(
          (workspace): workspace is WorkspaceDefinition =>
            typeof workspace?.id === "string" &&
            typeof workspace?.name === "string" &&
            typeof workspace?.path === "string" &&
            workspace.path.trim().length > 0,
        )
        .map((workspace) => ({
          id: workspace.id,
          name: workspace.name.trim() || "Workspace",
          path: workspace.path.trim(),
        }))
    : [];
  const workspaces =
    savedWorkspaces.length > 0
      ? savedWorkspaces
      : legacyWorkspacePath
        ? [
            {
              id: "migrated-workspace",
              name:
                legacyWorkspacePath.split("/").filter(Boolean).slice(-1)[0] ??
                "Workspace",
              path: legacyWorkspacePath,
            },
          ]
        : [];
  const activeWorkspaceId = workspaces.some(
    (workspace) => workspace.id === preferences?.activeWorkspaceId,
  )
    ? (preferences?.activeWorkspaceId ?? null)
    : (workspaces[0]?.id ?? null);
  const activeWorkspace = workspaces.find(
    (workspace) => workspace.id === activeWorkspaceId,
  );

  return {
    theme:
      preferences?.theme === "light" || preferences?.theme === "system"
        ? preferences.theme
        : "dark",
    accentColor:
      preferences?.accentColor === "blue" ||
      preferences?.accentColor === "cyan" ||
      preferences?.accentColor === "green"
        ? preferences.accentColor
        : "violet",
    density:
      preferences?.density === "compact" ? "compact" : "comfortable",
    reducedMotion: preferences?.reducedMotion === true,
    defaultModel:
      typeof preferences?.defaultModel === "string"
        ? preferences.defaultModel
        : "gpt-5.6-luna",
    activeAiProvider:
      preferences?.activeAiProvider === "ollama" ? "ollama" : "codex",
    defaultAgentStatus:
      preferences?.defaultAgentStatus === "Working" ||
      preferences?.defaultAgentStatus === "Paused"
        ? preferences.defaultAgentStatus
        : "Waiting",
    defaultTaskCategory:
      preferences?.defaultTaskCategory === "Development" ||
      preferences?.defaultTaskCategory === "Research" ||
      preferences?.defaultTaskCategory === "Browsing" ||
      preferences?.defaultTaskCategory === "Finance" ||
      preferences?.defaultTaskCategory === "Business" ||
      preferences?.defaultTaskCategory === "Communication" ||
      preferences?.defaultTaskCategory === "System Control"
        ? preferences.defaultTaskCategory
        : "General",
    defaultTaskPriority:
      preferences?.defaultTaskPriority === "Low" ||
      preferences?.defaultTaskPriority === "High" ||
      preferences?.defaultTaskPriority === "Critical"
        ? preferences.defaultTaskPriority
        : "Normal",
    defaultPerformance: normalizePerformance(preferences?.defaultPerformance),
    workspacePath: activeWorkspace?.path ?? legacyWorkspacePath,
    workspaces,
    activeWorkspaceId,
    agentTimeoutMinutes: clampNumber(
      preferences?.agentTimeoutMinutes,
      1,
      120,
      30,
    ),
    safetyMode:
      preferences?.safetyMode === "strict" ||
      preferences?.safetyMode === "locked"
        ? preferences.safetyMode
        : "balanced",
    approvalExpiryMinutes: clampNumber(
      preferences?.approvalExpiryMinutes,
      5,
      120,
      30,
    ),
    defaultRoutingMode:
      preferences?.defaultRoutingMode === "automatic"
        ? "automatic"
        : "selected",
    reviewMode:
      preferences?.reviewMode === "off" ||
      preferences?.reviewMode === "automatic"
        ? preferences.reviewMode
        : "manual",
    backgroundVoiceEnabled: preferences?.backgroundVoiceEnabled !== false,
    voiceControlMasterEnabled:
      preferences?.voiceControlMasterEnabled !== false,
    voiceWakePhrase:
      typeof preferences?.voiceWakePhrase === "string" &&
      preferences.voiceWakePhrase.trim() &&
      preferences.voiceWakePhrase.trim().toLowerCase() !== "lucy activate, on"
        ? preferences.voiceWakePhrase.trim().toLowerCase()
        : "lucy",
    voiceDeactivatePhrase:
      typeof preferences?.voiceDeactivatePhrase === "string" &&
      preferences.voiceDeactivatePhrase.trim()
        ? preferences.voiceDeactivatePhrase.trim().toLowerCase()
        : "lucy deactivate",
    voiceOpenPhrases:
      typeof preferences?.voiceOpenPhrases === "string" &&
      preferences.voiceOpenPhrases.trim()
        ? preferences.voiceOpenPhrases.trim().toLowerCase()
        : "open, launch, start",
    voiceClosePhrases:
      typeof preferences?.voiceClosePhrases === "string" &&
      preferences.voiceClosePhrases.trim()
        ? preferences.voiceClosePhrases.trim().toLowerCase()
        : "close, quit, exit",
    voiceCommandReplacements:
      typeof preferences?.voiceCommandReplacements === "string"
        ? preferences.voiceCommandReplacements.toLowerCase()
        : "fire fox = firefox\nvisual studio = visual studio code",
    voiceState:
      preferences?.voiceState === "VOICE_ACTIVE" ||
      preferences?.voiceState === "VOICE_OFF"
        ? preferences.voiceState
        : "VOICE_PASSIVE",
  };
}
