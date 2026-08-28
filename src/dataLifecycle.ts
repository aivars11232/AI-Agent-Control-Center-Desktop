import type { AgentTask, ApplicationState } from "./applicationState";

export type MonitoringRevision = {
  applicationState: number;
  taskOrchestration: number;
  runCoordinator: number;
  reviewOrchestration: number;
  dataLifecycle: number;
};

export type RetentionPruneCounts = {
  tasks: number;
  attempts: number;
  reviewFlows: number;
  activity: number;
  approvals: number;
  reminders: number;
  systemActionAudits: number;
  memoryRecords: number;
  reminderOccurrences: number;
  managementHandoffs: number;
};

export type RetentionMaintenanceResult = {
  lifecycleRevision: number;
  applicationStateRevision: number;
  triggerKind: "startup" | "interval" | "settings" | "import" | "test";
  status: "succeeded" | "failed" | "clock_rollback";
  startedAtUnixMs: number;
  completedAtUnixMs: number;
  taskCutoffUnixMs: number | null;
  activityCutoffUnixMs: number | null;
  pruned: RetentionPruneCounts;
  skippedProtected: number;
  backlogRemaining: boolean;
  errorCode: string | null;
  errorMessage: string | null;
};

export type MonitoringCounts = {
  configuredAgents: number;
  activeAgents: number;
  totalTasks: number;
  runningTasks: number;
  pendingTasks: number;
  blockedTasks: number;
  completedTasks: number;
  failedTasks: number;
  activityEntries: number;
  pendingApprovals: number;
  upcomingReminders: number;
  retainedRunAttempts: number;
  activeRunAttempts: number;
};

export type MonitoringSnapshot = {
  authoritative: boolean;
  generatedAtUnixMs: number;
  revision: MonitoringRevision;
  counts: MonitoringCounts;
  lifecycle: {
    taskRetention: string;
    activityRetention: string;
    lastObservedAtUnixMs: number | null;
    lastSuccessAtUnixMs: number | null;
    lastErrorCode: string | null;
    lastErrorMessage: string | null;
    totalRuns: number;
    totalPruned: RetentionPruneCounts;
    inferredTimestampCount: number;
    latestRun: RetentionMaintenanceResult | null;
  };
};

export type MonitoringTaskRecord = {
  ownerAgentId: number;
  ownerName: string;
  ownerRole: string;
  executorName: string | null;
  createdAtUnixMs: number;
  completedAtUnixMs: number | null;
  task: AgentTask;
};

export type MonitoringTaskPage = {
  authoritative: boolean;
  revision: MonitoringRevision;
  offset: number;
  limit: number;
  total: number;
  records: MonitoringTaskRecord[];
};

export type MonitoringActivityRecord = {
  ownerAgentId: number;
  ownerName: string;
  ownerRole: string;
  entryId: number;
  message: string;
  createdAt: string;
  createdAtUnixMs: number;
};

export type MonitoringActivityPage = {
  authoritative: boolean;
  revision: MonitoringRevision;
  offset: number;
  limit: number;
  total: number;
  records: MonitoringActivityRecord[];
};

export type MonitoringMutationResult = {
  deletedCount: number;
  snapshot: MonitoringSnapshot;
};

export type BackupRecordCounts = {
  agents: number;
  tasks: number;
  activity: number;
  models: number;
  approvalHistory: number;
  reminders: number;
  workspaces: number;
  memoryRecords: number;
};

export type BackupSanitizationCounts = {
  heldTasks: number;
  expiredApprovals: number;
  clearedTaskEvidence: number;
  disabledVoiceRuntime: boolean;
  portalDeliveriesDisabled: number;
};

export type BackupExport = {
  fileName: string;
  backupJson: string;
  byteLength: number;
  counts: BackupRecordCounts;
  sanitizations: BackupSanitizationCounts;
  omittedDomains: string[];
};

export type BackupImportPreview = {
  formatVersion: number;
  sourceSchemaVersion: number | null;
  byteLength: number;
  counts: BackupRecordCounts;
  sanitizations: BackupSanitizationCounts;
  omittedDomains: string[];
  replacesCurrentState: boolean;
  clearsRunAndReviewHistory: boolean;
  securityChangeSummary: string | null;
};

const zeroPruned: RetentionPruneCounts = {
  tasks: 0,
  attempts: 0,
  reviewFlows: 0,
  activity: 0,
  approvals: 0,
  reminders: 0,
  systemActionAudits: 0,
  memoryRecords: 0,
  reminderOccurrences: 0,
  managementHandoffs: 0,
};

export function previewMonitoringSnapshot(
  state: ApplicationState,
  retainedRunAttempts: number,
  activeRunAttempts: number,
): MonitoringSnapshot {
  const tasks = state.agents.flatMap((agent) => agent.tasks);
  return {
    authoritative: false,
    generatedAtUnixMs: Date.now(),
    revision: {
      applicationState: 0,
      taskOrchestration: 0,
      runCoordinator: 0,
      reviewOrchestration: 0,
      dataLifecycle: 0,
    },
    counts: {
      configuredAgents: state.agents.length,
      activeAgents: state.agents.filter((agent) => agent.status === "Working")
        .length,
      totalTasks: tasks.length,
      runningTasks: tasks.filter((task) => task.status === "Running").length,
      pendingTasks: tasks.filter((task) => task.status === "Pending").length,
      blockedTasks: tasks.filter((task) => task.status === "Blocked").length,
      completedTasks: tasks.filter((task) => task.status === "Completed").length,
      failedTasks: tasks.filter((task) => task.status === "Failed").length,
      activityEntries: state.agents.reduce(
        (total, agent) => total + agent.activity.length,
        0,
      ),
      pendingApprovals: state.approvalRequests.filter(
        (request) => request.status === "Pending",
      ).length,
      upcomingReminders: state.reminders.filter(
        (reminder) => reminder.status === "Upcoming",
      ).length,
      retainedRunAttempts,
      activeRunAttempts,
    },
    lifecycle: {
      taskRetention: String(state.taskRetentionDays),
      activityRetention: String(state.activityRetentionDays),
      lastObservedAtUnixMs: null,
      lastSuccessAtUnixMs: null,
      lastErrorCode: null,
      lastErrorMessage: null,
      totalRuns: 0,
      totalPruned: { ...zeroPruned },
      inferredTimestampCount: 0,
      latestRun: null,
    },
  };
}
