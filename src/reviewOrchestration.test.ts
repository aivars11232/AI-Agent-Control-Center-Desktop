import { describe, expect, it } from "vitest";
import {
  emptyReviewOrchestrationSnapshot,
  reviewFlowForTask,
  reviewFlowStatus,
  type ReviewFlow,
} from "./reviewOrchestration";

function flow(overrides: Partial<ReviewFlow> = {}): ReviewFlow {
  return {
    id: 1,
    taskOwnerAgentId: 2,
    taskId: 41,
    executorAgentId: 3,
    state: "awaiting_review",
    revisionRound: 0,
    maxRevisions: 3,
    currentLevel: "senior",
    requiredLevels: ["senior", "teamLeader", "supervisor"],
    latestExecutionAttemptId: 9,
    reviewMode: "manual",
    lastErrorCode: null,
    lastErrorMessage: null,
    createdAtUnixMs: 1,
    updatedAtUnixMs: 2,
    completedAtUnixMs: null,
    stages: [],
    ...overrides,
  };
}

describe("structured review orchestration projection", () => {
  it("selects the active backend flow ahead of terminal history", () => {
    const terminal = flow({ id: 1, state: "completed" });
    const active = flow({ id: 2, state: "awaiting_review" });
    expect(
      reviewFlowForTask(
        { revision: 4, flows: [terminal, active] },
        2,
        41,
      )?.id,
    ).toBe(2);
  });

  it("renders level and human-fallback states without choosing a reviewer", () => {
    expect(reviewFlowStatus(flow())).toBe("Senior review ready");
    expect(
      reviewFlowStatus(
        flow({ state: "awaiting_human", currentLevel: "teamLeader" }),
      ),
    ).toBe("Trusted human decision required");
    expect(emptyReviewOrchestrationSnapshot()).toEqual({
      revision: 0,
      flows: [],
    });
  });
});
