import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ApprovalRequest,
  RuntimeProviderId,
  StateEnvelope,
} from "../applicationState";
import type { AgentRegistrySnapshot } from "../agentRegistry";
import type { InvokeFunction } from "../persistence";
import type { ProviderRegistrySnapshot } from "../providerRegistry";
import type {
  ReviewIntentContext,
  ReviewOrchestrationSnapshot,
  ReviewStageStart,
  ReviewVerdict,
} from "../reviewOrchestration";
import type {
  RunCoordinatorEvent,
  RunCoordinatorSnapshot,
} from "../runCoordinator";
import type { TaskOrchestrationSnapshot } from "../taskOrchestration";
import type { WorkspaceChangeEvidence } from "../workspaceEvidence";
import type {
  BackupExport,
  BackupImportPreview,
  MonitoringActivityPage,
  MonitoringMutationResult,
  MonitoringRevision,
  MonitoringSnapshot,
  MonitoringTaskPage,
} from "../dataLifecycle";
import type { CanonicalVoiceIntent } from "../voiceCommand";

export type AgentRunResult = {
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
  workspaceChanges: WorkspaceChangeEvidence;
  durationSeconds: number;
};

export type VoiceRuntimeStatus = {
  installed: boolean;
  listening: boolean;
  highAccuracyAvailable: boolean;
  message: string;
};

export type VoiceTranscriptEvent = {
  kind:
    | "activated"
    | "deactivated"
    | "off_requested"
    | "listening"
    | "ready"
    | "error"
    | "command"
    | "heard";
  transcript: string;
};

export type DesktopControlStatus = {
  enabled: boolean;
  message: string;
};

export type SubmitVoiceIntentRequest = {
  requestId: string;
  intent: CanonicalVoiceIntent;
};

export type SystemActionAuditRecord = {
  id: number;
  requestId: string;
  requestFingerprint: string;
  intentKind: string;
  riskClass: string;
  targetKind: string;
  targetId: string;
  agentId: number;
  taskOwnerAgentId: number | null;
  taskId: number | null;
  approvalId: number | null;
  authorizationKind: string;
  intentFingerprintSha256: string;
  policyFingerprintSha256: string;
  status:
    | "approvalRequired"
    | "taskCreated"
    | "applied"
    | "dispatched"
    | "rejected"
    | "failed"
    | "uncertain";
  detailCode: string | null;
  detailMessage: string | null;
  contentSha256: string | null;
  contentLength: number | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
};

export type SystemActionAuditPage = {
  records: SystemActionAuditRecord[];
  limit: number;
};

export type VoiceIntentResult = {
  requestId: string;
  status: SystemActionAuditRecord["status"];
  message: string;
  approval: ApprovalRequest | null;
  taskOwnerAgentId: number | null;
  taskId: number | null;
  audit: SystemActionAuditRecord;
};

export type BackendActionIntent =
  | {
      kind: "runTask";
      agentId: number;
      taskOwnerAgentId: number;
      taskId: number;
      runMode: "execute" | "review";
      reviewContext?: ReviewIntentContext;
    }
  | {
      kind: "openWorkspaceItem";
      agentId: number;
      workspaceId: string;
      itemPath: string;
    }
  | { kind: "enableDesktopControl"; agentId: number }
  | { kind: "installVoiceRuntime"; agentId: number }
  | { kind: "installHighAccuracyVoiceRuntime"; agentId: number }
  | { kind: "startVoiceListener"; agentId: number };

export type AuthorizationOutcome = {
  decision: "allowed" | "approvalRequired";
  approval: ApprovalRequest | null;
  evidence: {
    agentId: number;
    intentFingerprint: string;
    policyFingerprint: string;
    riskLevel: string;
  };
};

export type RunAgentTaskRequest = {
  runId: string;
  runMode: "execute" | "review";
  agentId: number;
  taskOwnerAgentId: number;
  taskId: number;
  reviewContext?: ReviewIntentContext;
};

export type DesktopListenFunction = <T>(
  eventName: string,
  handler: (payload: T) => void,
) => Promise<() => void>;

const invokeDesktop: InvokeFunction = <T,>(
  command: string,
  args?: Record<string, unknown>,
) => invoke<T>(command, args);

const listenDesktop: DesktopListenFunction = <T,>(
  eventName: string,
  handler: (payload: T) => void,
) => listen<T>(eventName, (event) => handler(event.payload));

