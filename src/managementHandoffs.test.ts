import { describe, expect, it } from "vitest";
import { handoffsForAgent, type ManagementHandoffSnapshot } from "./managementHandoffs";

describe("TASK-0018 management workspace projection", () => {
  it("shows only direct or managed-chain handoffs", () => {
    const snapshot: ManagementHandoffSnapshot = {
      revision: 1,
      applicationStateRevision: 1,
      handoffs: [
        {
          id: 1,
          taskOwnerAgentId: 10,
          taskId: 1,
          kind: "task_plan",
          fromAgentId: 10,
          toAgentId: 11,
          ownerRole: "team_leader",
          revisionRound: 0,
          runAttemptId: null,
          reviewFlowId: null,
          reviewStageAttemptId: null,
          source: "task_orchestration",
          summary: "Visible",
          payload: {},
          idempotencyKey: "one",
          createdAtUnixMs: 1,
        },
        {
          id: 2,
          taskOwnerAgentId: 20,
          taskId: 2,
          kind: "task_plan",
          fromAgentId: 20,
          toAgentId: 21,
          ownerRole: "team_leader",
          revisionRound: 0,
          runAttemptId: null,
          reviewFlowId: null,
          reviewStageAttemptId: null,
          source: "task_orchestration",
          summary: "Hidden",
          payload: {},
          idempotencyKey: "two",
          createdAtUnixMs: 2,
        },
      ],
    };
    expect(handoffsForAgent(snapshot, 1, [10]).map((handoff) => handoff.id)).toEqual([1]);
  });
});
