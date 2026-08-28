import { describe, expect, it } from "vitest";
import {
  classifyScheduledItem,
  defaultLocalDateTime,
  type ScheduledItem,
} from "./reminderScheduler";

function item(dueAtUnixMs: number | null): ScheduledItem {
  return {
    id: 1,
    position: 0,
    revision: 1,
    kind: "reminder",
    title: "Fixture",
    notes: "",
    localDueAt: "2026-08-28T12:00:00",
    timeZone: "UTC",
    dueAt: "2026-08-28T12:00:00Z",
    dueAtUnixMs,
    eventEndLocal: null,
    eventEndUnixMs: null,
    dstResolution: "exact",
    status: "scheduled",
    recurrence: { kind: "none", interval: 1, occurrenceLimit: null, untilUnixMs: null },
    nextOccurrenceSequence: 0,
    missedOccurrenceCount: 0,
    deliveryMode: "in_app",
    privacyMode: "generic",
    scheduleFingerprint: null,
    subjectAgentId: null,
    workspaceId: null,
    taskOwnerAgentId: null,
    taskId: null,
    schedulerAgentId: null,
    scheduleIssueCode: null,
    scheduleIssueMessage: null,
    createdAt: "2026-08-28T10:00:00Z",
    createdAtUnixMs: 1,
    resolvedAtUnixMs: null,
    updatedAtUnixMs: 1,
  };
}

describe("TASK-0018 reminder projection", () => {
  it("keeps overdue items out of the due-soon window", () => {
    expect(classifyScheduledItem(item(99), 100, 200)).toBe("overdue");
    expect(classifyScheduledItem(item(100), 100, 200)).toBe("due_now");
    expect(classifyScheduledItem(item(150), 100, 200)).toBe("due_soon");
  });

  it("creates datetime-local defaults in local wall-clock form", () => {
    const value = defaultLocalDateTime(new Date("2026-08-28T10:15:20Z"));
    expect(value).toMatch(/^2026-08-28T\d{2}:15$/);
    expect(value.endsWith("Z")).toBe(false);
  });
});

