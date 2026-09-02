import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Agent,
  AgentTask,
  AppPreferences,
  ApprovalRequest,
  HistoryRetentionDays,
  ModelDefinition,
  Reminder,
  ReviewStatus,
  RoutingMode,
  RuntimeProviderId,
  TaskCategory,
  TaskPhase,
  TaskPriority,
  TaskStatus,
  ApplicationState,
} from "../applicationState";
import { createDefaultApplicationState } from "../applicationState";
import {
  ApplicationStateWriter,
  LEGACY_STORAGE_KEYS,
  bootstrapDesktopApplicationState,
  persistenceErrorMessage,
  type TaskOrchestrationCommand,
} from "../persistence";
import {
  applyRunCoordinatorEvent,
  applyRunCoordinatorSnapshot,
  createRunCoordinatorUiState,
  markRunStopRequested,
  type RunCoordinatorUiState,
} from "../runCoordinator";
import {
  providerRuntimeStatus,
  unknownProviderRegistrySnapshot,
  type ProviderRegistrySnapshot,
} from "../providerRegistry";
import {
  activeRegistryAgents,
  normalizeLegacyAgentRegistry,
  normalizeLegacyAgentRegistrySet,
  type AgentRegistrySnapshot,
} from "../agentRegistry";
import {
  emptyTaskOrchestrationSnapshot,
  type RoutingEvidence,
  type TaskOrchestrationSnapshot,
} from "../taskOrchestration";
import {
  emptyReviewOrchestrationSnapshot,
  type ReviewOrchestrationSnapshot,
} from "../reviewOrchestration";
import { normalizeWorkspaceEvidence } from "../workspaceEvidence";
import {
  defaultAppPreferences,
  normalizeApprovalRequest,
  normalizePerformance,
  normalizePreferences,
} from "../domain/normalization";
import { errorMessage } from "../domain/errors";
import {
  ollamaCodingModel,
  ollamaCodingModelName,
} from "../domain/models";
import { desktopClient, isDesktopRuntime } from "../services/desktopClient";
import {
  isSpecialistTaskRequest,
  type SpecialistTaskRequest,
} from "../specialistCapabilities";
import { ActivityPage } from "../features/activity/ActivityPage";
import { AgentsPage } from "../features/agents/AgentsPage";
import { ApprovalsPage } from "../features/approvals/ApprovalsPage";
import { DashboardPage } from "../features/dashboard/DashboardPage";
import { ModelsPage } from "../features/models/ModelsPage";
import { RemindersPage } from "../features/reminders/RemindersPage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { TasksPage } from "../features/tasks/TasksPage";
import { VoiceControlPage } from "../features/voice/VoiceControlPage";
import { AppShell } from "./AppShell";
import {
  PersistenceStatusView,
  type PersistencePhase,
} from "./PersistenceStatusView";
import type { Page } from "./navigation";
import {
  previewMonitoringSnapshot,
  type BackupExport,
  type BackupImportPreview,
  type MonitoringMutationResult,
  type MonitoringSnapshot,
} from "../dataLifecycle";
import {
  emptyReminderSchedulerSnapshot,
  type ReminderSchedulerCommand,
  type ReminderSchedulerSnapshot,
} from "../reminderScheduler";
import {
  emptyStructuredMemorySnapshot,
  type StructuredMemoryCommand,
  type StructuredMemorySnapshot,
} from "../structuredMemory";
import {
  emptyManagementHandoffSnapshot,
  type ManagementHandoffSnapshot,
} from "../managementHandoffs";
import "../App.css";

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

