export type ReviewLevel = "senior" | "teamLeader" | "supervisor";

export type ReviewVerdict = "approved" | "changesRequested";

export type ReviewIntentContext = {
  flowId: number;
  stageAttemptId: number;
  revisionRound: number;
  level: ReviewLevel;
  requestFingerprint: string;
};

export type ReviewStageAttempt = {
  id: number;
  flowId: number;
  revisionRound: number;
  level: ReviewLevel;
  attemptNumber: number;
  actor: "agent" | "human";
  reviewerAgentId: number | null;
  state:
    | "pending"
    | "admitted"
    | "running"
    | "approved"
    | "changes_requested"
    | "invalid"
    | "cancelled"
    | "failed"
    | "interrupted";
  requestFingerprint: string;
  verdict: ReviewVerdict | null;
  feedback: string | null;
  runAttemptId: number | null;
  errorCode: string | null;
  errorMessage: string | null;
  createdAtUnixMs: number;
  startedAtUnixMs: number | null;
  completedAtUnixMs: number | null;
};

export type ReviewFlow = {
  id: number;
  taskOwnerAgentId: number;
  taskId: number;
  executorAgentId: number;
  state:
    | "awaiting_execution"
    | "awaiting_review"
    | "review_pending"
    | "reviewing"
    | "awaiting_human"
    | "revision_queued"
    | "completed"
    | "failed"
    | "cancelled";
  revisionRound: number;
  maxRevisions: number;
  currentLevel: ReviewLevel | null;
  requiredLevels: ReviewLevel[];
  latestExecutionAttemptId: number | null;
  reviewMode: "manual" | "automatic";
  lastErrorCode: string | null;
  lastErrorMessage: string | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
  completedAtUnixMs: number | null;
  stages: ReviewStageAttempt[];
};

export type ReviewOrchestrationSnapshot = {
  revision: number;
  flows: ReviewFlow[];
};

export type ReviewStageStart = {
  snapshot: ReviewOrchestrationSnapshot;
  stage: ReviewStageAttempt | null;
  context: ReviewIntentContext | null;
  blockedCode: string | null;
  blockedMessage: string | null;
};

export const emptyReviewOrchestrationSnapshot =
  (): ReviewOrchestrationSnapshot => ({ revision: 0, flows: [] });

export function reviewFlowForTask(
  snapshot: ReviewOrchestrationSnapshot,
  taskOwnerAgentId: number,
  taskId: number,
): ReviewFlow | null {
  return (
    snapshot.flows.find(
      (flow) =>
        flow.taskOwnerAgentId === taskOwnerAgentId &&
        flow.taskId === taskId &&
        !["completed", "failed", "cancelled"].includes(flow.state),
    ) ??
    snapshot.flows.find(
      (flow) =>
        flow.taskOwnerAgentId === taskOwnerAgentId && flow.taskId === taskId,
    ) ??
    null
  );
}

export function reviewLevelLabel(level: ReviewLevel | null): string {
  if (level === "senior") return "Senior";
  if (level === "teamLeader") return "Team Leader";
  if (level === "supervisor") return "Supervisor";
  return "Human";
}

export function reviewFlowStatus(flow: ReviewFlow): string {
  switch (flow.state) {
    case "awaiting_execution":
    case "revision_queued":
      return `Revision ${flow.revisionRound} queued`;
    case "awaiting_review":
      return `${reviewLevelLabel(flow.currentLevel)} review ready`;
    case "review_pending":
      return `${reviewLevelLabel(flow.currentLevel)} review admitted`;
    case "reviewing":
      return `${reviewLevelLabel(flow.currentLevel)} review running`;
    case "awaiting_human":
      return "Trusted human decision required";
    case "completed":
      return "Review pipeline approved";
    case "failed":
      return "Review pipeline failed";
    case "cancelled":
      return "Review pipeline cancelled";
  }
}
