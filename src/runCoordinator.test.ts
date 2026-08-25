import { describe, expect, it } from "vitest";
import {
  applyRunCoordinatorEvent,
  applyRunCoordinatorSnapshot,
  createRunCoordinatorUiState,
  markRunStopRequested,
  type RunAttempt,
  type RunCoordinatorEvent,
  type RunCoordinatorSnapshot,
} from "./runCoordinator";
import { unavailableWorkspaceEvidence } from "./workspaceEvidence";

function attempt(id: number): RunAttempt {
  return {
    id,
    requestId: `request-${id}`,
    agentId: 2,
    taskOwnerAgentId: 2,
    taskId: 41,
    taskTitle: "Fixture task",
    runMode: "execute",
    status: "running",
    provider: "OpenAI",
    model: "test-model",
    workspaceId: "workspace",
    approvalId: null,
    reviewFlowId: null,
    reviewStageAttemptId: null,
    reviewRevisionRound: null,
    admittedAtUnixMs: 1,
    startedAtUnixMs: 2,
    cancelRequestedAtUnixMs: null,
    completedAtUnixMs: null,
    durationSeconds: null,
    outputSummary: null,
    stderrExcerpt: null,
    responseId: null,
    usage: { inputTokens: null, outputTokens: null, totalTokens: null },
    changedFiles: [],
    diff: null,
    workspaceChanges: unavailableWorkspaceEvidence(),
    errorCode: null,
    errorMessage: null,
    progressEventCount: 0,
    recoveryDisposition: null,
    truncation: {
      stdoutTruncated: false,
      stderrTruncated: false,
      summaryTruncated: false,
      diffTruncated: false,
      changedFilesTruncated: false,
      progressTruncated: false,
      beforeSnapshotTruncated: false,
      afterSnapshotTruncated: false,
      originalStdoutBytes: 0,
      originalStderrBytes: 0,
      originalSummaryBytes: 0,
      originalDiffBytes: 0,
      originalChangedFileCount: 0,
      omittedProgressEventCount: 0,
    },
  };
}

function snapshot(revision: number, activeAttempt: RunAttempt | null): RunCoordinatorSnapshot {
  return {
    revision,
    activeAttempt,
    recentAttempts: [],
    retainedAttemptCount: activeAttempt ? 1 : 0,
    retainedPayloadBytes: 0,
    prunedAttemptCount: 0,
    lastPrunedAtUnixMs: null,
  };
}

describe("authoritative run coordinator projection", () => {
  it("resets local progress only when the authoritative attempt changes", () => {
    const first = applyRunCoordinatorSnapshot(
      createRunCoordinatorUiState(),
      snapshot(1, attempt(1)),
    );
    const event: RunCoordinatorEvent = {
      coordinatorRevision: 2,
      attemptId: 1,
      requestId: "request-1",
      sequence: 1,
      kind: "progress",
      status: "running",
      message: "working",
      messageTruncated: false,
      createdAtUnixMs: 3,
    };
    const progressed = applyRunCoordinatorEvent(first, event);
    expect(progressed.progress).toEqual(["working"]);
    expect(
      applyRunCoordinatorSnapshot(progressed, snapshot(2, attempt(1))).progress,
    ).toEqual(["working"]);
    expect(
      applyRunCoordinatorSnapshot(progressed, snapshot(3, attempt(2))).progress,
    ).toEqual([]);
  });

  it("ignores stale and cross-attempt events", () => {
    const state = applyRunCoordinatorSnapshot(
      createRunCoordinatorUiState(),
      snapshot(5, attempt(5)),
    );
    const stale = applyRunCoordinatorEvent(state, {
      coordinatorRevision: 4,
      attemptId: 5,
      requestId: "request-5",
      sequence: 1,
      kind: "progress",
      status: "running",
      message: "stale",
      messageTruncated: false,
      createdAtUnixMs: 6,
    });
    const wrongAttempt = applyRunCoordinatorEvent(state, {
      coordinatorRevision: 6,
      attemptId: 6,
      requestId: "request-6",
      sequence: 1,
      kind: "progress",
      status: "running",
      message: "wrong",
      messageTruncated: false,
      createdAtUnixMs: 6,
    });
    expect(stale).toBe(state);
    expect(wrongAttempt).toBe(state);
  });

  it("keeps stop state global until the backend clears the active attempt", () => {
    const active = applyRunCoordinatorSnapshot(
      createRunCoordinatorUiState(),
      snapshot(1, attempt(1)),
    );
    const stopping = markRunStopRequested(active, true);
    expect(stopping.stopRequested).toBe(true);
    const completed = applyRunCoordinatorSnapshot(stopping, snapshot(2, null));
    expect(completed.stopRequested).toBe(false);
  });
});