export function AppController() {
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
  const [taskOrchestration, setTaskOrchestration] =
    useState<TaskOrchestrationSnapshot>(emptyTaskOrchestrationSnapshot);
  const [reviewOrchestration, setReviewOrchestration] =
    useState<ReviewOrchestrationSnapshot>(emptyReviewOrchestrationSnapshot);
  const [persistencePhase, setPersistencePhase] = useState<PersistencePhase>(
    desktopRuntime ? "loading" : "ready",
  );
  const [persistenceMessage, setPersistenceMessage] = useState("");
  // Bumped by the recovery screen's retry action. It is the only dependency
  // that re-runs the authoritative bootstrap, so a retry replays exactly the
  // same backend load rather than reconstructing state in the renderer.
  const [bootstrapAttempt, setBootstrapAttempt] = useState(0);
  const persistenceWriter = useRef<ApplicationStateWriter | null>(null);
  // Whether the authoritative state has loaded at least once in this process.
  // A failure before that is a startup failure: the backend opens the database
  // once at startup, so a renderer retry would replay the same stored error and
  // must not be offered.
  const hasLoadedOnce = useRef(false);
  const suppressNextPersistenceWrite = useRef(false);
  const [agentRegistrySnapshot, setAgentRegistrySnapshot] =
    useState<AgentRegistrySnapshot | null>(null);
  const [monitoringSnapshot, setMonitoringSnapshot] =
    useState<MonitoringSnapshot | null>(null);
  const [reminderScheduler, setReminderScheduler] =
    useState<ReminderSchedulerSnapshot>(emptyReminderSchedulerSnapshot);
  const [structuredMemory, setStructuredMemory] =
    useState<StructuredMemorySnapshot>(emptyStructuredMemorySnapshot);
  const [managementHandoffs, setManagementHandoffs] =
    useState<ManagementHandoffSnapshot>(emptyManagementHandoffSnapshot);

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

  const setBackendApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  > = (update) => {
    if (desktopRuntime) {
      suppressNextPersistenceWrite.current = true;
    }
    setApprovalRequests(update);
  };

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

  const defaultAgents = createDefaultApplicationState().agents;

  function normalizeAgent(agent: Partial<Agent>): Agent {
    return normalizeLegacyAgentRegistry({
      id: typeof agent.id === "number" ? agent.id : Date.now(),
      templateKey: agent.templateKey,
      registryState: agent.registryState,
      registryIssue: agent.registryIssue,
      deletedAtUnixMs: agent.deletedAtUnixMs,
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
              workspaceChanges?: unknown;
              durationSeconds?: number | null;
              routingMode?: RoutingMode;
              routedFromAgentId?: number | null;
              routingReason?: string | null;
              queueState?: AgentTask["queueState"];
              enqueueSequence?: number | null;
              routingEvidence?: RoutingEvidence | null;
              specialistRequest?: SpecialistTaskRequest | null;
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
            const queueState =
              legacyTask.queueState === "queued" ||
              legacyTask.queueState === "held" ||
              legacyTask.queueState === "admitted" ||
              legacyTask.queueState === "running" ||
              legacyTask.queueState === "notQueued"
                ? legacyTask.queueState
                : legacyStatus === "Pending" || legacyStatus === "Planned"
                  ? "queued"
                  : legacyStatus === "Blocked"
                    ? "held"
                    : legacyStatus === "Running"
                      ? "running"
                      : "notQueued";
            const enqueueSequence =
              queueState === "notQueued"
                ? null
                : typeof legacyTask.enqueueSequence === "number" &&
                    Number.isSafeInteger(legacyTask.enqueueSequence) &&
                    legacyTask.enqueueSequence > 0
                  ? legacyTask.enqueueSequence
                  : legacyTask.id;
            const routingEvidence =
              legacyTask.routingEvidence &&
              typeof legacyTask.routingEvidence.algorithmVersion ===
                "string" &&
              Array.isArray(legacyTask.routingEvidence.candidates)
                ? legacyTask.routingEvidence
                : null;

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
              workspaceChanges: normalizeWorkspaceEvidence(
                legacyTask.workspaceChanges,
              ),
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
              queueState,
              enqueueSequence,
              routingEvidence,
              specialistRequest: isSpecialistTaskRequest(
                legacyTask.specialistRequest,
              )
                ? legacyTask.specialistRequest
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
    });
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

      const normalizedAgents = normalizeLegacyAgentRegistrySet(
        parsedAgents.map(normalizeAgent),
      );
      return normalizedAgents;
    } catch {
      return defaultAgents;
    }
  });
  const operationalAgents = activeRegistryAgents(agents);
  const displayedMonitoringSnapshot = desktopRuntime
    ? monitoringSnapshot
    : previewMonitoringSnapshot(
        {
          agents,
          models,
          approvalRequests,
          reminders,
          taskRetentionDays,
          activityRetentionDays,
          preferences,
        },
        runCoordinator.snapshot.retainedAttemptCount,
        runCoordinator.snapshot.activeAttempt ? 1 : 0,
      );

  async function refreshAgentRegistrySnapshot() {
    if (!desktopRuntime) return;
    const snapshot = await desktopClient.agentRegistrySnapshot();
    setAgentRegistrySnapshot(snapshot);
  }

  async function refreshTaskOrchestrationSnapshot() {
    if (!desktopRuntime) return;
    const snapshot = await desktopClient.taskOrchestrationSnapshot();
    setTaskOrchestration(snapshot);
  }

  async function refreshReviewOrchestrationSnapshot() {
    if (!desktopRuntime) return;
    const snapshot = await desktopClient.reviewOrchestrationSnapshot();
    setReviewOrchestration(snapshot);
  }

  async function refreshReminderSchedulerSnapshot() {
    if (!desktopRuntime) return;
    setReminderScheduler(await desktopClient.reminderSchedulerSnapshot());
  }

  async function refreshStructuredMemorySnapshot() {
    if (!desktopRuntime) return;
    setStructuredMemory(await desktopClient.structuredMemorySnapshot());
  }

  async function refreshManagementHandoffSnapshot() {
    if (!desktopRuntime) return;
    setManagementHandoffs(await desktopClient.managementHandoffSnapshot());
  }

  async function refreshTask18Snapshots() {
    if (!desktopRuntime) return;
    await Promise.all([
      refreshReminderSchedulerSnapshot(),
      refreshStructuredMemorySnapshot(),
      refreshManagementHandoffSnapshot(),
    ]);
  }

  const refreshMonitoringSnapshot = useCallback(async () => {
    if (!desktopRuntime) return null;
    const snapshot = await desktopClient.monitoringSnapshot();
    setMonitoringSnapshot(snapshot);
    return snapshot;
  }, [desktopRuntime]);

  async function refreshAfterVoiceGatewayMutation() {
    if (!desktopRuntime) return;
    try {
      const envelope = await desktopClient.loadApplicationState();
      if (envelope) {
        persistenceWriter.current?.adoptRevision(envelope.revision);
        applyAuthoritativeApplicationState(envelope.state);
      }
      await refreshTaskOrchestrationSnapshot();
      await refreshReviewOrchestrationSnapshot();
      await refreshMonitoringSnapshot();
      await refreshTask18Snapshots();
    } catch (error) {
      setPersistenceMessage(
        `The voice action completed, but authoritative projections could not be refreshed: ${persistenceErrorMessage(error)}`,
      );
    }
  }

  async function adoptReviewOrchestrationSnapshot(
    snapshot: ReviewOrchestrationSnapshot,
  ) {
    setReviewOrchestration(snapshot);
    await persistenceWriter.current?.flush();
    const envelope = await desktopClient.loadApplicationState();
    if (envelope) {
      persistenceWriter.current?.adoptRevision(envelope.revision);
      applyAuthoritativeApplicationState(envelope.state);
    }
    await refreshTaskOrchestrationSnapshot();
    await refreshManagementHandoffSnapshot();
  }

  async function refreshAgentRegistrySnapshotAfterCommit() {
    try {
      await refreshAgentRegistrySnapshot();
    } catch (error) {
      setPersistenceMessage(
        `Application state was updated, but agent templates could not be refreshed: ${persistenceErrorMessage(error)}`,
      );
    }
  }

  async function refreshTaskOrchestrationAfterCommit() {
    try {
      await refreshTaskOrchestrationSnapshot();
      await refreshReviewOrchestrationSnapshot();
      await refreshMonitoringSnapshot();
      await refreshManagementHandoffSnapshot();
    } catch (error) {
      setPersistenceMessage(
        `Application state was updated, but the task queue could not be refreshed: ${persistenceErrorMessage(error)}`,
      );
    }
  }

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
    void bootstrapDesktopApplicationState(
      desktopClient.invokeApplicationState,
      localStorage,
    )
      .then(({ envelope, cleanupWarning }) => {
        if (!active) {
          return;
        }
        persistenceWriter.current = new ApplicationStateWriter(
          desktopClient.invokeApplicationState,
          envelope.revision,
          (error) => {
            if (active) {
              setPersistenceMessage(persistenceErrorMessage(error));
              setPersistencePhase("error");
            }
          },
          () => {
            void refreshMonitoringSnapshot().catch((error: unknown) => {
              if (active) setPersistenceMessage(persistenceErrorMessage(error));
            });
          },
        );
        setPersistenceMessage(cleanupWarning ?? "");
        hydrateApplicationState(envelope.state);
        void refreshAgentRegistrySnapshot().catch((error: unknown) => {
          if (active) setPersistenceMessage(persistenceErrorMessage(error));
        });
        void refreshTaskOrchestrationSnapshot().catch((error: unknown) => {
          if (active) setPersistenceMessage(persistenceErrorMessage(error));
        });
        void refreshReviewOrchestrationSnapshot().catch((error: unknown) => {
          if (active) setPersistenceMessage(persistenceErrorMessage(error));
        });
        void refreshMonitoringSnapshot().catch((error: unknown) => {
          if (active) setPersistenceMessage(persistenceErrorMessage(error));
        });
        void refreshTask18Snapshots().catch((error: unknown) => {
          if (active) setPersistenceMessage(persistenceErrorMessage(error));
        });
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
  }, [desktopRuntime, bootstrapAttempt]);

  function retryPersistenceBootstrap() {
    if (persistencePhase !== "error") {
      return;
    }
    persistenceWriter.current = null;
    setPersistenceMessage("");
    setPersistencePhase("loading");
    setBootstrapAttempt((attempt) => attempt + 1);
  }

  useEffect(() => {
    if (persistencePhase === "hydrating") {
      setPersistencePhase("ready");
    }
    if (persistencePhase === "ready") {
      hasLoadedOnce.current = true;
    }
  }, [persistencePhase]);

  useEffect(() => {
    if (!desktopRuntime || persistencePhase !== "ready") return;
    const interval = window.setInterval(() => {
      void refreshMonitoringSnapshot().catch((error: unknown) => {
        setPersistenceMessage(persistenceErrorMessage(error));
      });
    }, 60_000);
    return () => window.clearInterval(interval);
  }, [desktopRuntime, persistencePhase, refreshMonitoringSnapshot]);

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
        await persistenceWriter.current?.flush();
        const envelope = await desktopClient.loadApplicationState();
        if (active && envelope) {
          persistenceWriter.current?.adoptRevision(envelope.revision);
          applyAuthoritativeApplicationState(envelope.state);
          await refreshTaskOrchestrationSnapshot();
          await refreshReviewOrchestrationSnapshot();
          await refreshMonitoringSnapshot();
          await refreshManagementHandoffSnapshot();
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

    void desktopClient.runCoordinatorSnapshot()
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
    void desktopClient.onRunCoordinatorEvent((event) => {
      if (active) {
        setRunCoordinator((current) =>
          applyRunCoordinatorEvent(current, event),
        );
      }
    }).then((stop) => {
      if (active) unlisten.push(stop);
      else stop();
    });
    void desktopClient.onRunCoordinatorSnapshot(
      (snapshot) => {
        if (!active) return;
        setRunCoordinator((current) =>
          applyRunCoordinatorSnapshot(current, snapshot),
        );
        if (
          snapshot.activeAttempt === null ||
          snapshot.activeAttempt.startedAtUnixMs !== null
        ) {
          void refreshAuthoritativeState();
        }
      },
    ).then((stop) => {
      if (active) unlisten.push(stop);
      else stop();
    });
    void desktopClient.onReminderSchedulerSnapshot((snapshot) => {
      if (!active) return;
      setReminderScheduler(snapshot);
      void refreshAuthoritativeState();
    }).then((stop) => {
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
      const envelope = await writer.applyBackupImport(backupJson);
      hydrateApplicationState(envelope.state);
      await refreshAgentRegistrySnapshotAfterCommit();
      await refreshTaskOrchestrationAfterCommit();
      await refreshTask18Snapshots();
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        suppressNextPersistenceWrite.current = true;
        setPersistencePhase("ready");
      }
      throw error;
    }
  }

  async function previewApplicationBackup(
    backupJson: string,
  ): Promise<BackupImportPreview> {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Application persistence is not ready.");
    }
    return writer.previewBackupImport(backupJson);
  }

  async function exportApplicationBackup(): Promise<BackupExport> {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Application persistence is not ready.");
    }
    await writer.flush();
    return desktopClient.exportBackup();
  }

  async function saveApplicationBackup(
    fileName: string,
    backupJson: string,
  ): Promise<string | null> {
    if (!desktopRuntime) {
      throw new Error("Application persistence is not ready.");
    }
    return desktopClient.saveBackupFile(fileName, backupJson);
  }

  async function adoptMonitoringMutation(
    result: MonitoringMutationResult,
  ): Promise<void> {
    setMonitoringSnapshot(result.snapshot);
    const envelope = await desktopClient.loadApplicationState();
    if (envelope) {
      persistenceWriter.current?.adoptRevision(envelope.revision);
      applyAuthoritativeApplicationState(envelope.state);
    }
  }

  async function deleteActivityEntry(
    ownerAgentId: number,
    entryId: number,
  ): Promise<void> {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Authoritative activity controls require the desktop app.");
    }
    await writer.flush();
    const snapshot = await refreshMonitoringSnapshot();
    if (!snapshot) return;
    const result = await desktopClient.deleteMonitoringActivity({
      expectedRevision: snapshot.revision,
      ownerAgentId,
      entryId,
    });
    await adoptMonitoringMutation(result);
  }

  async function clearActivityHistory(): Promise<void> {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Authoritative activity controls require the desktop app.");
    }
    await writer.flush();
    const snapshot = await refreshMonitoringSnapshot();
    if (!snapshot) return;
    const result = await desktopClient.clearMonitoringActivity(
      snapshot.revision,
    );
    await adoptMonitoringMutation(result);
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
      await refreshAgentRegistrySnapshotAfterCommit();
      await refreshTaskOrchestrationAfterCommit();
      await refreshTask18Snapshots();
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        suppressNextPersistenceWrite.current = true;
        setPersistencePhase("ready");
      }
      throw error;
    }
  }

  async function mutateAgentRegistry(
    command: "create_agent" | "update_agent" | "delete_agent" | "restore_agent_template",
    request: Record<string, unknown>,
  ) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("The authoritative agent registry is available in the desktop app.");
    }
    setPersistenceMessage("");
    setPersistencePhase("mutating");
    try {
      const envelope = await writer.mutateAgentRegistry(command, request);
      hydrateApplicationState(envelope.state);
      await refreshAgentRegistrySnapshotAfterCommit();
      await refreshTaskOrchestrationAfterCommit();
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        suppressNextPersistenceWrite.current = true;
        setPersistencePhase("ready");
      }
      throw error;
    }
  }

  async function mutateTaskOrchestration(
    command: TaskOrchestrationCommand,
    request: Record<string, unknown>,
  ) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error(
        "Authoritative task orchestration is available in the desktop app.",
      );
    }
    setPersistenceMessage("");
    try {
      const envelope = await writer.mutateTaskOrchestration(command, request);
      applyAuthoritativeApplicationState(envelope.state);
      await refreshTaskOrchestrationAfterCommit();
    } catch (error) {
      if (writer.hasFailed) {
        setPersistenceMessage(persistenceErrorMessage(error));
        setPersistencePhase("error");
      } else {
        setPersistenceMessage(persistenceErrorMessage(error));
      }
      throw error;
    }
  }

  async function adoptTask18ApplicationRevision() {
    const envelope = await desktopClient.loadApplicationState();
    if (envelope) {
      persistenceWriter.current?.adoptRevision(envelope.revision);
      applyAuthoritativeApplicationState(envelope.state);
    }
    await refreshMonitoringSnapshot();
  }

  async function mutateReminderScheduler(
    command: ReminderSchedulerCommand,
    request: Record<string, unknown>,
  ) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("The authoritative reminder scheduler requires the desktop app.");
    }
    await writer.flush();
    const snapshot = await desktopClient.mutateReminderScheduler(command, request);
    setReminderScheduler(snapshot);
    await adoptTask18ApplicationRevision();
  }

  async function mutateStructuredMemory(
    command: StructuredMemoryCommand,
    request: Record<string, unknown>,
  ) {
    const writer = persistenceWriter.current;
    if (!desktopRuntime || !writer) {
      throw new Error("Authoritative structured memory requires the desktop app.");
    }
    await writer.flush();
    const snapshot = await desktopClient.mutateStructuredMemory(command, request);
    setStructuredMemory(snapshot);
    await adoptTask18ApplicationRevision();
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
      const snapshot = await desktopClient.providerRegistryStatus();
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

    const stopListening: Array<() => void> = [];
    void desktopClient.onVoiceControlOpen(() => setActivePage("Voice Control"))
      .then((unlisten) => {
        stopListening.push(unlisten);
      })
      .catch(() => undefined);
    void desktopClient.onRemindersOpen(() => setActivePage("Reminders"))
      .then((unlisten) => {
        stopListening.push(unlisten);
      })
      .catch(() => undefined);

    return () => stopListening.forEach((unlisten) => unlisten());
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
      desktopRuntime || taskRetentionDays === "never"
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
  }, [taskRetentionDays, activityRetentionDays, desktopRuntime]);

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
    persistencePhase,
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
      const accepted = await desktopClient.cancelAgentRun(
        globalActiveRun.requestId,
      );
      if (!accepted) {
        setRunCoordinator((current) => markRunStopRequested(current, false));
      }
    } catch (error) {
      setPersistenceMessage(errorMessage(error));
      setRunCoordinator((current) => markRunStopRequested(current, false));
    }
  }

  if (desktopRuntime && persistencePhase !== "ready") {
    return (
      <PersistenceStatusView
        phase={persistencePhase}
        message={persistenceMessage}
        onRetry={
          hasLoadedOnce.current ? retryPersistenceBootstrap : undefined
        }
      />
    );
  }

  return (
    <AppShell
      activePage={activePage}
      activeRun={globalActiveRun}
      latestRunProgress={latestRunProgress}
      onNavigate={setActivePage}
      onProviderChange={(provider) => void selectAiProvider(provider)}
      onStopRun={() => void stopGlobalRun()}
      pendingApprovalCount={approvalRequests.filter(
        (request) => request.status === "Pending",
      ).length}
      persistenceMessage={persistenceMessage}
      provider={{
        activeProvider: activeAiProvider,
        busy: aiProviderBusy,
        connected: activeAiProviderConnected,
        disabled: !desktopRuntime || Boolean(globalActiveRun),
        hint: activeAiProviderHint,
        message: aiProviderMessage,
        name: activeAiProviderName,
      }}
      stopRequested={runCoordinator.stopRequested}
    >
        {activePage === "Dashboard" ? (
          <DashboardPage
            agents={operationalAgents}
            approvalRequests={approvalRequests}
            taskOrchestration={taskOrchestration}
            runCoordinator={runCoordinator}
            monitoringSnapshot={displayedMonitoringSnapshot}
            onOpenAgents={() => setActivePage("Agents")}
            onOpenTasks={() => setActivePage("Tasks")}
            onOpenApprovals={() => setActivePage("Approvals")}
          />
        ) : activePage === "Agents" ? (
          <AgentsPage
            agents={agents}
            setAgents={setAgents}
            templates={agentRegistrySnapshot?.templates ?? []}
            onRegistryMutation={mutateAgentRegistry}
            onTaskMutation={mutateTaskOrchestration}
            authoritativeRegistry={desktopRuntime}
            authoritativeTaskOrchestration={desktopRuntime}
            taskOrchestration={taskOrchestration}
            reviewOrchestration={reviewOrchestration}
            onReviewSnapshot={adoptReviewOrchestrationSnapshot}
            models={models}
            providerRegistry={providerRegistry}
            preferences={preferences}
            runCoordinator={runCoordinator}
            setRunCoordinator={setRunCoordinator}
            approvalRequests={approvalRequests}
            setApprovalRequests={setBackendApprovalRequests}
            onOpenApprovals={() => setActivePage("Approvals")}
            structuredMemory={structuredMemory}
            managementHandoffs={managementHandoffs}
            authoritativeMemory={desktopRuntime}
            onMemoryMutation={mutateStructuredMemory}
          />
        ) : activePage === "Voice Control" ? (
          <VoiceControlPage
            agents={operationalAgents}
            onGatewayMutation={refreshAfterVoiceGatewayMutation}
            setApprovalRequests={setBackendApprovalRequests}
            preferences={preferences}
            setPreferences={setPreferences}
          />
        ) : activePage === "Tasks" ? (
          <TasksPage
            agents={operationalAgents}
            taskOrchestration={taskOrchestration}
            onTaskMutation={mutateTaskOrchestration}
            runActive={Boolean(globalActiveRun)}
            setApprovalRequests={setBackendApprovalRequests}
            monitoringSnapshot={displayedMonitoringSnapshot}
            onMonitoringStale={refreshMonitoringSnapshot}
          />
        ) : activePage === "Approvals" ? (
          <ApprovalsPage
            agents={operationalAgents}
            approvalRequests={approvalRequests}
            setApprovalRequests={setBackendApprovalRequests}
            workspaces={preferences.workspaces}
            onOpenAgents={() => setActivePage("Agents")}
          />
        ) : activePage === "Reminders" ? (
          <RemindersPage
            agents={operationalAgents}
            snapshot={reminderScheduler}
            authoritative={desktopRuntime}
            onMutation={mutateReminderScheduler}
          />
        ) : activePage === "Activity" ? (
          <ActivityPage
            agents={agents}
            setAgents={setAgents}
            runCoordinator={runCoordinator}
            retentionDays={activityRetentionDays}
            setRetentionDays={setActivityRetentionDays}
            monitoringSnapshot={displayedMonitoringSnapshot}
            onMonitoringStale={refreshMonitoringSnapshot}
            onDeleteActivity={deleteActivityEntry}
            onClearActivity={clearActivityHistory}
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
            agents={operationalAgents}
            applicationAgents={agents}
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
            onPreviewBackup={previewApplicationBackup}
            onExportBackup={exportApplicationBackup}
            onSaveBackup={saveApplicationBackup}
            onResetApplication={resetPersistedApplication}
            monitoringSnapshot={displayedMonitoringSnapshot}
          />
        ) : (
          <PlaceholderPage page={activePage} />
        )}
        {activePage !== "Voice Control" && (
          <VoiceControlPage
            agents={operationalAgents}
            onGatewayMutation={refreshAfterVoiceGatewayMutation}
            setApprovalRequests={setBackendApprovalRequests}
            preferences={preferences}
            setPreferences={setPreferences}
            visible={false}
          />
        )}
    </AppShell>
  );
}

export default AppController;