export function createDesktopClient(
  invokeFn: InvokeFunction,
  listenFn: DesktopListenFunction,
) {
  return {
    invokeApplicationState: invokeFn,

    requestAuthorization(intent: BackendActionIntent) {
      return invokeFn<AuthorizationOutcome>("request_authorization", {
        intent,
      });
    },

    resolveApproval(
      approvalId: number,
      resolution: "approve" | "deny",
    ) {
      return invokeFn<ApprovalRequest>("resolve_approval", {
        request: { approvalId, resolution },
      });
    },

    enableDesktopControl(agentId: number) {
      return invokeFn<DesktopControlStatus>("enable_desktop_control", {
        agentId,
      });
    },

    desktopControlStatus() {
      return invokeFn<DesktopControlStatus>("desktop_control_status");
    },

    submitVoiceIntent(request: SubmitVoiceIntentRequest) {
      return invokeFn<VoiceIntentResult>("submit_voice_intent", { request });
    },

    querySystemActionAudits(limit = 50) {
      return invokeFn<SystemActionAuditPage>("query_system_action_audits", {
        limit,
      });
    },

    reviewOrchestrationSnapshot() {
      return invokeFn<ReviewOrchestrationSnapshot>(
        "review_orchestration_snapshot",
      );
    },

    startReviewStage(request: {
      expectedRevision: number;
      taskOwnerAgentId: number;
      taskId: number;
    }) {
      return invokeFn<ReviewStageStart>("start_review_stage", { request });
    },

    recordHumanReviewDecision(request: {
      expectedRevision: number;
      taskOwnerAgentId: number;
      taskId: number;
      flowId: number;
      verdict: ReviewVerdict;
      feedback: string;
    }) {
      return invokeFn<ReviewOrchestrationSnapshot>(
        "record_human_review_decision",
        { request },
      );
    },

    runAgentTask(request: RunAgentTaskRequest) {
      return invokeFn<AgentRunResult>("run_agent_task", { request });
    },

    cancelAgentRun(runId: string) {
      return invokeFn<boolean>("cancel_agent_run", { runId });
    },

    openWorkspaceItem(request: {
      agentId: number;
      workspaceId: string;
      itemPath: string;
    }) {
      return invokeFn<void>("open_workspace_item", { request });
    },

    voiceRuntimeStatus() {
      return invokeFn<VoiceRuntimeStatus>("voice_runtime_status");
    },

    onVoiceRuntimeStatus(handler: (status: VoiceRuntimeStatus) => void) {
      return listenFn<VoiceRuntimeStatus>("voice-runtime-status", handler);
    },

    onVoiceTranscript(handler: (event: VoiceTranscriptEvent) => void) {
      return listenFn<VoiceTranscriptEvent>("voice-transcript", handler);
    },

    installVoiceRuntime(agentId: number) {
      return invokeFn<void>("install_voice_runtime", { agentId });
    },

    installHighAccuracyVoiceRuntime(agentId: number) {
      return invokeFn<void>("install_high_accuracy_voice_runtime", {
        agentId,
      });
    },

    startVoiceListener(agentId: number) {
      return invokeFn<void>("start_voice_listener", { agentId });
    },

    stopVoiceListener() {
      return invokeFn<void>("stop_voice_listener");
    },

    chooseWorkspaceFolder() {
      return invokeFn<string | null>("choose_workspace_folder");
    },

    agentRegistrySnapshot() {
      return invokeFn<AgentRegistrySnapshot>("agent_registry_snapshot");
    },

    taskOrchestrationSnapshot() {
      return invokeFn<TaskOrchestrationSnapshot>(
        "task_orchestration_snapshot",
      );
    },

    loadApplicationState() {
      return invokeFn<StateEnvelope | null>("load_application_state");
    },

    runCoordinatorSnapshot() {
      return invokeFn<RunCoordinatorSnapshot>("run_coordinator_snapshot");
    },

    monitoringSnapshot() {
      return invokeFn<MonitoringSnapshot>("monitoring_snapshot");
    },

    queryMonitoringTasks(request: {
      expectedRevision: MonitoringRevision;
      status: string | null;
      category: string | null;
      offset: number;
      limit: number;
    }) {
      return invokeFn<MonitoringTaskPage>("query_monitoring_tasks", {
        request,
      });
    },

    queryMonitoringActivity(request: {
      expectedRevision: MonitoringRevision;
      offset: number;
      limit: number;
    }) {
      return invokeFn<MonitoringActivityPage>("query_monitoring_activity", {
        request,
      });
    },

    deleteMonitoringActivity(request: {
      expectedRevision: MonitoringRevision;
      ownerAgentId: number;
      entryId: number;
    }) {
      return invokeFn<MonitoringMutationResult>(
        "delete_monitoring_activity",
        { request },
      );
    },

    clearMonitoringActivity(expectedRevision: MonitoringRevision) {
      return invokeFn<MonitoringMutationResult>("clear_monitoring_activity", {
        request: { expectedRevision },
      });
    },

    exportBackup() {
      return invokeFn<BackupExport>("export_backup");
    },

    previewBackupImport(
      expectedRevision: number,
      backupJson: string,
    ) {
      return invokeFn<BackupImportPreview>("preview_backup_import", {
        request: { expectedRevision, backupJson },
      });
    },

    applyBackupImport(expectedRevision: number, backupJson: string) {
      return invokeFn<StateEnvelope>("apply_backup_import", {
        request: { expectedRevision, backupJson },
      });
    },

    onRunCoordinatorEvent(handler: (event: RunCoordinatorEvent) => void) {
      return listenFn<RunCoordinatorEvent>("run-coordinator-event", handler);
    },

    onRunCoordinatorSnapshot(
      handler: (snapshot: RunCoordinatorSnapshot) => void,
    ) {
      return listenFn<RunCoordinatorSnapshot>(
        "run-coordinator-snapshot",
        handler,
      );
    },

    providerRegistryStatus() {
      return invokeFn<ProviderRegistrySnapshot>("provider_registry_status");
    },

    onVoiceControlOpen(handler: () => void) {
      return listenFn<unknown>("voice-control-open", handler);
    },
  };
}

export const desktopClient = createDesktopClient(
  invokeDesktop,
  listenDesktop,
);

export function isDesktopRuntime(): boolean {
  return (
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  );
}
