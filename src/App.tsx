import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ActivityEntry,
  Agent,
  AgentCategory,
  AgentPerformance,
  AgentRole,
  AgentStatus,
  AgentTask,
  AppPreferences,
  ApprovalMode,
  ApprovalRequest,
  ApprovalRequestStatus,
  AuthorityLevel,
  ExecutionFocus,
  HistoryRetentionDays,
  InterfaceDensity,
  ModelDefinition,
  ModelProvider,
  OverflowAction,
  Reminder,
  ReminderStatus,
  ReviewMode,
  ReviewStatus,
  RiskLevel,
  RoutingMode,
  RuntimeProviderId,
  SafetyMode,
  SafetyScope,
  TaskCategory,
  TaskPhase,
  TaskPriority,
  TaskStatus,
  ThemeMode,
  VoiceState,
  WorkspaceDefinition,
  ApplicationState,
  StateEnvelope,
  AccentColor,
} from "./applicationState";
export type {
  Agent,
  AgentPerformance,
  AgentTask,
  AppPreferences,
  ApprovalRequest,
  ModelDefinition,
  SafetyMode,
  TaskCategory,
  TaskPriority,
  VoiceState,
} from "./applicationState";
import {
  ApplicationStateWriter,
  LEGACY_STORAGE_KEYS,
  bootstrapDesktopApplicationState,
  persistenceErrorMessage,
  type InvokeFunction,
} from "./persistence";
import { interpretVoiceCommand } from "./voiceCommand";
import {
  applyRunCoordinatorEvent,
  applyRunCoordinatorSnapshot,
  createRunCoordinatorUiState,
  hasVisibleTruncation,
  markRunStopRequested,
  type RunCoordinatorEvent,
  type RunCoordinatorSnapshot,
  type RunCoordinatorUiState,
} from "./runCoordinator";
import {
  executableModels,
  providerRuntimeStatus,
  resolveModelAvailability,
  unknownProviderRegistrySnapshot,
  type ProviderRegistrySnapshot,
} from "./providerRegistry";
import logoUrl from "../AI-Agents.png";
import "./App.css";

type AgentRunResult = {
  providerId: RuntimeProviderId | null;
  output: string;
  responseId: string | null;
  model: string;
  usage: {
    inputTokens: number | null;
    outputTokens: number | null;
    totalTokens: number | null;
  };
  changedFiles: string[];
  diff: string | null;
  durationSeconds: number;
};

type VoiceRuntimeStatus = {
  installed: boolean;
  listening: boolean;
  highAccuracyAvailable: boolean;
  message: string;
};

type VoiceTranscriptEvent = {
  kind: "activated" | "deactivated" | "off_requested" | "listening" | "ready" | "error" | "command" | "heard";
  transcript: string;
};

type VoiceUiState = "VOICE OFF" | "PASSIVE" | "LISTENING" | "PROCESSING" | "EXECUTING" | "SUCCESS" | "ERROR";

type DesktopControlStatus = {
  enabled: boolean;
  message: string;
};

type BackendActionIntent =
  | {
      kind: "runTask";
      agentId: number;
      taskOwnerAgentId: number;
      taskId: number;
      runMode: "execute" | "review";
    }
  | {
      kind: "openWorkspaceItem";
      agentId: number;
      workspaceId: string;
      itemPath: string;
    }
  | { kind: "launchAllowedApplication"; agentId: number; application: string }
  | { kind: "launchDesktopApplication"; agentId: number; application: string }
  | { kind: "openStandardFolder"; agentId: number; folder: string }
  | { kind: "closeAllowedApplication"; agentId: number; application: string }
  | { kind: "closeActiveApplication"; agentId: number }
  | { kind: "desktopKeyboard"; agentId: number; action: string }
  | { kind: "desktopWindow"; agentId: number; application: string; action: string }
  | { kind: "typeDesktopText"; agentId: number; text: string }
  | { kind: "enableDesktopControl"; agentId: number }
  | { kind: "desktopPointer"; agentId: number; action: string }
  | { kind: "installVoiceRuntime"; agentId: number }
  | { kind: "installHighAccuracyVoiceRuntime"; agentId: number }
  | { kind: "startVoiceListener"; agentId: number };

type AuthorizationOutcome = {
  decision: "allowed" | "approvalRequired";
  approval: ApprovalRequest | null;
};

type AuthorizationReadiness = {
  ready: boolean;
  approval: ApprovalRequest | null;
};

function upsertApprovalRequest(
  requests: ApprovalRequest[],
  approval: ApprovalRequest,
): ApprovalRequest[] {
  const existingIndex = requests.findIndex((request) => request.id === approval.id);
  if (existingIndex === -1) {
    return [approval, ...requests];
  }
  return requests.map((request) =>
    request.id === approval.id ? approval : request,
  );
}

async function prepareBackendAuthorization(
  intent: BackendActionIntent,
  setApprovalRequests: React.Dispatch<React.SetStateAction<ApprovalRequest[]>>,
): Promise<AuthorizationReadiness> {
  const outcome = await invoke<AuthorizationOutcome>("request_authorization", {
    intent,
  });
  if (outcome.approval) {
    setApprovalRequests((requests) =>
      upsertApprovalRequest(requests, outcome.approval as ApprovalRequest),
    );
  }
  return {
    ready:
      outcome.decision === "allowed" || outcome.approval?.status === "Approved",
    approval: outcome.approval,
  };
}

function markApprovalConsumed(
  setApprovalRequests: React.Dispatch<React.SetStateAction<ApprovalRequest[]>>,
  approval: ApprovalRequest | null,
) {
  if (!approval || approval.status !== "Approved") {
    return;
  }
  setApprovalRequests((requests) =>
    requests.map((request) =>
      request.id === approval.id
        ? { ...request, consumedAt: new Date().toISOString() }
        : request,
    ),
  );
}

function isDesktopRuntime() {
  return "__TAURI_INTERNALS__" in window;
}

const invokeApplicationState: InvokeFunction = <T,>(
  command: string,
  args?: Record<string, unknown>,
) => invoke<T>(command, args);

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

type SpeechRecognitionLike = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start: () => void;
  onresult: ((event: {
    results: ArrayLike<ArrayLike<{ transcript: string }>>;
  }) => void) | null;
  onerror: ((event: { error: string }) => void) | null;
  onend: (() => void) | null;
};

type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

type Page =
  | "Dashboard"
  | "Agents"
  | "Voice Control"
  | "Tasks"
  | "Approvals"
  | "Reminders"
  | "Activity"
  | "Models"
  | "Settings";

const pages: Page[] = [
  "Dashboard",
  "Agents",
  "Voice Control",
  "Tasks",
  "Approvals",
  "Reminders",
  "Activity",
  "Models",
  "Settings",
];

