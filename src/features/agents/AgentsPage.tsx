import { useEffect, useState } from "react";
import type { ActivityEntry, Agent, AgentCategory, AgentRole, AgentTask, AppPreferences, ApprovalMode, ApprovalRequest, ModelDefinition, RoutingMode, TaskCategory, TaskPriority } from "../../applicationState";
import { createDefaultApplicationState } from "../../applicationState";
import { persistenceErrorMessage } from "../../persistence";
import { hasVisibleTruncation, markRunStopRequested } from "../../runCoordinator";
import type { RunCoordinatorUiState } from "../../runCoordinator";
import { executableModels, resolveModelAvailability } from "../../providerRegistry";
import type { ProviderRegistrySnapshot } from "../../providerRegistry";
import { authorityForRole, availableAgentGroups, projectAgentGroup, registryIssueMessage, validManagerCandidates } from "../../agentRegistry";
import type { AgentGroup, AgentRegistrySnapshot } from "../../agentRegistry";
import { queueEntryForTask, queueStateLabel, taskCanEnterExecuteSlot } from "../../taskOrchestration";
import type { TaskOrchestrationSnapshot } from "../../taskOrchestration";
import { reviewFlowForTask, reviewFlowStatus, reviewLevelLabel } from "../../reviewOrchestration";
import type { ReviewIntentContext, ReviewOrchestrationSnapshot, ReviewVerdict } from "../../reviewOrchestration";
import { workspaceChangeCanOpen, workspaceChangeLabel, workspaceEvidenceHasVisibleLimit, workspaceEvidenceStatusLabel, workspaceReviewabilityLabel } from "../../workspaceEvidence";
import { taskSafetyAssessment } from "../../domain/taskSafety";
import { desktopClient, isDesktopRuntime } from "../../services/desktopClient";
import type { AgentRunResult, BackendActionIntent } from "../../services/desktopClient";
import { Dialog } from "../../components/Dialog";
import { Tabs, tabId, tabPanelId } from "../../components/Tabs";
import { KeyboardAction } from "../../components/KeyboardAction";
import { errorMessage } from "../../domain/errors";
import type { TaskOrchestrationMutation } from "../contracts";
import { markApprovalConsumed, prepareBackendAuthorization, type AuthorizationReadiness } from "../../services/authorization";
import {
  buildSpecialistTaskRequest,
  createSpecialistTaskDraft,
  specialistProfileForTemplate,
  type SpecialistProfile,
  type SpecialistResult,
  type SpecialistTaskDraft,
  type WorkspaceMutationClass,
} from "../../specialistCapabilities";

type WorkspaceTab = "Overview" | "Capabilities" | "Memory" | "Tasks" | "Activity";

type CapabilityKey = keyof Agent["capabilities"];

type ApprovalKey = keyof Agent["approvals"];

const workspaceTabs = [
  { value: "Overview", label: "Overview" },
  { value: "Capabilities", label: "Capabilities" },
  { value: "Memory", label: "Memory" },
  { value: "Tasks", label: "Tasks" },
  { value: "Activity", label: "Activity" },
] as const satisfies ReadonlyArray<{ value: WorkspaceTab; label: string }>;

function SpecialistComposerFields({
  profile,
  draft,
  onChange,
}: {
  profile: SpecialistProfile;
  draft: SpecialistTaskDraft;
  onChange: (update: Partial<SpecialistTaskDraft>) => void;
}) {
  function toggleMutation(mutation: WorkspaceMutationClass) {
    onChange({
      mutationClasses: draft.mutationClasses.includes(mutation)
        ? draft.mutationClasses.filter((item) => item !== mutation)
        : [...draft.mutationClasses, mutation],
    });
  }

  return (
    <>
      <div className="specialist-profile" role="status">
        <strong>{profile.label} contract</strong>
        <span>{profile.summary}</span>
        <ul>
          {profile.ceilings.map((ceiling) => (
            <li key={ceiling}>{ceiling}</li>
          ))}
        </ul>
      </div>

      {profile.templateKey === "coding" && (
        <div className="specialist-fields">
          <label className="form-field">
            <span>Acceptance criteria · one per line</span>
            <textarea
              rows={3}
              value={draft.acceptanceCriteria}
              onChange={(event) => onChange({ acceptanceCriteria: event.target.value })}
              placeholder="The focused check passes\nUnrelated behavior remains unchanged"
            />
          </label>
          <label className="form-field">
            <span>Constraints · one per line</span>
            <textarea
              rows={3}
              value={draft.constraints}
              onChange={(event) => onChange({ constraints: event.target.value })}
              placeholder="Preserve the public API"
            />
          </label>
          <fieldset className="specialist-mutations">
            <legend>Allowed workspace mutations</legend>
            {(["create", "modify", "delete", "rename"] as const).map((mutation) => (
              <label key={mutation}>
                <input
                  type="checkbox"
                  checked={draft.mutationClasses.includes(mutation)}
                  onChange={() => toggleMutation(mutation)}
                />
                {mutation}
              </label>
            ))}
          </fieldset>
          <label className="form-field">
            <span>Requested safe checks · one command per line</span>
            <textarea
              rows={3}
              value={draft.requestedChecks}
              onChange={(event) => onChange({ requestedChecks: event.target.value })}
              placeholder="npm test -- --runInBand"
            />
          </label>
          <label className="specialist-checkbox">
            <input
              type="checkbox"
              checked={draft.allowWebResearch}
              onChange={(event) => onChange({ allowWebResearch: event.target.checked })}
            />
            Allow hosted read-only web research for this task
          </label>
        </div>
      )}

      {profile.templateKey === "debugging" && (
        <div className="specialist-fields">
          <label className="form-field">
            <span>Observed symptoms · one per line</span>
            <textarea
              rows={3}
              value={draft.symptoms}
              onChange={(event) => onChange({ symptoms: event.target.value })}
              placeholder="The command exits with code 1"
            />
          </label>
          <label className="form-field">
            <span>Expected behavior</span>
            <textarea
              rows={3}
              value={draft.expectedBehavior}
              onChange={(event) => onChange({ expectedBehavior: event.target.value })}
            />
          </label>
          <label className="form-field">
            <span>Reproduction steps · one per line</span>
            <textarea
              rows={3}
              value={draft.reproductionSteps}
              onChange={(event) => onChange({ reproductionSteps: event.target.value })}
            />
          </label>
          <label className="form-field">
            <span>Requested read-only checks · one command per line</span>
            <textarea
              rows={3}
              value={draft.requestedChecks}
              onChange={(event) => onChange({ requestedChecks: event.target.value })}
            />
          </label>
        </div>
      )}

      {profile.templateKey === "browser" && (
        <div className="specialist-fields">
          <label className="form-field">
            <span>Allowed source domains · optional, comma or line separated</span>
            <textarea
              rows={3}
              value={draft.allowedDomains}
              onChange={(event) => onChange({ allowedDomains: event.target.value })}
              placeholder="kde.org, freedesktop.org"
            />
          </label>
          <label className="form-field">
            <span>Maximum sources</span>
            <input
              type="number"
              min={1}
              max={20}
              value={draft.maxSources}
              onChange={(event) => onChange({ maxSources: Number(event.target.value) })}
            />
          </label>
          <label className="form-field">
            <span>Freshness context · optional</span>
            <input
              type="text"
              value={draft.freshnessContext}
              onChange={(event) => onChange({ freshnessContext: event.target.value })}
              placeholder="Prefer sources updated in 2026"
            />
          </label>
        </div>
      )}

      {profile.templateKey === "financial" && (
        <div className="specialist-fields">
          <label className="form-field">
            <span>Currency · optional three-letter code</span>
            <input
              type="text"
              maxLength={3}
              value={draft.currency}
              onChange={(event) => onChange({ currency: event.target.value })}
              placeholder="EUR"
            />
          </label>
          <label className="form-field">
            <span>Assumptions · one per line</span>
            <textarea
              rows={3}
              value={draft.assumptions}
              onChange={(event) => onChange({ assumptions: event.target.value })}
            />
          </label>
          <label className="form-field specialist-calculations">
            <span>Fixed-point calculations · optional, one per line</span>
            <textarea
              rows={4}
              value={draft.calculations}
              onChange={(event) => onChange({ calculations: event.target.value })}
              placeholder="margin | percentOf | 2500.00, 12.5 | 2"
            />
            <small>
              Format: id | sum, difference, product, quotient, percentOf, or percentChange | comma-separated operands | output scale 0–12.
            </small>
          </label>
        </div>
      )}
    </>
  );
}

