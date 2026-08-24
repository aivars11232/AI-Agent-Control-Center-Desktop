export type TaskQueueState =
  | "queued"
  | "held"
  | "admitted"
  | "running"
  | "notQueued";

export type RouteScoreComponent = {
  code: string;
  points: number;
};

export type RouteDisqualification = {
  code: string;
  message: string;
};

export type RouteCandidateEvidence = {
  agentId: number;
  agentName: string;
  category: string;
  role: string;
  model: string;
  eligible: boolean;
  disqualifications: RouteDisqualification[];
  score: number;
  scoreComponents: RouteScoreComponent[];
  workload: number;
  queueThreshold: number;
  overloaded: boolean;
  overflowAction: string;
  redirectAgentId: number | null;
  selectionExcludedCode: string | null;
};

export type RoutingEvidence = {
  algorithmVersion: string;
  routingMode: "selected" | "automatic";
  preferredAgentId: number | null;
  selectedAgentId: number | null;
  winningAgentId: number;
  outcomeCode: string;
  reason: string;
  manualOverride: boolean;
  candidates: RouteCandidateEvidence[];
};

export type TaskQueueEntry = {
  taskOwnerAgentId: number;
  taskId: number;
  assignedAgentId: number;
  title: string;
  priority: "Low" | "Normal" | "High" | "Critical";
  queueState: TaskQueueState;
  enqueueSequence: number;
  queuePosition: number | null;
};

export type TaskOrchestrationSnapshot = {
  revision: number;
  executeQueue: TaskQueueEntry[];
  heldTasks: TaskQueueEntry[];
  activeExecute: TaskQueueEntry | null;
};

export const emptyTaskOrchestrationSnapshot =
  (): TaskOrchestrationSnapshot => ({
    revision: 0,
    executeQueue: [],
    heldTasks: [],
    activeExecute: null,
  });

export function queueEntryForTask(
  snapshot: TaskOrchestrationSnapshot,
  taskOwnerAgentId: number,
  taskId: number,
): TaskQueueEntry | null {
  if (
    snapshot.activeExecute?.taskOwnerAgentId === taskOwnerAgentId &&
    snapshot.activeExecute.taskId === taskId
  ) {
    return snapshot.activeExecute;
  }

  return (
    snapshot.executeQueue.find(
      (entry) =>
        entry.taskOwnerAgentId === taskOwnerAgentId && entry.taskId === taskId,
    ) ??
    snapshot.heldTasks.find(
      (entry) =>
        entry.taskOwnerAgentId === taskOwnerAgentId && entry.taskId === taskId,
    ) ??
    null
  );
}

export function queueStateLabel(entry: TaskQueueEntry | null): string {
  if (!entry) {
    return "Outside execute queue";
  }
  if (entry.queueState === "queued") {
    return entry.queuePosition === null
      ? "Queued"
      : `Queue position ${entry.queuePosition}`;
  }
  if (entry.queueState === "held") {
    return "Held outside admission";
  }
  if (entry.queueState === "admitted") {
    return "Admitted for dispatch";
  }
  if (entry.queueState === "running") {
    return "Running in the single execute slot";
  }
  return "Outside execute queue";
}

export function taskCanEnterExecuteSlot(
  entry: TaskQueueEntry | null,
  activeExecute: TaskQueueEntry | null,
): boolean {
  return (
    entry?.queueState === "queued" &&
    entry.queuePosition === 1 &&
    activeExecute === null
  );
}
