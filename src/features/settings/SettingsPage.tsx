import { useState } from "react";
import type { Agent, AgentPerformance, AgentStatus, AppPreferences, ApprovalRequest, ExecutionFocus, HistoryRetentionDays, InterfaceDensity, ModelDefinition, OverflowAction, Reminder, ReviewMode, RoutingMode, SafetyMode, TaskCategory, TaskPriority, ThemeMode, WorkspaceDefinition, AccentColor } from "../../applicationState";
import { LEGACY_STORAGE_KEYS, persistenceErrorMessage } from "../../persistence";
import { executableModels, providerRuntimeStatus } from "../../providerRegistry";
import type { ProviderRegistrySnapshot } from "../../providerRegistry";
import { normalizeLegacyAgentRegistrySet } from "../../agentRegistry";
import { normalizeWorkspaceEvidence } from "../../workspaceEvidence";
import { normalizeApprovalRequest, normalizePerformance, normalizePreferences } from "../../domain/normalization";
import { desktopClient, isDesktopRuntime } from "../../services/desktopClient";
import { errorMessage } from "../../domain/errors";
import { ollamaCodingModel, ollamaCodingModelName } from "../../domain/models";
import { markApprovalConsumed, prepareBackendAuthorization } from "../../services/authorization";
import type {
  BackupExport,
  BackupImportPreview,
  MonitoringSnapshot,
} from "../../dataLifecycle";

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

