export type ManagementHandoffKind =
  | "task_plan"
  | "assignment"
  | "execution_evidence"
  | "review_decision"
  | "revision_request"
  | "human_override"
  | "failure"
  | "recovery";
export type ManagementOwnerRole = "senior" | "team_leader" | "supervisor" | "human";
export type ManagementHandoffSource =
  | "task_orchestration"
  | "run_coordinator"
  | "review_orchestration"
  | "human_decision"
  | "migration_v11";

export type ManagementHandoff = {
  id: number;
  taskOwnerAgentId: number;
  taskId: number;
  kind: ManagementHandoffKind;
  fromAgentId: number | null;
  toAgentId: number | null;
  ownerRole: ManagementOwnerRole;
  revisionRound: number;
  runAttemptId: number | null;
  reviewFlowId: number | null;
  reviewStageAttemptId: number | null;
  source: ManagementHandoffSource;
  summary: string;
  payload: unknown;
  idempotencyKey: string;
  createdAtUnixMs: number;
};

export type ManagementHandoffSnapshot = {
  revision: number;
  applicationStateRevision: number;
  handoffs: ManagementHandoff[];
};

export const emptyManagementHandoffSnapshot: ManagementHandoffSnapshot = {
  revision: 0,
  applicationStateRevision: 0,
  handoffs: [],
};

export function handoffsForAgent(
  snapshot: ManagementHandoffSnapshot,
  agentId: number,
  managedAgentIds: number[],
): ManagementHandoff[] {
  return snapshot.handoffs.filter(
    (handoff) =>
      handoff.taskOwnerAgentId === agentId ||
      handoff.fromAgentId === agentId ||
      handoff.toAgentId === agentId ||
      managedAgentIds.includes(handoff.taskOwnerAgentId) ||
      (handoff.fromAgentId !== null && managedAgentIds.includes(handoff.fromAgentId)) ||
      (handoff.toAgentId !== null && managedAgentIds.includes(handoff.toAgentId)),
  );
}
