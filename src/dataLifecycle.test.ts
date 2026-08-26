import { describe, expect, it } from "vitest";
import { createDefaultApplicationState } from "./applicationState";
import { previewMonitoringSnapshot } from "./dataLifecycle";

describe("browser monitoring preview", () => {
  it("is explicitly non-authoritative while preserving bounded local counts", () => {
    const state = createDefaultApplicationState();
    state.agents[0].activity.push({
      id: 1,
      message: "Local preview event",
      createdAt: "2026-08-26T10:00:00.000Z",
    });

    const snapshot = previewMonitoringSnapshot(state, 0, 0);

    expect(snapshot.authoritative).toBe(false);
    expect(snapshot.counts.configuredAgents).toBe(state.agents.length);
    expect(snapshot.counts.activityEntries).toBe(1);
    expect(snapshot.lifecycle.totalRuns).toBe(0);
    expect(snapshot.lifecycle.latestRun).toBeNull();
  });
});