export function SettingsPage({
  models,
  setModels,
  agents,
  applicationAgents,
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
  onPreviewBackup,
  onExportBackup,
  onResetApplication,
  monitoringSnapshot,
}: {
  models: ModelDefinition[];
  setModels: React.Dispatch<React.SetStateAction<ModelDefinition[]>>;
  agents: Agent[];
  applicationAgents: Agent[];
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
  onPreviewBackup: (backupJson: string) => Promise<BackupImportPreview>;
  onExportBackup: () => Promise<BackupExport>;
  onResetApplication: (confirmation: string) => Promise<void>;
  monitoringSnapshot: MonitoringSnapshot | null;
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
  const [backupPreview, setBackupPreview] = useState<{
    backupJson: string;
    preview: BackupImportPreview;
  } | null>(null);
  const [backupMessage, setBackupMessage] = useState("");
  const [backupBusy, setBackupBusy] = useState(false);
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
      const selectedPath = await desktopClient.chooseWorkspaceFolder();
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
      await desktopClient.openWorkspaceItem({
        agentId: workspaceAgent.id,
        workspaceId: workspace.id,
        itemPath: ".",
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

  function downloadBackup(contents: string, fileName: string) {
    const blob = new Blob([contents], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");

    anchor.href = url;
    anchor.download = fileName;

    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
  }

  async function exportData() {
    setBackupBusy(true);
    setBackupMessage("");
    try {
      if (isDesktopRuntime()) {
        const backup = await onExportBackup();
        downloadBackup(backup.backupJson, backup.fileName);
        setBackupMessage(
          `Exported ${backup.byteLength.toLocaleString()} bytes from the authoritative backend with ${backup.counts.reminders} reminder/event schedule(s) and ${backup.counts.memoryRecords} structured memory record(s). Runtime authority, portal delivery evidence, management handoffs, and provider credentials were omitted.`,
        );
        return;
      }
      const backup = {
        version: 2,
        exportedAt: new Date().toISOString(),
        agents: applicationAgents,
        models,
        approvalRequests,
        reminders,
        taskRetentionDays,
        activityRetentionDays,
        preferences,
      };
      downloadBackup(
        JSON.stringify(backup, null, 2),
        `ai-agent-control-center-browser-preview-${new Date()
          .toISOString()
          .slice(0, 10)}.json`,
      );
      setBackupMessage(
        "Exported a non-authoritative browser preview in legacy version 2 format.",
      );
    } catch (error) {
      setBackupMessage(persistenceErrorMessage(error));
    } finally {
      setBackupBusy(false);
    }
  }

  function importData(file: File) {
    const reader = new FileReader();

    reader.onload = async () => {
      const backupJson = String(reader.result);
      if (isDesktopRuntime()) {
        setBackupBusy(true);
        setBackupMessage("");
        try {
          const preview = await onPreviewBackup(backupJson);
          setBackupPreview({ backupJson, preview });
        } catch (error) {
          setBackupPreview(null);
          setBackupMessage(persistenceErrorMessage(error));
        } finally {
          setBackupBusy(false);
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
          normalizeLegacyAgentRegistrySet(parsed.agents.map((agent) => ({
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
                  workspaceChanges: normalizeWorkspaceEvidence(
                    task.workspaceChanges,
                  ),
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
          }))),
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

  async function applyPreviewedBackup() {
    if (!backupPreview) return;
    setBackupBusy(true);
    setBackupMessage("");
    try {
      await onImportBackup(backupPreview.backupJson);
      setBackupPreview(null);
      setBackupMessage("The validated portable backup was imported.");
    } catch (error) {
      setBackupMessage(persistenceErrorMessage(error));
    } finally {
      setBackupBusy(false);
    }
  }

  async function resetApplication() {
    const confirmation = window.prompt(
      "Type RESET to replace portable application state and run/review history with defaults. Maintenance evidence and database files are retained for the later physical-purge task.",
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
            <span>Multi-level review</span>
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
              Reviews follow the exact active reporting chain under read-only policy,
              one stage at a time.
            </small>
          </label>
        </div>

        <div className="routing-flow">
          <span>Task</span>
          <span>Best specialist</span>
          <span>Required reporting chain</span>
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
            label="Default queue workload threshold"
            value={preferences.defaultPerformance.queueThreshold}
            minimum={1}
            maximum={100}
            hint="Queued and active execute tasks assigned to an agent before overflow handling applies."
            onChange={(queueThreshold) =>
              updateDefaultPerformance({ queueThreshold })
            }
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
            <h2>Strength and workload controls</h2>
            <p className="page-message">
              Tune routing weight and choose what happens when assigned queue
              workload reaches a real backend threshold.
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
                label="Queue workload threshold"
                value={managedAgent.performance.queueThreshold}
                minimum={1}
                maximum={100}
                hint="Backend-counted queued and active execute tasks before this agent is overloaded."
                onChange={(queueThreshold) =>
                  updateManagedAgentPerformance({ queueThreshold })
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
              Queue thresholds and overflow actions are enforced by backend
              routing. CPU/GPU percentage metadata is retained only for data
              compatibility and is not presented as an operating-system quota.
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
            <p className="page-message">
              {monitoringSnapshot?.authoritative
                ? "The backend applies these policies at startup, every 15 minutes, and after relevant mutations."
                : "Browser preview only; continuous backend retention is available in the desktop app."}
            </p>
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

        {monitoringSnapshot?.authoritative && (
          <div className="runtime-message" role="status">
            Maintenance runs: {monitoringSnapshot.lifecycle.totalRuns} · Last
            successful check:{" "}
            {monitoringSnapshot.lifecycle.lastSuccessAtUnixMs === null
              ? "not yet recorded"
              : new Date(
                  monitoringSnapshot.lifecycle.lastSuccessAtUnixMs,
                ).toLocaleString()}
            {monitoringSnapshot.lifecycle.latestRun?.backlogRemaining
              ? " · bounded backlog remains and will retry in one minute"
              : ""}
            {monitoringSnapshot.lifecycle.lastErrorCode
              ? ` · ${monitoringSnapshot.lifecycle.lastErrorCode}: ${monitoringSnapshot.lifecycle.lastErrorMessage ?? "maintenance needs attention"}`
              : ""}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">BACKUP</span>
            <h2>Export and import</h2>
            <p className="page-message">
              {isDesktopRuntime()
                ? "Version 4 portable backups are strictly validated and sanitized by the backend. Reminder/event schedules and structured memory are portable; provider credentials, portal grants and delivery evidence, management handoffs, and run/review runtime history are excluded."
                : "Browser preview only: export and import use the non-authoritative legacy version 2 shape."}
            </p>
          </div>
        </div>

        <div className="button-row">
          <button
            className="primary-button"
            disabled={backupBusy}
            onClick={() => void exportData()}
          >
            Export backup
          </button>

          <label className="secondary-button" style={{ cursor: "pointer" }}>
            Import backup
            <input
              type="file"
              disabled={backupBusy}
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

        {backupPreview && (
          <div className="runtime-message" role="status">
            Validated backup v{backupPreview.preview.formatVersion}:{" "}
            {backupPreview.preview.counts.tasks} tasks, {" "}
            {backupPreview.preview.counts.activity} activity entries, {" "}
            {backupPreview.preview.counts.reminders} reminder/event schedules, {" "}
            {backupPreview.preview.counts.memoryRecords} memory records, and {" "}
            {backupPreview.preview.counts.approvalHistory} approval-history
            records. {backupPreview.preview.sanitizations.heldTasks} task(s)
            will be held and {backupPreview.preview.sanitizations.expiredApprovals}{" "}
            approval(s) expired. {backupPreview.preview.sanitizations.portalDeliveriesDisabled}{" "}
            portal delivery setting(s) will become in-app only. Current run,
            review, handoff, and notification-delivery history will be cleared.
            <div className="button-row">
              <button
                className="danger-button"
                disabled={backupBusy}
                onClick={() => void applyPreviewedBackup()}
              >
                Import and replace portable state
              </button>
              <button
                className="secondary-button"
                disabled={backupBusy}
                onClick={() => setBackupPreview(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {backupMessage && (
          <div className="runtime-message" role="status">
            {backupMessage}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">DANGER ZONE</span>
            <h2>Reset portable application state</h2>
            <p className="page-message">
              Reset restores defaults and clears current run/review history. It
              does not physically erase the SQLite database, bounded
              maintenance evidence, or desktop files; physical purge belongs
              to TASK-0019.
            </p>
          </div>
        </div>

        <button className="danger-button" onClick={resetApplication}>
          Reset portable state
        </button>
      </section>
    </>
  );
}