function DashboardPage({
  agents,
  approvalRequests,
  onOpenAgents,
  onOpenTasks,
  onOpenApprovals,
}: {
  agents: Agent[];
  approvalRequests: ApprovalRequest[];
  onOpenAgents: () => void;
  onOpenTasks: () => void;
  onOpenApprovals: () => void;
}) {
  const [activeAgentGroup, setActiveAgentGroup] = useState<
    "Development" | "Finance and Events" | "Web and PC Control"
  >("Development");
  const activeAgentCount = agents.filter(
    (agent) => agent.status === "Working",
  ).length;

  const runningTaskCount = agents.reduce(
    (total, agent) =>
      total +
      agent.tasks.filter((task) => task.status === "Running").length,
    0,
  );

  const waitingTaskCount = agents.reduce(
    (total, agent) =>
      total +
      agent.tasks.filter((task) => task.status === "Pending").length,
    0,
  );

  const totalActivityCount = agents.reduce(
    (total, agent) => total + agent.activity.length,
    0,
  );

  const supervisorQueue = agents
    .flatMap((agent) =>
      agent.tasks
        .filter(
          (task) =>
            task.status === "Blocked" ||
            task.status === "Under Review" ||
            task.status === "Pending",
        )
        .map((task) => ({ task, agent })),
    )
    .sort((left, right) => {
      const priority = { Critical: 0, High: 1, Normal: 2, Low: 3 };
      return priority[left.task.priority] - priority[right.task.priority];
    })
    .slice(0, 8);
  const pendingApprovalCount = approvalRequests.filter(
    (request) => request.status === "Pending",
  ).length;

  const groupAgentIds: Record<typeof activeAgentGroup, number[]> = {
    Development: [1, 2, 3, 6],
    "Finance and Events": [1, 5, 6, 8, 10, 11],
    "Web and PC Control": [1, 4, 6, 7, 9, 11],
  };
  const groupedAgents = agents.filter((agent) =>
    groupAgentIds[activeAgentGroup].includes(agent.id),
  );

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">OVERVIEW</span>
          <h1>Dashboard</h1>
          <p className="page-message">
            Live status across all configured agents.
          </p>
        </div>

        <button className="primary-button" onClick={onOpenAgents}>
          Manage agents
        </button>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Active agents</span>
          <strong>{activeAgentCount}</strong>
          <small>{agents.length} configured</small>
        </article>

        <article className="summary-card">
          <span>Tasks running</span>
          <strong>{runningTaskCount}</strong>
          <small>{waitingTaskCount} pending</small>
        </article>

        <article className="summary-card">
          <span>Total tasks</span>
          <strong>
            {agents.reduce(
              (total, agent) => total + agent.tasks.length,
              0,
            )}
          </strong>
          <small>Across all agents</small>
        </article>

        <article className="summary-card">
          <span>Activity events</span>
          <strong>{totalActivityCount}</strong>
          <small>Recorded locally</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">SUPERVISOR QUEUE</span>
            <h2>Needs attention</h2>
            <p className="page-message">
              Prioritized work that needs delegation, review, or human authorization.
            </p>
          </div>

          <div className="topbar-actions">
            <button className="secondary-button" onClick={onOpenTasks}>
              Open task pipeline
            </button>
            {pendingApprovalCount > 0 && (
              <button className="danger-button" onClick={onOpenApprovals}>
                Review {pendingApprovalCount} approval{pendingApprovalCount === 1 ? "" : "s"}
              </button>
            )}
          </div>
        </div>

        {supervisorQueue.length === 0 ? (
          <p className="page-message">
            No pending, blocked, or under-review work needs attention.
          </p>
        ) : (
          <div className="agent-list">
            {supervisorQueue.map(({ task, agent }) => (
              <article className="agent-card" key={`${agent.id}-${task.id}`}>
                <div>
                  <h3>{task.title}</h3>
                  <p>
                    {agent.name} · {task.category} · {task.priority} priority
                  </p>
                  <small>
                    Phase: {task.phase}
                    {task.reviewStatus !== "Not Requested"
                      ? ` · Review: ${task.reviewStatus}`
                      : ""}
                  </small>
                </div>
                <span
                  className={`agent-status ${
                    task.status === "Blocked"
                      ? "paused"
                      : task.status === "Under Review"
                        ? "working"
                        : "waiting"
                  }`}
                >
                  {task.status}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">YOUR TEAM</span>
            <h2>Agent workspaces</h2>
            <p className="page-message">
              Select a group to scan its hierarchy and active specialists.
            </p>
          </div>

          <button className="primary-button" onClick={onOpenAgents}>
            Manage groups
          </button>
        </div>

        <div className="dashboard-group-tabs" role="tablist" aria-label="Dashboard agent groups">
          {(["Development", "Finance and Events", "Web and PC Control"] as const).map(
            (group) => (
              <button
                key={group}
                role="tab"
                aria-selected={activeAgentGroup === group}
                className={activeAgentGroup === group ? "dashboard-group-tab active" : "dashboard-group-tab"}
                onClick={() => setActiveAgentGroup(group)}
              >
                <strong>{group}</strong>
                <small>{groupAgentIds[group].filter((id) => id !== 1 && id !== 6).length} specialists</small>
              </button>
            ),
          )}
        </div>

        {groupedAgents.length === 0 ? (
          <p className="page-message">
            No agents configured yet.
          </p>
        ) : (
          <div className="dashboard-agent-grid">
            {groupedAgents.map((agent) => {
              const runningTask =
                agent.tasks.find(
                  (task) => task.status === "Running",
                ) ?? null;

              const pendingTaskCount = agent.tasks.filter(
                (task) => task.status === "Pending",
              ).length;

              const superior = agents.find((item) => item.id === agent.reportsTo);

              return (
                <article className={`dashboard-agent-tile role-${agent.role.toLowerCase().replace(/\s+/g, "-")}`} key={agent.id}>
                  <div className="dashboard-agent-tile-art">
                    <span>{agent.name.slice(0, 1)}</span>
                    <span className={`agent-status ${agent.status.toLowerCase()}`}>{agent.status}</span>
                  </div>
                  <div className="dashboard-agent-tile-body">
                    <h3>{agent.name}</h3>
                    <p>{agent.role} · {agent.category}</p>
                    <small>
                      {runningTask
                        ? `Running: ${runningTask.title}`
                        : pendingTaskCount > 0
                          ? `${pendingTaskCount} pending task${pendingTaskCount === 1 ? "" : "s"}`
                          : superior
                            ? `Reports to ${superior.name}`
                            : "No active tasks"}
                    </small>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}


type WorkspaceTab = "Overview" | "Capabilities" | "Memory" | "Tasks" | "Activity";
type CapabilityKey = keyof Agent["capabilities"];
type ApprovalKey = keyof Agent["approvals"];

const ollamaCodingModelName = "qwen2.5-coder:7b";

function ollamaCodingModel(id: number): ModelDefinition {
  return {
    id,
    name: ollamaCodingModelName,
    provider: "Ollama",
  };
}


export type TaskSafetyAssessment = {
  riskLevel: RiskLevel;
  scopes: SafetyScope[];
  approvalScopes: SafetyScope[];
  requiresApproval: boolean;
  destructive: boolean;
  writesWorkspace: boolean;
  blockedReason: string | null;
  reason: string;
};

const safetyScopeLabels: Record<SafetyScope, string> = {
  files: "workspace files",
  internet: "web access",
  clipboard: "clipboard",
  terminal: "terminal commands",
  system: "system control",
};

export function taskSafetyAssessment(
  task: AgentTask,
  agent: Agent,
  safetyMode: SafetyMode,
): TaskSafetyAssessment {
  const text = `${task.title} ${task.category}`.toLowerCase();
  const scopes = new Set<SafetyScope>();
  const mutatesWorkspace =
    /\b(create|write|edit|modify|change|update|refactor|fix|move|rename|replace|generate|add|implement)\b/i.test(
      text,
    );
  const destructive =
    /\b(delete|remove|erase|wipe|truncate|overwrite|reset\s+--hard|clean\s+-[a-z]*f|rm\s|rmdir|unlink)\b/i.test(
      text,
    );
  const terminal =
    /\b(command|terminal|shell|bash|execute|run\s+(?:the\s+)?command|npm|pnpm|yarn|cargo|rustc|git|python|pytest|sleep|build|compile|install)\b/i.test(
      text,
    );
  const internet =
    task.category === "Browsing" ||
    /\b(internet|website|web\s+search|browse|download|upload|curl|wget|url|online)\b/i.test(
      text,
    );
  const clipboard = /\bclipboard|copy\s+to|paste\s+from\b/i.test(text);
  const system =
    task.category === "System Control" ||
    /\b(systemctl|reboot|shutdown|power\s*off|desktop\s+control|computer\s+control|open\s+(?:an\s+)?app|close\s+(?:an\s+)?app)\b/i.test(
      text,
    );
  const privileged =
    /\b(sudo|doas|mkfs|chown|chmod|mount|umount|pacman|apt|dnf|account\s+management|package\s+removal)\b/i.test(
      text,
    );
  const writesWorkspace =
    mutatesWorkspace ||
    destructive ||
    /\b(build|compile|install|format)\b/i.test(text);

  if (writesWorkspace || task.category === "Development") {
    scopes.add("files");
  }
  if (terminal) scopes.add("terminal");
  if (internet) scopes.add("internet");
  if (clipboard) scopes.add("clipboard");
  if (system) scopes.add("system");

  const scopeList = Array.from(scopes);
  const missingCapabilities = scopeList.filter((scope) => {
    if (scope === "files" || scope === "internet" || scope === "clipboard") {
      return agent.capabilities[scope] === "none";
    }
    return agent.capabilities[scope] === "none";
  });
  const deniedScopes = scopeList.filter(
    (scope) => agent.approvals[scope] === "deny",
  );
  const lockedAction =
    safetyMode === "locked" &&
    (mutatesWorkspace || destructive || terminal || internet || clipboard || system);

  let blockedReason: string | null = null;
  if (privileged) {
    blockedReason =
      "Privileged and operating-system package commands are blocked by the desktop safety boundary.";
  } else if (scopes.has("terminal") && agent.capabilities.terminal === "admin") {
    blockedReason =
      "Administrator terminal access is blocked. Change this agent to Safe or User commands.";
  } else if (system) {
    blockedReason =
      "System-control tools are not enabled yet. This release confines Codex to the selected workspace.";
  } else if (lockedAction) {
    blockedReason =
      "Locked mode permits inspection only. Change the Safety mode in Settings to run this action.";
  } else if (missingCapabilities.length > 0) {
    blockedReason = `The agent lacks ${missingCapabilities
      .map((scope) => safetyScopeLabels[scope])
      .join(", ")} capability.`;
  } else if (deniedScopes.length > 0) {
    blockedReason = `${deniedScopes
      .map((scope) => safetyScopeLabels[scope])
      .join(", ")} is denied by this agent's approval policy.`;
  }

  const elevatedScopes = scopeList.filter(
    (scope) => agent.approvals[scope] === "ask",
  );
  const approvalScopes = Array.from(
    new Set<SafetyScope>([
      ...elevatedScopes,
      ...(destructive ? (["files"] as SafetyScope[]) : []),
      ...(task.priority === "Critical" ? scopeList : []),
      ...(safetyMode === "strict" ? scopeList : []),
    ]),
  );
  const riskLevel: RiskLevel = privileged || system
    ? "Critical"
    : destructive || task.priority === "Critical"
      ? "High"
      : terminal || internet || clipboard || mutatesWorkspace
        ? "Medium"
        : "Low";
  const requiresApproval = blockedReason === null && approvalScopes.length > 0;
  const reason = blockedReason
    ? blockedReason
    : requiresApproval
      ? `${riskLevel}-risk task requests ${approvalScopes
          .map((scope) => safetyScopeLabels[scope])
          .join(", ")}. Authorization applies to one run only.`
      : `${riskLevel}-risk task is permitted by the current agent and application policies.`;

  return {
    riskLevel,
    scopes: scopeList,
    approvalScopes,
    requiresApproval,
    destructive,
    writesWorkspace,
    blockedReason,
    reason,
  };
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

export function executionRouteForTask(
  agents: Agent[],
  models: ModelDefinition[],
  category: TaskCategory,
  preferredAgentId: number,
  activeProvider: RuntimeProviderId,
  providerRegistry: ProviderRegistrySnapshot,
) {
  const candidates = agents
    .filter(
      (agent) =>
        agent.status !== "Paused" &&
        agent.model.trim().toLowerCase() !== "none" &&
        resolveModelAvailability(
          models,
          agent.model,
          providerRegistry,
          activeProvider,
        ).eligible,
    )
    .map((agent) => {
      let score = agent.status === "Waiting" ? 18 : 10;
      const categoryMatch =
        agent.category === category ||
        (category === "General" && agent.category === "Management");
      if (categoryMatch) score += 50;
      if (agent.id === preferredAgentId) score += categoryMatch ? 12 : 3;
      if (agent.role === "Specialist") score += 10;
      if (category === "Development") {
        if (
          agent.capabilities.files === "write" ||
          agent.capabilities.files === "full"
        ) {
          score += 18;
        }
        if (agent.capabilities.terminal !== "none") score += 8;
      }
      if (
        (category === "Browsing" || category === "Research") &&
        agent.capabilities.internet !== "none"
      ) {
        score += 16;
      }
      score -= agent.tasks.filter(
        (task) =>
          task.status === "Pending" ||
          task.status === "Running" ||
          task.status === "Under Review",
      ).length * 4;
      return { agent, score, categoryMatch };
    })
    .sort(
      (left, right) =>
        right.score - left.score || left.agent.id - right.agent.id,
    );

  const winner = candidates[0] ?? null;
  if (!winner) {
    return null;
  }

  return {
    agent: winner.agent,
    reason: winner.categoryMatch
      ? `${winner.agent.name} was selected for ${category} expertise and current availability.`
      : `${winner.agent.name} was selected as the strongest available active agent.`,
  };
}

export function reviewAgentForTask(
  agents: Agent[],
  ownerAgentId: number,
  category: TaskCategory,
  models: ModelDefinition[],
  activeProvider: RuntimeProviderId,
  providerRegistry: ProviderRegistrySnapshot,
) {
  return (
    agents
      .filter(
        (agent) =>
          agent.id !== ownerAgentId &&
          agent.status !== "Paused" &&
          agent.model.trim().toLowerCase() !== "none" &&
          resolveModelAvailability(
            models,
            agent.model,
            providerRegistry,
            activeProvider,
          ).eligible &&
          agent.capabilities.files !== "none" &&
          (agent.role === "Senior Agent" ||
            agent.role === "Team Leader" ||
            agent.role === "Supervisor"),
      )
      .map((agent) => {
        let score = agent.status === "Waiting" ? 14 : 8;
        if (agent.category === category) score += 24;
        if (agent.role === "Senior Agent") score += 55;
        else if (agent.role === "Team Leader") score += 45;
        else if (agent.role === "Supervisor") score += 35;
        else score += 8;
        score += agent.authorityLevel * 3;
        score -= agent.tasks.filter(
          (task) => task.status === "Running" || task.status === "Under Review",
        ).length * 5;
        return { agent, score };
      })
      .sort(
        (left, right) =>
          right.score - left.score || left.agent.id - right.agent.id,
      )[0]?.agent ?? null
  );
}

const defaultAgentPerformance: AgentPerformance = {
  strength: 5,
  focus: "balanced",
  cpuLimit: 70,
  gpuLimit: 50,
  overflowAction: "queue",
  redirectAgentId: null,
};

const defaultAppPreferences: AppPreferences = {
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
  voiceCommandReplacements: "fire fox = firefox\nvisual studio = visual studio code",
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
    overflowAction:
      performance?.overflowAction === "redirect"
        ? "redirect"
        : "queue",
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
      preferences?.theme === "light" ||
      preferences?.theme === "system"
        ? preferences.theme
        : "dark",
    accentColor:
      preferences?.accentColor === "blue" ||
      preferences?.accentColor === "cyan" ||
      preferences?.accentColor === "green"
        ? preferences.accentColor
        : "violet",
    density:
      preferences?.density === "compact"
        ? "compact"
        : "comfortable",
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
    defaultPerformance: normalizePerformance(
      preferences?.defaultPerformance,
    ),
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
    voiceControlMasterEnabled: preferences?.voiceControlMasterEnabled !== false,
    voiceWakePhrase:
      typeof preferences?.voiceWakePhrase === "string" &&
      preferences.voiceWakePhrase.trim() &&
      preferences.voiceWakePhrase.trim().toLowerCase() !== "lucy activate, on"
        ? preferences.voiceWakePhrase.trim().toLowerCase()
        : "lucy",
    voiceDeactivatePhrase:
      typeof preferences?.voiceDeactivatePhrase === "string" && preferences.voiceDeactivatePhrase.trim()
        ? preferences.voiceDeactivatePhrase.trim().toLowerCase()
        : "lucy deactivate",
    voiceOpenPhrases:
      typeof preferences?.voiceOpenPhrases === "string" && preferences.voiceOpenPhrases.trim()
        ? preferences.voiceOpenPhrases.trim().toLowerCase()
        : "open, launch, start",
    voiceClosePhrases:
      typeof preferences?.voiceClosePhrases === "string" && preferences.voiceClosePhrases.trim()
        ? preferences.voiceClosePhrases.trim().toLowerCase()
        : "close, quit, exit",
    voiceCommandReplacements:
      typeof preferences?.voiceCommandReplacements === "string"
        ? preferences.voiceCommandReplacements.toLowerCase()
        : "fire fox = firefox\nvisual studio = visual studio code",
    voiceState:
      preferences?.voiceState === "VOICE_ACTIVE" || preferences?.voiceState === "VOICE_OFF"
        ? preferences.voiceState
        : "VOICE_PASSIVE",
  };
}

function AgentsPage({
  agents,
  setAgents,
  models,
  providerRegistry,
  preferences,
  runCoordinator,
  setRunCoordinator,
  approvalRequests,
  setApprovalRequests,
  onOpenApprovals,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  models: ModelDefinition[];
  providerRegistry: ProviderRegistrySnapshot;
  preferences: AppPreferences;
  runCoordinator: RunCoordinatorUiState;
  setRunCoordinator: React.Dispatch<
    React.SetStateAction<RunCoordinatorUiState>
  >;
  approvalRequests: ApprovalRequest[];
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  onOpenApprovals: () => void;
}) {
  const [isCreating, setIsCreating] = useState(false);
  const [editingAgentId, setEditingAgentId] = useState<number | null>(null);
  const [agentName, setAgentName] = useState("");
  const [agentDescription, setAgentDescription] = useState("");
  const [agentRole, setAgentRole] = useState<AgentRole>("Specialist");
  const [agentCategory, setAgentCategory] =
    useState<AgentCategory>("General");
  const [agentReportsTo, setAgentReportsTo] = useState<number | null>(null);
  const [agentAuthorityLevel, setAgentAuthorityLevel] =
    useState<AuthorityLevel>(1);
  const [newTaskTitle, setNewTaskTitle] = useState("");
  const [newTaskCategory, setNewTaskCategory] =
    useState<TaskCategory>(preferences.defaultTaskCategory);
  const [newTaskPriority, setNewTaskPriority] =
    useState<TaskPriority>(preferences.defaultTaskPriority);
  const [newTaskRoutingMode, setNewTaskRoutingMode] =
    useState<RoutingMode>(preferences.defaultRoutingMode);
  const [newTaskWorkspaceId, setNewTaskWorkspaceId] = useState<string | null>(
    preferences.activeWorkspaceId,
  );
  const [selectedAgentId, setSelectedAgentId] = useState<number | null>(null);
  const [activeAgentGroup, setActiveAgentGroup] = useState<
    "Development" | "Finance and Events" | "Web and PC Control"
  >("Development");
  const [activeWorkspaceTab, setActiveWorkspaceTab] =
    useState<WorkspaceTab>("Overview");
  const [runtimeError, setRuntimeError] = useState("");
  const [systemCapabilityMessage, setSystemCapabilityMessage] = useState("");

  const selectedAgent =
    agents.find((agent) => agent.id === selectedAgentId) ?? null;
  const activeRun = runCoordinator.snapshot.activeAttempt;
  const runActive = activeRun !== null;
  const runningTaskId =
    activeRun && activeRun.taskOwnerAgentId === selectedAgentId
      ? activeRun.taskId
      : null;
  const activeRunId = activeRun?.requestId ?? null;
  const activeRunKind = activeRun?.runMode === "review" ? "review" : "execution";
  const runtimeProgress = runCoordinator.progress;
  const cancelRequested = runCoordinator.stopRequested;
  const availableModels = executableModels(
    models,
    providerRegistry,
    preferences.activeAiProvider,
  );

  const selectedAgentTasks = selectedAgent?.tasks ?? [];
  const completedTaskCount = selectedAgentTasks.filter(
    (task) => task.status === "Completed",
  ).length;
  const failedTaskCount = selectedAgentTasks.filter(
    (task) => task.status === "Failed",
  ).length;
  const remainingTaskCount = selectedAgentTasks.filter(
    (task) =>
      task.status === "Pending" ||
      task.status === "Running" ||
      task.status === "Blocked" ||
      task.status === "Under Review",
  ).length;
  const currentTask =
    selectedAgentTasks.find((task) => task.status === "Running") ??
    selectedAgentTasks.find((task) => task.status === "Under Review") ??
    selectedAgentTasks.find((task) => task.status === "Pending") ??
    null;
  const latestActivity = selectedAgent?.activity[0] ?? null;
  const memoryCharacterCount = selectedAgent?.memory.trim().length ?? 0;

  const isEditing = editingAgentId !== null;
  const isModalOpen = isCreating || isEditing;

  useEffect(() => {
    if (
      newTaskWorkspaceId === null ||
      !preferences.workspaces.some(
        (workspace) => workspace.id === newTaskWorkspaceId,
      )
    ) {
      setNewTaskWorkspaceId(preferences.activeWorkspaceId);
    }
  }, [preferences.activeWorkspaceId, preferences.workspaces, newTaskWorkspaceId]);

  function workspaceForTask(task: AgentTask) {
    return (
      preferences.workspaces.find(
        (workspace) => workspace.id === task.workspaceId,
      ) ??
      preferences.workspaces.find(
        (workspace) => workspace.id === preferences.activeWorkspaceId,
      ) ??
      null
    );
  }

  function latestApprovalForTask(task: AgentTask) {
    return approvalRequests.find(
      (request) =>
        request.agentId === selectedAgent?.id && request.taskId === task.id,
    );
  }

  function awaitingRunApproval(task: AgentTask) {
    const approval = latestApprovalForTask(task);
    return (
      approval?.consumedAt === null &&
      (approval.status === "Pending" || approval.status === "Approved")
    );
  }

  function latestRunForTask(task: AgentTask) {
    return runCoordinator.snapshot.recentAttempts.find(
      (attempt) =>
        attempt.taskOwnerAgentId === selectedAgent?.id &&
        attempt.taskId === task.id,
    );
  }

  function createActivity(message: string): ActivityEntry {
    return {
      id: Date.now() + Math.floor(Math.random() * 1000),
      message,
      createdAt: new Date().toISOString(),
    };
  }

  function resetForm() {
    setAgentName("");
    setAgentDescription("");
    setAgentRole("Specialist");
    setAgentCategory("General");
    setAgentReportsTo(null);
    setAgentAuthorityLevel(1);
    setIsCreating(false);
    setEditingAgentId(null);
  }

  function openCreateAgent() {
    setAgentName("");
    setAgentDescription("");
    setAgentRole("Specialist");
    setAgentCategory("General");
    setAgentReportsTo(null);
    setAgentAuthorityLevel(1);
    setEditingAgentId(null);
    setIsCreating(true);
  }

  function openEditAgent(agent: Agent) {
    setAgentName(agent.name);
    setAgentDescription(agent.description);
    setAgentRole(agent.role);
    setAgentCategory(agent.category);
    setAgentReportsTo(agent.reportsTo);
    setAgentAuthorityLevel(agent.authorityLevel);
    setIsCreating(false);
    setEditingAgentId(agent.id);
  }

  function saveAgent() {
    const trimmedName = agentName.trim();
    const trimmedDescription = agentDescription.trim();

    if (!trimmedName || !trimmedDescription) {
      return;
    }

    if (
      editingAgentId === null &&
      preferences.defaultModel.toLowerCase() !== "none"
    ) {
      const defaultAvailability = resolveModelAvailability(
        models,
        preferences.defaultModel,
        providerRegistry,
        preferences.activeAiProvider,
      );
      if (!defaultAvailability.eligible) {
        setRuntimeError(
          `The default model is unavailable: ${defaultAvailability.reason}`,
        );
        return;
      }
    }

    if (editingAgentId !== null) {
      setAgents((currentAgents) =>
        currentAgents.map((agent) =>
          agent.id === editingAgentId
            ? {
                ...agent,
                name: trimmedName,
                description: trimmedDescription,
                role: agentRole,
                category: agentCategory,
                reportsTo:
                  agentReportsTo === editingAgentId ? null : agentReportsTo,
                authorityLevel: agentAuthorityLevel,
              }
            : agent,
        ),
      );
    } else {
      const newAgent: Agent = {
        id: Date.now(),
        name: trimmedName,
        description: trimmedDescription,
        status: preferences.defaultAgentStatus,
        role: agentRole,
        category: agentCategory,
        reportsTo: agentReportsTo,
        authorityLevel: agentAuthorityLevel,
        model: preferences.defaultModel,
        memory: "",
        tasks: [],
        activity: [],
        performance: {
          ...preferences.defaultPerformance,
        },

        capabilities: {
          files: "read",
          internet: "none",
          clipboard: "none",
          terminal: "none",
          system: "none",
        },

        approvals: {
          files: "ask",
          internet: "ask",
          clipboard: "ask",
          terminal: "ask",
          system: "ask",
        },
      };
      setAgents((currentAgents) => [...currentAgents, newAgent]);
    }

    resetForm();
  }

  function setAgentStatus(
    agentId: number,
    status: Agent["status"],
  ) {
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === agentId
          ? {
              ...agent,
              status,
              activity: [
                createActivity(`Agent status changed to ${status}.`),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function updateCapability(
    key: CapabilityKey,
    value: Agent["capabilities"][CapabilityKey],
  ) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? {
              ...agent,
              capabilities: {
                ...agent.capabilities,
                [key]: value,
              } as Agent["capabilities"],
              activity: [
                createActivity(
                  `${key} access changed to ${String(value)}.`,
                ),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );

    if (key !== "system" || selectedAgent?.name !== "PC Control Agent") {
      return;
    }
    if (value !== "full") {
      setSystemCapabilityMessage("Full desktop input is disabled. PC Control can no longer move the pointer or send clicks.");
      return;
    }
    if (!isDesktopRuntime()) {
      setSystemCapabilityMessage("Full system access is saved. KDE desktop input can only be enabled in the installed app, not the browser preview.");
      return;
    }

    setSystemCapabilityMessage(
      "Confirm the capability change in the trusted desktop dialog, then select Enable KDE desktop input.",
    );
  }

  async function enableDesktopInput() {
    if (!isDesktopRuntime()) {
      setSystemCapabilityMessage("KDE desktop input can only be enabled in the installed app, not the browser preview.");
      return;
    }
    if (!selectedAgent) {
      setSystemCapabilityMessage("Select PC Control Agent first.");
      return;
    }
    setSystemCapabilityMessage("Requesting backend authorization...");
    try {
      const authorization = await prepareBackendAuthorization(
        { kind: "enableDesktopControl", agentId: selectedAgent.id },
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setSystemCapabilityMessage(
          "Desktop input is waiting for trusted authorization in Approvals.",
        );
        onOpenApprovals();
        return;
      }
      const status = await invoke<DesktopControlStatus>("enable_desktop_control", {
        agentId: selectedAgent.id,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setSystemCapabilityMessage(status.message);
    } catch (error) {
      setSystemCapabilityMessage(errorMessage(error));
    }
  }

  function updateApproval(key: ApprovalKey, value: ApprovalMode) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? {
              ...agent,
              approvals: {
                ...agent.approvals,
                [key]: value,
              },
              activity: [
                createActivity(
                  `${key} approval policy changed to ${value}.`,
                ),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function updateAgentModel(model: string) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? {
              ...agent,
              model,
              activity: [
                createActivity(`Model changed to ${model}.`),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function updateMemory(memory: string) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId ? { ...agent, memory } : agent,
      ),
    );
  }

  function addTask() {
    const trimmedTitle = newTaskTitle.trim();

    if (selectedAgentId === null || !trimmedTitle) {
      return;
    }

    const route =
      newTaskRoutingMode === "automatic"
        ? executionRouteForTask(
            agents,
            models,
            newTaskCategory,
            selectedAgentId,
            preferences.activeAiProvider,
            providerRegistry,
          )
        : null;
    if (newTaskRoutingMode === "automatic" && !route) {
      setRuntimeError(
        "No active agent has a model executable through the active provider for automatic routing.",
      );
      return;
    }
    const targetAgentId = route?.agent.id ?? selectedAgentId;
    const newTask: AgentTask = {
      id: Date.now(),
      title: trimmedTitle,
      category: newTaskCategory,
      priority: newTaskPriority,
      assignedAgentId: targetAgentId,
      status: "Pending",
      phase: "Assigned",
      createdAt: new Date().toISOString(),
      completedAt: null,
      result: null,
      responseId: null,
      runtimeModel: null,
      totalTokens: null,
      workspaceId: newTaskWorkspaceId,
      changedFiles: [],
      diff: null,
      durationSeconds: null,
      routingMode: newTaskRoutingMode,
      routedFromAgentId:
        newTaskRoutingMode === "automatic" ? selectedAgentId : null,
      routingReason: route?.reason ?? null,
      reviewAgentId: null,
      reviewStatus: "Not Requested",
      reviewResult: null,
      reviewModel: null,
      reviewDurationSeconds: null,
      reviewedAt: null,
    };

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === targetAgentId
          ? {
              ...agent,
              tasks: [...agent.tasks, newTask],
              activity: [
                createActivity(
                  route
                    ? `Task automatically routed here: ${trimmedTitle}`
                    : `Task created: ${trimmedTitle}`,
                ),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );

    setNewTaskTitle("");
    setNewTaskCategory(preferences.defaultTaskCategory);
    setNewTaskPriority(preferences.defaultTaskPriority);
    setNewTaskRoutingMode(preferences.defaultRoutingMode);
    setNewTaskWorkspaceId(preferences.activeWorkspaceId);
    if (targetAgentId !== selectedAgentId) {
      setSelectedAgentId(targetAgentId);
    }
  }

  function autoRouteTask(task: AgentTask) {
    if (!selectedAgent) {
      return;
    }
    const route = executionRouteForTask(
      agents,
      models,
      task.category,
      selectedAgent.id,
      preferences.activeAiProvider,
      providerRegistry,
    );
    if (!route) {
      setRuntimeError(
        "No active agent has a model executable through the active provider for automatic routing.",
      );
      return;
    }

    const routedTask: AgentTask = {
      ...task,
      assignedAgentId: route.agent.id,
      routingMode: "automatic",
      routedFromAgentId: selectedAgent.id,
      routingReason: route.reason,
    };
    setAgents((currentAgents) =>
      currentAgents.map((agent) => {
        if (route.agent.id === selectedAgent.id && agent.id === selectedAgent.id) {
          return {
            ...agent,
            tasks: agent.tasks.map((item) =>
              item.id === task.id ? routedTask : item,
            ),
            activity: [
              createActivity(`Routing confirmed for "${task.title}".`),
              ...agent.activity,
            ],
          };
        }
        if (agent.id === selectedAgent.id) {
          return {
            ...agent,
            tasks: agent.tasks.filter((item) => item.id !== task.id),
            activity: [
              createActivity(
                `Routed "${task.title}" to ${route.agent.name}.`,
              ),
              ...agent.activity,
            ],
          };
        }
        if (agent.id === route.agent.id) {
          return {
            ...agent,
            tasks: [...agent.tasks, routedTask],
            activity: [
              createActivity(
                `Received automatically routed task "${task.title}".`,
              ),
              ...agent.activity,
            ],
          };
        }
        return agent;
      }),
    );
    setSelectedAgentId(route.agent.id);
    setRuntimeError("");
  }

  function setTaskWorkflow(
    taskId: number,
    status: TaskStatus,
    phase: TaskPhase,
  ) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? {
              ...agent,
              tasks: agent.tasks.map((task) => {
                if (
                  status === "Running" &&
                  task.status === "Running" &&
                  task.id !== taskId
                ) {
                  return {
                    ...task,
                    status: "Pending",
                    phase: "Assigned",
                  };
                }

                return task.id === taskId
                  ? {
                      ...task,
                      status,
                      phase,
                      completedAt:
                        status === "Completed" || status === "Failed"
                          ? new Date().toISOString()
                          : null,
                    }
                  : task;
              }),
              activity: [
                createActivity(
                  `Task "${
                    agent.tasks.find((task) => task.id === taskId)?.title ??
                    "Unknown"
                  }" changed to ${phase}.`,
                ),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function advanceTask(task: AgentTask) {
    if (task.phase === "Assigned") {
      setTaskWorkflow(task.id, "Running", "Specialist Work");
      return;
    }

    if (task.phase === "Specialist Work") {
      setTaskWorkflow(task.id, "Under Review", "Senior Review");
      return;
    }

    if (task.phase === "Senior Review") {
      setTaskWorkflow(task.id, "Under Review", "Team Leader Review");
      return;
    }

    if (task.phase === "Team Leader Review") {
      setTaskWorkflow(task.id, "Under Review", "Supervisor Approval");
      return;
    }

    if (task.phase === "Supervisor Approval") {
      setTaskWorkflow(task.id, "Completed", "Finished");
    }
  }

  async function runSeniorReview(
    task: AgentTask,
    _executionResult?: AgentRunResult,
    continuation = false,
  ) {
    if (!selectedAgent || (!continuation && runActive)) {
      return;
    }
    if (!isDesktopRuntime()) {
      setRuntimeError("Senior reviews run only in the installed desktop app.");
      return;
    }

    const workspace = workspaceForTask(task);
    const reviewer = reviewAgentForTask(
      agents,
      selectedAgent.id,
      task.category,
      models,
      preferences.activeAiProvider,
      providerRegistry,
    );
    if (!workspace) {
      setRuntimeError("The task workspace is no longer available.");
      return;
    }
    if (!reviewer) {
      setRuntimeError(
        "No active senior reviewer has file-read access and a model executable through the active provider.",
      );
      return;
    }
    let reviewAuthorization: AuthorizationReadiness;
    try {
      reviewAuthorization = await prepareBackendAuthorization(
        {
          kind: "runTask",
          agentId: reviewer.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
          runMode: "review",
        },
        setApprovalRequests,
      );
    } catch (error) {
      setRuntimeError(errorMessage(error));
      return;
    }
    if (!reviewAuthorization.ready) {
      setRuntimeError(
        "This review is waiting for backend authorization. Open Approvals to approve or deny it.",
      );
      onOpenApprovals();
      return;
    }
    const reviewRunId = `review-${task.id}-${Date.now()}`;
    setRuntimeError("");

    try {
      const result = await invoke<AgentRunResult>("run_agent_task", {
        request: {
          runId: reviewRunId,
          runMode: "review",
          agentId: reviewer.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
        },
      });
      const approved = /\bverdict\s*:\s*approved\b/i.test(result.output);
      const changesRequested = /\bverdict\s*:\s*changes requested\b/i.test(
        result.output,
      );
      if (!approved && !changesRequested) {
        setRuntimeError(
          "The reviewer returned no valid verdict. Review its result and run the senior review again.",
        );
      }
    } catch (error) {
      setRuntimeError(errorMessage(error));
    }
  }

  async function runTaskWithAgent(task: AgentTask) {
    if (!selectedAgent || runActive) {
      return;
    }

    if (!isDesktopRuntime()) {
      setRuntimeError(
        "Real agents run in the installed desktop app, not the browser preview.",
      );
      return;
    }

    const selectedModelAvailability = resolveModelAvailability(
      models,
      selectedAgent.model,
      providerRegistry,
      preferences.activeAiProvider,
    );
    if (!selectedModelAvailability.eligible) {
      setRuntimeError(
        `The model assigned to ${selectedAgent.name} is unavailable: ${selectedModelAvailability.reason}`,
      );
      return;
    }

    const workspace = workspaceForTask(task);
    if (!workspace) {
      setRuntimeError(
        "This task has no workspace. Add or select one in Settings first.",
      );
      return;
    }

    const assessment = taskSafetyAssessment(
      task,
      selectedAgent,
      preferences.safetyMode,
    );
    if (assessment.blockedReason) {
      setRuntimeError(assessment.blockedReason);
      setAgents((currentAgents) =>
        currentAgents.map((agent) =>
          agent.id === selectedAgent.id
            ? {
                ...agent,
                status: "Waiting",
                tasks: agent.tasks.map((item) =>
                  item.id === task.id
                    ? {
                        ...item,
                        status: "Blocked",
                        phase: "Supervisor Approval",
                        completedAt: null,
                      }
                    : item,
                ),
                activity: [
                  createActivity(
                    `Safety boundary blocked "${task.title}": ${assessment.blockedReason}`,
                  ),
                  ...agent.activity,
                ],
              }
            : agent,
        ),
      );
      return;
    }

    let authorization: AuthorizationReadiness;
    try {
      authorization = await prepareBackendAuthorization(
        {
          kind: "runTask",
          agentId: selectedAgent.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
          runMode: "execute",
        },
        setApprovalRequests,
      );
    } catch (error) {
      setRuntimeError(errorMessage(error));
      return;
    }
    if (!authorization.ready) {
      setRuntimeError(
        "This run is waiting for backend authorization. Open Approvals to approve or deny it.",
      );
      return;
    }

    setRuntimeError("");
    const runId = `task-${task.id}-${Date.now()}`;

    try {
      const result = await invoke<AgentRunResult>("run_agent_task", {
        request: {
          runId,
          runMode: "execute",
          agentId: selectedAgent.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
        },
      });

      if (preferences.reviewMode === "automatic") {
        await runSeniorReview(task, result, true);
      }
    } catch (error) {
      setRuntimeError(errorMessage(error));
    }
  }

  async function cancelActiveRun() {
    if (!activeRunId || cancelRequested) {
      return;
    }

    setRunCoordinator((current) => markRunStopRequested(current, true));
    try {
      const accepted = await invoke<boolean>("cancel_agent_run", {
        runId: activeRunId,
      });
      if (!accepted) {
        setRunCoordinator((current) => markRunStopRequested(current, false));
      }
    } catch (error) {
      setRuntimeError(errorMessage(error));
      setRunCoordinator((current) => markRunStopRequested(current, false));
    }
  }

  async function openTaskFile(task: AgentTask, itemPath: string) {
    const workspace = workspaceForTask(task);
    if (!workspace) {
      setRuntimeError("The task workspace is no longer available.");
      return;
    }

    try {
      if (!selectedAgent) {
        throw new Error("Select the task agent before opening workspace files.");
      }
      const authorization = await prepareBackendAuthorization(
        {
          kind: "openWorkspaceItem",
          agentId: selectedAgent.id,
          workspaceId: workspace.id,
          itemPath,
        },
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setRuntimeError(
          "Opening this workspace item is waiting for backend authorization.",
        );
        onOpenApprovals();
        return;
      }
      await invoke("open_workspace_item", {
        request: {
          agentId: selectedAgent.id,
          workspaceId: workspace.id,
          itemPath,
        },
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
    } catch (error) {
      setRuntimeError(errorMessage(error));
    }
  }

  function deleteTask(taskId: number) {
    if (selectedAgentId === null) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? {
              ...agent,
              tasks: agent.tasks.filter((task) => task.id !== taskId),
              activity: [
                createActivity(
                  `Task deleted: ${agent.tasks.find((task) => task.id === taskId)?.title ?? "Unknown"}`,
                ),
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function clearActivity() {
    if (selectedAgentId === null || !selectedAgent) {
      return;
    }

    const shouldClear = window.confirm(
      `Clear all activity for "${selectedAgent.name}"?`,
    );

    if (!shouldClear) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === selectedAgentId
          ? { ...agent, activity: [] }
          : agent,
      ),
    );
  }

  function deleteAgent(agentId: number) {
    const agent = agents.find((item) => item.id === agentId);

    if (!agent) {
      return;
    }

    const shouldDelete = window.confirm(
      `Delete "${agent.name}"? This cannot be undone.`,
    );

    if (!shouldDelete) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.filter((item) => item.id !== agentId),
    );
  }

  if (selectedAgent) {
    return (
      <>
        <header className="topbar">
          <div>
            <button
              className="secondary-button"
              onClick={() => {
                setSelectedAgentId(null);
                setActiveWorkspaceTab("Overview");
              }}
              style={{ marginBottom: "14px" }}
            >
              ← Back to agents
            </button>

            <span className="eyebrow">AGENT WORKSPACE</span>
            <h1>{selectedAgent.name}</h1>
            <p className="page-message">{selectedAgent.description}</p>
          </div>

          <span
            className={`agent-status ${selectedAgent.status.toLowerCase()}`}
          >
            {selectedAgent.status}
          </span>
        </header>

        <div className="workspace-tabs">
          {(
            [
              "Overview",
              "Capabilities",
              "Memory",
              "Tasks",
              "Activity",
            ] as WorkspaceTab[]
          ).map((tab) => (
            <button
              key={tab}
              className={
                activeWorkspaceTab === tab
                  ? "primary-button"
                  : "secondary-button"
              }
              onClick={() => setActiveWorkspaceTab(tab)}
            >
              {tab}
            </button>
          ))}
        </div>

        {activeWorkspaceTab === "Overview" ? (
          <section className="panel">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">OVERVIEW</span>
                <h2>Agent details</h2>
              </div>
            </div>

            <div className="summary-grid">
              <article className="summary-card">
                <span>Model</span>
                <select
                  value={selectedAgent.model}
                  onChange={(event) => updateAgentModel(event.target.value)}
                  style={{ marginTop: "12px", width: "100%" }}
                >
                  <option value="None">None</option>
                  {selectedAgent.model.toLowerCase() !== "none" &&
                    !availableModels.some(
                      (model) => model.name === selectedAgent.model,
                    ) && (
                      <option value={selectedAgent.model} disabled>
                        {selectedAgent.model} · unavailable
                      </option>
                    )}
                  {availableModels.map((model) => (
                    <option value={model.name} key={model.id}>
                      {model.name} · {model.provider}
                    </option>
                  ))}
                </select>
                <small>
                  Executable through {preferences.activeAiProvider}
                </small>
              </article>

              <article className="summary-card">
                <span>Status</span>
                <strong>{selectedAgent.status}</strong>
                <small>Current operating state</small>
              </article>

              <article className="summary-card">
                <span>Role</span>
                <strong>{selectedAgent.role}</strong>
                <small>{selectedAgent.category}</small>
              </article>

              <article className="summary-card">
                <span>Reports to</span>
                <strong>
                  {selectedAgent.reportsTo === null
                    ? "User / Top level"
                    : agents.find(
                        (agent) => agent.id === selectedAgent.reportsTo,
                      )?.name ?? "Unknown agent"}
                </strong>
                <small>
                  Authority level {selectedAgent.authorityLevel}
                </small>
              </article>

              <article className="summary-card">
                <span>Current task</span>
                <strong>{currentTask ? currentTask.title : "Idle"}</strong>
                <small>
                  {currentTask
                    ? `${remainingTaskCount} task${
                        remainingTaskCount === 1 ? "" : "s"
                      } remaining`
                    : selectedAgentTasks.length > 0
                      ? "All assigned tasks are complete"
                      : "No task assigned"}
                </small>
              </article>

              <article className="summary-card">
                <span>Task progress</span>
                <strong>
                  {completedTaskCount} / {selectedAgentTasks.length}
                </strong>
                <small>
                  {selectedAgentTasks.length === 0
                    ? "No tasks in queue"
                    : `${completedTaskCount} completed, ${remainingTaskCount} active${
                        failedTaskCount > 0
                          ? `, ${failedTaskCount} failed`
                          : ""
                      }`}
                </small>
              </article>

              <article className="summary-card">
                <span>Memory</span>
                <strong>
                  {memoryCharacterCount > 0 ? "Configured" : "Empty"}
                </strong>
                <small>
                  {memoryCharacterCount > 0
                    ? `${memoryCharacterCount} characters stored`
                    : "No persistent memory stored"}
                </small>
              </article>

              <article className="summary-card">
                <span>Latest activity</span>
                <strong>
                  {latestActivity ? latestActivity.message : "None"}
                </strong>
                <small>
                  {latestActivity
                    ? new Date(latestActivity.createdAt).toLocaleString()
                    : "No activity recorded"}
                </small>
              </article>
            </div>
          </section>
        ) : activeWorkspaceTab === "Capabilities" ? (
          <section className="panel">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">CAPABILITIES</span>
                <h2>Access and approval policies</h2>
                <p className="page-message">
                  Changes are saved automatically for this agent.
                </p>
              </div>
            </div>

            <div
              style={{
                display: "grid",
                gap: "14px",
              }}
            >
              {(
                [
                  {
                    key: "files",
                    label: "Files",
                    description: "Read, create, edit, and delete local files.",
                    options: [
                      ["none", "None"],
                      ["read", "Read only"],
                      ["write", "Read and write"],
                      ["full", "Full control"],
                    ],
                  },
                  {
                    key: "internet",
                    label: "Internet",
                    description: "Access websites and online services.",
                    options: [
                      ["none", "None"],
                      ["read", "Read only"],
                      ["write", "Read and submit"],
                      ["full", "Full control"],
                    ],
                  },
                  {
                    key: "clipboard",
                    label: "Clipboard",
                    description: "Read from or write to the system clipboard.",
                    options: [
                      ["none", "None"],
                      ["read", "Read only"],
                      ["write", "Read and write"],
                      ["full", "Full control"],
                    ],
                  },
                  {
                    key: "terminal",
                    label: "Terminal",
                    description: "Run commands with a defined privilege level.",
                    options: [
                      ["none", "None"],
                      ["safe", "Safe commands"],
                      ["user", "User commands"],
                      ["admin", "Administrator"],
                    ],
                  },
                  {
                    key: "system",
                    label: "System",
                    description: "Use notifications, power controls, and OS APIs.",
                    options: [
                      ["none", "None"],
                      ["notifications", "Notifications"],
                      ["power", "Power controls"],
                      ["full", "Full control"],
                    ],
                  },
                ] as {
                  key: CapabilityKey;
                  label: string;
                  description: string;
                  options: [string, string][];
                }[]
              ).map((capability) => (
                <article className="summary-card" key={capability.key}>
                  <div style={{ marginBottom: "14px" }}>
                    <strong style={{ display: "block" }}>
                      {capability.label}
                    </strong>
                    <small>{capability.description}</small>
                  </div>

                  <div className="form-grid">
                    <label className="form-field">
                      <span>Access level</span>
                      <select
                        value={selectedAgent.capabilities[capability.key]}
                        onChange={(event) =>
                          updateCapability(
                            capability.key,
                            event.target.value as Agent["capabilities"][CapabilityKey],
                          )
                        }
                      >
                        {capability.options.map(([value, label]) => (
                          <option value={value} key={value}>
                            {label}
                          </option>
                        ))}
                      </select>
                    </label>

                    <label className="form-field">
                      <span>Approval policy</span>
                      <select
                        value={selectedAgent.approvals[capability.key]}
                        onChange={(event) =>
                          updateApproval(
                            capability.key,
                            event.target.value as ApprovalMode,
                          )
                        }
                      >
                        <option value="allow">Always allow</option>
                        <option value="ask">Ask every time</option>
                        <option value="deny">Never allow</option>
                      </select>
                    </label>
                  </div>
                  {selectedAgent.name === "PC Control Agent" && capability.key === "system" && (
                    <div className="form-hint">
                      {selectedAgent.capabilities.system === "full"
                        ? "Full system access is selected. Enable KDE desktop input to allow pointer and keyboard actions across applications."
                        : "Select Full control to enable KDE desktop-input permission for pointer and keyboard actions."}
                      {selectedAgent.capabilities.system === "full" && (
                        <button className="secondary-button" onClick={enableDesktopInput}>Enable KDE desktop input</button>
                      )}
                      {systemCapabilityMessage && <span>{systemCapabilityMessage}</span>}
                    </div>
                  )}
                </article>
              ))}
            </div>
          </section>
        ) : activeWorkspaceTab === "Memory" ? (
          <section className="panel">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">MEMORY</span>
                <h2>Agent memory</h2>
                <p className="page-message">
                  Store durable instructions, context, and facts this agent
                  should remember.
                </p>
              </div>
            </div>

            <label className="form-field">
              <span>Persistent memory</span>
              <textarea
                rows={14}
                value={selectedAgent.memory}
                onChange={(event) => updateMemory(event.target.value)}
                placeholder="Example: This project uses React, TypeScript, and Tauri. Prefer small, testable changes and explain risky actions before running them."
              />
              <small>
                Changes are saved automatically for this agent.
              </small>
            </label>
          </section>
        ) : activeWorkspaceTab === "Tasks" ? (
          <section className="panel">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">TASKS</span>
                <h2>Agent task queue</h2>
                <p className="page-message">
                  Manage assignment, execution, review, approval, and completion phases.
                </p>
              </div>
            </div>

            <div className="task-composer">
              <label className="form-field">
                <span>Task title</span>
                <input
                  type="text"
                  value={newTaskTitle}
                  onChange={(event) => setNewTaskTitle(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      addTask();
                    }
                  }}
                  placeholder="Add a task for this agent"
                />
              </label>

              <label className="form-field">
                <span>Workspace</span>
                <select
                  value={newTaskWorkspaceId ?? ""}
                  onChange={(event) =>
                    setNewTaskWorkspaceId(event.target.value || null)
                  }
                >
                  {preferences.workspaces.length === 0 ? (
                    <option value="">Add a workspace in Settings</option>
                  ) : (
                    preferences.workspaces.map((workspace) => (
                      <option key={workspace.id} value={workspace.id}>
                        {workspace.name}
                      </option>
                    ))
                  )}
                </select>
              </label>

              <label className="form-field">
                <span>Category</span>
                <select
                  value={newTaskCategory}
                  onChange={(event) =>
                    setNewTaskCategory(
                      event.target.value as TaskCategory,
                    )
                  }
                >
                  <option value="Development">Development</option>
                  <option value="Research">Research</option>
                  <option value="Browsing">Browsing</option>
                  <option value="Finance">Finance</option>
                  <option value="Business">Business</option>
                  <option value="Communication">Communication</option>
                  <option value="System Control">System Control</option>
                  <option value="General">General</option>
                </select>
              </label>

              <label className="form-field">
                <span>Priority</span>
                <select
                  value={newTaskPriority}
                  onChange={(event) =>
                    setNewTaskPriority(
                      event.target.value as TaskPriority,
                    )
                  }
                >
                  <option value="Low">Low</option>
                  <option value="Normal">Normal</option>
                  <option value="High">High</option>
                  <option value="Critical">Critical</option>
                </select>
              </label>

              <label className="form-field">
                <span>Routing</span>
                <select
                  value={newTaskRoutingMode}
                  onChange={(event) =>
                    setNewTaskRoutingMode(event.target.value as RoutingMode)
                  }
                >
                  <option value="selected">This agent</option>
                  <option value="automatic">Automatic</option>
                </select>
              </label>

              <button className="primary-button" onClick={addTask}>
                Add task
              </button>
            </div>

            {runtimeError && (
              <div className="runtime-message error" role="alert">
                {runtimeError}
              </div>
            )}

            {selectedAgent.tasks.length === 0 ? (
              <p className="page-message">
                No tasks yet. Add the first task above.
              </p>
            ) : (
              <div className="agent-list">
                {selectedAgent.tasks.map((task) => (
                  <article className="agent-card task-card" key={task.id}>
                    <div className="task-card-content">
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "10px",
                          flexWrap: "wrap",
                          marginBottom: "8px",
                        }}
                      >
                        <h3 style={{ margin: 0 }}>{task.title}</h3>

                        <span
                          className={`agent-status ${
                            task.status === "Running" ||
                            task.reviewStatus === "Running"
                              ? "working"
                              : task.status === "Completed"
                                ? "waiting"
                                : task.status === "Failed"
                                  ? "paused"
                                  : "waiting"
                          }`}
                        >
                          {task.status}
                        </span>
                      </div>

                      <p>
                        {task.category} · {task.priority} priority
                      </p>
                      <small>
                        Phase: {task.phase} · Assigned to{" "}
                        {agents.find(
                          (agent) => agent.id === task.assignedAgentId,
                        )?.name ?? "Unknown agent"} · Workspace:{" "}
                        {workspaceForTask(task)?.name ?? "Missing"}
                      </small>

                      {task.routingMode === "automatic" && (
                        <div className="routing-note">
                          <strong>Automatically routed</strong>
                          <small>
                            {task.routingReason ??
                              "Assigned to the best available matching agent."}
                          </small>
                        </div>
                      )}

                      <div
                        className={`safety-summary risk-${taskSafetyAssessment(
                          task,
                          selectedAgent,
                          preferences.safetyMode,
                        ).riskLevel.toLowerCase()}`}
                      >
                        <div>
                          <strong>
                            Safety check ·{" "}
                            {
                              taskSafetyAssessment(
                                task,
                                selectedAgent,
                                preferences.safetyMode,
                              ).riskLevel
                            } risk
                          </strong>
                          <small>
                            {
                              taskSafetyAssessment(
                                task,
                                selectedAgent,
                                preferences.safetyMode,
                              ).reason
                            }
                          </small>
                        </div>
                        {latestApprovalForTask(task) && (
                          <span
                            className={`authorization-state ${latestApprovalForTask(
                              task,
                            )?.status.toLowerCase()}`}
                          >
                            {latestApprovalForTask(task)?.consumedAt
                              ? "Authorization used"
                              : latestApprovalForTask(task)?.status}
                          </span>
                        )}
                      </div>

                      {runningTaskId === task.id && runtimeProgress.length > 0 && (
                        <div className="run-progress" aria-live="polite">
                          <div className="run-progress-heading">
                            <strong>
                              {activeRunKind === "review"
                                ? "Live senior review"
                                : "Live agent progress"}
                            </strong>
                            <small>{cancelRequested ? "Stopping…" : "Running"}</small>
                          </div>
                          <div className="run-progress-lines">
                            {runtimeProgress.map((line, index) => (
                              <span key={`${index}-${line}`}>{line}</span>
                            ))}
                          </div>
                        </div>
                      )}

                      {latestRunForTask(task) &&
                        hasVisibleTruncation(latestRunForTask(task)!) && (
                          <div className="run-evidence-warning" role="status">
                            Stored run evidence reached a safety bound. The run
                            ledger records which output, progress, diff, file list,
                            or workspace snapshot was truncated.
                          </div>
                        )}

                      {latestRunForTask(task)?.status === "interrupted" && (
                        <div className="run-evidence-warning" role="alert">
                          {latestRunForTask(task)?.recoveryDisposition ===
                          "safe_to_retry"
                            ? "The previous run stopped before dispatch and is safe to retry."
                            : "The previous run may have reached the workspace. Inspect its files before retrying."}
                        </div>
                      )}

                      {task.result && (
                        <div className="agent-result">
                          <div className="agent-result-heading">
                            <strong>Agent result</strong>
                            <small>
                              {task.runtimeModel ?? "Codex"}
                              {task.totalTokens
                                ? ` · ${task.totalTokens.toLocaleString()} tokens`
                                : ""}
                              {task.durationSeconds !== null
                                ? ` · ${task.durationSeconds}s`
                                : ""}
                            </small>
                          </div>
                          <div className="agent-result-text">{task.result}</div>
                        </div>
                      )}

                      {task.reviewResult && (
                        <div
                          className={`review-result review-${task.reviewStatus
                            .toLowerCase()
                            .replace(/\s+/g, "-")}`}
                        >
                          <div className="agent-result-heading">
                            <strong>
                              Senior review · {task.reviewStatus}
                            </strong>
                            <small>
                              {agents.find(
                                (agent) => agent.id === task.reviewAgentId,
                              )?.name ?? "Reviewer"}
                              {task.reviewModel ? ` · ${task.reviewModel}` : ""}
                              {task.reviewDurationSeconds !== null
                                ? ` · ${task.reviewDurationSeconds}s`
                                : ""}
                            </small>
                          </div>
                          <div className="agent-result-text">
                            {task.reviewResult}
                          </div>
                        </div>
                      )}

                      {task.changedFiles.length > 0 && (
                        <div className="changed-files">
                          <strong>Changed files ({task.changedFiles.length})</strong>
                          <div className="file-chip-list">
                            {task.changedFiles.map((file) => (
                              <button
                                type="button"
                                className="file-chip"
                                key={file}
                                onClick={() => openTaskFile(task, file)}
                              >
                                {file}
                              </button>
                            ))}
                          </div>
                        </div>
                      )}

                      {task.diff && (
                        <details className="diff-review">
                          <summary>Review working-tree diff</summary>
                          <pre>{task.diff}</pre>
                        </details>
                      )}
                    </div>

                    <div className="task-card-actions">
                      {task.status === "Blocked" &&
                        latestApprovalForTask(task)?.status === "Pending" && (
                          <button
                            className="primary-button"
                            disabled={runActive}
                            onClick={onOpenApprovals}
                          >
                            Review approval
                          </button>
                        )}

                      {task.routingMode !== "automatic" &&
                        task.status === "Pending" &&
                        task.reviewStatus === "Not Requested" &&
                        !awaitingRunApproval(task) && (
                          <button
                            className="secondary-button"
                            disabled={runActive}
                            onClick={() => autoRouteTask(task)}
                          >
                            Auto-route
                          </button>
                        )}

                      {task.status !== "Blocked" && (
                        <button
                          className={
                            task.reviewStatus === "Pending" ||
                            task.reviewStatus === "Failed"
                              ? "secondary-button"
                              : "primary-button"
                          }
                          disabled={runActive}
                          onClick={() => runTaskWithAgent(task)}
                        >
                          {runningTaskId === task.id
                            ? activeRunKind === "review"
                              ? "Reviewer working…"
                              : "Agent working…"
                            : task.reviewStatus === "Changes Requested"
                              ? "Run revisions"
                            : task.reviewStatus === "Pending" ||
                                task.reviewStatus === "Failed"
                              ? "Run specialist again"
                            : task.result
                              ? "Run again"
                              : "Run Codex agent"}
                        </button>
                      )}

                      {runningTaskId === task.id && (
                        <button
                          className="danger-button"
                          disabled={cancelRequested}
                          onClick={cancelActiveRun}
                        >
                          {cancelRequested ? "Stopping…" : "Stop agent"}
                        </button>
                      )}

                      {preferences.reviewMode !== "off" &&
                        task.result &&
                        task.reviewStatus !== "Running" &&
                        task.reviewStatus !== "Approved" &&
                        task.reviewStatus !== "Changes Requested" && (
                          <button
                            className="primary-button"
                            disabled={runActive}
                            onClick={() => runSeniorReview(task)}
                          >
                            Run senior review
                          </button>
                        )}

                      {!awaitingRunApproval(task) &&
                        task.reviewStatus === "Not Requested" &&
                        task.phase !== "Finished" &&
                        task.phase !== "Failed" && (
                          <button
                            className="secondary-button"
                            disabled={runActive}
                            onClick={() => advanceTask(task)}
                          >
                            {task.phase === "Assigned"
                              ? "Start"
                              : task.phase === "Specialist Work"
                                ? "Send to Senior"
                                : task.phase === "Senior Review"
                                  ? "Send to Team Leader"
                                  : task.phase === "Team Leader Review"
                                    ? "Send to Supervisor"
                                    : "Approve"}
                          </button>
                        )}

                      {!awaitingRunApproval(task) &&
                        task.reviewStatus === "Not Requested" &&
                        task.status !== "Blocked" &&
                        task.status !== "Completed" &&
                        task.status !== "Failed" && (
                          <button
                            className="secondary-button"
                            disabled={runActive}
                            onClick={() =>
                              setTaskWorkflow(
                                task.id,
                                "Blocked",
                                task.phase,
                              )
                            }
                          >
                            Block
                          </button>
                        )}

                      {task.status === "Blocked" &&
                        task.reviewStatus === "Not Requested" &&
                        !awaitingRunApproval(task) && (
                        <button
                          className="secondary-button"
                          disabled={runActive}
                          onClick={() =>
                            setTaskWorkflow(
                              task.id,
                              "Pending",
                              "Assigned",
                            )
                          }
                        >
                          Unblock
                        </button>
                      )}

                      {!awaitingRunApproval(task) &&
                        task.reviewStatus === "Not Requested" &&
                        task.status !== "Failed" &&
                        task.status !== "Completed" && (
                          <button
                            className="secondary-button"
                            disabled={runActive}
                            onClick={() =>
                              setTaskWorkflow(
                                task.id,
                                "Failed",
                                "Failed",
                              )
                            }
                          >
                            Fail
                          </button>
                        )}

                      {(task.status === "Failed" ||
                        task.status === "Completed") && (
                        <button
                          className="secondary-button"
                          disabled={runActive}
                          onClick={() =>
                            setTaskWorkflow(
                              task.id,
                              "Pending",
                              "Assigned",
                            )
                          }
                        >
                          Reset
                        </button>
                      )}

                      <button
                        className="danger-button"
                        disabled={runActive}
                        onClick={() => deleteTask(task.id)}
                      >
                        Delete
                      </button>
                    </div>
                  </article>
                ))}
              </div>
            )}
          </section>
        ) : activeWorkspaceTab === "Activity" ? (
          <section className="panel">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">ACTIVITY</span>
                <h2>Agent activity log</h2>
                <p className="page-message">
                  Recent configuration and task changes for this agent.
                </p>
              </div>

              {selectedAgent.activity.length > 0 && (
                <button
                  className="danger-button"
                  onClick={clearActivity}
                >
                  Clear activity
                </button>
              )}
            </div>

            {selectedAgent.activity.length === 0 ? (
              <p className="page-message">
                No activity recorded yet.
              </p>
            ) : (
              <div className="agent-list">
                {selectedAgent.activity.map((entry) => (
                  <article className="agent-card" key={entry.id}>
                    <div>
                      <h3>{entry.message}</h3>
                      <p>
                        {new Date(entry.createdAt).toLocaleString()}
                      </p>
                    </div>
                  </article>
                ))}
              </div>
            )}
          </section>
        ) : null}
      </>
    );
  }

  function renderHierarchy(
    managerId: number | null,
    visibleIds: Set<number>,
    depth = 0,
  ): React.ReactNode[] {
    return agents
      .filter(
        (agent) => agent.reportsTo === managerId && visibleIds.has(agent.id),
      )
      .flatMap((agent) => [
        <article
          className="agent-card hierarchy-card"
          key={`hierarchy-${agent.id}`}
          style={{ marginLeft: `${Math.min(depth, 4) * 28}px` }}
          onClick={() => setSelectedAgentId(agent.id)}
        >
          <div>
            <h3>{agent.name}</h3>
            <p>{agent.description}</p>
            <small>
              {agent.role} · {agent.category} · Authority level {agent.authorityLevel}
            </small>
          </div>
          <span className={`agent-status ${agent.status.toLowerCase()}`}>
            {agent.status}
          </span>
        </article>,
        ...renderHierarchy(agent.id, visibleIds, depth + 1),
      ]);
  }

  function visibleAgentIdsForGroup(group: typeof activeAgentGroup) {
    const ids = new Set<number>([1, 6]);
    if (group === "Development") {
      [2, 3].forEach((id) => ids.add(id));
    } else if (group === "Finance and Events") {
      [5, 8, 10, 11].forEach((id) => ids.add(id));
    } else {
      [4, 7, 9, 11].forEach((id) => ids.add(id));
    }
    return ids;
  }

  const visibleAgentIds = visibleAgentIdsForGroup(activeAgentGroup);
  const visibleAgents = agents.filter((agent) => visibleAgentIds.has(agent.id));

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">YOUR TEAM</span>
          <h1>Agents</h1>
        </div>

        <button className="primary-button" onClick={openCreateAgent}>
          Create agent
        </button>
      </header>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">ORGANIZATION</span>
            <h2>Reporting hierarchy</h2>
            <p className="page-message">
              Human owner at the top, followed by management, senior review, and specialist execution.
            </p>
          </div>
        </div>

        <div className="workspace-tabs agent-group-tabs" role="tablist" aria-label="Agent groups">
          {(["Development", "Finance and Events", "Web and PC Control"] as const).map(
            (group) => (
              <button
                key={group}
                role="tab"
                aria-selected={activeAgentGroup === group}
                className={activeAgentGroup === group ? "primary-button" : "secondary-button"}
                onClick={() => setActiveAgentGroup(group)}
              >
                {group}
              </button>
            ),
          )}
        </div>

        <div className="agent-list">
          {renderHierarchy(null, visibleAgentIds)}
        </div>
      </section>

      <section className="panel">
        <div className="agent-list">
          {visibleAgents.map((agent) => (
            <article
              className="agent-card team-agent-card"
              key={agent.id}
              onClick={() => {
                setSelectedAgentId(agent.id);
                setActiveWorkspaceTab("Overview");
              }}
              style={{ cursor: "pointer" }}
            >
              <div className="team-agent-summary">
                <h3>{agent.name}</h3>
                <p>{agent.description}</p>
                <small>
                  {agent.role} · {agent.category}
                  {agent.reportsTo !== null
                    ? ` · Reports to ${
                        agents.find(
                          (item) => item.id === agent.reportsTo,
                        )?.name ?? "Unknown"
                      }`
                    : ""}
                </small>
              </div>

              <div className="team-agent-actions">
                <span
                  className={`agent-status ${agent.status.toLowerCase()}`}
                >
                  {agent.status}
                </span>

                {agent.status === "Working" ? (
                  <button
                    className="secondary-button"
                    onClick={(event) => {
                      event.stopPropagation();
                      setAgentStatus(agent.id, "Paused");
                    }}
                  >
                    Pause
                  </button>
                ) : (
                  <button
                    className="primary-button"
                    onClick={(event) => {
                      event.stopPropagation();
                      setAgentStatus(agent.id, "Working");
                    }}
                  >
                    Start
                  </button>
                )}

                <button
                  className="secondary-button"
                  onClick={(event) => {
                    event.stopPropagation();
                    openEditAgent(agent);
                  }}
                >
                  Edit
                </button>

                <button
                  className="danger-button"
                  onClick={(event) => {
                    event.stopPropagation();
                    deleteAgent(agent.id);
                  }}
                >
                  Delete
                </button>
              </div>
            </article>
          ))}
        </div>
      </section>

      {isModalOpen && (
        <div className="modal-backdrop">
          <div className="modal">
            <div className="modal-heading">
              <div>
                <span className="eyebrow">
                  {isEditing ? "EDIT AGENT" : "NEW AGENT"}
                </span>
                <h2>{isEditing ? "Edit agent" : "Create agent"}</h2>
              </div>

              <button className="modal-close" onClick={resetForm}>
                ×
              </button>
            </div>

            <label className="form-field">
              <span>Agent name</span>
              <input
                type="text"
                value={agentName}
                onChange={(event) => setAgentName(event.target.value)}
                placeholder="Example: Research Agent"
              />
            </label>

            <label className="form-field">
              <span>What should this agent do?</span>
              <textarea
                rows={5}
                value={agentDescription}
                onChange={(event) =>
                  setAgentDescription(event.target.value)
                }
                placeholder="Describe the agent's responsibilities"
              />
            </label>

            <div
              style={{
                display: "grid",
                gridTemplateColumns:
                  "repeat(auto-fit, minmax(200px, 1fr))",
                gap: "14px",
              }}
            >
              <label className="form-field">
                <span>Role</span>
                <select
                  value={agentRole}
                  onChange={(event) =>
                    setAgentRole(event.target.value as AgentRole)
                  }
                >
                  <option value="Supervisor">Supervisor</option>
                  <option value="Team Leader">Team Leader</option>
                  <option value="Senior Agent">Senior Agent</option>
                  <option value="Specialist">Specialist</option>
                </select>
              </label>

              <label className="form-field">
                <span>Category</span>
                <select
                  value={agentCategory}
                  onChange={(event) =>
                    setAgentCategory(
                      event.target.value as AgentCategory,
                    )
                  }
                >
                  <option value="Management">Management</option>
                  <option value="Development">Development</option>
                  <option value="Research">Research</option>
                  <option value="Browsing">Browsing</option>
                  <option value="Finance">Finance</option>
                  <option value="Business">Business</option>
                  <option value="Communication">Communication</option>
                  <option value="System Control">System Control</option>
                  <option value="General">General</option>
                </select>
              </label>

              <label className="form-field">
                <span>Reports to</span>
                <select
                  value={agentReportsTo ?? ""}
                  onChange={(event) =>
                    setAgentReportsTo(
                      event.target.value
                        ? Number(event.target.value)
                        : null,
                    )
                  }
                >
                  <option value="">User / Top level</option>
                  {agents
                    .filter((agent) => agent.id !== editingAgentId)
                    .map((agent) => (
                      <option value={agent.id} key={agent.id}>
                        {agent.name} · {agent.role}
                      </option>
                    ))}
                </select>
              </label>

              <label className="form-field">
                <span>Authority level</span>
                <select
                  value={agentAuthorityLevel}
                  onChange={(event) =>
                    setAgentAuthorityLevel(
                      Number(event.target.value) as AuthorityLevel,
                    )
                  }
                >
                  <option value={1}>1 · Specialist</option>
                  <option value={2}>2 · Senior</option>
                  <option value={3}>3 · Team Leader</option>
                  <option value={4}>4 · Supervisor</option>
                </select>
              </label>
            </div>

            <div className="modal-actions">
              <button className="secondary-button" onClick={resetForm}>
                Cancel
              </button>

              <button className="primary-button" onClick={saveAgent}>
                {isEditing ? "Save changes" : "Create agent"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}


function TasksPage({
  agents,
  setAgents,
  retentionDays,
  setRetentionDays,
  setApprovalRequests,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  retentionDays: HistoryRetentionDays;
  setRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
}) {
  const [statusFilter, setStatusFilter] =
    useState<TaskStatus | "All">("All");
  const [categoryFilter, setCategoryFilter] =
    useState<TaskCategory | "All">("All");

  const allTasks = agents.flatMap((agent) =>
    agent.tasks.map((task) => ({
      task,
      agent,
    })),
  );

  const filteredTasks = allTasks.filter(({ task }) => {
    const matchesStatus =
      statusFilter === "All" || task.status === statusFilter;
    const matchesCategory =
      categoryFilter === "All" || task.category === categoryFilter;

    return matchesStatus && matchesCategory;
  });

  const phaseOrder: TaskPhase[] = [
    "Assigned",
    "Specialist Work",
    "Senior Review",
    "Team Leader Review",
    "Supervisor Approval",
    "Finished",
    "Failed",
  ];

  function updateGlobalTask(
    ownerAgentId: number,
    taskId: number,
    status: TaskStatus,
    phase: TaskPhase,
  ) {
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === ownerAgentId
          ? {
              ...agent,
              tasks: agent.tasks.map((task) =>
                task.id === taskId
                  ? {
                      ...task,
                      status,
                      phase,
                      completedAt:
                        status === "Completed" || status === "Failed"
                          ? new Date().toISOString()
                          : null,
                    }
                  : status === "Running" && task.status === "Running"
                    ? {
                        ...task,
                        status: "Pending",
                        phase: "Assigned",
                      }
                    : task,
              ),
              activity: [
                {
                  id: Date.now() + Math.floor(Math.random() * 1000),
                  message: `Task "${
                    agent.tasks.find((task) => task.id === taskId)?.title ??
                    "Unknown"
                  }" changed to ${phase}.`,
                  createdAt: new Date().toISOString(),
                },
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  function advanceGlobalTask(ownerAgentId: number, task: AgentTask) {
    if (task.phase === "Assigned") {
      updateGlobalTask(
        ownerAgentId,
        task.id,
        "Running",
        "Specialist Work",
      );
      return;
    }

    if (task.phase === "Specialist Work") {
      updateGlobalTask(
        ownerAgentId,
        task.id,
        "Under Review",
        "Senior Review",
      );
      return;
    }

    if (task.phase === "Senior Review") {
      updateGlobalTask(
        ownerAgentId,
        task.id,
        "Under Review",
        "Team Leader Review",
      );
      return;
    }

    if (task.phase === "Team Leader Review") {
      updateGlobalTask(
        ownerAgentId,
        task.id,
        "Under Review",
        "Supervisor Approval",
      );
      return;
    }

    if (task.phase === "Supervisor Approval") {
      updateGlobalTask(
        ownerAgentId,
        task.id,
        "Completed",
        "Finished",
      );
    }
  }

  const summary = {
    total: allTasks.length,
    active: allTasks.filter(
      ({ task }) =>
        task.status === "Running" ||
        task.status === "Under Review",
    ).length,
    pending: allTasks.filter(
      ({ task }) => task.status === "Pending",
    ).length,
    blocked: allTasks.filter(
      ({ task }) => task.status === "Blocked",
    ).length,
  };

  function deleteGlobalTask(ownerAgentId: number, taskId: number) {
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === ownerAgentId
          ? {
              ...agent,
              tasks: agent.tasks.filter((task) => task.id !== taskId),
            }
          : agent,
      ),
    );
  }

  function clearFinishedTasks() {
    const shouldClear = window.confirm(
      "Delete all completed and failed tasks from every agent?",
    );

    if (!shouldClear) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) => ({
        ...agent,
        tasks: agent.tasks.filter(
          (task) =>
            task.status !== "Completed" && task.status !== "Failed",
        ),
      })),
    );
  }

  async function requestTaskApproval(agent: Agent, task: AgentTask) {
    const assessment = taskSafetyAssessment(task, agent, "strict");
    if (assessment.blockedReason) {
      window.alert(assessment.blockedReason);
      return;
    }
    let authorization: AuthorizationReadiness;
    try {
      authorization = await prepareBackendAuthorization(
        {
          kind: "runTask",
          agentId: agent.id,
          taskOwnerAgentId: agent.id,
          taskId: task.id,
          runMode: "execute",
        },
        setApprovalRequests,
      );
    } catch (error) {
      window.alert(errorMessage(error));
      return;
    }
    if (authorization.ready && !authorization.approval) {
      window.alert("Current backend policy allows this task without an approval record.");
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((currentAgent) =>
        currentAgent.id === agent.id
          ? {
              ...currentAgent,
              tasks: currentAgent.tasks.map((item) =>
                item.id === task.id
                  ? {
                      ...item,
                      status: authorization.ready ? "Pending" : "Blocked",
                      phase: authorization.ready ? "Assigned" : "Supervisor Approval",
                    }
                  : item,
              ),
              activity: [
                {
                  id: Date.now() + Math.floor(Math.random() * 1000),
                  message: `Backend authorization ${authorization.ready ? "is ready" : "was requested"} for task "${task.title}".`,
                  createdAt: new Date().toISOString(),
                },
                ...currentAgent.activity,
              ],
            }
          : currentAgent,
      ),
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">GLOBAL WORKFLOW</span>
          <h1>Tasks</h1>
          <p className="page-message">
            Track every task across every agent and review phase.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Total tasks</span>
          <strong>{summary.total}</strong>
          <small>Across all agents</small>
        </article>

        <article className="summary-card">
          <span>Active</span>
          <strong>{summary.active}</strong>
          <small>Working or under review</small>
        </article>

        <article className="summary-card">
          <span>Pending</span>
          <strong>{summary.pending}</strong>
          <small>Waiting to begin</small>
        </article>

        <article className="summary-card">
          <span>Blocked</span>
          <strong>{summary.blocked}</strong>
          <small>Needs intervention</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">FILTERS</span>
            <h2>Task pipeline</h2>
          </div>

          <button className="danger-button" onClick={clearFinishedTasks}>
            Clear finished tasks
          </button>
        </div>

        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(220px, 320px)",
            marginBottom: "16px",
          }}
        >
          <label className="form-field">
            <span>Finished-task retention</span>
            <select
              value={retentionDays}
              onChange={(event) =>
                setRetentionDays(
                  event.target.value === "never"
                    ? "never"
                    : (Number(event.target.value) as 7 | 30 | 90),
                )
              }
            >
              <option value={7}>Delete after 7 days</option>
              <option value={30}>Delete after 30 days</option>
              <option value={90}>Delete after 90 days</option>
              <option value="never">Never delete automatically</option>
            </select>
          </label>
        </div>

        <div
          style={{
            display: "grid",
            gridTemplateColumns:
              "repeat(auto-fit, minmax(220px, 1fr))",
            gap: "14px",
            marginBottom: "20px",
          }}
        >
          <label className="form-field">
            <span>Status</span>
            <select
              value={statusFilter}
              onChange={(event) =>
                setStatusFilter(
                  event.target.value as TaskStatus | "All",
                )
              }
            >
              <option value="All">All statuses</option>
              <option value="Pending">Pending</option>
              <option value="Running">Running</option>
              <option value="Blocked">Blocked</option>
              <option value="Under Review">Under Review</option>
              <option value="Completed">Completed</option>
              <option value="Failed">Failed</option>
            </select>
          </label>

          <label className="form-field">
            <span>Category</span>
            <select
              value={categoryFilter}
              onChange={(event) =>
                setCategoryFilter(
                  event.target.value as TaskCategory | "All",
                )
              }
            >
              <option value="All">All categories</option>
              <option value="Development">Development</option>
              <option value="Research">Research</option>
              <option value="Browsing">Browsing</option>
              <option value="Finance">Finance</option>
              <option value="Business">Business</option>
              <option value="Communication">Communication</option>
              <option value="System Control">System Control</option>
              <option value="General">General</option>
            </select>
          </label>
        </div>

        {filteredTasks.length === 0 ? (
          <p className="page-message">
            No tasks match the selected filters.
          </p>
        ) : (
          <div style={{ display: "grid", gap: "22px" }}>
            {phaseOrder.map((phase) => {
              const phaseTasks = filteredTasks.filter(
                ({ task }) => task.phase === phase,
              );

              if (phaseTasks.length === 0) {
                return null;
              }

              return (
                <div key={phase}>
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      marginBottom: "10px",
                    }}
                  >
                    <h3 style={{ margin: 0 }}>{phase}</h3>
                    <small>{phaseTasks.length} task(s)</small>
                  </div>

                  <div className="agent-list">
                    {phaseTasks.map(({ task, agent }) => (
                      <article
                        className="agent-card task-card"
                        key={`${agent.id}-${task.id}`}
                      >
                        <div className="task-card-content">
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "10px",
                              flexWrap: "wrap",
                              marginBottom: "8px",
                            }}
                          >
                            <h3 style={{ margin: 0 }}>
                              {task.title}
                            </h3>

                            <span
                              className={`agent-status ${
                                task.status === "Running"
                                  ? "working"
                                  : task.status === "Completed"
                                    ? "waiting"
                                    : task.status === "Failed"
                                      ? "paused"
                                      : "waiting"
                              }`}
                            >
                              {task.status}
                            </span>
                          </div>

                          <p>
                            {task.category} · {task.priority} priority
                          </p>
                          <small>
                            Assigned agent: {agent.name} · {agent.role}
                            {task.routingMode === "automatic"
                              ? " · Automatically routed"
                              : ""}
                            {task.reviewStatus !== "Not Requested"
                              ? ` · Review: ${task.reviewStatus}`
                              : ""}
                          </small>
                        </div>

                        <div className="task-card-actions">
                          {task.reviewStatus === "Not Requested" &&
                            task.phase !== "Finished" &&
                            task.phase !== "Failed" && (
                              <button
                                className="primary-button"
                                onClick={() =>
                                  advanceGlobalTask(agent.id, task)
                                }
                              >
                                {task.phase === "Assigned"
                                  ? "Start"
                                  : task.phase === "Specialist Work"
                                    ? "Send to Senior"
                                    : task.phase === "Senior Review"
                                      ? "Send to Team Leader"
                                      : task.phase ===
                                          "Team Leader Review"
                                        ? "Send to Supervisor"
                                        : "Approve"}
                              </button>
                            )}

                          {task.status !== "Blocked" &&
                            task.status !== "Completed" &&
                            task.status !== "Failed" && (
                              <button
                                className="secondary-button"
                                onClick={() =>
                                  updateGlobalTask(
                                    agent.id,
                                    task.id,
                                    "Blocked",
                                    task.phase,
                                  )
                                }
                              >
                                Block
                              </button>
                            )}

                          {task.status === "Blocked" && (
                            <button
                              className="secondary-button"
                              onClick={() =>
                                updateGlobalTask(
                                  agent.id,
                                  task.id,
                                  "Pending",
                                  "Assigned",
                                )
                              }
                            >
                              Unblock
                            </button>
                          )}

                          {task.status !== "Failed" &&
                            task.status !== "Completed" && (
                              <button
                                className="secondary-button"
                                onClick={() =>
                                  updateGlobalTask(
                                    agent.id,
                                    task.id,
                                    "Failed",
                                    "Failed",
                                  )
                                }
                              >
                                Fail
                              </button>
                            )}

                          {task.status !== "Completed" &&
                            task.status !== "Failed" && (
                              <button
                                className="secondary-button"
                                onClick={() =>
                                  requestTaskApproval(agent, task)
                                }
                              >
                                Request approval
                              </button>
                            )}

                          <button
                            className="danger-button"
                            onClick={() =>
                              deleteGlobalTask(agent.id, task.id)
                            }
                          >
                            Delete
                          </button>
                        </div>
                      </article>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}



function ApprovalsPage({
  agents,
  setAgents,
  approvalRequests,
  setApprovalRequests,
  workspaces,
  onOpenAgents,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  approvalRequests: ApprovalRequest[];
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  workspaces: WorkspaceDefinition[];
  onOpenAgents: () => void;
}) {
  const [statusFilter, setStatusFilter] =
    useState<ApprovalRequestStatus | "All">("Pending");
  const [resolutionError, setResolutionError] = useState("");

  const filteredRequests = approvalRequests.filter(
    (request) =>
      statusFilter === "All" || request.status === statusFilter,
  );

  const pendingCount = approvalRequests.filter(
    (request) => request.status === "Pending",
  ).length;
  const approvedCount = approvalRequests.filter(
    (request) => request.status === "Approved",
  ).length;
  const deniedCount = approvalRequests.filter(
    (request) => request.status === "Denied",
  ).length;
  const expiredCount = approvalRequests.filter(
    (request) => request.status === "Expired",
  ).length;

  async function resolveApproval(
    requestId: number,
    status: "Approved" | "Denied",
  ) {
    const request = approvalRequests.find((item) => item.id === requestId);
    if (!request) {
      return;
    }
    setResolutionError("");
    let resolved: ApprovalRequest;
    try {
      resolved = await invoke<ApprovalRequest>("resolve_approval", {
        request: {
          approvalId: requestId,
          resolution: status === "Approved" ? "approve" : "deny",
        },
      });
    } catch (error) {
      setResolutionError(errorMessage(error));
      return;
    }
    setApprovalRequests((currentRequests) =>
      upsertApprovalRequest(currentRequests, resolved),
    );
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === request.agentId
          ? {
              ...agent,
              tasks: agent.tasks.map((task) =>
                task.id === request.taskId
                  ? {
                      ...task,
                      status: status === "Approved" ? "Pending" : "Blocked",
                      phase:
                        status === "Approved"
                          ? "Assigned"
                          : "Supervisor Approval",
                      completedAt: null,
                    }
                  : task,
              ),
              activity: [
                {
                  id: Date.now() + Math.floor(Math.random() * 1000),
                  message: `${status === "Approved" ? "Approved" : "Denied"} one-time authorization for "${request.taskSnapshot || request.title}".`,
                  createdAt: new Date().toISOString(),
                },
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">SAFETY GATE</span>
          <h1>Approvals</h1>
          <p className="page-message">
            Review and resolve actions that require human authorization.
            Approval history is managed by the backend.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Pending</span>
          <strong>{pendingCount}</strong>
          <small>Needs a decision</small>
        </article>

        <article className="summary-card">
          <span>Approved</span>
          <strong>{approvedCount}</strong>
          <small>Authorized requests</small>
        </article>

        <article className="summary-card">
          <span>Denied</span>
          <strong>{deniedCount}</strong>
          <small>Rejected requests</small>
        </article>

        <article className="summary-card">
          <span>Expired</span>
          <strong>{expiredCount}</strong>
          <small>Authorization window closed</small>
        </article>
      </section>

      <section className="panel">
        {resolutionError && (
          <p className="page-message" role="alert">{resolutionError}</p>
        )}
        <div className="panel-heading">
          <div>
            <span className="eyebrow">REQUEST QUEUE</span>
            <h2>Approval requests</h2>
          </div>
        </div>

        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(220px, 320px)",
            marginBottom: "20px",
          }}
        >
          <label className="form-field">
            <span>Status</span>
            <select
              value={statusFilter}
              onChange={(event) =>
                setStatusFilter(
                  event.target.value as
                    | ApprovalRequestStatus
                    | "All",
                )
              }
            >
              <option value="Pending">Pending</option>
              <option value="Approved">Approved</option>
              <option value="Denied">Denied</option>
              <option value="Expired">Expired</option>
              <option value="All">All requests</option>
            </select>
          </label>
        </div>

        {filteredRequests.length === 0 ? (
          <p className="page-message">
            No approval requests match this filter.
          </p>
        ) : (
          <div className="agent-list">
            {filteredRequests.map((request) => {
              const agent =
                agents.find(
                  (item) => item.id === request.agentId,
                ) ?? null;

              const task =
                agent?.tasks.find(
                  (item) => item.id === request.taskId,
                ) ?? null;

              return (
                <article
                  className="agent-card"
                  key={request.id}
                >
                  <div style={{ flex: 1 }}>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "10px",
                        flexWrap: "wrap",
                        marginBottom: "8px",
                      }}
                    >
                      <h3 style={{ margin: 0 }}>
                        {request.title}
                      </h3>

                      <span
                        className={`agent-status ${
                          request.status === "Approved"
                            ? "working"
                            : request.status === "Denied" ||
                                request.status === "Expired"
                              ? "paused"
                              : "waiting"
                        }`}
                      >
                        {request.status}
                      </span>
                    </div>

                    <p>{request.reason}</p>
                    <div className="approval-detail-grid">
                      <span>
                        <strong>Risk</strong>
                        {request.riskLevel}
                      </span>
                      <span>
                        <strong>Permission</strong>
                        {request.scopes.length > 0
                          ? request.scopes
                              .map((scope) => safetyScopeLabels[scope])
                              .join(", ")
                          : "Manual review"}
                      </span>
                      <span>
                        <strong>Workspace</strong>
                        {workspaces.find(
                          (workspace) => workspace.id === request.workspaceId,
                        )?.name ?? "Unknown"}
                      </span>
                    </div>
                    <small>
                      Agent: {agent?.name ?? "Unknown"} · Role:{" "}
                      {agent?.role ?? "Unknown"}
                      {task
                        ? ` · Task phase: ${task.phase}`
                        : ""}
                    </small>
                    <br />
                    <small>
                      Requested:{" "}
                      {new Date(
                        request.createdAt,
                      ).toLocaleString()}
                    </small>
                    <br />
                    <small>
                      Expires: {new Date(request.expiresAt).toLocaleString()}
                      {request.consumedAt
                        ? ` · Used: ${new Date(request.consumedAt).toLocaleString()}`
                        : " · One run only"}
                    </small>
                  </div>

                  <div
                    style={{
                      display: "flex",
                      gap: "8px",
                      flexWrap: "wrap",
                      justifyContent: "flex-end",
                    }}
                  >
                    {request.status === "Pending" && (
                      <>
                        <button
                          className="primary-button"
                          onClick={() =>
                            resolveApproval(
                              request.id,
                              "Approved",
                            )
                          }
                        >
                          Approve
                        </button>

                        <button
                          className="danger-button"
                          onClick={() =>
                            resolveApproval(
                              request.id,
                              "Denied",
                            )
                          }
                        >
                          Deny
                        </button>
                      </>
                    )}

                    {request.status === "Approved" &&
                      request.consumedAt === null && (
                        <button
                          className="secondary-button"
                          onClick={onOpenAgents}
                        >
                          Open agent
                        </button>
                      )}
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}

function ActivityPage({
  agents,
  setAgents,
  retentionDays,
  setRetentionDays,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  retentionDays: HistoryRetentionDays;
  setRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
}) {
  const allActivity = agents
    .flatMap((agent) =>
      agent.activity.map((entry) => ({
        ...entry,
        agentId: agent.id,
        agentName: agent.name,
        agentRole: agent.role,
      })),
    )
    .sort(
      (a, b) =>
        new Date(b.createdAt).getTime() -
        new Date(a.createdAt).getTime(),
    );

  const activeAgents = agents.filter((agent) =>
    agent.tasks.some(
      (task) =>
        task.status === "Running" ||
        task.status === "Under Review",
    ),
  );

  const blockedAgents = agents.filter((agent) =>
    agent.tasks.some((task) => task.status === "Blocked"),
  );

  const recentlyFinished = agents
    .flatMap((agent) =>
      agent.tasks
        .filter(
          (task) =>
            task.status === "Completed" ||
            task.status === "Failed",
        )
        .map((task) => ({
          agent,
          task,
        })),
    )
    .slice(0, 8);

  const nextAgents = agents.filter((agent) =>
    agent.tasks.some((task) => task.status === "Pending"),
  );

  function deleteActivityEntry(agentId: number, entryId: number) {
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === agentId
          ? {
              ...agent,
              activity: agent.activity.filter(
                (entry) => entry.id !== entryId,
              ),
            }
          : agent,
      ),
    );
  }

  function clearAllActivity() {
    const shouldClear = window.confirm(
      "Delete all recorded activity from every agent?",
    );

    if (!shouldClear) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) => ({
        ...agent,
        activity: [],
      })),
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">SYSTEM ACTIVITY</span>
          <h1>Activity</h1>
          <p className="page-message">
            Monitor active agents, workflow progress, and recent events.
          </p>
        </div>

        <button className="danger-button" onClick={clearAllActivity}>
          Clear activity history
        </button>
      </header>

      <section className="panel">
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(220px, 320px)",
          }}
        >
          <label className="form-field">
            <span>Activity retention</span>
            <select
              value={retentionDays}
              onChange={(event) =>
                setRetentionDays(
                  event.target.value === "never"
                    ? "never"
                    : (Number(event.target.value) as 7 | 30 | 90),
                )
              }
            >
              <option value={7}>Delete after 7 days</option>
              <option value={30}>Delete after 30 days</option>
              <option value={90}>Delete after 90 days</option>
              <option value="never">Never delete automatically</option>
            </select>
          </label>
        </div>
      </section>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Active agents</span>
          <strong>{activeAgents.length}</strong>
          <small>Working or reviewing</small>
        </article>

        <article className="summary-card">
          <span>Waiting next</span>
          <strong>{nextAgents.length}</strong>
          <small>Agents with pending work</small>
        </article>

        <article className="summary-card">
          <span>Blocked agents</span>
          <strong>{blockedAgents.length}</strong>
          <small>Needs intervention</small>
        </article>

        <article className="summary-card">
          <span>Activity events</span>
          <strong>{allActivity.length}</strong>
          <small>Recorded locally</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">CURRENTLY ACTIVE</span>
            <h2>Agents in progress</h2>
          </div>
        </div>

        {activeAgents.length === 0 ? (
          <p className="page-message">
            No agents are currently working or reviewing.
          </p>
        ) : (
          <div className="agent-list">
            {activeAgents.map((agent) => {
              const activeTask =
                agent.tasks.find(
                  (task) => task.status === "Running",
                ) ??
                agent.tasks.find(
                  (task) => task.status === "Under Review",
                ) ??
                null;

              return (
                <article className="agent-card" key={agent.id}>
                  <div>
                    <h3>{agent.name}</h3>
                    <p>
                      {activeTask
                        ? activeTask.title
                        : "Active without assigned task"}
                    </p>
                    <small>
                      {agent.role} ·{" "}
                      {activeTask
                        ? `${activeTask.phase} · ${activeTask.category}`
                        : agent.category}
                    </small>
                  </div>

                  <span
                    className={`agent-status ${agent.status.toLowerCase()}`}
                  >
                    {agent.status}
                  </span>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">WAITING NEXT</span>
            <h2>Queued agents</h2>
          </div>
        </div>

        {nextAgents.length === 0 ? (
          <p className="page-message">
            No agents are waiting for pending work.
          </p>
        ) : (
          <div className="agent-list">
            {nextAgents.map((agent) => {
              const nextTask =
                agent.tasks.find(
                  (task) => task.status === "Pending",
                ) ?? null;

              return (
                <article className="agent-card" key={agent.id}>
                  <div>
                    <h3>{agent.name}</h3>
                    <p>
                      {nextTask
                        ? nextTask.title
                        : "Pending work available"}
                    </p>
                    <small>
                      {agent.role}
                      {nextTask
                        ? ` · Next phase: ${nextTask.phase}`
                        : ""}
                    </small>
                  </div>

                  <span className="agent-status waiting">
                    Waiting
                  </span>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">RECENT OUTCOMES</span>
            <h2>Finished and failed work</h2>
          </div>
        </div>

        {recentlyFinished.length === 0 ? (
          <p className="page-message">
            No completed or failed tasks yet.
          </p>
        ) : (
          <div className="agent-list">
            {recentlyFinished.map(({ agent, task }) => (
              <article
                className="agent-card"
                key={`${agent.id}-${task.id}`}
              >
                <div>
                  <h3>{task.title}</h3>
                  <p>
                    {agent.name} · {agent.role}
                  </p>
                  <small>
                    {task.category} · {task.phase}
                  </small>
                </div>

                <span
                  className={`agent-status ${
                    task.status === "Completed"
                      ? "waiting"
                      : "paused"
                  }`}
                >
                  {task.status}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">TIMELINE</span>
            <h2>System audit trail</h2>
          </div>
        </div>

        {allActivity.length === 0 ? (
          <p className="page-message">
            No activity has been recorded yet.
          </p>
        ) : (
          <div className="agent-list">
            {allActivity.map((entry) => (
              <article
                className="agent-card"
                key={`${entry.agentId}-${entry.id}`}
              >
                <div>
                  <h3>{entry.message}</h3>
                  <p>
                    {entry.agentName} · {entry.agentRole}
                  </p>
                  <small>
                    {new Date(entry.createdAt).toLocaleString()}
                  </small>
                </div>

                <button
                  className="danger-button"
                  onClick={() =>
                    deleteActivityEntry(entry.agentId, entry.id)
                  }
                >
                  Delete
                </button>
              </article>
            ))}
          </div>
        )}
      </section>
    </>
  );
}

function VoiceControlPage({
  agents,
  setAgents,
  setApprovalRequests,
  preferences,
  setPreferences,
  visible = true,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  preferences: AppPreferences;
  setPreferences: React.Dispatch<React.SetStateAction<AppPreferences>>;
  visible?: boolean;
}) {
  const [command, setCommand] = useState("");
  const [message, setMessage] = useState("");
  const [isListening, setIsListening] = useState(false);
  const [pendingApplication, setPendingApplication] = useState<string | null>(null);
  const [voiceRuntime, setVoiceRuntime] = useState<VoiceRuntimeStatus | null>(null);
  const [voiceState, setVoiceState] = useState<VoiceState>(
    preferences.voiceControlMasterEnabled ? preferences.voiceState : "VOICE_OFF",
  );
  const [voiceUiState, setVoiceUiState] = useState<VoiceUiState>(
    preferences.voiceControlMasterEnabled ? "PASSIVE" : "VOICE OFF",
  );
  const [desktopControl, setDesktopControl] = useState<DesktopControlStatus | null>(null);
  const submitCommandRef = useRef<(value: string) => void>(() => {});
  const desktopRestoreAttempted = useRef(false);
  const pcAgent = agents.find((agent) => agent.name === "PC Control Agent") ?? null;
  const codingAgent = agents.find((agent) => agent.name === "Coding Agent") ?? null;
  const appAliases: Record<string, { key: string; label: string }> = {
    firefox: { key: "firefox", label: "Firefox" },
    dolphin: { key: "dolphin", label: "Dolphin" },
    "system settings": { key: "system-settings", label: "System Settings" },
    settings: { key: "system-settings", label: "System Settings" },
    terminal: { key: "terminal", label: "Terminal" },
    code: { key: "code", label: "Visual Studio Code" },
    "visual studio code": { key: "code", label: "Visual Studio Code" },
  };
  const pointerActions: Record<string, { key: string; label: string }> = {
    "move left": { key: "move-left", label: "move left" },
    "move right": { key: "move-right", label: "move right" },
    "move up": { key: "move-up", label: "move up" },
    "move down": { key: "move-down", label: "move down" },
    click: { key: "click", label: "click" },
    "double click": { key: "double-click", label: "double click" },
    "scroll up": { key: "scroll-up", label: "scroll up" },
    "scroll down": { key: "scroll-down", label: "scroll down" },
  };
  const desktopActionLabels: Record<string, string> = {
    "open-launcher": "open the application launcher",
    "volume-up": "increase volume",
    "volume-down": "decrease volume",
    "toggle-mute": "toggle mute",
    "minimize-window": "minimize the focused window",
    "maximize-window": "maximize the focused window",
    "restore-window": "restore the focused window",
    "next-window": "switch to the next window",
    "previous-window": "switch to the previous window",
    "snap-left": "snap the window left",
    "snap-right": "snap the window right",
    left: "move left",
    right: "move right",
    up: "move up",
    down: "move down",
    home: "go to the start",
    end: "go to the end",
    "page-up": "page up",
    "page-down": "page down",
    tab: "press Tab",
    "shift-tab": "press Shift+Tab",
    enter: "press Enter",
    escape: "press Escape",
    backspace: "press Backspace",
    delete: "press Delete",
    "select-all": "select all",
    copy: "copy",
    cut: "cut",
    paste: "paste",
    undo: "undo",
    redo: "redo",
  };
  const phraseList = (value: string) => value.split(",").map((phrase) => phrase.trim().toLowerCase()).filter(Boolean);
  const openPhrases = phraseList(preferences.voiceOpenPhrases);
  const closePhrases = phraseList(preferences.voiceClosePhrases);

  async function authorizeVoiceAction(
    intent: BackendActionIntent,
  ): Promise<AuthorizationReadiness | null> {
    try {
      const authorization = await prepareBackendAuthorization(
        intent,
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setMessage(
          "This action is waiting for trusted backend authorization. Open Approvals to approve or deny it.",
        );
        setVoiceUiState("PROCESSING");
        return null;
      }
      return authorization;
    } catch (error) {
      setMessage(errorMessage(error));
      setVoiceUiState("ERROR");
      return null;
    }
  }

  function createCodingTask(request: string) {
    if (!codingAgent) {
      setMessage("Lucy cannot route work because the Coding Agent is missing.");
      return;
    }

    const task: AgentTask = {
      id: Date.now(),
      title: request,
      category: "Development",
      priority: preferences.defaultTaskPriority,
      assignedAgentId: codingAgent.id,
      status: "Pending",
      phase: "Assigned",
      createdAt: new Date().toISOString(),
      completedAt: null,
      result: null,
      responseId: null,
      runtimeModel: null,
      totalTokens: null,
      workspaceId: preferences.activeWorkspaceId,
      changedFiles: [],
      diff: null,
      durationSeconds: null,
      routingMode: "selected",
      routedFromAgentId: pcAgent?.id ?? null,
      routingReason: "Created by Lucy voice control.",
      reviewAgentId: null,
      reviewStatus: "Not Requested",
      reviewResult: null,
      reviewModel: null,
      reviewDurationSeconds: null,
      reviewedAt: null,
    };
    setAgents((current) =>
      current.map((agent) =>
        agent.id === codingAgent.id
          ? {
              ...agent,
              tasks: [...agent.tasks, task],
              activity: [
                {
                  id: Date.now() + 1,
                  message: `Lucy created a coding task: ${request}`,
                  createdAt: new Date().toISOString(),
                },
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
    setMessage(`Lucy queued a coding task for ${codingAgent.name}.`);
  }

  async function launchApplication(application: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can open an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== application.key) {
      setPendingApplication(application.key);
      setMessage(`Confirm opening ${application.label}. This is a one-time minor system action.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "launchAllowedApplication",
        agentId: pcAgent.id,
        application: application.key,
      });
      if (!authorization) return;
      await invoke("launch_allowed_application", {
        agentId: pcAgent.id,
        application: application.key,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Opened ${application.label}.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function closeApplication(application: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can close an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    const approvalKey = `close:${application.key}`;
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== approvalKey) {
      setPendingApplication(approvalKey);
      setMessage(`Confirm closing ${application.label}. This may ask the application to save unsaved work.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Closing desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "closeAllowedApplication",
        agentId: pcAgent.id,
        application: application.key,
      });
      if (!authorization) return;
      await invoke("close_allowed_application", {
        agentId: pcAgent.id,
        application: application.key,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Requested that ${application.label} close.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function sendPointerAction(action: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice pointer control requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    const approvalKey = `pointer:${action.key}`;
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== approvalKey) {
      setPendingApplication(approvalKey);
      setMessage(`Confirm voice pointer action: ${action.label}.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice pointer control is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopPointer",
        agentId: pcAgent.id,
        action: action.key,
      });
      if (!authorization) return;
      await invoke("send_desktop_pointer_action", {
        agentId: pcAgent.id,
        action: action.key,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Voice pointer: ${action.label}.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function sendDesktopKeyboardAction(action: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice keyboard and volume controls require Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice keyboard controls are available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopKeyboard",
        agentId: pcAgent.id,
        action,
      });
      if (!authorization) return;
      await invoke("send_desktop_keyboard_action", { agentId: pcAgent.id, action });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Requested: ${desktopActionLabels[action] ?? action}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function controlNamedDesktopWindow(application: string, action: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Named application window controls require Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Named application window controls are available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopWindow",
        agentId: pcAgent.id,
        application,
        action,
      });
      if (!authorization) return;
      await invoke("control_named_desktop_window", {
        agentId: pcAgent.id,
        application,
        action,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Requested ${action} for the existing ${application} window.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function typeDesktopText(text: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice typing requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice typing is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "typeDesktopText",
        agentId: pcAgent.id,
        text,
      });
      if (!authorization) return;
      await invoke("type_desktop_text", { agentId: pcAgent.id, text });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage("Typed dictated text into the focused application.");
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function launchDesktopApplication(application: string) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can open an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "launchDesktopApplication",
        agentId: pcAgent.id,
        application,
      });
      if (!authorization) return;
      await invoke("launch_desktop_application", {
        agentId: pcAgent.id,
        application,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Opened ${application}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function openStandardFolder(folder: string) {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening folders is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "openStandardFolder",
        agentId: pcAgent.id,
        folder,
      });
      if (!authorization) return;
      await invoke("open_standard_folder", { agentId: pcAgent.id, folder });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Opened ${folder}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function closeActiveDesktopApplication() {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Closing the active application by voice requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Closing the active desktop application is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "closeActiveApplication",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await invoke("close_active_desktop_application", { agentId: pcAgent.id });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Requested that the active application close.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  function submitCommand(value = command) {
    const understood = interpretVoiceCommand(value, {
      openPhrases,
      closePhrases,
      replacements: preferences.voiceCommandReplacements,
    });
    setVoiceUiState("PROCESSING");
    setMessage(`Processing: ${understood.transcript}`);
    if (understood.intent === "coding_request") {
      createCodingTask(understood.entity);
      return;
    }
    if (understood.intent === "open_folder") {
      setVoiceUiState("EXECUTING");
      void openStandardFolder(understood.entity);
      return;
    }
    if (understood.intent === "open_application") {
      setVoiceUiState("EXECUTING");
      const application = appAliases[understood.entity];
      void (application ? launchApplication(application) : launchDesktopApplication(understood.entity));
      return;
    }
    if (understood.intent === "close_application") {
      setVoiceUiState("EXECUTING");
      const application = appAliases[understood.entity];
      void (application && application.key !== "terminal" ? closeApplication(application) : closeActiveDesktopApplication());
      return;
    }
    if (understood.intent === "pointer_action") {
      setVoiceUiState("EXECUTING");
      const action = Object.values(pointerActions).find((candidate) => candidate.key === understood.entity);
      if (action) void sendPointerAction(action);
      return;
    }
    if (understood.intent === "desktop_action") {
      setVoiceUiState("EXECUTING");
      void sendDesktopKeyboardAction(understood.entity);
      return;
    }
    if (understood.intent === "application_window_action") {
      setVoiceUiState("EXECUTING");
      void controlNamedDesktopWindow(understood.entity, understood.action ?? "restore");
      return;
    }
    if (understood.intent === "text_input") {
      setVoiceUiState("EXECUTING");
      void typeDesktopText(understood.entity);
      return;
    }
    setVoiceUiState("ERROR");
    setMessage("Lucy could not map that phrase to a supported, safe command.");
  }

  submitCommandRef.current = submitCommand;

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let active = true;
    void invoke<VoiceRuntimeStatus>("voice_runtime_status")
      .then((status) => {
        if (active) setVoiceRuntime(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    void invoke<DesktopControlStatus>("desktop_control_status")
      .then((status) => {
        if (active) setDesktopControl(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    const unlistenStatus = listen<VoiceRuntimeStatus>("voice-runtime-status", (event) => {
      setVoiceRuntime(event.payload);
      setIsListening(event.payload.listening);
      setMessage(event.payload.message);
    });
    const unlistenTranscript = listen<VoiceTranscriptEvent>("voice-transcript", (event) => {
      const { kind, transcript } = event.payload;
      if (kind === "activated") {
        setVoiceState("VOICE_ACTIVE");
        setVoiceUiState("LISTENING");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_ACTIVE" }));
        setMessage(`Lucy is active. Say ${preferences.voiceDeactivatePhrase} to return to wake-only mode.`);
        return;
      }
      if (kind === "deactivated") {
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
        setMessage(`Lucy command mode is off. Say ${preferences.voiceWakePhrase} when you want to control your PC.`);
        return;
      }
      if (kind === "off_requested") {
        setVoiceState("VOICE_OFF");
        setVoiceUiState("VOICE OFF");
        setIsListening(false);
        setPreferences((current) => ({
          ...current,
          voiceControlMasterEnabled: false,
          voiceState: "VOICE_OFF",
        }));
        void invoke("stop_voice_listener")
          .then(() => setMessage("Lucy voice control is off. Re-enable it from Voice Control to start listening again."))
          .catch((error) => setMessage(errorMessage(error)));
        return;
      }
      if (kind === "listening") {
        setVoiceUiState("LISTENING");
        setMessage("Lucy is listening for your command.");
        return;
      }
      if (kind === "ready") {
        setIsListening(true);
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
        setMessage(`Lucy wake listener is active. Say ${preferences.voiceWakePhrase} to begin giving commands.`);
        return;
      }
      if (kind === "error") {
        setIsListening(false);
        setVoiceUiState("ERROR");
        setMessage(transcript);
        return;
      }
      if (kind === "heard") {
        setVoiceUiState("LISTENING");
        setMessage(`Listening: ${transcript}`);
        return;
      }
      if (!transcript.trim()) return;
      setCommand(transcript);
      setMessage(`Executing: ${transcript}`);
      submitCommandRef.current(transcript);
    });
    return () => {
      active = false;
      void unlistenStatus.then((unlisten) => unlisten());
      void unlistenTranscript.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!isDesktopRuntime() || !preferences.backgroundVoiceEnabled || !preferences.voiceControlMasterEnabled) return;
    if (pcAgent) {
      void startListening();
    }
  }, [preferences.backgroundVoiceEnabled, preferences.voiceControlMasterEnabled, preferences.voiceWakePhrase]);

  function toggleBackgroundVoice() {
    const nextEnabled = !preferences.backgroundVoiceEnabled;
    setPreferences((current) => ({ ...current, backgroundVoiceEnabled: nextEnabled }));
    if (!nextEnabled) {
      void invoke("stop_voice_listener")
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({ ...current, voiceState: "VOICE_OFF" }));
          setMessage("Background voice mode is off. Manual in-app listening is still available.");
        })
        .catch((error) => setMessage(errorMessage(error)));
    }
  }

  function toggleLucyMaster() {
    const nextEnabled = !preferences.voiceControlMasterEnabled;
    setPreferences((current) => ({ ...current, voiceControlMasterEnabled: nextEnabled }));
    if (!nextEnabled) {
      void invoke("stop_voice_listener")
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({ ...current, voiceState: "VOICE_OFF" }));
          setMessage("Lucy is completely disabled. No microphone audio is being captured.");
        })
        .catch((error) => setMessage(errorMessage(error)));
      } else {
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
    }
  }

  function updateVoicePreference(key: "voiceWakePhrase" | "voiceDeactivatePhrase" | "voiceOpenPhrases" | "voiceClosePhrases" | "voiceCommandReplacements", value: string) {
    setPreferences((current) => ({ ...current, [key]: value.toLowerCase() }));
  }

  async function installOfflineVoice() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "installVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await invoke("install_voice_runtime", { agentId: pcAgent.id });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Downloading the local speech model. Keep the app open until installation finishes.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function installHighAccuracyVoice() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "installHighAccuracyVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await invoke("install_high_accuracy_voice_runtime", { agentId: pcAgent.id });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Building the high-accuracy speech engine and downloading its local model. This can take several minutes.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function enableDesktopControl() {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Set PC Control to Full system permission before enabling desktop input.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "enableDesktopControl",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      const status = await invoke<DesktopControlStatus>("enable_desktop_control", {
        agentId: pcAgent.id,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setDesktopControl(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  useEffect(() => {
    if (!isDesktopRuntime() || pcAgent?.capabilities.system !== "full") {
      desktopRestoreAttempted.current = false;
      return;
    }
    if (desktopControl?.enabled || desktopRestoreAttempted.current) return;
    desktopRestoreAttempted.current = true;
    void enableDesktopControl();
  }, [desktopControl?.enabled, pcAgent?.capabilities.system]);

  function activateFullSystemPermission() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    if (pcAgent.capabilities.system === "full") {
      void enableDesktopControl();
      return;
    }
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === pcAgent.id
          ? {
              ...agent,
              capabilities: { ...agent.capabilities, system: "full" },
              approvals: { ...agent.approvals, system: "allow" },
              activity: [
                {
                  id: Date.now(),
                  message: "Full system permission enabled from Voice Control.",
                  createdAt: new Date().toISOString(),
                },
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
    setMessage("Full system permission is active. Requesting KDE desktop input permission...");
    void enableDesktopControl();
  }

  function startListening() {
    if (isDesktopRuntime()) {
      if (!pcAgent) {
        setMessage("PC Control Agent is unavailable.");
        return;
      }
      void authorizeVoiceAction({
        kind: "startVoiceListener",
        agentId: pcAgent.id,
      }).then((authorization) => {
        if (!authorization) return;
        void invoke("start_voice_listener", { agentId: pcAgent.id })
          .then(() => {
            markApprovalConsumed(setApprovalRequests, authorization.approval);
            setVoiceUiState("PROCESSING");
            setMessage("Starting Lucy's microphone listener...");
          })
          .catch((error) => {
            setIsListening(false);
            setVoiceUiState("ERROR");
            setMessage(errorMessage(error));
          });
      });
      return;
    }
    const speechWindow = window as typeof window & {
      SpeechRecognition?: SpeechRecognitionConstructor;
      webkitSpeechRecognition?: SpeechRecognitionConstructor;
    };
    const Recognition = speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition;
    if (!Recognition) {
      setMessage("Speech recognition is not available in this webview. Type a command instead.");
      return;
    }
    const recognition = new Recognition();
    recognition.lang = "en-US";
    recognition.continuous = false;
    recognition.interimResults = false;
    recognition.onresult = (event) => {
      const transcript = event.results[0]?.[0]?.transcript ?? "";
      setCommand(transcript);
      submitCommand(transcript);
    };
    recognition.onerror = () => setMessage("Voice recognition could not understand that command. Try again or type it.");
    recognition.onend = () => setIsListening(false);
    setIsListening(true);
    recognition.start();
  }

  function stopListening() {
    if (isDesktopRuntime()) {
      void invoke("stop_voice_listener")
        .then(() => setIsListening(false))
        .catch((error) => setMessage(errorMessage(error)));
      return;
    }
    setIsListening(false);
  }

  const permissionLabel = pcAgent?.capabilities.system === "full"
    ? "Full system permission"
    : pcAgent?.capabilities.system === "power"
      ? "Elevated system permission"
      : pcAgent?.capabilities.system === "notifications"
        ? "Minor system permission"
        : "No system permission";

  return (
    <div hidden={!visible}>
      <header className="topbar">
        <div>
          <span className="eyebrow">VOICE AND TEXT COMMANDS</span>
          <h1>Voice Control</h1>
          <p className="page-message">Say an approved app name directly, or use Lucy to create a coding task.</p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card"><span>PC Control</span><strong>{pcAgent?.status ?? "Unavailable"}</strong><small>{permissionLabel}{pcAgent?.capabilities.system === "full" ? " active" : ""}</small></article>
        <article className="summary-card"><span>Approval policy</span><strong>{pcAgent?.approvals.system ?? "deny"}</strong><small>System action authorization</small></article>
        <article className="summary-card"><span>Lucy</span><strong>{codingAgent ? "Ready" : "Unavailable"}</strong><small>Routes coding requests to {codingAgent?.name ?? "no agent"}</small></article>
        <article className="summary-card"><span>Voice state</span><strong>{voiceState.replace("VOICE_", "")}</strong><small>{voiceState === "VOICE_OFF" ? "Microphone disabled" : voiceState === "VOICE_PASSIVE" ? `Waiting for ${preferences.voiceWakePhrase}` : "Accepting commands"}</small></article>
        <article className="summary-card"><span>Command status</span><strong>{voiceUiState}</strong><small>Voice command lifecycle</small></article>
      </section>

      {isDesktopRuntime() && (
        <section className="settings-note">
          Closing the window keeps the control center running in the system tray. Offline voice uses your microphone through PipeWire and processes speech locally; it does not use the webview speech API.
        </section>
      )}

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">COMMAND CENTER</span>
            <h2>Speak or type a request</h2>
          </div>
          <div className="button-row">
            {isDesktopRuntime() && !voiceRuntime?.installed && <button className="secondary-button" onClick={() => void installOfflineVoice()}>Install offline voice engine</button>}
            {isDesktopRuntime() && voiceRuntime?.installed && !voiceRuntime.highAccuracyAvailable && <button className="secondary-button" onClick={() => void installHighAccuracyVoice()}>Install high-accuracy voice</button>}
            {isDesktopRuntime() && <button className="secondary-button" onClick={toggleBackgroundVoice}>{preferences.backgroundVoiceEnabled ? "Disable wake listener" : "Enable wake listener"}</button>}
            {isDesktopRuntime() && <button className="secondary-button" onClick={toggleLucyMaster}>{preferences.voiceControlMasterEnabled ? "Disable Lucy completely" : "Enable Lucy"}</button>}
            <button className="primary-button microphone-button" onClick={isListening ? stopListening : startListening} disabled={!preferences.voiceControlMasterEnabled}>
              <span className={`microphone-indicator ${voiceState === "VOICE_ACTIVE" ? "is-active" : isListening ? "is-passive" : "is-off"}`} aria-hidden="true" />
              {isListening ? "Stop listening" : "Listen"}
            </button>
          </div>
        </div>
        <div className="model-composer">
          <label className="form-field">
            <span>Command</span>
            <input value={command} onChange={(event) => setCommand(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") submitCommand(); }} placeholder="Firefox, open Firefox, or Lucy fix the build error" />
          </label>
          <button className="primary-button" onClick={() => submitCommand()}>Run command</button>
        </div>
        {message && <div className="runtime-message">{message}</div>}
        {isDesktopRuntime() && desktopControl && <p className="form-hint">{desktopControl.message}</p>}
        {isDesktopRuntime() && !desktopControl?.enabled && <button className="secondary-button" onClick={() => void enableDesktopControl()}>Enable KDE desktop input</button>}
        {isDesktopRuntime() && voiceRuntime && <p className="form-hint">{voiceRuntime.message}{voiceRuntime.highAccuracyAvailable ? " Whisper base.en transcribes commands after Lucy wakes." : " Install high-accuracy voice for a broader command vocabulary."}{preferences.backgroundVoiceEnabled && preferences.voiceControlMasterEnabled ? " Lucy waits for its wake phrase while this app is in the tray." : ""}</p>}
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">LUCY PHRASES</span><h2>Wake and command vocabulary</h2></div></div>
        <div className="form-grid">
          <label className="form-field"><span>Wake phrase</span><input value={preferences.voiceWakePhrase} onChange={(event) => updateVoicePreference("voiceWakePhrase", event.target.value)} placeholder="lucy activate" /></label>
          <label className="form-field"><span>Deactivate phrase</span><input value={preferences.voiceDeactivatePhrase} onChange={(event) => updateVoicePreference("voiceDeactivatePhrase", event.target.value)} placeholder="lucy deactivate" /></label>
          <label className="form-field"><span>Open phrases</span><input value={preferences.voiceOpenPhrases} onChange={(event) => updateVoicePreference("voiceOpenPhrases", event.target.value)} placeholder="open, launch, start" /></label>
          <label className="form-field"><span>Close phrases</span><input value={preferences.voiceClosePhrases} onChange={(event) => updateVoicePreference("voiceClosePhrases", event.target.value)} placeholder="close, quit, exit" /></label>
          <label className="form-field"><span>Recognition replacements</span><textarea value={preferences.voiceCommandReplacements} onChange={(event) => updateVoicePreference("voiceCommandReplacements", event.target.value)} placeholder="fire fox = firefox" /></label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">PERMISSION LADDER</span><h2>System-control boundaries</h2></div></div>
        <div className="agent-list">
          <article className="agent-card"><div><h3>None</h3><p>No desktop actions are accepted.</p></div></article>
          <article className="agent-card"><div><h3>Minor</h3><p>Allowlisted application launches only. Firefox and “open Firefox” are equivalent.</p></div></article>
          <article className="agent-card"><div><h3>Elevated</h3><p>Reserved for future confirmed power actions. It does not enable arbitrary commands.</p></div></article>
          <article className="agent-card">
            <div>
              <h3>Full {pcAgent?.capabilities.system === "full" ? "- Active" : ""}</h3>
              <p>Enables KDE desktop pointer and keyboard permission. Administrator commands remain blocked.</p>
              <button className="secondary-button" onClick={activateFullSystemPermission} disabled={!pcAgent}>
                {pcAgent?.capabilities.system === "full"
                  ? desktopControl?.enabled
                    ? "Full control active"
                    : "Restore KDE desktop input"
                  : "Enable full control"}
              </button>
            </div>
          </article>
        </div>
      </section>
    </div>
  );
}

function RemindersPage({
  agents,
  reminders,
  setReminders,
}: {
  agents: Agent[];
  reminders: Reminder[];
  setReminders: React.Dispatch<React.SetStateAction<Reminder[]>>;
}) {
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [dueAt, setDueAt] = useState(() => {
    const date = new Date(Date.now() + 60 * 60 * 1000);
    date.setSeconds(0, 0);
    return date.toISOString().slice(0, 16);
  });
  const [agentId, setAgentId] = useState<number | null>(null);
  const [taskId, setTaskId] = useState<number | null>(null);
  const [statusFilter, setStatusFilter] = useState<ReminderStatus | "All">(
    "Upcoming",
  );

  const selectedAgent = agents.find((agent) => agent.id === agentId) ?? null;
  const selectedAgentTasks = selectedAgent?.tasks ?? [];
  const filteredReminders = reminders
    .filter((reminder) =>
      statusFilter === "All" ? true : reminder.status === statusFilter,
    )
    .sort(
      (left, right) =>
        new Date(left.dueAt).getTime() - new Date(right.dueAt).getTime(),
    );

  function addReminder() {
    const trimmedTitle = title.trim();
    if (!trimmedTitle || !dueAt) {
      return;
    }

    setReminders((current) => [
      {
        id: Date.now(),
        title: trimmedTitle,
        notes: notes.trim(),
        dueAt: new Date(dueAt).toISOString(),
        status: "Upcoming",
        agentId,
        taskId,
        createdAt: new Date().toISOString(),
      },
      ...current,
    ]);
    setTitle("");
    setNotes("");
    setTaskId(null);
  }

  function updateReminderStatus(id: number, status: ReminderStatus) {
    setReminders((current) =>
      current.map((reminder) =>
        reminder.id === id ? { ...reminder, status } : reminder,
      ),
    );
  }

  function deleteReminder(id: number) {
    setReminders((current) => current.filter((reminder) => reminder.id !== id));
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">TIME-BASED WORK</span>
          <h1>Reminders</h1>
          <p className="page-message">
            Keep deadlines and follow-up work visible alongside the agent workflow.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Upcoming</span>
          <strong>{reminders.filter((reminder) => reminder.status === "Upcoming").length}</strong>
          <small>Waiting for attention</small>
        </article>
        <article className="summary-card">
          <span>Due soon</span>
          <strong>
            {reminders.filter(
              (reminder) =>
                reminder.status === "Upcoming" &&
                new Date(reminder.dueAt).getTime() <= Date.now() + 24 * 60 * 60 * 1000,
            ).length}
          </strong>
          <small>Within the next 24 hours</small>
        </article>
        <article className="summary-card">
          <span>Completed</span>
          <strong>{reminders.filter((reminder) => reminder.status === "Completed").length}</strong>
          <small>Finished reminders</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">NEW REMINDER</span>
            <h2>Schedule follow-up</h2>
          </div>
        </div>

        <div className="task-composer reminder-composer">
          <label className="form-field">
            <span>Reminder title</span>
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") addReminder();
              }}
              placeholder="Send the project update"
            />
          </label>
          <label className="form-field">
            <span>Due</span>
            <input
              type="datetime-local"
              value={dueAt}
              onChange={(event) => setDueAt(event.target.value)}
            />
          </label>
          <label className="form-field">
            <span>Agent</span>
            <select
              value={agentId ?? ""}
              onChange={(event) => {
                setAgentId(event.target.value ? Number(event.target.value) : null);
                setTaskId(null);
              }}
            >
              <option value="">No agent</option>
              {agents.map((agent) => (
                <option value={agent.id} key={agent.id}>{agent.name}</option>
              ))}
            </select>
          </label>
          <label className="form-field">
            <span>Linked task</span>
            <select
              value={taskId ?? ""}
              disabled={!selectedAgent}
              onChange={(event) => setTaskId(event.target.value ? Number(event.target.value) : null)}
            >
              <option value="">No linked task</option>
              {selectedAgentTasks.map((task) => (
                <option value={task.id} key={task.id}>{task.title}</option>
              ))}
            </select>
          </label>
          <button className="primary-button" onClick={addReminder}>Add reminder</button>
        </div>

        <label className="form-field">
          <span>Notes</span>
          <textarea
            rows={3}
            value={notes}
            onChange={(event) => setNotes(event.target.value)}
            placeholder="Optional context for the reminder"
          />
        </label>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">REMINDER QUEUE</span>
            <h2>Scheduled follow-up</h2>
          </div>
          <label className="form-field filter-field">
            <span>Status</span>
            <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value as ReminderStatus | "All")}>
              <option value="Upcoming">Upcoming</option>
              <option value="Completed">Completed</option>
              <option value="Dismissed">Dismissed</option>
              <option value="All">All reminders</option>
            </select>
          </label>
        </div>

        {filteredReminders.length === 0 ? (
          <p className="page-message">No reminders match this filter.</p>
        ) : (
          <div className="agent-list">
            {filteredReminders.map((reminder) => {
              const linkedAgent = agents.find((agent) => agent.id === reminder.agentId);
              const linkedTask = linkedAgent?.tasks.find((task) => task.id === reminder.taskId);
              return (
                <article className="agent-card" key={reminder.id}>
                  <div>
                    <h3>{reminder.title}</h3>
                    <p>{new Date(reminder.dueAt).toLocaleString()}</p>
                    {reminder.notes && <small>{reminder.notes}</small>}
                    <small>
                      {linkedAgent ? `Agent: ${linkedAgent.name}` : "Unassigned"}
                      {linkedTask ? ` · Task: ${linkedTask.title}` : ""}
                    </small>
                  </div>
                  <div className="task-card-actions">
                    <span className={`agent-status ${reminder.status === "Completed" ? "working" : reminder.status === "Dismissed" ? "paused" : "waiting"}`}>
                      {reminder.status}
                    </span>
                    {reminder.status === "Upcoming" && (
                      <>
                        <button className="primary-button" onClick={() => updateReminderStatus(reminder.id, "Completed")}>Complete</button>
                        <button className="secondary-button" onClick={() => updateReminderStatus(reminder.id, "Dismissed")}>Dismiss</button>
                      </>
                    )}
                    {(reminder.status === "Completed" || reminder.status === "Dismissed") && (
                      <button className="secondary-button" onClick={() => updateReminderStatus(reminder.id, "Upcoming")}>Restore</button>
                    )}
                    <button className="danger-button" onClick={() => deleteReminder(reminder.id)}>Delete</button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}

function ModelsPage({
  models,
  setModels,
  providerRegistry,
  activeProvider,
  registryBusy,
  registryMessage,
  onRefreshRegistry,
}: {
  models: ModelDefinition[];
  setModels: React.Dispatch<React.SetStateAction<ModelDefinition[]>>;
  providerRegistry: ProviderRegistrySnapshot;
  activeProvider: RuntimeProviderId;
  registryBusy: boolean;
  registryMessage: string;
  onRefreshRegistry: () => Promise<void>;
}) {
  const [modelName, setModelName] = useState("");
  const [provider, setProvider] = useState<ModelProvider>("OpenAI");
  const ollamaStatus = providerRuntimeStatus(providerRegistry, "ollama");
  const ollamaReady = ollamaStatus?.availability === "ready";
  const ollamaMessage = registryMessage || ollamaStatus?.message || "";
  const selectedBinding = providerRegistry.catalogBindings.find(
    (binding) => binding.catalogProvider === provider,
  );

  function addModel() {
    const trimmedName = modelName.trim();

    if (!trimmedName) {
      return;
    }

    const alreadyExists = models.some(
      (model) => model.name.toLowerCase() === trimmedName.toLowerCase(),
    );

    if (alreadyExists) {
      window.alert("A model with this name already exists.");
      return;
    }

    setModels((currentModels) => [
      ...currentModels,
      {
        id: Date.now(),
        name: trimmedName,
        provider,
      },
    ]);

    setModelName("");
  }

  function deleteModel(modelId: number) {
    const model = models.find((item) => item.id === modelId);

    if (!model) {
      return;
    }

    const shouldDelete = window.confirm(
      `Delete model "${model.name}" from the catalog?`,
    );

    if (!shouldDelete) {
      return;
    }

    setModels((currentModels) =>
      currentModels.filter((item) => item.id !== modelId),
    );
  }

  function addDiscoveredOllamaModel(name: string) {
    setModels((currentModels) => {
      if (
        currentModels.some(
          (model) => model.name.toLowerCase() === name.toLowerCase(),
        )
      ) {
        return currentModels;
      }
      return [
        ...currentModels,
        { id: Date.now(), name, provider: "Ollama" },
      ];
    });
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">MODEL CATALOG</span>
          <h1>Models</h1>
          <p className="page-message">
            Manage catalog entries and see which active runtime can execute
            them.
          </p>
        </div>
      </header>

      <section className="panel provider-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">LOCAL LLM AND CODING AGENT</span>
            <h2>Ollama</h2>
            <p className="page-message">
              Local Ollama models can be assigned to agents. Tool-capable models
              run workspace coding tasks through the app’s bounded local agent.
            </p>
          </div>

          <span
            className={`connection-badge ${
              ollamaReady ? "connected" : "disconnected"
            }`}
          >
            {ollamaReady ? "Ready" : "Unavailable"}
          </span>
        </div>

        <div className="provider-connection-grid">
          <div className="provider-actions">
            <button
              className="primary-button"
              disabled={registryBusy}
              onClick={() => void onRefreshRegistry()}
            >
              {registryBusy ? "Checking…" : "Refresh provider registry"}
            </button>
          </div>
        </div>

        {ollamaStatus && (
          <div className="runtime-facts" aria-label="Ollama runtime details">
            <span>
              <strong>Version</strong>
              {ollamaStatus.version ?? "Unavailable"}
            </span>
            <span>
              <strong>Registry adapter</strong>
              {ollamaStatus.provider.displayName}
            </span>
            <span>
              <strong>Installed models</strong>
              {ollamaStatus.models.length}
            </span>
          </div>
        )}

        {ollamaMessage && (
          <div
            className={`runtime-message ${ollamaReady && !registryMessage ? "success" : "error"}`}
            role="status"
          >
            {ollamaMessage}
          </div>
        )}

        {ollamaStatus?.models.length ? (
          <div className="agent-list" style={{ marginTop: "18px" }}>
            {ollamaStatus.models.map((model) => {
              const alreadyRegistered = models.some(
                (item) => item.name.toLowerCase() === model.name.toLowerCase(),
              );
              const modelReady = model.availability === "ready";
              const toolCapable =
                modelReady &&
                model.capabilities.some(
                  (capability) => capability.toLowerCase() === "tools",
                );
              return (
                <article className="agent-card" key={model.name}>
                  <div>
                    <h3>{model.name}</h3>
                    <p>
                      Local Ollama model
                      {!modelReady
                        ? " · metadata unavailable"
                        : toolCapable
                          ? " · coding-agent ready"
                          : " · LLM only"}
                    </p>
                    <small>
                      {model.contextLength
                        ? `${model.contextLength.toLocaleString()} token context`
                        : "Context length unavailable"}
                      {model.contextLength !== null &&
                      model.contextLength < 64_000
                        ? " · keep complex coding tasks focused"
                        : ""}
                    </small>
                    <small>{model.message}</small>
                  </div>

                  <button
                    className={
                      alreadyRegistered ? "secondary-button" : "primary-button"
                    }
                    disabled={alreadyRegistered || !modelReady}
                    onClick={() => addDiscoveredOllamaModel(model.name)}
                  >
                    {alreadyRegistered
                      ? "In catalog"
                      : modelReady
                        ? "Add to catalog"
                        : "Unavailable"}
                  </button>
                </article>
              );
            })}
          </div>
        ) : null}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">ADD MODEL</span>
            <h2>Register a model</h2>
          </div>
        </div>

        <div className="model-composer">
          <label className="form-field">
            <span>Model name</span>
            <input
              type="text"
              value={modelName}
              onChange={(event) => setModelName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  addModel();
                }
              }}
              placeholder="Example: gpt-5"
            />
          </label>

          <label className="form-field">
            <span>Provider</span>
            <select
              value={provider}
              onChange={(event) =>
                setProvider(event.target.value as ModelProvider)
              }
            >
              <option value="OpenAI">OpenAI</option>
              <option value="Anthropic">Anthropic</option>
              <option value="Google">Google</option>
              <option value="Ollama">Ollama</option>
              <option value="Custom">Custom</option>
            </select>
            <small>
              {selectedBinding?.message ??
                "This catalog provider has no registry binding."}
            </small>
          </label>

          <button className="primary-button" onClick={addModel}>
            Add model
          </button>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AVAILABLE MODELS</span>
            <h2>Model catalog</h2>
          </div>
        </div>

        {models.length === 0 ? (
          <p className="page-message">
            No models registered yet. Add your first model above.
          </p>
        ) : (
          <div className="agent-list">
            {models.map((model) => {
              const availability = resolveModelAvailability(
                models,
                model.name,
                providerRegistry,
                activeProvider,
              );
              return (
                <article className="agent-card" key={model.id}>
                  <div>
                    <h3>{model.name}</h3>
                    <p>
                      {model.provider} · {availability.providerId ?? "no adapter"}
                    </p>
                    <small>{availability.reason}</small>
                  </div>

                  <div className="task-card-actions">
                    <span
                      className={`connection-badge ${availability.eligible ? "connected" : "disconnected"}`}
                    >
                      {availability.eligible ? "Executable" : "Unavailable"}
                    </span>
                    <button
                      className="danger-button"
                      onClick={() => deleteModel(model.id)}
                    >
                      Delete
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}


function RangeSetting({
  label,
  value,
  minimum,
  maximum,
  step = 1,
  suffix = "",
  hint,
  onChange,
}: {
  label: string;
  value: number;
  minimum: number;
  maximum: number;
  step?: number;
  suffix?: string;
  hint: string;
  onChange: (value: number) => void;
}) {
  return (
    <article className="range-setting">
      <div className="range-setting-heading">
        <span>{label}</span>
        <output>
          {value}
          {suffix}
        </output>
      </div>

      <input
        type="range"
        min={minimum}
        max={maximum}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <small>{hint}</small>
    </article>
  );
}

function SettingsPage({
  models,
  setModels,
  agents,
  setAgents,
  approvalRequests,
  setApprovalRequests,
  reminders,
  setReminders,
  taskRetentionDays,
  setTaskRetentionDays,
  activityRetentionDays,
  setActivityRetentionDays,
  preferences,
  setPreferences,
  providerRegistry,
  registryBusy,
  registryMessage,
  onRefreshRegistry,
  onImportBackup,
  onResetApplication,
}: {
  models: ModelDefinition[];
  setModels: React.Dispatch<React.SetStateAction<ModelDefinition[]>>;
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  approvalRequests: ApprovalRequest[];
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  reminders: Reminder[];
  setReminders: React.Dispatch<React.SetStateAction<Reminder[]>>;
  taskRetentionDays: HistoryRetentionDays;
  setTaskRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
  activityRetentionDays: HistoryRetentionDays;
  setActivityRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
  preferences: AppPreferences;
  setPreferences: React.Dispatch<
    React.SetStateAction<AppPreferences>
  >;
  providerRegistry: ProviderRegistrySnapshot;
  registryBusy: boolean;
  registryMessage: string;
  onRefreshRegistry: () => Promise<void>;
  onImportBackup: (backupJson: string) => Promise<void>;
  onResetApplication: (confirmation: string) => Promise<void>;
}) {
  const [managedAgentId, setManagedAgentId] = useState<number | null>(
    agents[0]?.id ?? null,
  );

  const managedAgent =
    agents.find((agent) => agent.id === managedAgentId) ?? agents[0] ?? null;
  const [providerMessage, setProviderMessage] = useState("");
  const [providerMessageKind, setProviderMessageKind] =
    useState<"success" | "error">("success");
  const [editingWorkspaceId, setEditingWorkspaceId] = useState<string | null>(
    null,
  );
  const [workspaceName, setWorkspaceName] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const codexStatus = providerRuntimeStatus(providerRegistry, "codex");
  const codexReady = codexStatus?.availability === "ready";
  const codexMessage =
    providerMessage || registryMessage || codexStatus?.message || "";
  const codexMessageKind = providerMessage
    ? providerMessageKind
    : codexReady && !registryMessage
      ? "success"
      : "error";
  const availableModels = executableModels(
    models,
    providerRegistry,
    preferences.activeAiProvider,
  );

  async function refreshRegistryFromSettings() {
    setProviderMessage("");
    await onRefreshRegistry();
  }

  function suggestedWorkspaceName(path: string) {
    return path.split("/").filter(Boolean).slice(-1)[0] ?? "Workspace";
  }

  async function browseForWorkspace() {
    if (!isDesktopRuntime() || workspaceBusy) {
      return;
    }

    setWorkspaceBusy(true);
    try {
      const selectedPath = await invoke<string | null>(
        "choose_workspace_folder",
      );
      if (selectedPath) {
        setWorkspacePath(selectedPath);
        setWorkspaceName((current) =>
          current.trim() ? current : suggestedWorkspaceName(selectedPath),
        );
      }
    } catch (error) {
      setProviderMessageKind("error");
      setProviderMessage(errorMessage(error));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  function saveWorkspace() {
    const name = workspaceName.trim();
    const path = workspacePath.trim();
    if (!name || !path) {
      setProviderMessageKind("error");
      setProviderMessage("Enter both a workspace name and folder path.");
      return;
    }

    const workspaceId = editingWorkspaceId ?? `workspace-${Date.now()}`;
    setPreferences((current) => {
      const existingIndex = current.workspaces.findIndex(
        (workspace) => workspace.id === workspaceId,
      );
      const nextWorkspace = { id: workspaceId, name, path };
      const workspaces = [...current.workspaces];
      if (existingIndex >= 0) {
        workspaces[existingIndex] = nextWorkspace;
      } else {
        workspaces.push(nextWorkspace);
      }

      return {
        ...current,
        workspaces,
        activeWorkspaceId: workspaceId,
        workspacePath: path,
      };
    });
    setEditingWorkspaceId(null);
    setWorkspaceName("");
    setWorkspacePath("");
    setProviderMessageKind("success");
    setProviderMessage(
      editingWorkspaceId ? "Workspace updated." : "Workspace added and selected.",
    );
  }

  function editWorkspace(workspace: WorkspaceDefinition) {
    setEditingWorkspaceId(workspace.id);
    setWorkspaceName(workspace.name);
    setWorkspacePath(workspace.path);
  }

  function selectWorkspace(workspace: WorkspaceDefinition) {
    setPreferences((current) => ({
      ...current,
      activeWorkspaceId: workspace.id,
      workspacePath: workspace.path,
    }));
  }

  function removeWorkspace(workspace: WorkspaceDefinition) {
    const assignedTaskCount = agents.reduce(
      (total, agent) =>
        total +
        agent.tasks.filter((task) => task.workspaceId === workspace.id).length,
      0,
    );
    const shouldRemove = window.confirm(
      assignedTaskCount > 0
        ? `Remove "${workspace.name}"? ${assignedTaskCount} task(s) will be reassigned to the next workspace.`
        : `Remove "${workspace.name}" from the workspace list? No files will be deleted.`,
    );
    if (!shouldRemove) {
      return;
    }

    const remaining = preferences.workspaces.filter(
      (item) => item.id !== workspace.id,
    );
    const nextWorkspace = remaining[0] ?? null;
    setPreferences((current) => ({
      ...current,
      workspaces: remaining,
      activeWorkspaceId:
        current.activeWorkspaceId === workspace.id
          ? (nextWorkspace?.id ?? null)
          : current.activeWorkspaceId,
      workspacePath:
        current.activeWorkspaceId === workspace.id
          ? (nextWorkspace?.path ?? "")
          : current.workspacePath,
    }));
    setAgents((currentAgents) =>
      currentAgents.map((agent) => ({
        ...agent,
        tasks: agent.tasks.map((task) =>
          task.workspaceId === workspace.id
            ? { ...task, workspaceId: nextWorkspace?.id ?? null }
            : task,
        ),
      })),
    );
    if (editingWorkspaceId === workspace.id) {
      setEditingWorkspaceId(null);
      setWorkspaceName("");
      setWorkspacePath("");
    }
  }

  async function openWorkspace(workspace: WorkspaceDefinition) {
    try {
      const workspaceAgent =
        agents.find(
          (agent) => agent.status !== "Paused" && agent.capabilities.files !== "none",
        ) ?? null;
      if (!workspaceAgent) {
        throw new Error("No active file-capable agent is available.");
      }
      const authorization = await prepareBackendAuthorization(
        {
          kind: "openWorkspaceItem",
          agentId: workspaceAgent.id,
          workspaceId: workspace.id,
          itemPath: ".",
        },
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setProviderMessageKind("error");
        setProviderMessage(
          "Opening this workspace is waiting for trusted backend authorization.",
        );
        return;
      }
      await invoke("open_workspace_item", {
        request: {
          agentId: workspaceAgent.id,
          workspaceId: workspace.id,
          itemPath: ".",
        },
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
    } catch (error) {
      setProviderMessageKind("error");
      setProviderMessage(errorMessage(error));
    }
  }

  function updateManagedAgentPerformance(
    patch: Partial<AgentPerformance>,
  ) {
    if (!managedAgent) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === managedAgent.id
          ? {
              ...agent,
              performance: normalizePerformance({
                ...agent.performance,
                ...patch,
              }),
            }
          : agent,
      ),
    );
  }

  function updateDefaultPerformance(
    patch: Partial<AgentPerformance>,
  ) {
    setPreferences((current) => ({
      ...current,
      defaultPerformance: normalizePerformance({
        ...current.defaultPerformance,
        ...patch,
      }),
    }));
  }

  function exportData() {
    const backup = {
      version: 2,
      exportedAt: new Date().toISOString(),
      agents,
      models,
      approvalRequests,
      reminders,
      taskRetentionDays,
      activityRetentionDays,
      preferences,
    };

    const blob = new Blob(
      [JSON.stringify(backup, null, 2)],
      { type: "application/json" },
    );
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");

    anchor.href = url;
    anchor.download = `ai-agent-control-center-backup-${new Date()
      .toISOString()
      .slice(0, 10)}.json`;

    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
  }

  function importData(file: File) {
    const reader = new FileReader();

    reader.onload = async () => {
      const backupJson = String(reader.result);
      if (isDesktopRuntime()) {
        const shouldImport = window.confirm(
          "Import this backup? Current application data will be replaced.",
        );
        if (!shouldImport) {
          return;
        }
        try {
          await onImportBackup(backupJson);
          window.alert("Backup imported successfully.");
        } catch (error) {
          window.alert(persistenceErrorMessage(error));
        }
        return;
      }

      try {
        const parsed = JSON.parse(backupJson) as {
          agents?: Agent[];
          models?: ModelDefinition[];
          approvalRequests?: ApprovalRequest[];
          reminders?: Partial<Reminder>[];
          taskRetentionDays?: HistoryRetentionDays;
          activityRetentionDays?: HistoryRetentionDays;
          preferences?: AppPreferences;
        };

        if (
          !Array.isArray(parsed.agents) ||
          !Array.isArray(parsed.models) ||
          !Array.isArray(parsed.approvalRequests)
        ) {
          throw new Error("Invalid backup structure");
        }

        const shouldImport = window.confirm(
          "Import this backup? Current local data will be replaced.",
        );

        if (!shouldImport) {
          return;
        }

        const importedPreferences = parsed.preferences
          ? normalizePreferences(parsed.preferences)
          : preferences;
        setAgents(
          parsed.agents.map((agent) => ({
            ...agent,
            performance: normalizePerformance(agent.performance),
            tasks: Array.isArray(agent.tasks)
              ? agent.tasks.map((task) => ({
                  ...task,
                  workspaceId:
                    typeof task.workspaceId === "string"
                      ? task.workspaceId
                      : importedPreferences.activeWorkspaceId,
                  changedFiles: Array.isArray(task.changedFiles)
                    ? task.changedFiles
                    : [],
                  diff: typeof task.diff === "string" ? task.diff : null,
                  durationSeconds:
                    typeof task.durationSeconds === "number"
                      ? task.durationSeconds
                      : null,
                  routingMode:
                    task.routingMode === "automatic"
                      ? "automatic"
                      : "selected",
                  routedFromAgentId:
                    typeof task.routedFromAgentId === "number"
                      ? task.routedFromAgentId
                      : null,
                  routingReason:
                    typeof task.routingReason === "string"
                      ? task.routingReason
                      : null,
                  reviewAgentId:
                    typeof task.reviewAgentId === "number"
                      ? task.reviewAgentId
                      : null,
                  reviewStatus:
                    task.reviewStatus === "Pending" ||
                    task.reviewStatus === "Running" ||
                    task.reviewStatus === "Approved" ||
                    task.reviewStatus === "Changes Requested" ||
                    task.reviewStatus === "Failed"
                      ? task.reviewStatus
                      : "Not Requested",
                  reviewResult:
                    typeof task.reviewResult === "string"
                      ? task.reviewResult
                      : null,
                  reviewModel:
                    typeof task.reviewModel === "string"
                      ? task.reviewModel
                      : null,
                  reviewDurationSeconds:
                    typeof task.reviewDurationSeconds === "number"
                      ? task.reviewDurationSeconds
                      : null,
                  reviewedAt:
                    typeof task.reviewedAt === "string"
                      ? task.reviewedAt
                      : null,
                }))
              : [],
          })),
        );
        setModels(
          parsed.models.some(
            (model) =>
              typeof model?.name === "string" &&
              model.name.toLowerCase() === ollamaCodingModelName,
          )
            ? parsed.models
            : [...parsed.models, ollamaCodingModel(Date.now())],
        );
        setApprovalRequests(
          parsed.approvalRequests
            .map(normalizeApprovalRequest)
            .filter((request): request is ApprovalRequest => request !== null),
        );
        setReminders(
          Array.isArray(parsed.reminders)
            ? parsed.reminders
                .filter(
                  (reminder): reminder is Partial<Reminder> & {
                    id: number;
                    title: string;
                    dueAt: string;
                  } =>
                    typeof reminder?.id === "number" &&
                    typeof reminder.title === "string" &&
                    typeof reminder.dueAt === "string",
                )
                .map((reminder) => ({
                  id: reminder.id,
                  title: reminder.title,
                  notes: typeof reminder.notes === "string" ? reminder.notes : "",
                  dueAt: reminder.dueAt,
                  status:
                    reminder.status === "Completed" || reminder.status === "Dismissed"
                      ? reminder.status
                      : "Upcoming",
                  agentId: typeof reminder.agentId === "number" ? reminder.agentId : null,
                  taskId: typeof reminder.taskId === "number" ? reminder.taskId : null,
                  createdAt:
                    typeof reminder.createdAt === "string"
                      ? reminder.createdAt
                      : new Date().toISOString(),
                }))
            : [],
        );

        if (
          parsed.taskRetentionDays === 7 ||
          parsed.taskRetentionDays === 30 ||
          parsed.taskRetentionDays === 90 ||
          parsed.taskRetentionDays === "never"
        ) {
          setTaskRetentionDays(parsed.taskRetentionDays);
        }

        if (
          parsed.activityRetentionDays === 7 ||
          parsed.activityRetentionDays === 30 ||
          parsed.activityRetentionDays === 90 ||
          parsed.activityRetentionDays === "never"
        ) {
          setActivityRetentionDays(parsed.activityRetentionDays);
        }

        setPreferences(importedPreferences);

        window.alert("Backup imported successfully.");
      } catch {
        window.alert(
          "The selected file is not a valid AI Agent Control Center backup.",
        );
      }
    };

    reader.readAsText(file);
  }

  async function resetApplication() {
    const confirmation = window.prompt(
      'Type RESET to delete all local agents, tasks, activity, models, approvals, and settings.',
    );

    if (confirmation !== "RESET") {
      return;
    }

    if (isDesktopRuntime()) {
      try {
        await onResetApplication(confirmation);
        window.alert("Application data was reset.");
      } catch (error) {
        window.alert(persistenceErrorMessage(error));
      }
      return;
    }

    for (const key of Object.values(LEGACY_STORAGE_KEYS)) {
      localStorage.removeItem(key);
    }

    window.location.reload();
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">APPLICATION SETTINGS</span>
          <h1>Settings</h1>
          <p className="page-message">
            Control appearance, agent performance, routing, defaults, and data.
          </p>
        </div>
      </header>

      <section className="panel provider-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AGENT RUNTIME</span>
            <h2>Codex connection</h2>
            <p className="page-message">
              Uses your ChatGPT-authenticated Codex CLI instead of separately billed API credits.
            </p>
          </div>

          <span
            className={`connection-badge ${codexReady ? "connected" : "disconnected"}`}
          >
            {codexReady ? "Ready" : "Unavailable"}
          </span>
        </div>

        <div className="provider-connection-grid">
          <label className="form-field">
            <span>Agent timeout</span>
            <select
              value={preferences.agentTimeoutMinutes}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  agentTimeoutMinutes: Number(event.target.value),
                }))
              }
            >
              <option value={5}>5 minutes</option>
              <option value={15}>15 minutes</option>
              <option value={30}>30 minutes</option>
              <option value={60}>60 minutes</option>
              <option value={120}>120 minutes</option>
            </select>
            <small>
              A run is stopped automatically if it exceeds this limit.
            </small>
          </label>

          <div className="provider-actions">
            <button
              className="primary-button"
              disabled={registryBusy}
              onClick={() => void refreshRegistryFromSettings()}
            >
              {registryBusy ? "Checking…" : "Refresh provider registry"}
            </button>
          </div>
        </div>

        {codexStatus && (
          <div className="runtime-facts" aria-label="Codex runtime details">
            <span>
              <strong>Version</strong>
              {codexStatus.version ?? "Unavailable"}
            </span>
            <span>
              <strong>Availability</strong>
              {codexStatus.availability}
            </span>
            <span>
              <strong>Workspace access</strong>
              {codexStatus.provider.capabilities.workspaceWrite
                ? "Read and write"
                : "Read only"}
            </span>
          </div>
        )}

        {codexMessage && (
          <div
            className={`runtime-message ${codexMessageKind}`}
            role="status"
          >
            {codexMessage}
          </div>
        )}
      </section>

      <section className="panel safety-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">SAFETY</span>
            <h2>Execution authorization</h2>
            <p className="page-message">
              Control when agents must stop and request a one-time permission.
            </p>
          </div>

          <span className="connection-badge connected">
            {preferences.safetyMode === "balanced"
              ? "Balanced"
              : preferences.safetyMode === "strict"
                ? "Strict"
                : "Locked"}
          </span>
        </div>

        <div className="choice-grid safety-mode-grid">
          {(
            [
              [
                "balanced",
                "Balanced",
                "Use each agent's policies and always review destructive actions.",
              ],
              [
                "strict",
                "Strict",
                "Ask before every file, terminal, web, or clipboard action.",
              ],
              [
                "locked",
                "Locked",
                "Allow inspection only; block changes and external actions.",
              ],
            ] as [SafetyMode, string, string][]
          ).map(([value, label, description]) => (
            <button
              type="button"
              key={value}
              className={`choice-card ${
                preferences.safetyMode === value ? "active" : ""
              }`}
              aria-pressed={preferences.safetyMode === value}
              onClick={() =>
                setPreferences((current) => ({
                  ...current,
                  safetyMode: value,
                }))
              }
            >
              <strong>{label}</strong>
              <span>{description}</span>
            </button>
          ))}
        </div>

        <div className="settings-grid">
          <label className="form-field settings-field">
            <span>Approval validity</span>
            <select
              value={preferences.approvalExpiryMinutes}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  approvalExpiryMinutes: Number(event.target.value),
                }))
              }
            >
              <option value={5}>5 minutes</option>
              <option value={15}>15 minutes</option>
              <option value={30}>30 minutes</option>
              <option value={60}>1 hour</option>
              <option value={120}>2 hours</option>
            </select>
            <small>
              An unused approval expires automatically and never authorizes more than one run.
            </small>
          </label>

          <div className="safety-boundary-note">
            <strong>Hard boundary</strong>
            <small>
              Privileged commands and system control remain blocked. Codex stays confined to the selected workspace.
            </small>
          </div>
        </div>
      </section>

      <section className="panel routing-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">ORCHESTRATION</span>
            <h2>Routing and review</h2>
            <p className="page-message">
              Choose how new work is assigned and independently verified.
            </p>
          </div>
        </div>

        <div className="settings-grid">
          <label className="form-field settings-field">
            <span>Default task routing</span>
            <select
              value={preferences.defaultRoutingMode}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  defaultRoutingMode: event.target.value as RoutingMode,
                }))
              }
            >
              <option value="selected">Keep with selected agent</option>
              <option value="automatic">Choose best available agent</option>
            </select>
            <small>
              Automatic routing scores expertise, capabilities, status, and current workload.
            </small>
          </label>

          <label className="form-field settings-field">
            <span>Senior review</span>
            <select
              value={preferences.reviewMode}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  reviewMode: event.target.value as ReviewMode,
                }))
              }
            >
              <option value="off">Off</option>
              <option value="manual">Manual review</option>
              <option value="automatic">Automatic after every run</option>
            </select>
            <small>
              Reviews use a different active agent in a read-only Codex sandbox.
            </small>
          </label>
        </div>

        <div className="routing-flow">
          <span>Task</span>
          <span>Best specialist</span>
          <span>Senior reviewer</span>
          <span>Approved or revisions</span>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">PROJECTS</span>
            <h2>Workspace manager</h2>
            <p className="page-message">
              Keep projects separate and choose a workspace for every new task.
            </p>
          </div>

          <span className="connection-badge connected">
            {preferences.workspaces.length} configured
          </span>
        </div>

        {preferences.workspaces.length > 0 && (
          <div className="workspace-manager-list">
            {preferences.workspaces.map((workspace) => (
              <article
                className={`workspace-manager-card ${
                  preferences.activeWorkspaceId === workspace.id ? "active" : ""
                }`}
                key={workspace.id}
              >
                <div>
                  <strong>{workspace.name}</strong>
                  <small>{workspace.path}</small>
                </div>
                <div className="workspace-manager-actions">
                  {preferences.activeWorkspaceId !== workspace.id && (
                    <button
                      className="secondary-button"
                      onClick={() => selectWorkspace(workspace)}
                    >
                      Set active
                    </button>
                  )}
                  <button
                    className="secondary-button"
                    onClick={() => openWorkspace(workspace)}
                  >
                    Open folder
                  </button>
                  <button
                    className="secondary-button"
                    onClick={() => editWorkspace(workspace)}
                  >
                    Edit
                  </button>
                  <button
                    className="danger-button"
                    onClick={() => removeWorkspace(workspace)}
                  >
                    Remove
                  </button>
                </div>
              </article>
            ))}
          </div>
        )}

        <div className="workspace-editor">
          <label className="form-field">
            <span>Workspace name</span>
            <input
              type="text"
              value={workspaceName}
              onChange={(event) => setWorkspaceName(event.target.value)}
              placeholder="Example: Control Center"
            />
          </label>

          <label className="form-field workspace-path-field">
            <span>Folder path</span>
            <div>
              <input
                type="text"
                value={workspacePath}
                readOnly
                placeholder="Choose a folder with the native picker"
                spellCheck={false}
              />
              <button
                className="secondary-button"
                disabled={workspaceBusy}
                onClick={browseForWorkspace}
              >
                {workspaceBusy ? "Opening…" : "Choose folder"}
              </button>
            </div>
          </label>

          <div className="workspace-editor-actions">
            <button className="primary-button" onClick={saveWorkspace}>
              {editingWorkspaceId ? "Save workspace" : "Add workspace"}
            </button>
            {editingWorkspaceId && (
              <button
                className="secondary-button"
                onClick={() => {
                  setEditingWorkspaceId(null);
                  setWorkspaceName("");
                  setWorkspacePath("");
                }}
              >
                Cancel
              </button>
            )}
          </div>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">APPEARANCE</span>
            <h2>Look and feel</h2>
            <p className="page-message">
              Changes apply immediately across the entire control center.
            </p>
          </div>
        </div>

        <div className="settings-block">
          <h3>Theme</h3>
          <div className="choice-grid">
            {(
              [
                ["dark", "Dark", "Low-light interface"],
                ["light", "Light", "Bright interface"],
                ["system", "System", "Follow your device"],
              ] as [ThemeMode, string, string][]
            ).map(([value, label, description]) => (
              <button
                type="button"
                key={value}
                className={`choice-card ${
                  preferences.theme === value ? "active" : ""
                }`}
                aria-pressed={preferences.theme === value}
                onClick={() =>
                  setPreferences((current) => ({
                    ...current,
                    theme: value,
                  }))
                }
              >
                <strong>{label}</strong>
                <span>{description}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="settings-grid">
          <div className="settings-block">
            <h3>Accent color</h3>
            <div className="accent-picker" aria-label="Accent color">
              {(
                ["violet", "blue", "cyan", "green"] as AccentColor[]
              ).map((color) => (
                <button
                  type="button"
                  key={color}
                  className={`accent-swatch ${color} ${
                    preferences.accentColor === color ? "active" : ""
                  }`}
                  aria-label={color}
                  aria-pressed={preferences.accentColor === color}
                  onClick={() =>
                    setPreferences((current) => ({
                      ...current,
                      accentColor: color,
                    }))
                  }
                />
              ))}
            </div>
          </div>

          <label className="form-field settings-field">
            <span>Interface density</span>
            <select
              value={preferences.density}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  density: event.target.value as InterfaceDensity,
                }))
              }
            >
              <option value="comfortable">Comfortable</option>
              <option value="compact">Compact</option>
            </select>
          </label>

          <label className="toggle-row">
            <input
              type="checkbox"
              checked={preferences.reducedMotion}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  reducedMotion: event.target.checked,
                }))
              }
            />
            <span>
              <strong>Reduce motion</strong>
              <small>Disable lift and transition effects.</small>
            </span>
          </label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">DEFAULTS</span>
            <h2>New agent and task defaults</h2>
            <p className="page-message">
              These values are applied whenever a new agent or task is created.
            </p>
          </div>
        </div>

        <div className="settings-grid">
          <label className="form-field">
            <span>Default model for new agents</span>
            <select
              value={preferences.defaultModel}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  defaultModel: event.target.value,
                }))
              }
            >
              <option value="None">None</option>
              {preferences.defaultModel.toLowerCase() !== "none" &&
                !availableModels.some(
                  (model) => model.name === preferences.defaultModel,
                ) && (
                  <option value={preferences.defaultModel} disabled>
                    {preferences.defaultModel} · unavailable
                  </option>
                )}
              {availableModels.map((model) => (
                <option value={model.name} key={model.id}>
                  {model.name} · {model.provider}
                </option>
              ))}
            </select>
          </label>

          <label className="form-field">
            <span>New agent starting status</span>
            <select
              value={preferences.defaultAgentStatus}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  defaultAgentStatus: event.target.value as AgentStatus,
                }))
              }
            >
              <option value="Waiting">Waiting</option>
              <option value="Working">Working</option>
              <option value="Paused">Paused</option>
            </select>
          </label>

          <label className="form-field">
            <span>Default task category</span>
            <select
              value={preferences.defaultTaskCategory}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  defaultTaskCategory: event.target.value as TaskCategory,
                }))
              }
            >
              <option value="Development">Development</option>
              <option value="Research">Research</option>
              <option value="Browsing">Browsing</option>
              <option value="Finance">Finance</option>
              <option value="Business">Business</option>
              <option value="Communication">Communication</option>
              <option value="System Control">System Control</option>
              <option value="General">General</option>
            </select>
          </label>

          <label className="form-field">
            <span>Default task priority</span>
            <select
              value={preferences.defaultTaskPriority}
              onChange={(event) =>
                setPreferences((current) => ({
                  ...current,
                  defaultTaskPriority:
                    event.target.value as TaskPriority,
                }))
              }
            >
              <option value="Low">Low</option>
              <option value="Normal">Normal</option>
              <option value="High">High</option>
              <option value="Critical">Critical</option>
            </select>
          </label>
        </div>

        <div className="slider-grid">
          <RangeSetting
            label="Default agent strength"
            value={preferences.defaultPerformance.strength}
            minimum={1}
            maximum={10}
            hint="Higher values favor deeper work."
            onChange={(strength) => updateDefaultPerformance({ strength })}
          />
          <RangeSetting
            label="Default CPU limit"
            value={preferences.defaultPerformance.cpuLimit}
            minimum={10}
            maximum={100}
            step={5}
            suffix="%"
            hint="Maximum CPU allocation for a new agent."
            onChange={(cpuLimit) => updateDefaultPerformance({ cpuLimit })}
          />
          <RangeSetting
            label="Default GPU limit"
            value={preferences.defaultPerformance.gpuLimit}
            minimum={0}
            maximum={100}
            step={5}
            suffix="%"
            hint="Set to 0% when GPU acceleration is not needed."
            onChange={(gpuLimit) => updateDefaultPerformance({ gpuLimit })}
          />
        </div>

        <div className="settings-grid">
          <label className="form-field">
            <span>Default execution focus</span>
            <select
              value={preferences.defaultPerformance.focus}
              onChange={(event) =>
                updateDefaultPerformance({
                  focus: event.target.value as ExecutionFocus,
                })
              }
            >
              <option value="speed">Speed</option>
              <option value="balanced">Balanced</option>
              <option value="strength">Strength</option>
            </select>
          </label>

          <label className="form-field">
            <span>Default overload action</span>
            <select
              value={preferences.defaultPerformance.overflowAction}
              onChange={(event) => {
                const overflowAction = event.target.value as OverflowAction;
                updateDefaultPerformance({
                  overflowAction,
                  redirectAgentId:
                    overflowAction === "queue"
                      ? null
                      : preferences.defaultPerformance.redirectAgentId,
                });
              }}
            >
              <option value="queue">Keep task queued</option>
              <option value="redirect">Redirect to another agent</option>
            </select>
          </label>

          {preferences.defaultPerformance.overflowAction === "redirect" && (
            <label className="form-field">
              <span>Default redirect agent</span>
              <select
                value={preferences.defaultPerformance.redirectAgentId ?? ""}
                onChange={(event) =>
                  updateDefaultPerformance({
                    redirectAgentId: event.target.value
                      ? Number(event.target.value)
                      : null,
                  })
                }
              >
                <option value="">Choose an agent</option>
                {agents.map((agent) => (
                  <option value={agent.id} key={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AGENT PERFORMANCE</span>
            <h2>Strength and resource controls</h2>
            <p className="page-message">
              Tune each agent and choose where overloaded work should go.
            </p>
          </div>
        </div>

        {managedAgent ? (
          <>
            <div className="settings-grid">
              <label className="form-field">
                <span>Agent to configure</span>
                <select
                  value={managedAgent.id}
                  onChange={(event) =>
                    setManagedAgentId(Number(event.target.value))
                  }
                >
                  {agents.map((agent) => (
                    <option value={agent.id} key={agent.id}>
                      {agent.name} · {agent.role}
                    </option>
                  ))}
                </select>
              </label>

              <label className="form-field">
                <span>Execution focus</span>
                <select
                  value={managedAgent.performance.focus}
                  onChange={(event) =>
                    updateManagedAgentPerformance({
                      focus: event.target.value as ExecutionFocus,
                    })
                  }
                >
                  <option value="speed">Speed</option>
                  <option value="balanced">Balanced</option>
                  <option value="strength">Strength</option>
                </select>
              </label>
            </div>

            <div className="slider-grid">
              <RangeSetting
                label="Agent strength"
                value={managedAgent.performance.strength}
                minimum={1}
                maximum={10}
                hint="1 is light and fast; 10 favors maximum reasoning depth."
                onChange={(strength) =>
                  updateManagedAgentPerformance({ strength })
                }
              />
              <RangeSetting
                label="CPU limit"
                value={managedAgent.performance.cpuLimit}
                minimum={10}
                maximum={100}
                step={5}
                suffix="%"
                hint="Maximum CPU share available to this agent."
                onChange={(cpuLimit) =>
                  updateManagedAgentPerformance({ cpuLimit })
                }
              />
              <RangeSetting
                label="GPU limit"
                value={managedAgent.performance.gpuLimit}
                minimum={0}
                maximum={100}
                step={5}
                suffix="%"
                hint="0% disables GPU use for this agent."
                onChange={(gpuLimit) =>
                  updateManagedAgentPerformance({ gpuLimit })
                }
              />
            </div>

            <div className="settings-grid">
              <label className="form-field">
                <span>When this agent is overloaded</span>
                <select
                  value={managedAgent.performance.overflowAction}
                  onChange={(event) => {
                    const overflowAction = event.target.value as OverflowAction;
                    updateManagedAgentPerformance({
                      overflowAction,
                      redirectAgentId:
                        overflowAction === "queue"
                          ? null
                          : managedAgent.performance.redirectAgentId,
                    });
                  }}
                >
                  <option value="queue">Keep task queued</option>
                  <option value="redirect">Redirect to another agent</option>
                </select>
              </label>

              {managedAgent.performance.overflowAction === "redirect" && (
                <label className="form-field">
                  <span>Redirect tasks to</span>
                  <select
                    value={managedAgent.performance.redirectAgentId ?? ""}
                    onChange={(event) =>
                      updateManagedAgentPerformance({
                        redirectAgentId: event.target.value
                          ? Number(event.target.value)
                          : null,
                      })
                    }
                  >
                    <option value="">Choose another agent</option>
                    {agents
                      .filter((agent) => agent.id !== managedAgent.id)
                      .map((agent) => (
                        <option value={agent.id} key={agent.id}>
                          {agent.name} · {agent.status}
                        </option>
                      ))}
                  </select>
                </label>
              )}
            </div>

            <p className="settings-note">
              These controls are saved now and are ready for the agent runtime
              to enforce when execution is connected.
            </p>
          </>
        ) : (
          <p className="page-message">
            Create an agent before configuring performance and routing.
          </p>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">HISTORY</span>
            <h2>Retention policy</h2>
          </div>
        </div>

        <div className="settings-grid">
          <label className="form-field">
            <span>Finished-task retention</span>
            <select
              value={taskRetentionDays}
              onChange={(event) =>
                setTaskRetentionDays(
                  event.target.value === "never"
                    ? "never"
                    : (Number(event.target.value) as 7 | 30 | 90),
                )
              }
            >
              <option value={7}>7 days</option>
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
              <option value="never">Never delete automatically</option>
            </select>
          </label>

          <label className="form-field">
            <span>Activity retention</span>
            <select
              value={activityRetentionDays}
              onChange={(event) =>
                setActivityRetentionDays(
                  event.target.value === "never"
                    ? "never"
                    : (Number(event.target.value) as 7 | 30 | 90),
                )
              }
            >
              <option value={7}>7 days</option>
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
              <option value="never">Never delete automatically</option>
            </select>
          </label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">BACKUP</span>
            <h2>Export and import</h2>
            <p className="page-message">
              Save or restore agents, tasks, models, approvals, and settings.
            </p>
          </div>
        </div>

        <div className="button-row">
          <button className="primary-button" onClick={exportData}>
            Export backup
          </button>

          <label className="secondary-button" style={{ cursor: "pointer" }}>
            Import backup
            <input
              type="file"
              accept="application/json,.json"
              style={{ display: "none" }}
              onChange={(event) => {
                const file = event.target.files?.[0];

                if (file) {
                  importData(file);
                }

                event.target.value = "";
              }}
            />
          </label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">DANGER ZONE</span>
            <h2>Reset local application</h2>
            <p className="page-message">
              Permanently remove all locally saved application data.
            </p>
          </div>
        </div>

        <button className="danger-button" onClick={resetApplication}>
          Reset all local data
        </button>
      </section>
    </>
  );
}

function PlaceholderPage({ page }: { page: Page }) {
  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">AI AGENT CONTROL CENTER</span>
          <h1>{page}</h1>
        </div>
      </header>

      <section className="panel">
        <h2>{page}</h2>
        <p className="page-message">
          This page is ready for its controls and content.
        </p>
      </section>
    </>
  );
}

function App() {
  const desktopRuntime = isDesktopRuntime();
  const [activePage, setActivePage] = useState<Page>("Dashboard");
  const [providerRegistry, setProviderRegistry] =
    useState<ProviderRegistrySnapshot>(() =>
      unknownProviderRegistrySnapshot(
        desktopRuntime
          ? "Provider readiness has not been inspected."
          : "Open the installed desktop app to inspect provider readiness.",
      ),
    );
  const [aiProviderBusy, setAiProviderBusy] = useState(false);
  const [aiProviderMessage, setAiProviderMessage] = useState("");
  const [runCoordinator, setRunCoordinator] =
    useState<RunCoordinatorUiState>(createRunCoordinatorUiState);
  const [persistencePhase, setPersistencePhase] = useState<
    "loading" | "mutating" | "hydrating" | "ready" | "error"
  >(desktopRuntime ? "loading" : "ready");
  const [persistenceMessage, setPersistenceMessage] = useState("");
  const persistenceWriter = useRef<ApplicationStateWriter | null>(null);
  const suppressNextPersistenceWrite = useRef(false);

  const [taskRetentionDays, setTaskRetentionDays] =
    useState<HistoryRetentionDays>(() => {
      if (desktopRuntime) {
        return 30;
      }
      const saved = localStorage.getItem(LEGACY_STORAGE_KEYS.taskRetentionDays);

      return saved === "7" || saved === "30" || saved === "90"
        ? (Number(saved) as 7 | 30 | 90)
        : saved === "never"
          ? "never"
          : 30;
    });

  const [activityRetentionDays, setActivityRetentionDays] =
    useState<HistoryRetentionDays>(() => {
      if (desktopRuntime) {
        return 30;
      }
      const saved = localStorage.getItem(
        LEGACY_STORAGE_KEYS.activityRetentionDays,
      );

      return saved === "7" || saved === "30" || saved === "90"
        ? (Number(saved) as 7 | 30 | 90)
        : saved === "never"
          ? "never"
          : 30;
    });

  const [preferences, setPreferences] =
    useState<AppPreferences>(() => {
      if (desktopRuntime) {
        return defaultAppPreferences;
      }
      const saved = localStorage.getItem(LEGACY_STORAGE_KEYS.preferences);

      if (!saved) {
        return defaultAppPreferences;
      }

      try {
        const parsed = JSON.parse(saved) as Partial<AppPreferences>;

        return normalizePreferences(parsed);
      } catch {
        return defaultAppPreferences;
      }
    });

  const [approvalRequests, setApprovalRequests] =
    useState<ApprovalRequest[]>(() => {
      if (desktopRuntime) {
        return [];
      }
      const saved = localStorage.getItem(
        LEGACY_STORAGE_KEYS.approvalRequests,
      );

      if (!saved) {
        return [];
      }

      try {
        const parsed = JSON.parse(saved) as Partial<ApprovalRequest>[];
        return Array.isArray(parsed)
          ? parsed
              .map(normalizeApprovalRequest)
              .filter((request): request is ApprovalRequest => request !== null)
          : [];
      } catch {
        return [];
      }
    });

  const [reminders, setReminders] = useState<Reminder[]>(() => {
    if (desktopRuntime) {
      return [];
    }
    const saved = localStorage.getItem(LEGACY_STORAGE_KEYS.reminders);
    if (!saved) return [];

    try {
      const parsed = JSON.parse(saved) as Partial<Reminder>[];
      return Array.isArray(parsed)
        ? parsed
            .filter(
              (reminder): reminder is Partial<Reminder> & {
                id: number;
                title: string;
                dueAt: string;
              } =>
                typeof reminder?.id === "number" &&
                typeof reminder.title === "string" &&
                typeof reminder.dueAt === "string",
            )
            .map((reminder) => ({
              id: reminder.id,
              title: reminder.title,
              notes: typeof reminder.notes === "string" ? reminder.notes : "",
              dueAt: reminder.dueAt,
              status:
                reminder.status === "Completed" || reminder.status === "Dismissed"
                  ? reminder.status
                  : "Upcoming",
              agentId: typeof reminder.agentId === "number" ? reminder.agentId : null,
              taskId: typeof reminder.taskId === "number" ? reminder.taskId : null,
              createdAt:
                typeof reminder.createdAt === "string"
                  ? reminder.createdAt
                  : new Date().toISOString(),
            }))
        : [];
    } catch {
      return [];
    }
  });

  const defaultModels: ModelDefinition[] = [
    { id: 1, name: "gpt-5.6-sol", provider: "OpenAI" },
    { id: 2, name: "gpt-5.6-terra", provider: "OpenAI" },
    { id: 3, name: "gpt-5.6-luna", provider: "OpenAI" },
    { id: 4, name: "claude-sonnet", provider: "Anthropic" },
    { id: 5, name: "gemini-2.5-pro", provider: "Google" },
    ollamaCodingModel(6),
  ];

  const [models, setModels] = useState<ModelDefinition[]>(() => {
    if (desktopRuntime) {
      return defaultModels;
    }
    const savedModels = localStorage.getItem(LEGACY_STORAGE_KEYS.models);

    if (!savedModels) {
      return defaultModels;
    }

    try {
      const parsedModels = JSON.parse(savedModels) as ModelDefinition[];

      if (!Array.isArray(parsedModels)) {
        return defaultModels;
      }
      return parsedModels.some(
        (model) =>
          typeof model?.name === "string" &&
          model.name.toLowerCase() === ollamaCodingModelName,
      )
        ? parsedModels
        : [...parsedModels, ollamaCodingModel(Date.now())];
    } catch {
      return defaultModels;
    }
  });

  const defaultCapabilities: Agent["capabilities"] = {
    files: "read",
    internet: "none",
    clipboard: "none",
    terminal: "none",
    system: "none",
  };

  const defaultApprovals: Agent["approvals"] = {
    files: "ask",
    internet: "ask",
    clipboard: "ask",
    terminal: "ask",
    system: "ask",
  };

  const defaultAgents: Agent[] = [
    {
      id: 1,
      name: "Supervisor",
      role: "Supervisor",
      category: "Management",
      reportsTo: null,
      authorityLevel: 4,
      description: "Coordinates tasks and delegates work",
      status: "Working",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 8,
        focus: "strength",
        cpuLimit: 80,
        gpuLimit: 60,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "full",
        internet: "read",
        clipboard: "write",
        terminal: "user",
        system: "notifications",
      },
      approvals: {
        files: "ask",
        internet: "ask",
        clipboard: "allow",
        terminal: "ask",
        system: "ask",
      },
    },
    {
      id: 2,
      name: "Coding Agent",
      role: "Specialist",
      category: "Development",
      reportsTo: 3,
      authorityLevel: 1,
      description: "Builds and edits project files",
      status: "Working",
      model: ollamaCodingModelName,
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 8,
        focus: "balanced",
        cpuLimit: 85,
        gpuLimit: 70,
        overflowAction: "redirect",
        redirectAgentId: 3,
      },
      capabilities: {
        files: "write",
        internet: "read",
        clipboard: "write",
        terminal: "safe",
        system: "none",
      },
      approvals: {
        files: "allow",
        internet: "ask",
        clipboard: "allow",
        terminal: "ask",
        system: "deny",
      },
    },
    {
      id: 3,
      name: "Debugging Agent",
      role: "Senior Agent",
      category: "Development",
      reportsTo: 6,
      authorityLevel: 2,
      description: "Finds errors and verifies fixes",
      status: "Waiting",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 7,
        focus: "strength",
        cpuLimit: 75,
        gpuLimit: 60,
        overflowAction: "redirect",
        redirectAgentId: 1,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "read",
        terminal: "safe",
        system: "none",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "ask",
        terminal: "ask",
        system: "deny",
      },
    },
    {
      id: 4,
      name: "Browser Agent",
      role: "Specialist",
      category: "Browsing",
      reportsTo: 9,
      authorityLevel: 1,
      description: "Uses websites when permission is granted",
      status: "Paused",
      model: "gpt-5.6-luna",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 4,
        focus: "speed",
        cpuLimit: 45,
        gpuLimit: 10,
        overflowAction: "redirect",
        redirectAgentId: 1,
      },
      capabilities: {
        files: "none",
        internet: "read",
        clipboard: "read",
        terminal: "none",
        system: "none",
      },
      approvals: {
        files: "deny",
        internet: "ask",
        clipboard: "ask",
        terminal: "deny",
        system: "deny",
      },
    },
    {
      id: 5,
      name: "Financial Agent",
      role: "Specialist",
      category: "Finance",
      reportsTo: 10,
      authorityLevel: 1,
      description: "Tracks financial tasks and reports",
      status: "Paused",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 5,
        focus: "balanced",
        cpuLimit: 55,
        gpuLimit: 25,
        overflowAction: "redirect",
        redirectAgentId: 1,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "none",
        terminal: "none",
        system: "none",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "deny",
        terminal: "deny",
        system: "deny",
      },
    },
    {
      id: 6,
      name: "Development Team Leader",
      role: "Team Leader",
      category: "Management",
      reportsTo: 1,
      authorityLevel: 3,
      description: "Coordinates development work and reviews delivery progress",
      status: "Waiting",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 8,
        focus: "strength",
        cpuLimit: 75,
        gpuLimit: 55,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "none",
        terminal: "safe",
        system: "none",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "deny",
        terminal: "ask",
        system: "deny",
      },
    },
    {
      id: 7,
      name: "PC Control Agent",
      role: "Specialist",
      category: "System Control",
      reportsTo: 11,
      authorityLevel: 1,
      description: "Handles safe computer-control requests with explicit approval",
      status: "Paused",
      model: "gpt-5.6-luna",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 5,
        focus: "speed",
        cpuLimit: 50,
        gpuLimit: 15,
        overflowAction: "redirect",
        redirectAgentId: 1,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "read",
        terminal: "none",
        system: "notifications",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "ask",
        terminal: "deny",
        system: "ask",
      },
    },
    {
      id: 8,
      name: "Event and Reminder Agent",
      role: "Specialist",
      category: "Business",
      reportsTo: 11,
      authorityLevel: 1,
      description: "Organizes reminders, deadlines, and scheduled follow-up work",
      status: "Paused",
      model: "gpt-5.6-luna",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 4,
        focus: "speed",
        cpuLimit: 45,
        gpuLimit: 10,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "none",
        terminal: "none",
        system: "notifications",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "deny",
        terminal: "deny",
        system: "ask",
      },
    },
    {
      id: 9,
      name: "Research and Web Senior",
      role: "Senior Agent",
      category: "Browsing",
      reportsTo: 6,
      authorityLevel: 2,
      description: "Guides research and browser work and verifies external findings",
      status: "Waiting",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 7,
        focus: "strength",
        cpuLimit: 70,
        gpuLimit: 45,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "read",
        internet: "read",
        clipboard: "read",
        terminal: "none",
        system: "none",
      },
      approvals: {
        files: "ask",
        internet: "ask",
        clipboard: "ask",
        terminal: "deny",
        system: "deny",
      },
    },
    {
      id: 10,
      name: "Finance Senior",
      role: "Senior Agent",
      category: "Finance",
      reportsTo: 6,
      authorityLevel: 2,
      description: "Reviews financial analysis, estimates, and reporting work",
      status: "Waiting",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 7,
        focus: "balanced",
        cpuLimit: 65,
        gpuLimit: 35,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "read",
        internet: "read",
        clipboard: "none",
        terminal: "none",
        system: "none",
      },
      approvals: {
        files: "ask",
        internet: "ask",
        clipboard: "deny",
        terminal: "deny",
        system: "deny",
      },
    },
    {
      id: 11,
      name: "Operations Senior",
      role: "Senior Agent",
      category: "Business",
      reportsTo: 6,
      authorityLevel: 2,
      description: "Reviews computer-control, scheduling, and operational work",
      status: "Waiting",
      model: "gpt-5.6-terra",
      memory: "",
      tasks: [],
      activity: [],
      performance: {
        strength: 7,
        focus: "balanced",
        cpuLimit: 65,
        gpuLimit: 35,
        overflowAction: "queue",
        redirectAgentId: null,
      },
      capabilities: {
        files: "read",
        internet: "none",
        clipboard: "read",
        terminal: "none",
        system: "notifications",
      },
      approvals: {
        files: "ask",
        internet: "deny",
        clipboard: "ask",
        terminal: "deny",
        system: "ask",
      },
    },
  ];

  function normalizeAgent(agent: Partial<Agent>): Agent {
    return {
      id: typeof agent.id === "number" ? agent.id : Date.now(),
      name: agent.name ?? "Unnamed Agent",
      description: agent.description ?? "No description provided",
      status: agent.status ?? "Waiting",
      role: agent.role ?? "Specialist",
      category: agent.category ?? "General",
      reportsTo:
        typeof agent.reportsTo === "number" ? agent.reportsTo : null,
      authorityLevel:
        agent.authorityLevel === 1 ||
        agent.authorityLevel === 2 ||
        agent.authorityLevel === 3 ||
        agent.authorityLevel === 4
          ? agent.authorityLevel
          : 1,
      model: agent.model ?? "gpt-5.6-luna",
      memory: agent.memory ?? "",
      tasks: Array.isArray(agent.tasks)
        ? agent.tasks.map((task) => {
            const legacyTask = task as unknown as {
              id: number;
              title: string;
              completed?: boolean;
              status?: TaskStatus | "Planned";
              category?: TaskCategory;
              priority?: TaskPriority;
              assignedAgentId?: number;
              phase?: TaskPhase;
              result?: string | null;
              responseId?: string | null;
              runtimeModel?: string | null;
              totalTokens?: number | null;
              workspaceId?: string | null;
              changedFiles?: string[];
              diff?: string | null;
              durationSeconds?: number | null;
              routingMode?: RoutingMode;
              routedFromAgentId?: number | null;
              routingReason?: string | null;
              reviewAgentId?: number | null;
              reviewStatus?: ReviewStatus;
              reviewResult?: string | null;
              reviewModel?: string | null;
              reviewDurationSeconds?: number | null;
              reviewedAt?: string | null;
            };

            const legacyStatus =
              legacyTask.status ??
              (legacyTask.completed ? "Completed" : "Pending");

            return {
              id: legacyTask.id,
              title: legacyTask.title,
              category:
                "category" in legacyTask
                  ? (legacyTask.category as TaskCategory)
                  : "General",
              priority:
                "priority" in legacyTask
                  ? (legacyTask.priority as TaskPriority)
                  : "Normal",
              assignedAgentId:
                "assignedAgentId" in legacyTask &&
                typeof legacyTask.assignedAgentId === "number"
                  ? legacyTask.assignedAgentId
                  : typeof agent.id === "number"
                    ? agent.id
                    : 0,
              status:
                legacyStatus === "Planned"
                  ? "Pending"
                  : (legacyStatus as TaskStatus),
              phase:
                "phase" in legacyTask
                  ? (legacyTask.phase as TaskPhase)
                  : legacyStatus === "Completed"
                    ? "Finished"
                    : legacyStatus === "Failed"
                      ? "Failed"
                      : legacyStatus === "Running"
                        ? "Specialist Work"
                        : "Assigned",
              createdAt:
                "createdAt" in legacyTask &&
                typeof legacyTask.createdAt === "string"
                  ? legacyTask.createdAt
                  : new Date().toISOString(),
              completedAt:
                "completedAt" in legacyTask &&
                typeof legacyTask.completedAt === "string"
                  ? legacyTask.completedAt
                  : legacyStatus === "Completed" ||
                      legacyStatus === "Failed"
                    ? new Date().toISOString()
                    : null,
              result:
                typeof legacyTask.result === "string"
                  ? legacyTask.result
                  : null,
              responseId:
                typeof legacyTask.responseId === "string"
                  ? legacyTask.responseId
                  : null,
              runtimeModel:
                typeof legacyTask.runtimeModel === "string"
                  ? legacyTask.runtimeModel
                  : null,
              totalTokens:
                typeof legacyTask.totalTokens === "number"
                  ? legacyTask.totalTokens
                  : null,
              workspaceId:
                typeof legacyTask.workspaceId === "string"
                  ? legacyTask.workspaceId
                  : preferences.activeWorkspaceId,
              changedFiles: Array.isArray(legacyTask.changedFiles)
                ? legacyTask.changedFiles.filter(
                    (file): file is string => typeof file === "string",
                  )
                : [],
              diff:
                typeof legacyTask.diff === "string" ? legacyTask.diff : null,
              durationSeconds:
                typeof legacyTask.durationSeconds === "number"
                  ? legacyTask.durationSeconds
                  : null,
              routingMode:
                legacyTask.routingMode === "automatic"
                  ? "automatic"
                  : "selected",
              routedFromAgentId:
                typeof legacyTask.routedFromAgentId === "number"
                  ? legacyTask.routedFromAgentId
                  : null,
              routingReason:
                typeof legacyTask.routingReason === "string"
                  ? legacyTask.routingReason
                  : null,
              reviewAgentId:
                typeof legacyTask.reviewAgentId === "number"
                  ? legacyTask.reviewAgentId
                  : null,
              reviewStatus:
                legacyTask.reviewStatus === "Pending" ||
                legacyTask.reviewStatus === "Running" ||
                legacyTask.reviewStatus === "Approved" ||
                legacyTask.reviewStatus === "Changes Requested" ||
                legacyTask.reviewStatus === "Failed"
                  ? legacyTask.reviewStatus
                  : "Not Requested",
              reviewResult:
                typeof legacyTask.reviewResult === "string"
                  ? legacyTask.reviewResult
                  : null,
              reviewModel:
                typeof legacyTask.reviewModel === "string"
                  ? legacyTask.reviewModel
                  : null,
              reviewDurationSeconds:
                typeof legacyTask.reviewDurationSeconds === "number"
                  ? legacyTask.reviewDurationSeconds
                  : null,
              reviewedAt:
                typeof legacyTask.reviewedAt === "string"
                  ? legacyTask.reviewedAt
                  : null,
            };
          })
        : [],
      activity: Array.isArray(agent.activity) ? agent.activity : [],
      performance: normalizePerformance(agent.performance),
      capabilities: {
        ...defaultCapabilities,
        ...agent.capabilities,
      },
      approvals: {
        ...defaultApprovals,
        ...agent.approvals,
      },
    };
  }

  const [agents, setAgents] = useState<Agent[]>(() => {
    if (desktopRuntime) {
      return defaultAgents;
    }
    const savedAgents = localStorage.getItem(LEGACY_STORAGE_KEYS.agents);

    if (!savedAgents) {
      return defaultAgents;
    }

    try {
      const parsedAgents = JSON.parse(savedAgents) as Partial<Agent>[];

      if (!Array.isArray(parsedAgents)) {
        return defaultAgents;
      }

      const normalizedAgents = parsedAgents.map(normalizeAgent);
      const existingNames = new Set(
        normalizedAgents.map((agent) => agent.name.trim().toLowerCase()),
      );
      const migratedAgents = normalizedAgents.map((agent) => {
        const seniorForAgent: Record<string, number> = {
          "Browser Agent": 9,
          "Financial Agent": 10,
          "PC Control Agent": 11,
          "Event and Reminder Agent": 11,
        };
        const seniorId = seniorForAgent[agent.name];
        if (seniorId && (agent.reportsTo === 1 || agent.reportsTo === null)) {
          return { ...agent, reportsTo: seniorId };
        }
        if (
          agent.name === "Coding Agent" &&
          agent.reportsTo === 1 &&
          normalizedAgents.some((item) => item.name === "Debugging Agent")
        ) {
          return { ...agent, reportsTo: 3 };
        }
        if (
          agent.name === "Debugging Agent" &&
          agent.reportsTo === 1
        ) {
          return { ...agent, reportsTo: 6 };
        }
        return agent;
      });
      const missingDefaults = defaultAgents.filter(
        (agent) => !existingNames.has(agent.name.trim().toLowerCase()),
      );

      return [...migratedAgents, ...missingDefaults];
    } catch {
      return defaultAgents;
    }
  });

  function applyAuthoritativeApplicationState(state: ApplicationState) {
    suppressNextPersistenceWrite.current = true;
    setAgents(state.agents);
    setModels(state.models);
    setApprovalRequests(state.approvalRequests);
    setReminders(state.reminders);
    setTaskRetentionDays(state.taskRetentionDays);
    setActivityRetentionDays(state.activityRetentionDays);
    setPreferences(state.preferences);
  }

  function hydrateApplicationState(state: ApplicationState) {
    setPersistencePhase("hydrating");
    applyAuthoritativeApplicationState(state);
  }

  useEffect(() => {
    if (!desktopRuntime) {
      return;
    }
    let active = true;
    void bootstrapDesktopApplicationState(invokeApplicationState, localStorage)
      .then(({ envelope, cleanupWarning }) => {
        if (!active) {
          return;
        }
        persistenceWriter.current = new ApplicationStateWriter(
          invokeApplicationState,
          envelope.revision,
          (error) => {
            if (active) {
              setPersistenceMessage(persistenceErrorMessage(error));
              setPersistencePhase("error");
            }
          },
        );
        setPersistenceMessage(cleanupWarning ?? "");
        hydrateApplicationState(envelope.state);
      })
      .catch((error: unknown) => {
        if (active) {
          setPersistenceMessage(persistenceErrorMessage(error));
          setPersistencePhase("error");
        }
      });
    return () => {
      active = false;
    };
  }, [desktopRuntime]);

  useEffect(() => {
    if (persistencePhase === "hydrating") {
      setPersistencePhase("ready");
    }
  }, [persistencePhase]);

  useEffect(() => {
    if (!desktopRuntime || persistencePhase !== "ready") {
      return;
    }
    let active = true;
    const unlisten: Array<() => void> = [];
    let refreshInFlight = false;
    let refreshQueued = false;

    const refreshAuthoritativeState = async () => {
      if (refreshInFlight) {
        refreshQueued = true;
        return;
      }
      refreshInFlight = true;
      try {
        const envelope = await invoke<StateEnvelope | null>(
          "load_application_state",
        );
        if (active && envelope) {
          applyAuthoritativeApplicationState(envelope.state);
        }
      } catch (error) {
        if (active) {
          setPersistenceMessage(persistenceErrorMessage(error));
        }
      } finally {
        refreshInFlight = false;
        if (active && refreshQueued) {
          refreshQueued = false;
          void refreshAuthoritativeState();
        }
      }
    };

    void invoke<RunCoordinatorSnapshot>("run_coordinator_snapshot")
      .then((snapshot) => {
        if (active) {
          setRunCoordinator((current) =>
            applyRunCoordinatorSnapshot(current, snapshot),
          );
        }
      })
      .catch((error) => {
        if (active) {
          setPersistenceMessage(errorMessage(error));
        }
      });
    void listen<RunCoordinatorEvent>("run-coordinator-event", (event) => {
      if (active) {
        setRunCoordinator((current) =>
          applyRunCoordinatorEvent(current, event.payload),
        );
      }
    }).then((stop) => {
      if (active) unlisten.push(stop);
      else stop();
    });
    void listen<RunCoordinatorSnapshot>(
      "run-coordinator-snapshot",
      (event) => {
        if (!active) return;
        setRunCoordinator((current) =>
          applyRunCoordinatorSnapshot(current, event.payload),
        );
        if (
          event.payload.activeAttempt === null ||
          event.payload.activeAttempt.startedAtUnixMs !== null
        ) {
          void refreshAuthoritativeState();
        }
      },
    ).then((stop) => {
      if (active) unlisten.push(stop);
      else stop();
    });

    return () => {
      active = false;
      unlisten.forEach((stop) => stop());
    };
  }, [desktopRuntime, persistencePhase]);

  async function importApplicationBackup(backupJson: string) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Application persistence is not ready.");
    }
    setPersistenceMessage("");
    setPersistencePhase("mutating");
    try {
      const envelope = await writer.importLegacyBackup(backupJson);
      hydrateApplicationState(envelope.state);
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        setPersistencePhase("ready");
      }
      throw error;
    }
  }

  async function resetPersistedApplication(confirmation: string) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Application persistence is not ready.");
    }
    setPersistenceMessage("");
    setPersistencePhase("mutating");
    try {
      const envelope = await writer.reset(confirmation);
      hydrateApplicationState(envelope.state);
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        setPersistencePhase("ready");
      }
      throw error;
    }
  }

  const activeAiProvider = preferences.activeAiProvider;
  const activeAiProviderStatus = providerRuntimeStatus(
    providerRegistry,
    activeAiProvider,
  );
  const activeAiProviderName =
    activeAiProviderStatus?.provider.displayName ??
    (activeAiProvider === "ollama" ? "Ollama" : "Codex");
  const activeAiProviderConnected =
    activeAiProviderStatus?.availability === "ready";
  const activeAiProviderModelCount = activeAiProviderStatus?.models.length ?? 0;
  const activeAiProviderHint =
    activeAiProviderConnected
      ? activeAiProvider === "ollama"
        ? activeAiProviderModelCount === 1
          ? "1 local model available"
          : `${activeAiProviderModelCount} local models available`
        : "Configured catalog models can run through Codex"
      : activeAiProviderStatus?.message ??
        "The active provider registry entry is missing.";

  async function refreshProviderRegistry() {
    if (!isDesktopRuntime()) {
      setAiProviderMessage(
        "Open the installed desktop app to inspect provider readiness.",
      );
      return;
    }

    setAiProviderBusy(true);
    setAiProviderMessage("");
    try {
      const snapshot = await invoke<ProviderRegistrySnapshot>(
        "provider_registry_status",
      );
      setProviderRegistry(snapshot);
    } catch (error) {
      const message = errorMessage(error);
      setProviderRegistry(unknownProviderRegistrySnapshot(message));
      setAiProviderMessage(message);
    } finally {
      setAiProviderBusy(false);
    }
  }

  async function selectAiProvider(nextProvider: RuntimeProviderId) {
    if (aiProviderBusy || nextProvider === activeAiProvider) {
      return;
    }
    if (runCoordinator.snapshot.activeAttempt) {
      setAiProviderMessage(
        "Stop the active run before changing the authoritative AI provider.",
      );
      return;
    }
    if (!isDesktopRuntime()) {
      setAiProviderMessage(
        "Open the installed desktop app to select an AI provider.",
      );
      return;
    }

    setAiProviderBusy(true);
    setAiProviderMessage("");
    setPreferences((current) => ({
      ...current,
      activeAiProvider: nextProvider,
    }));
    await refreshProviderRegistry();
  }

  useEffect(() => {
    if (!desktopRuntime) {
      return;
    }
    void refreshProviderRegistry();
  }, []);

  useEffect(() => {
    if (!isDesktopRuntime()) {
      return;
    }

    let stopListening: (() => void) | undefined;
    void listen("voice-control-open", () => setActivePage("Voice Control"))
      .then((unlisten) => {
        stopListening = unlisten;
      })
      .catch(() => undefined);

    return () => stopListening?.();
  }, []);

  useEffect(() => {
    const root = document.documentElement;

    root.dataset.theme = preferences.theme;
    root.dataset.accent = preferences.accentColor;
    root.dataset.density = preferences.density;
    root.dataset.motion = preferences.reducedMotion ? "reduced" : "full";
  }, [
    preferences.theme,
    preferences.accentColor,
    preferences.density,
    preferences.reducedMotion,
  ]);

  useEffect(() => {
    if (
      taskRetentionDays === "never" &&
      activityRetentionDays === "never"
    ) {
      return;
    }

    const now = Date.now();
    const taskCutoff =
      taskRetentionDays === "never"
        ? null
        : now - taskRetentionDays * 24 * 60 * 60 * 1000;
    const activityCutoff =
      activityRetentionDays === "never"
        ? null
        : now - activityRetentionDays * 24 * 60 * 60 * 1000;

    setAgents((currentAgents) => {
      let changed = false;
      const retainedAgents = currentAgents.map((agent) => {
        const tasks =
          taskCutoff === null
            ? agent.tasks
            : agent.tasks.filter((task) => {
                if (
                  task.status !== "Completed" &&
                  task.status !== "Failed"
                ) {
                  return true;
                }

                if (!task.completedAt) {
                  return true;
                }

                return new Date(task.completedAt).getTime() >= taskCutoff;
              });
        const activity =
          activityCutoff === null
            ? agent.activity
            : agent.activity.filter(
                (entry) =>
                  new Date(entry.createdAt).getTime() >= activityCutoff,
              );
        if (
          tasks.length === agent.tasks.length &&
          activity.length === agent.activity.length
        ) {
          return agent;
        }
        changed = true;
        return { ...agent, tasks, activity };
      });
      return changed ? retainedAgents : currentAgents;
    });
  }, [taskRetentionDays, activityRetentionDays]);

  useEffect(() => {
    const state: ApplicationState = {
      agents,
      models,
      approvalRequests,
      reminders,
      taskRetentionDays,
      activityRetentionDays,
      preferences,
    };
    if (desktopRuntime) {
      if (persistencePhase === "ready") {
        if (suppressNextPersistenceWrite.current) {
          suppressNextPersistenceWrite.current = false;
          return;
        }
        persistenceWriter.current?.enqueue(state);
      }
      return;
    }

    localStorage.setItem(
      LEGACY_STORAGE_KEYS.agents,
      JSON.stringify(agents),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.models,
      JSON.stringify(models),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.approvalRequests,
      JSON.stringify(approvalRequests),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.reminders,
      JSON.stringify(reminders),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.taskRetentionDays,
      String(taskRetentionDays),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.activityRetentionDays,
      String(activityRetentionDays),
    );
    localStorage.setItem(
      LEGACY_STORAGE_KEYS.preferences,
      JSON.stringify(preferences),
    );
  }, [
    agents,
    models,
    approvalRequests,
    reminders,
    taskRetentionDays,
    activityRetentionDays,
    preferences,
    desktopRuntime,
  ]);

  const globalActiveRun = runCoordinator.snapshot.activeAttempt;
  const latestRunProgress =
    runCoordinator.progress[runCoordinator.progress.length - 1];

  async function stopGlobalRun() {
    if (!globalActiveRun || runCoordinator.stopRequested) {
      return;
    }
    setRunCoordinator((current) => markRunStopRequested(current, true));
    try {
      const accepted = await invoke<boolean>("cancel_agent_run", {
        runId: globalActiveRun.requestId,
      });
      if (!accepted) {
        setRunCoordinator((current) => markRunStopRequested(current, false));
      }
    } catch (error) {
      setPersistenceMessage(errorMessage(error));
      setRunCoordinator((current) => markRunStopRequested(current, false));
    }
  }

  if (desktopRuntime && persistencePhase !== "ready") {
    const failed = persistencePhase === "error";
    return (
      <div className="app-shell">
        <main className="main-content">
          <section className="panel" role={failed ? "alert" : "status"}>
            <span className="eyebrow">APPLICATION STATE</span>
            <h1>{failed ? "Application data unavailable" : "Loading application data"}</h1>
            <p className="page-message">
              {failed
                ? persistenceMessage
                : persistencePhase === "mutating"
                  ? "Updating the versioned local application database…"
                  : "Opening the versioned local application database…"}
            </p>
            {failed && (
              <p className="form-hint">
                No desktop data was written to browser storage. Resolve the database error and restart the app.
              </p>
            )}
          </section>
        </main>
      </div>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-icon">
            <img src={logoUrl} alt="" />
          </div>
          <div>
            <strong>AI Agent</strong>
            <span>Control Center</span>
          </div>
        </div>

        <nav>
          {pages.map((page) => (
            <button
              key={page}
              className={`nav-item ${activePage === page ? "active" : ""}`}
              onClick={() => setActivePage(page)}
            >
              <span>{page}</span>
              {page === "Approvals" &&
                approvalRequests.some(
                  (request) => request.status === "Pending",
                ) && (
                  <span className="nav-count">
                    {
                      approvalRequests.filter(
                        (request) => request.status === "Pending",
                      ).length
                    }
                  </span>
                )}
            </button>
          ))}
        </nav>

        <div className="system-status">
          <span
            className={`status-dot ${activeAiProviderConnected ? "" : "offline"}`}
          />
          <div>
            <strong>
              {activeAiProviderName}{" "}
              {activeAiProviderConnected ? "connected" : "unavailable"}
            </strong>
            <small>{activeAiProviderHint}</small>
            <label className="system-provider-select">
              <span>AI provider</span>
              <select
                aria-label="AI provider"
                value={activeAiProvider}
                disabled={
                  !isDesktopRuntime() ||
                  aiProviderBusy ||
                  Boolean(globalActiveRun)
                }
                onChange={(event) =>
                  void selectAiProvider(
                    event.target.value as RuntimeProviderId,
                  )
                }
              >
                <option value="codex">Codex</option>
                <option value="ollama">Ollama</option>
              </select>
            </label>
            {aiProviderMessage && (
              <small className="system-provider-message" role="status">
                {aiProviderMessage}
              </small>
            )}
          </div>
        </div>
      </aside>

      <main className="main-content">
        {persistenceMessage && (
          <p className="page-message" role="status">
            {persistenceMessage}
          </p>
        )}
        {globalActiveRun && (
          <section className="global-run-banner" aria-live="polite">
            <div>
              <span className="eyebrow">ONE ACTIVE AI RUN</span>
              <strong>{globalActiveRun.taskTitle}</strong>
              <small>
                {globalActiveRun.runMode === "review"
                  ? "Senior review"
                  : "Task execution"}
                {` · ${globalActiveRun.status.replace(/_/g, " ")}`}
                {globalActiveRun.provider
                  ? ` · ${globalActiveRun.provider}`
                  : ""}
                {globalActiveRun.model ? ` · ${globalActiveRun.model}` : ""}
              </small>
              {latestRunProgress && (
                <span className="global-run-progress">
                  {latestRunProgress}
                </span>
              )}
            </div>
            <button
              className="danger-button"
              disabled={runCoordinator.stopRequested}
              onClick={() => void stopGlobalRun()}
            >
              {runCoordinator.stopRequested ? "Stopping…" : "Stop active run"}
            </button>
          </section>
        )}
        {activePage === "Dashboard" ? (
          <DashboardPage
            agents={agents}
            approvalRequests={approvalRequests}
            onOpenAgents={() => setActivePage("Agents")}
            onOpenTasks={() => setActivePage("Tasks")}
            onOpenApprovals={() => setActivePage("Approvals")}
          />
        ) : activePage === "Agents" ? (
          <AgentsPage
            agents={agents}
            setAgents={setAgents}
            models={models}
            providerRegistry={providerRegistry}
            preferences={preferences}
            runCoordinator={runCoordinator}
            setRunCoordinator={setRunCoordinator}
            approvalRequests={approvalRequests}
            setApprovalRequests={setApprovalRequests}
            onOpenApprovals={() => setActivePage("Approvals")}
          />
        ) : activePage === "Voice Control" ? (
          <VoiceControlPage
            agents={agents}
            setAgents={setAgents}
            setApprovalRequests={setApprovalRequests}
            preferences={preferences}
            setPreferences={setPreferences}
          />
        ) : activePage === "Tasks" ? (
          <TasksPage
            agents={agents}
            setAgents={setAgents}
            retentionDays={taskRetentionDays}
            setRetentionDays={setTaskRetentionDays}
            setApprovalRequests={setApprovalRequests}
          />
        ) : activePage === "Approvals" ? (
          <ApprovalsPage
            agents={agents}
            setAgents={setAgents}
            approvalRequests={approvalRequests}
            setApprovalRequests={setApprovalRequests}
            workspaces={preferences.workspaces}
            onOpenAgents={() => setActivePage("Agents")}
          />
        ) : activePage === "Reminders" ? (
          <RemindersPage
            agents={agents}
            reminders={reminders}
            setReminders={setReminders}
          />
        ) : activePage === "Activity" ? (
          <ActivityPage
            agents={agents}
            setAgents={setAgents}
            retentionDays={activityRetentionDays}
            setRetentionDays={setActivityRetentionDays}
          />
        ) : activePage === "Models" ? (
          <ModelsPage
            models={models}
            setModels={setModels}
            providerRegistry={providerRegistry}
            activeProvider={activeAiProvider}
            registryBusy={aiProviderBusy}
            registryMessage={aiProviderMessage}
            onRefreshRegistry={refreshProviderRegistry}
          />
        ) : activePage === "Settings" ? (
          <SettingsPage
            models={models}
            setModels={setModels}
            agents={agents}
            setAgents={setAgents}
            approvalRequests={approvalRequests}
            setApprovalRequests={setApprovalRequests}
            reminders={reminders}
            setReminders={setReminders}
            taskRetentionDays={taskRetentionDays}
            setTaskRetentionDays={setTaskRetentionDays}
            activityRetentionDays={activityRetentionDays}
            setActivityRetentionDays={setActivityRetentionDays}
            preferences={preferences}
            setPreferences={setPreferences}
            providerRegistry={providerRegistry}
            registryBusy={aiProviderBusy}
            registryMessage={aiProviderMessage}
            onRefreshRegistry={refreshProviderRegistry}
            onImportBackup={importApplicationBackup}
            onResetApplication={resetPersistedApplication}
          />
        ) : (
          <PlaceholderPage page={activePage} />
        )}
        {activePage !== "Voice Control" && (
          <VoiceControlPage
            agents={agents}
            setAgents={setAgents}
            setApprovalRequests={setApprovalRequests}
            preferences={preferences}
            setPreferences={setPreferences}
            visible={false}
          />
        )}
      </main>
    </div>
  );
}

export default App;