function SpecialistResultView({ result }: { result: SpecialistResult }) {
  const checks = result.kind === "coding"
    ? result.verification
    : result.kind === "debugging"
      ? result.checks
      : [];
  const limitations = result.kind === "coding" || result.kind === "browserResearch"
    ? result.limitations
    : result.kind === "financialAnalysis"
      ? result.caveats
      : [];
  return (
    <div className="specialist-result">
      <div className="agent-result-heading">
        <strong>Validated {result.kind.replace(/([A-Z])/g, " $1")} result</strong>
        <small>Backend-validated structured evidence</small>
      </div>
      <p>
        {result.kind === "browserResearch"
          ? result.answer
          : result.kind === "financialAnalysis"
            ? result.report
            : result.summary}
      </p>
      {result.kind === "coding" && result.changes.length > 0 && (
        <div><strong>Reported changes</strong><ul>{result.changes.map((item) => <li key={item}>{item}</li>)}</ul></div>
      )}
      {result.kind === "debugging" && (
        <>
          {result.findings.length > 0 && <div><strong>Findings</strong><ul>{result.findings.map((item) => <li key={item}>{item}</li>)}</ul></div>}
          {result.rootCauses.length > 0 && <div><strong>Root causes</strong><ul>{result.rootCauses.map((item) => <li key={item}>{item}</li>)}</ul></div>}
          {result.recommendedFixes.length > 0 && <div><strong>Recommended fixes</strong><ul>{result.recommendedFixes.map((item) => <li key={item}>{item}</li>)}</ul></div>}
          <small>Workspace changed: {result.workspaceChanged ? "reported changed" : "no"}</small>
        </>
      )}
      {checks.length > 0 && (
        <div>
          <strong>Checks</strong>
          <ul>{checks.map((check, index) => <li key={`${check.command}-${index}`}><code>{check.command}</code> · {check.status} · {check.summary}</li>)}</ul>
        </div>
      )}
      {result.kind === "browserResearch" && result.sources.length > 0 && (
        <div>
          <strong>Sources</strong>
          <ul>{result.sources.map((source) => <li key={source.url}><span>{source.title}</span><br /><code>{source.url}</code><br /><small>{source.retrievedAt} · {source.supports}</small></li>)}</ul>
        </div>
      )}
      {result.kind === "financialAnalysis" && (
        <>
          {result.calculationResults.length > 0 && <div><strong>Authoritative calculations</strong><ul>{result.calculationResults.map((calculation) => <li key={calculation.id}><code>{calculation.id}</code> = {calculation.value}</li>)}</ul></div>}
          <small>Decision authority: {result.decisionAuthority} · External effects: {result.externalEffects.length}</small>
        </>
      )}
      {limitations.length > 0 && <div><strong>Limitations</strong><ul>{limitations.map((item) => <li key={item}>{item}</li>)}</ul></div>}
    </div>
  );
}

