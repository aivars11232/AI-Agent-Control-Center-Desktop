import { describe, expect, it } from "vitest";
import {
  emptyTaskOrchestrationSnapshot,
  queueEntryForTask,
  queueStateLabel,
  taskCanEnterExecuteSlot,
  type TaskQueueEntry,
} from "./taskOrchestration";

function entry(
  overrides: Partial<TaskQueueEntry> = {},
): TaskQueueEntry {
  return {
    taskOwnerAgentId: 1,
    taskId: 101,
    assignedAgentId: 4,
    title: "Implement deterministic routing",
    priority: "High",
    queueState: "queued",
    enqueueSequence: 9,
    queuePosition: 1,
    ...overrides,
  };
}

describe("authoritative task queue projection", () => {
  it("uses backend-provided queue positions without re-sorting", () => {
    const second = entry({ taskId: 102, queuePosition: 2 });
    const first = entry();
    const snapshot = {
      ...emptyTaskOrchestrationSnapshot(),
      executeQueue: [second, first],
    };

    expect(queueEntryForTask(snapshot, 1, 101)).toBe(first);
    expect(queueStateLabel(first)).toBe("Queue position 1");
    expect(snapshot.executeQueue.map((item) => item.taskId)).toEqual([
      102, 101,
    ]);
  });

  it("only enables the authoritative head when the execute slot is free", () => {
    const head = entry();
    const later = entry({ taskId: 102, queuePosition: 2 });

    expect(taskCanEnterExecuteSlot(head, null)).toBe(true);
    expect(taskCanEnterExecuteSlot(later, null)).toBe(false);
    expect(taskCanEnterExecuteSlot(head, entry({ queueState: "running" }))).toBe(
      false,
    );
  });

  it("finds active and held tasks in their authoritative projections", () => {
    const active = entry({ queueState: "running", queuePosition: null });
    const held = entry({
      taskId: 103,
      queueState: "held",
      queuePosition: null,
    });
    const snapshot = {
      ...emptyTaskOrchestrationSnapshot(),
      activeExecute: active,
      heldTasks: [held],
    };

    expect(queueEntryForTask(snapshot, 1, 101)).toBe(active);
    expect(queueEntryForTask(snapshot, 1, 103)).toBe(held);
    expect(queueStateLabel(held)).toBe("Held outside admission");
  });
});
