import type { WorkspaceChangeEvidence } from "./workspaceEvidence";
import type {
  SpecialistResult,
  SpecialistRunContract,
} from "./specialistCapabilities";

export type RunAttemptMode = "execute" | "review";

export type RunAttemptStatus =
  | "admitted"
  | "starting"
  | "dispatching"
  | "running"
  | "cancel_requested"
  | "succeeded"
  | "cancelled"
  | "timed_out"
  | "startup_failed"
  | "failed"
  | "interrupted";

export type RunTruncationEvidence = {
  stdoutTruncated: boolean;
  stderrTruncated: boolean;
  summaryTruncated: boolean;
  diffTruncated: boolean;
  changedFilesTruncated: boolean;
  progressTruncated: boolean;
  beforeSnapshotTruncated: boolean;
  afterSnapshotTruncated: boolean;
  originalStdoutBytes: number;
  originalStderrBytes: number;
  originalSummaryBytes: number;
  originalDiffBytes: number;
  originalChangedFileCount: number;
  omittedProgressEventCount: number;
};

export type RunUsage = {
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
};

export type RunAttempt = {
  id: number;
  requestId: string;
  agentId: number;
  taskOwnerAgentId: number;
  taskId: number;
  taskTitle: string;
  runMode: RunAttemptMode;
  status: RunAttemptStatus;
  provider: string | null;
  model: string | null;
  workspaceId: string | null;
  approvalId: number | null;
  reviewFlowId: number | null;
  reviewStageAttemptId: number | null;
  reviewRevisionRound: number | null;
  admittedAtUnixMs: number;
  startedAtUnixMs: number | null;
  cancelRequestedAtUnixMs: number | null;
  completedAtUnixMs: number | null;
  durationSeconds: number | null;
  outputSummary: string | null;
  stderrExcerpt: string | null;
  responseId: string | null;
  usage: RunUsage;
  changedFiles: string[];
  diff: string | null;
  workspaceChanges: WorkspaceChangeEvidence;
  specialistContract: SpecialistRunContract | null;
  specialistResult: SpecialistResult | null;
  errorCode: string | null;
  errorMessage: string | null;
  progressEventCount: number;
  recoveryDisposition: string | null;
  truncation: RunTruncationEvidence;
};

export type RunCoordinatorSnapshot = {
  revision: number;
  activeAttempt: RunAttempt | null;
  recentAttempts: RunAttempt[];
  retainedAttemptCount: number;
  retainedPayloadBytes: number;
  prunedAttemptCount: number;
  lastPrunedAtUnixMs: number | null;
};

export type RunCoordinatorEvent = {
  coordinatorRevision: number;
  attemptId: number;
  requestId: string;
  sequence: number;
  kind: "status" | "progress" | "complete" | "error";
  status: RunAttemptStatus;
  message: string;
  messageTruncated: boolean;
  createdAtUnixMs: number;
};

export type RunCoordinatorUiState = {
  snapshot: RunCoordinatorSnapshot;
  progress: string[];
  stopRequested: boolean;
  lastEventRevision: number;
};

export const EMPTY_RUN_COORDINATOR_SNAPSHOT: RunCoordinatorSnapshot = {
  revision: 0,
  activeAttempt: null,
  recentAttempts: [],
  retainedAttemptCount: 0,
  retainedPayloadBytes: 0,
  prunedAttemptCount: 0,
  lastPrunedAtUnixMs: null,
};

export function createRunCoordinatorUiState(): RunCoordinatorUiState {
  return {
    snapshot: EMPTY_RUN_COORDINATOR_SNAPSHOT,
    progress: [],
    stopRequested: false,
    lastEventRevision: 0,
  };
}

export function applyRunCoordinatorSnapshot(
  current: RunCoordinatorUiState,
  snapshot: RunCoordinatorSnapshot,
): RunCoordinatorUiState {
  if (snapshot.revision < current.lastEventRevision) {
    return current;
  }
  const activeChanged =
    current.snapshot.activeAttempt?.id !== snapshot.activeAttempt?.id;
  return {
    snapshot,
    progress: activeChanged ? [] : current.progress,
    stopRequested: snapshot.activeAttempt
      ? activeChanged
        ? false
        : current.stopRequested
      : false,
    lastEventRevision: snapshot.revision,
  };
}

export function applyRunCoordinatorEvent(
  current: RunCoordinatorUiState,
  event: RunCoordinatorEvent,
): RunCoordinatorUiState {
  if (
    event.coordinatorRevision <= current.lastEventRevision ||
    current.snapshot.activeAttempt?.id !== event.attemptId ||
    current.snapshot.activeAttempt.requestId !== event.requestId
  ) {
    return current;
  }
  return {
    ...current,
    progress: [...current.progress, event.message].slice(-14),
    lastEventRevision: event.coordinatorRevision,
  };
}

export function markRunStopRequested(
  current: RunCoordinatorUiState,
  requested: boolean,
): RunCoordinatorUiState {
  if (!current.snapshot.activeAttempt) {
    return current;
  }
  return { ...current, stopRequested: requested };
}

export function hasVisibleTruncation(attempt: RunAttempt): boolean {
  const evidence = attempt.truncation;
  return (
    evidence.stdoutTruncated ||
    evidence.stderrTruncated ||
    evidence.summaryTruncated ||
    evidence.diffTruncated ||
    evidence.changedFilesTruncated ||
    evidence.progressTruncated ||
    evidence.beforeSnapshotTruncated ||
    evidence.afterSnapshotTruncated ||
    attempt.workspaceChanges.status === "partial" ||
    attempt.workspaceChanges.status === "unavailable"
  );
}