export function AgentsPage({
  agents,
  setAgents,
  templates,
  onRegistryMutation,
  onTaskMutation,
  authoritativeRegistry,
  authoritativeTaskOrchestration,
  taskOrchestration,
  reviewOrchestration,
  onReviewSnapshot,
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
  templates: AgentRegistrySnapshot["templates"];
  onRegistryMutation: (
    command: "create_agent" | "update_agent" | "delete_agent" | "restore_agent_template",
    request: Record<string, unknown>,
  ) => Promise<void>;
  onTaskMutation: TaskOrchestrationMutation;
  authoritativeRegistry: boolean;
  authoritativeTaskOrchestration: boolean;
  taskOrchestration: TaskOrchestrationSnapshot;
  reviewOrchestration: ReviewOrchestrationSnapshot;
  onReviewSnapshot: (snapshot: ReviewOrchestrationSnapshot) => Promise<void>;
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
  const [specialistDraft, setSpecialistDraft] = useState<SpecialistTaskDraft>(
    createSpecialistTaskDraft,
  );
  const [specialistDraftMessage, setSpecialistDraftMessage] = useState("");
  const [selectedAgentId, setSelectedAgentId] = useState<number | null>(null);
  const [activeAgentGroup, setActiveAgentGroup] =
    useState<AgentGroup>("All agents");
  const [activeWorkspaceTab, setActiveWorkspaceTab] =
    useState<WorkspaceTab>("Overview");
  const [runtimeError, setRuntimeError] = useState("");
  const [systemCapabilityMessage, setSystemCapabilityMessage] = useState("");
  const [taskMutationBusy, setTaskMutationBusy] = useState(false);

  const selectedAgent =
    agents.find((agent) => agent.id === selectedAgentId) ?? null;
  const selectedSpecialistProfile = specialistProfileForTemplate(
    selectedAgent?.templateKey,
  );
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
  const selectedAgentRuns = runCoordinator.snapshot.recentAttempts
    .filter((attempt) => attempt.agentId === selectedAgentId)
    .slice(0, 8);
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
        request.agentId === task.assignedAgentId && request.taskId === task.id,
    );
  }

  function executorForTask(task: AgentTask) {
    return agents.find((agent) => agent.id === task.assignedAgentId) ?? null;
  }

  function queueEntry(task: AgentTask) {
    return selectedAgent
      ? queueEntryForTask(taskOrchestration, selectedAgent.id, task.id)
      : null;
  }

  function latestRunForTask(task: AgentTask) {
    return runCoordinator.snapshot.recentAttempts.find(
      (attempt) =>
        attempt.taskOwnerAgentId === selectedAgent?.id &&
        attempt.taskId === task.id,
    );
  }

  function workspaceEvidenceForTask(task: AgentTask) {
    return task.workspaceChanges ?? latestRunForTask(task)?.workspaceChanges ?? null;
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
    setIsCreating(false);
    setEditingAgentId(null);
  }

  function openCreateAgent() {
    setAgentName("");
    setAgentDescription("");
    setAgentRole("Specialist");
    setAgentCategory("General");
    setAgentReportsTo(null);
    setEditingAgentId(null);
    setIsCreating(true);
  }

  function openEditAgent(agent: Agent) {
    setAgentName(agent.name);
    setAgentDescription(agent.description);
    setAgentRole(agent.role);
    setAgentCategory(agent.category);
    setAgentReportsTo(agent.reportsTo);
    setIsCreating(false);
    setEditingAgentId(agent.id);
  }

  async function saveAgent() {
    const trimmedName = agentName.trim();
    const trimmedDescription = agentDescription.trim();

    if (!trimmedName || !trimmedDescription) {
      setRuntimeError("Agent name and description are required.");
      return;
    }
    if (agentRole !== "Supervisor" && agentReportsTo === null) {
      setRuntimeError("Select an active manager with greater authority.");
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

    setRuntimeError("");
    try {
      const request = {
        name: trimmedName,
        description: trimmedDescription,
        role: agentRole,
        category: agentCategory,
        reportsTo: agentRole === "Supervisor" ? null : agentReportsTo,
      };
      if (authoritativeRegistry) {
        await onRegistryMutation(
          editingAgentId === null ? "create_agent" : "update_agent",
          editingAgentId === null ? request : { ...request, agentId: editingAgentId },
        );
      } else if (editingAgentId !== null) {
        setAgents((currentAgents) =>
          currentAgents.map((agent) =>
            agent.id === editingAgentId
              ? {
                  ...agent,
                  ...request,
                  authorityLevel: authorityForRole(agentRole),
                  registryState: "active",
                  registryIssue: null,
                  deletedAtUnixMs: null,
                }
              : agent,
          ),
        );
      } else {
        setAgents((currentAgents) => [
          ...currentAgents,
          {
            id: Math.max(0, ...currentAgents.map((agent) => agent.id)) + 1,
            templateKey: null,
            registryState: "active",
            registryIssue: null,
            deletedAtUnixMs: null,
            ...request,
            status: preferences.defaultAgentStatus,
            authorityLevel: authorityForRole(agentRole),
            model: preferences.defaultModel,
            memory: "",
            tasks: [],
            activity: [],
            performance: { ...preferences.defaultPerformance },
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
          },
        ]);
      }
      resetForm();
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    }
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

    if (key !== "system") {
      return;
    }
    if (value !== "full") {
      setSystemCapabilityMessage("Full desktop input is disabled for this agent.");
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
      setSystemCapabilityMessage("Select an active agent first.");
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
      const status = await desktopClient.enableDesktopControl(selectedAgent.id);
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

  async function addTask() {
    const trimmedTitle = newTaskTitle.trim();

    if (selectedAgentId === null || !trimmedTitle) {
      return;
    }
    if (!authoritativeTaskOrchestration) {
      setRuntimeError("Task creation is available in the installed desktop app.");
      return;
    }
    if (!newTaskWorkspaceId) {
      setRuntimeError("Select a workspace before creating the task.");
      return;
    }
    const specialist = buildSpecialistTaskRequest(
      selectedAgent?.templateKey,
      trimmedTitle,
      specialistDraft,
    );
    if (specialist.error) {
      setRuntimeError(specialist.error);
      return;
    }
    setTaskMutationBusy(true);
    setRuntimeError("");
    setSpecialistDraftMessage("");
    try {
      await onTaskMutation("create_routed_task", {
        taskOwnerAgentId: selectedAgentId,
        title: trimmedTitle,
        category: selectedSpecialistProfile?.category ?? newTaskCategory,
        priority: newTaskPriority,
        workspaceId: newTaskWorkspaceId,
        routingMode: newTaskRoutingMode,
        preferredAgentId: selectedAgentId,
        selectedAgentId:
          newTaskRoutingMode === "selected" ? selectedAgentId : null,
        specialistRequest: specialist.request,
      });
      setNewTaskTitle("");
      setSpecialistDraft(createSpecialistTaskDraft());
      setNewTaskCategory(preferences.defaultTaskCategory);
      setNewTaskPriority(preferences.defaultTaskPriority);
      setNewTaskRoutingMode(preferences.defaultRoutingMode);
      setNewTaskWorkspaceId(preferences.activeWorkspaceId);
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    } finally {
      setTaskMutationBusy(false);
    }
  }

  async function rerouteTask(task: AgentTask, routingMode: RoutingMode) {
    if (!selectedAgent || !task.workspaceId) {
      setRuntimeError("The task owner or workspace is unavailable.");
      return;
    }
    setTaskMutationBusy(true);
    setRuntimeError("");
    try {
      await onTaskMutation("reroute_task", {
        taskOwnerAgentId: selectedAgent.id,
        taskId: task.id,
        title: task.title,
        category: task.category,
        priority: task.priority,
        workspaceId: task.workspaceId,
        routingMode,
        preferredAgentId: selectedAgent.id,
        selectedAgentId:
          routingMode === "selected" ? selectedAgent.id : null,
        specialistRequest: task.specialistRequest,
      });
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    } finally {
      setTaskMutationBusy(false);
    }
  }

  function prefillCodingTaskFromDebugging(task: AgentTask) {
    const result = latestRunForTask(task)?.specialistResult;
    const codingAgent = agents.find(
      (agent) =>
        agent.registryState === "active" && agent.templateKey === "coding",
    );
    if (!codingAgent || result?.kind !== "debugging") {
      setRuntimeError(
        "A validated Debugging result and active stable Coding Agent are required.",
      );
      return;
    }
    const draft = createSpecialistTaskDraft();
    draft.acceptanceCriteria = result.recommendedFixes.length > 0
      ? result.recommendedFixes.join("\n")
      : "Implement a bounded correction for the validated diagnosis.";
    draft.constraints = [
      `Review Debugging task ${task.id} evidence before editing.`,
      ...result.rootCauses.map((cause) => `Diagnosed cause: ${cause}`),
    ].join("\n");
    draft.requestedChecks = result.checks
      .map((check) => check.command)
      .join("\n");
    setSpecialistDraft(draft);
    setNewTaskTitle(`Implement fix for: ${task.title}`);
    setNewTaskCategory("Development");
    setNewTaskPriority(task.priority);
    setNewTaskRoutingMode("selected");
    setNewTaskWorkspaceId(task.workspaceId ?? preferences.activeWorkspaceId);
    setSelectedAgentId(codingAgent.id);
    setActiveWorkspaceTab("Tasks");
    setRuntimeError("");
    setSpecialistDraftMessage(
      "Coding draft prefilled from validated Debugging evidence. Review it before adding; no task has been created or dispatched.",
    );
  }

  async function setQueueDisposition(
    task: AgentTask,
    disposition: "hold" | "resume" | "resetTerminal",
  ) {
    if (!selectedAgent) return;
    setTaskMutationBusy(true);
    setRuntimeError("");
    try {
      await onTaskMutation("set_task_queue_disposition", {
        taskOwnerAgentId: selectedAgent.id,
        taskId: task.id,
        disposition,
      });
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    } finally {
      setTaskMutationBusy(false);
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
      setRuntimeError("Structured reviews run only in the installed desktop app.");
      return;
    }
    setRuntimeError("");
    try {
      let snapshot = await desktopClient.reviewOrchestrationSnapshot();
      await onReviewSnapshot(snapshot);
      for (let stageIndex = 0; stageIndex < 3; stageIndex += 1) {
        let flow = reviewFlowForTask(snapshot, selectedAgent.id, task.id);
        let context: ReviewIntentContext | null =
          flow?.state === "review_pending"
            ? (() => {
                const pending = [...flow.stages]
                  .reverse()
                  .find((stage) => stage.state === "pending");
                return pending
                  ? {
                      flowId: flow.id,
                      stageAttemptId: pending.id,
                      revisionRound: pending.revisionRound,
                      level: pending.level,
                      requestFingerprint: pending.requestFingerprint,
                    }
                  : null;
              })()
            : null;
        let reviewerAgentId =
          flow?.state === "review_pending"
            ? [...flow.stages]
                .reverse()
                .find((stage) => stage.state === "pending")
                ?.reviewerAgentId ?? null
            : null;

        let boundContext = context;
        if (!boundContext) {
          const start = await desktopClient.startReviewStage({
            expectedRevision: snapshot.revision,
            taskOwnerAgentId: selectedAgent.id,
            taskId: task.id,
          });
          snapshot = start.snapshot;
          await onReviewSnapshot(snapshot);
          if (start.blockedCode || !start.context || !start.stage) {
            setRuntimeError(
              start.blockedMessage ??
                "This review requires a trusted human decision.",
            );
            return;
          }
          boundContext = start.context;
          reviewerAgentId = start.stage.reviewerAgentId;
        }

        const reviewer = agents.find(
          (agent) => agent.id === reviewerAgentId,
        );
        if (!reviewer) {
          setRuntimeError(
            "The backend-selected reporting-chain reviewer is unavailable.",
          );
          return;
        }
        const reviewIntent: BackendActionIntent = {
          kind: "runTask",
          agentId: reviewer.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
          runMode: "review",
          reviewContext: boundContext,
        };
        const reviewAuthorization = await prepareBackendAuthorization(
          reviewIntent,
          setApprovalRequests,
        );
        if (!reviewAuthorization.ready) {
          setRuntimeError(
            "This exact review stage is waiting for backend authorization. Open Approvals to approve or deny it.",
          );
          onOpenApprovals();
          return;
        }
        await desktopClient.runAgentTask({
          runId: `review-${boundContext.flowId}-${boundContext.stageAttemptId}-${Date.now()}`,
          runMode: "review",
          agentId: reviewer.id,
          taskOwnerAgentId: selectedAgent.id,
          taskId: task.id,
          reviewContext: boundContext,
        });
        snapshot = await desktopClient.reviewOrchestrationSnapshot();
        await onReviewSnapshot(snapshot);
        flow = reviewFlowForTask(snapshot, selectedAgent.id, task.id);
        if (!continuation || flow?.state !== "awaiting_review") {
          return;
        }
      }
    } catch (error) {
      setRuntimeError(errorMessage(error));
    }
  }

  async function recordHumanReviewDecision(
    task: AgentTask,
    verdict: ReviewVerdict,
  ) {
    if (!selectedAgent || runActive || !isDesktopRuntime()) return;
    const feedback =
      verdict === "changesRequested"
        ? window.prompt("Required revision feedback")
        : "";
    if (feedback === null) return;
    try {
      const snapshot = await desktopClient.reviewOrchestrationSnapshot();
      const flow = reviewFlowForTask(snapshot, selectedAgent.id, task.id);
      if (!flow || flow.state !== "awaiting_human") {
        setRuntimeError(
          "This task is not awaiting a trusted human review decision.",
        );
        return;
      }
      const updated = await desktopClient.recordHumanReviewDecision({
        expectedRevision: snapshot.revision,
        taskOwnerAgentId: selectedAgent.id,
        taskId: task.id,
        flowId: flow.id,
        verdict,
        feedback,
      });
      await onReviewSnapshot(updated);
      setRuntimeError("");
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

    const executor = executorForTask(task);
    if (!executor) {
      setRuntimeError("The backend-selected task executor is unavailable.");
      return;
    }
    const entry = queueEntry(task);
    if (!taskCanEnterExecuteSlot(entry, taskOrchestration.activeExecute)) {
      setRuntimeError(
        entry?.queueState === "queued"
          ? `Only queue position 1 can enter the execute slot. This task is ${queueStateLabel(entry).toLowerCase()}.`
          : "This task is not queued for execute admission.",
      );
      return;
    }

    let authorization: AuthorizationReadiness;
    try {
      authorization = await prepareBackendAuthorization(
        {
          kind: "runTask",
          agentId: executor.id,
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
      onOpenApprovals();
      return;
    }

    setRuntimeError("");
    const runId = `task-${task.id}-${Date.now()}`;

    try {
      const result = await desktopClient.runAgentTask({
        runId,
        runMode: "execute",
        agentId: executor.id,
        taskOwnerAgentId: selectedAgent.id,
        taskId: task.id,
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
      const accepted = await desktopClient.cancelAgentRun(activeRunId);
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
      const executor = executorForTask(task);
      if (!executor) {
        throw new Error("The backend-selected task executor is unavailable.");
      }
      const authorization = await prepareBackendAuthorization(
        {
          kind: "openWorkspaceItem",
          agentId: executor.id,
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
      await desktopClient.openWorkspaceItem({
        agentId: executor.id,
        workspaceId: workspace.id,
        itemPath,
      });
      markApprovalConsumed(setApprovalRequests, authorization.approval);
    } catch (error) {
      setRuntimeError(errorMessage(error));
    }
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

  async function deleteAgent(agentId: number) {
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

    const directReports = agents.filter(
      (item) => item.registryState === "active" && item.reportsTo === agentId,
    );
    let replacementManagerId: number | null = null;
    if (directReports.length > 0) {
      const candidates = agents.filter(
        (candidate) =>
          candidate.registryState === "active" &&
          candidate.id !== agentId &&
          directReports.every(
            (report) => candidate.authorityLevel > report.authorityLevel,
          ),
      );
      if (candidates.length === 0) {
        setRuntimeError(
          "No active manager can accept this agent's direct reports. Reassign them first.",
        );
        return;
      }
      const entered = window.prompt(
        `Reassign ${directReports.length} direct report(s) to one of these manager IDs:\n${candidates
          .map((candidate) => `${candidate.id} · ${candidate.name}`)
          .join("\n")}`,
        String(candidates[0].id),
      );
      if (entered === null) return;
      const selected = Number(entered);
      if (!candidates.some((candidate) => candidate.id === selected)) {
        setRuntimeError("Select one of the listed replacement managers.");
        return;
      }
      replacementManagerId = selected;
    }

    setRuntimeError("");
    try {
      if (authoritativeRegistry) {
        await onRegistryMutation("delete_agent", {
          agentId,
          replacementManagerId,
        });
      } else {
        setAgents((currentAgents) =>
          currentAgents.map((item) => {
            if (item.id === agentId) {
              return {
                ...item,
                registryState: "deleted",
                registryIssue: null,
                deletedAtUnixMs: Date.now(),
                status: "Paused",
                reportsTo: null,
              };
            }
            if (item.reportsTo === agentId) {
              return { ...item, reportsTo: replacementManagerId };
            }
            return item;
          }),
        );
      }
      setSelectedAgentId(null);
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    }
  }

  async function restoreTemplate(templateKey: string) {
    setRuntimeError("");
    try {
      if (authoritativeRegistry) {
        await onRegistryMutation("restore_agent_template", {
          templateKey,
          reportsTo: null,
        });
        return;
      }
      const defaults = createDefaultApplicationState().agents;
      const template = defaults.find((agent) => agent.templateKey === templateKey);
      if (!template) throw new Error("That template is not available.");
      setAgents((currentAgents) => {
        const existingIndex = currentAgents.findIndex(
          (agent) => agent.templateKey === template.templateKey,
        );
        const managerTemplateKey = defaults.find(
          (agent) => agent.id === template.reportsTo,
        )?.templateKey;
        const reportsTo = managerTemplateKey
          ? currentAgents.find(
              (agent) =>
                agent.registryState === "active" &&
                agent.templateKey === managerTemplateKey,
            )?.id ?? null
          : null;
        const restored = {
          ...template,
          reportsTo,
          id:
            existingIndex >= 0
              ? currentAgents[existingIndex].id
              : Math.max(0, ...currentAgents.map((agent) => agent.id)) + 1,
          tasks: existingIndex >= 0 ? currentAgents[existingIndex].tasks : [],
          activity: existingIndex >= 0 ? currentAgents[existingIndex].activity : [],
          memory: existingIndex >= 0 ? currentAgents[existingIndex].memory : "",
        };
        if (existingIndex < 0) return [...currentAgents, restored];
        return currentAgents.map((agent, index) =>
          index === existingIndex ? restored : agent,
        );
      });
    } catch (error) {
      setRuntimeError(persistenceErrorMessage(error));
    }
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

        <Tabs
          idPrefix={`agent-workspace-${selectedAgent.id}`}
          label={`${selectedAgent.name} workspace sections`}
          tabs={workspaceTabs}
          value={activeWorkspaceTab}
          onChange={setActiveWorkspaceTab}
        />

        <div
          id={tabPanelId(
            `agent-workspace-${selectedAgent.id}`,
            activeWorkspaceTab,
          )}
          role="tabpanel"
          aria-labelledby={tabId(
            `agent-workspace-${selectedAgent.id}`,
            activeWorkspaceTab,
          )}
          tabIndex={0}
        >
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
                  {capability.key === "system" && (
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
                <span>
                  {selectedSpecialistProfile?.primaryLabel ?? "Task title"}
                </span>
                <input
                  type="text"
                  value={newTaskTitle}
                  onChange={(event) => {
                    setNewTaskTitle(event.target.value);
                    setSpecialistDraftMessage("");
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      void addTask();
                    }
                  }}
                  placeholder={
                    selectedSpecialistProfile
                      ? `Enter the ${selectedSpecialistProfile.label.toLowerCase()} objective`
                      : "Add a task for this agent"
                  }
                />
              </label>

              {selectedSpecialistProfile && (
                <SpecialistComposerFields
                  profile={selectedSpecialistProfile}
                  draft={specialistDraft}
                  onChange={(update) => {
                    setSpecialistDraft((current) => ({ ...current, ...update }));
                    setSpecialistDraftMessage("");
                  }}
                />
              )}

              <label className="form-field">
                <span>
                  {selectedSpecialistProfile?.templateKey === "browser" ||
                  selectedSpecialistProfile?.templateKey === "financial"
                    ? "Task workspace · not exposed as this specialist's run workspace"
                    : "Workspace"}
                </span>
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
                  value={selectedSpecialistProfile?.category ?? newTaskCategory}
                  disabled={Boolean(selectedSpecialistProfile)}
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

              <button
                className="primary-button"
                disabled={
                  taskMutationBusy || !authoritativeTaskOrchestration
                }
                onClick={() => void addTask()}
              >
                {taskMutationBusy
                  ? "Updating queue…"
                  : selectedSpecialistProfile
                    ? `Add ${selectedSpecialistProfile.label} task`
                    : "Add task"}
              </button>
            </div>

            {specialistDraftMessage && (
              <div className="runtime-message" role="status">
                {specialistDraftMessage}
              </div>
            )}

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
                {selectedAgent.tasks.map((task) => {
                  const entry = queueEntry(task);
                  const executor = executorForTask(task);
                  const latestRun = latestRunForTask(task);
                  const specialistResult = latestRun?.specialistResult ?? null;
                  const taskSpecialistProfile = specialistProfileForTemplate(
                    executor?.templateKey,
                  );
                  const workspaceEvidence = workspaceEvidenceForTask(task);
                  const reviewFlow = reviewFlowForTask(
                    reviewOrchestration,
                    selectedAgent.id,
                    task.id,
                  );
                  const assessment = executor
                    ? taskSafetyAssessment(
                        task,
                        executor,
                        preferences.safetyMode,
                      )
                    : null;
                  return (
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
                        {executor?.name ?? "Unknown agent"} · Workspace:{" "}
                        {workspaceForTask(task)?.name ?? "Missing"}
                      </small>

                      {taskSpecialistProfile && task.specialistRequest && (
                        <div className="specialist-task-contract">
                          <strong>{taskSpecialistProfile.label} profile v1</strong>
                          <small>{taskSpecialistProfile.summary}</small>
                          {latestRun?.specialistContract ? (
                            <small>
                              Effective backend ceiling: workspace {latestRun.specialistContract.tools.workspace} · terminal {latestRun.specialistContract.tools.terminal} · internet {latestRun.specialistContract.tools.internet} · calculator {latestRun.specialistContract.tools.calculator} · external effects {latestRun.specialistContract.tools.externalEffects}
                            </small>
                          ) : (
                            <small>
                              The immutable effective tool contract is recorded at run admission.
                            </small>
                          )}
                        </div>
                      )}

                      {taskSpecialistProfile && !task.specialistRequest && (
                        <div className="routing-note" role="alert">
                          <strong>Typed specialist contract required</strong>
                          <small>
                            This pre-schema-v10 task cannot run under a core specialist profile. Recreate it with the typed composer so the backend can bind its role, tools, and result contract.
                          </small>
                        </div>
                      )}

                      {reviewFlow && (
                        <div className="routing-note">
                          <strong>{reviewFlowStatus(reviewFlow)}</strong>
                          <small>
                            Round {reviewFlow.revisionRound} of{" "}
                            {reviewFlow.maxRevisions} · Required chain:{" "}
                            {reviewFlow.requiredLevels.length > 0
                              ? reviewFlow.requiredLevels
                                  .map((level) => reviewLevelLabel(level))
                                  .join(" → ")
                              : "trusted human"}
                          </small>
                          {reviewFlow.lastErrorMessage && (
                            <small>{reviewFlow.lastErrorMessage}</small>
                          )}
                        </div>
                      )}

                      <div className="routing-note">
                        <strong>{queueStateLabel(entry)}</strong>
                        <small>
                          Owner: {selectedAgent.name} · Executor:{" "}
                          {executor?.name ?? "Unavailable"}
                          {task.enqueueSequence !== null
                            ? ` · Enqueue sequence ${task.enqueueSequence}`
                            : ""}
                        </small>
                      </div>

                      {task.routingEvidence && (
                        <div className="routing-note">
                          <strong>
                            {task.routingMode === "automatic"
                              ? "Backend automatic route"
                              : "Backend selected-agent route"}
                          </strong>
                          <small>
                            {task.routingEvidence.reason}
                          </small>
                          <details>
                            <summary>
                              Routing evidence · {task.routingEvidence.outcomeCode}
                            </summary>
                            <small>
                              Algorithm {task.routingEvidence.algorithmVersion} ·{" "}
                              {task.routingEvidence.manualOverride
                                ? "user override recorded"
                                : "no user override"}
                            </small>
                            <ul>
                              {task.routingEvidence.candidates.map((candidate) => (
                                <li key={candidate.agentId}>
                                  {candidate.agentName}: {candidate.eligible
                                    ? `eligible, score ${candidate.score}, workload ${candidate.workload}/${candidate.queueThreshold}`
                                    : candidate.disqualifications
                                        .map((item) => item.code)
                                        .join(", ")}
                                  {candidate.selectionExcludedCode
                                    ? ` · ${candidate.selectionExcludedCode}`
                                    : ""}
                                </li>
                              ))}
                            </ul>
                          </details>
                        </div>
                      )}

                      {assessment && <div
                        className={`safety-summary risk-${assessment.riskLevel.toLowerCase()}`}
                      >
                        <div>
                          <strong>
                            Safety preview · {assessment.riskLevel} risk
                          </strong>
                          <small>{assessment.reason}</small>
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
                      </div>}

                      {runningTaskId === task.id && runtimeProgress.length > 0 && (
                        <div className="run-progress" aria-live="polite">
                          <div className="run-progress-heading">
                            <strong>
                              {activeRunKind === "review"
                                ? "Live structured review"
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

                      {latestRun && hasVisibleTruncation(latestRun) && (
                          <div className="run-evidence-warning" role="status">
                            Stored run evidence reached a safety bound. The run
                            ledger records which output, progress, diff, file list,
                            or workspace snapshot was truncated.
                          </div>
                        )}

                      {latestRun?.status === "interrupted" && (
                        <div className="run-evidence-warning" role="alert">
                          {latestRun.recoveryDisposition === "safe_to_retry"
                            ? "The previous run stopped before dispatch and is safe to retry."
                            : "The previous run may have reached the workspace. Inspect its files before retrying."}
                        </div>
                      )}

                      {specialistResult && (
                        <SpecialistResultView result={specialistResult} />
                      )}

                      {task.result && !specialistResult && (
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
                              Structured review · {task.reviewStatus}
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

                      {workspaceEvidence && (
                        <div className="workspace-evidence">
                          <div className="agent-result-heading">
                            <strong>
                              {workspaceEvidenceStatusLabel(workspaceEvidence)}
                            </strong>
                            <small>
                              {workspaceReviewabilityLabel(workspaceEvidence)} · observed during run
                            </small>
                          </div>
                          <div className="workspace-evidence-counts">
                            <span>{workspaceEvidence.summary.totalChanges} changes</span>
                            <span>{workspaceEvidence.summary.staged} staged</span>
                            <span>{workspaceEvidence.summary.unstaged} unstaged</span>
                            <span>{workspaceEvidence.summary.untracked} untracked</span>
                            <span>{workspaceEvidence.summary.binary} binary</span>
                            {workspaceEvidence.baselineGitHead !== workspaceEvidence.finalGitHead && (
                              <span>
                                HEAD {workspaceEvidence.baselineGitHead?.slice(0, 12) ?? "unborn"} → {workspaceEvidence.finalGitHead?.slice(0, 12) ?? "unborn"}
                              </span>
                            )}
                          </div>
                          {workspaceEvidenceHasVisibleLimit(workspaceEvidence) && (
                            <div className="run-evidence-warning" role="status">
                              Evidence is explicitly partial. Use human review and inspect the listed issues before relying on it.
                            </div>
                          )}
                          {workspaceEvidence.issues.length > 0 && (
                            <details className="evidence-issues">
                              <summary>
                                Collection issues ({workspaceEvidence.issues.length}
                                {workspaceEvidence.issuesTruncated ? "+" : ""})
                              </summary>
                              <ul>
                                {workspaceEvidence.issues.map((issue, index) => (
                                  <li key={`${issue.code}-${issue.path ?? index}`}>
                                    <code>{issue.code}</code> · {issue.message}
                                    {issue.path ? ` · ${issue.path}` : ""}
                                  </li>
                                ))}
                              </ul>
                            </details>
                          )}
                          {workspaceEvidence.changes.length > 0 && (
                            <div className="evidence-change-list">
                              {workspaceEvidence.changes.map((change) => {
                                const openable = workspaceChangeCanOpen(change);
                                const displayPath = change.previousPath
                                  ? `${change.previousPath} → ${change.path}`
                                  : change.path;
                                const state = change.after ?? change.before;
                                return (
                                  <div className="evidence-change-row" key={`${change.previousPath ?? ""}-${change.path}`}>
                                    {openable ? (
                                      <button
                                        type="button"
                                        className="file-chip"
                                        onClick={() => openTaskFile(task, change.path)}
                                      >
                                        {displayPath}
                                      </button>
                                    ) : (
                                      <span className="file-chip evidence-file-unavailable">
                                        {displayPath}
                                      </span>
                                    )}
                                    <small>
                                      {workspaceChangeLabel(change)}
                                      {change.gitAfter?.indexStatus ? ` · staged ${change.gitAfter.indexStatus}` : ""}
                                      {change.gitAfter?.worktreeStatus ? ` · unstaged ${change.gitAfter.worktreeStatus}` : ""}
                                      {change.gitAfter?.untracked ? " · untracked" : ""}
                                      {state?.sizeBytes !== null && state?.sizeBytes !== undefined
                                        ? ` · ${state.sizeBytes.toLocaleString()} bytes`
                                        : ""}
                                      {state?.sha256 ? ` · sha256 ${state.sha256.slice(0, 12)}…` : ""}
                                      {change.binary ? " · binary" : ""}
                                      {change.contentRedacted ? " · content redacted" : ""}
                                      {change.detailTruncated ? " · detail truncated" : ""}
                                      {!openable ? " · not openable from final workspace state" : ""}
                                    </small>
                                  </div>
                                );
                              })}
                            </div>
                          )}
                          {workspaceEvidence.details.some((detail) => detail.content) && (
                            <details className="diff-review">
                              <summary>Review bounded redacted details</summary>
                              {workspaceEvidence.details
                                .filter((detail) => detail.content)
                                .map((detail) => (
                                  <section key={`${detail.kind}-${detail.path}`}>
                                    <strong>{detail.path} · {detail.kind}</strong>
                                    <pre>{detail.content}</pre>
                                  </section>
                                ))}
                            </details>
                          )}
                        </div>
                      )}

                      {!workspaceEvidence && task.changedFiles.length > 0 && (
                        <div className="changed-files">
                          <strong>Legacy changed files ({task.changedFiles.length})</strong>
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
                          <summary>Review redacted compatibility diff</summary>
                          <pre>{task.diff}</pre>
                        </details>
                      )}
                    </div>

                    <div className="task-card-actions">
                      {specialistResult?.kind === "debugging" && (
                        <button
                          className="secondary-button"
                          disabled={runActive || taskMutationBusy}
                          onClick={() => prefillCodingTaskFromDebugging(task)}
                        >
                          Draft Coding fix
                        </button>
                      )}
                      {latestApprovalForTask(task)?.status === "Pending" && (
                          <button
                            className="primary-button"
                            disabled={runActive}
                            onClick={onOpenApprovals}
                          >
                            Review approval
                          </button>
                        )}

                      {(task.queueState === "queued" ||
                        task.queueState === "held") && (
                          <button
                            className="secondary-button"
                            disabled={runActive || taskMutationBusy}
                            onClick={() =>
                              void rerouteTask(
                                task,
                                task.routingMode === "automatic"
                                  ? "selected"
                                  : "automatic",
                              )
                            }
                          >
                            {task.routingMode === "automatic"
                              ? "Assign to owner"
                              : "Auto-route"}
                          </button>
                        )}

                      {task.queueState === "queued" && (
                        <button
                          className="primary-button"
                          disabled={
                            runActive ||
                            taskMutationBusy ||
                            !taskCanEnterExecuteSlot(
                              entry,
                              taskOrchestration.activeExecute,
                            )
                          }
                          onClick={() => void runTaskWithAgent(task)}
                        >
                          {runningTaskId === task.id
                            ? "Agent working…"
                            : entry?.queuePosition === 1
                              ? "Run queued task"
                              : `Waiting at queue #${entry?.queuePosition ?? "—"}`}
                        </button>
                      )}

                      {runningTaskId === task.id && (
                        <button
                          className="danger-button"
                          disabled={cancelRequested}
                          onClick={() => void cancelActiveRun()}
                        >
                          {cancelRequested ? "Stopping…" : "Stop agent"}
                        </button>
                      )}

                      {reviewFlow &&
                        (reviewFlow.state === "awaiting_review" ||
                          reviewFlow.state === "review_pending") && (
                          <button
                            className="primary-button"
                            disabled={runActive}
                            onClick={() =>
                              void runSeniorReview(
                                task,
                                undefined,
                                preferences.reviewMode === "automatic",
                              )
                            }
                          >
                            Run {reviewLevelLabel(reviewFlow.currentLevel)} review
                          </button>
                        )}

                      {reviewFlow?.state === "awaiting_human" && (
                        <>
                          <button
                            className="primary-button"
                            disabled={runActive}
                            onClick={() =>
                              void recordHumanReviewDecision(task, "approved")
                            }
                          >
                            Confirm human approval
                          </button>
                          <button
                            className="danger-button"
                            disabled={runActive}
                            onClick={() =>
                              void recordHumanReviewDecision(
                                task,
                                "changesRequested",
                              )
                            }
                          >
                            Request revision
                          </button>
                        </>
                      )}

                      {task.queueState === "queued" && (
                          <button
                            className="secondary-button"
                            disabled={runActive || taskMutationBusy}
                            onClick={() =>
                              void setQueueDisposition(task, "hold")
                            }
                          >
                            Hold
                          </button>
                        )}

                      {task.queueState === "held" && (
                          <button
                            className="secondary-button"
                            disabled={runActive || taskMutationBusy}
                            onClick={() =>
                              void setQueueDisposition(task, "resume")
                            }
                          >
                            Resume in original queue age
                          </button>
                        )}

                      {task.queueState === "notQueued" &&
                        (task.status === "Failed" ||
                          task.status === "Completed") && (
                        <button
                          className="secondary-button"
                          disabled={runActive || taskMutationBusy}
                          onClick={() =>
                            void setQueueDisposition(task, "resetTerminal")
                          }
                        >
                          Reset with new queue age
                        </button>
                      )}
                    </div>
                  </article>
                  );
                })}
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
                  Immutable run-ledger evidence followed by local configuration events.
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

            <div className="authoritative-run-list">
              <h3>Authoritative run ledger</h3>
              {selectedAgentRuns.length === 0 ? (
                <p className="page-message">No retained run attempts for this agent.</p>
              ) : (
                selectedAgentRuns.map((attempt) => (
                  <article className="agent-card" key={attempt.id}>
                    <div>
                      <h3>{attempt.taskTitle}</h3>
                      <p>
                        {attempt.runMode === "review" ? "Review" : "Execution"} · {attempt.status.replace(/_/g, " ")}
                      </p>
                      <small>
                        {workspaceEvidenceStatusLabel(attempt.workspaceChanges)} · {attempt.workspaceChanges.summary.totalChanges} observed changes
                      </small>
                    </div>
                  </article>
                ))
              )}
            </div>

            {selectedAgent.activity.length === 0 ? (
              <p className="page-message">
                No local configuration activity recorded yet.
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
        </div>
      </>
    );
  }

  const agentGroups = availableAgentGroups(agents);
  const effectiveAgentGroup = agentGroups.includes(activeAgentGroup)
    ? activeAgentGroup
    : agentGroups[0];
  const groupProjection = projectAgentGroup(agents, effectiveAgentGroup);
  const visibleAgents = groupProjection.visibleAgents;

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

      {runtimeError && <div className="inline-error">{runtimeError}</div>}

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

        <div className="workspace-tabs agent-group-tabs" role="group" aria-label="Agent groups">
          {agentGroups.map((group) => (
              <button
                key={group}
                type="button"
                aria-pressed={effectiveAgentGroup === group}
                className={effectiveAgentGroup === group ? "primary-button" : "secondary-button"}
                onClick={() => setActiveAgentGroup(group)}
              >
                {group}
              </button>
            ))}
        </div>

        <div className="agent-list">
          {groupProjection.rows.map(({ agent, depth, detached }) => (
            <article
              className="agent-card hierarchy-card"
              key={`hierarchy-${agent.id}`}
              style={{ marginLeft: `${depth * 28}px` }}
            >
              <KeyboardAction
                className="hierarchy-card-action"
                label={`${agent.registryState === "active" ? "Open" : "Edit"} ${agent.name}`}
                onActivate={() =>
                  agent.registryState === "active"
                    ? setSelectedAgentId(agent.id)
                    : openEditAgent(agent)
                }
              >
                <div>
                  <h3>{agent.name}</h3>
                  <p>{agent.description}</p>
                  <small>
                    {agent.role} · {agent.category} · Authority level {agent.authorityLevel}
                    {detached ? " · Detached hierarchy" : ""}
                  </small>
                  {agent.registryState === "unassigned" && (
                    <small className="registry-warning">
                      {registryIssueMessage(agent.registryIssue)}
                    </small>
                  )}
                </div>
                <span className={`agent-status ${agent.status.toLowerCase()}`}>
                  {agent.registryState === "unassigned" ? "Needs assignment" : agent.status}
                </span>
              </KeyboardAction>
            </article>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="agent-list">
          {visibleAgents.map((agent) => (
            <article
              className="agent-card team-agent-card"
              key={agent.id}
            >
              <KeyboardAction
                className="team-agent-summary"
                label={`${agent.registryState === "active" ? "Open" : "Edit"} ${agent.name}`}
                onActivate={() => {
                  if (agent.registryState === "active") {
                    setSelectedAgentId(agent.id);
                    setActiveWorkspaceTab("Overview");
                  } else {
                    openEditAgent(agent);
                  }
                }}
              >
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
              </KeyboardAction>

              <div className="team-agent-actions">
                <span
                  className={`agent-status ${agent.status.toLowerCase()}`}
                >
                  {agent.registryState === "unassigned" ? "Needs assignment" : agent.status}
                </span>

                {agent.registryState === "active" && (agent.status === "Working" ? (
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
                ))}

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
                    void deleteAgent(agent.id);
                  }}
                >
                  Delete
                </button>
              </div>
            </article>
          ))}
        </div>
      </section>

      {templates.some((template) => template.restorable) && (
        <section className="panel">
          <div className="panel-heading">
            <div>
              <span className="eyebrow">TEMPLATES</span>
              <h2>Restore a default agent</h2>
              <p className="page-message">
                Deleted defaults stay deleted until you explicitly restore their template.
              </p>
            </div>
          </div>
          <div className="agent-list">
            {templates
              .filter((template) => template.restorable)
              .map((template) => (
                <article className="agent-card" key={template.templateKey}>
                  <div>
                    <h3>{template.name}</h3>
                    <p>{template.description}</p>
                    <small>{template.role} · {template.category}</small>
                  </div>
                  <button
                    className="secondary-button"
                    onClick={() => void restoreTemplate(template.templateKey)}
                  >
                    Restore template
                  </button>
                </article>
              ))}
          </div>
        </section>
      )}

      <Dialog
        open={isModalOpen}
        labelledBy="agent-editor-title"
        onClose={resetForm}
      >
            <div className="modal-heading">
              <div>
                <span className="eyebrow">
                  {isEditing ? "EDIT AGENT" : "NEW AGENT"}
                </span>
                <h2 id="agent-editor-title">
                  {isEditing ? "Edit agent" : "Create agent"}
                </h2>
              </div>

              <button
                type="button"
                className="modal-close"
                aria-label="Close agent editor"
                onClick={resetForm}
              >
                ×
              </button>
            </div>

            <label className="form-field">
              <span>Agent name</span>
              <input
                type="text"
                data-dialog-initial-focus
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
                  <option value="">
                    {agentRole === "Supervisor" ? "Top level" : "Select a manager"}
                  </option>
                  {validManagerCandidates(agents, agentRole, editingAgentId)
                    .map((agent) => (
                      <option value={agent.id} key={agent.id}>
                        {agent.name} · {agent.role}
                      </option>
                    ))}
                </select>
              </label>

              <label className="form-field">
                <span>Authority level</span>
                <input value={authorityForRole(agentRole)} readOnly />
                <small>Authority is derived from the selected role.</small>
              </label>
            </div>

            <div className="modal-actions">
              <button type="button" className="secondary-button" onClick={resetForm}>
                Cancel
              </button>

              <button type="button" className="primary-button" onClick={saveAgent}>
                {isEditing ? "Save changes" : "Create agent"}
              </button>
            </div>
      </Dialog>
    </>
  );
}
