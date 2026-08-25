export type WorkspaceEvidenceMode =
  | "git"
  | "filesystem"
  | "notCollected"
  | "unavailable";

export type WorkspaceEvidenceStatus =
  | "complete"
  | "partial"
  | "notCollected"
  | "unavailable";

export type WorkspaceEvidenceReviewability =
  | "agentEligible"
  | "humanReviewRequired"
  | "unavailable";

export type WorkspaceChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "typeChanged"
  | "statusChanged";

export type WorkspaceFileKind =
  | "file"
  | "directory"
  | "blockedSymlink"
  | "unsupported";

export type WorkspaceDetailKind =
  | "gitStaged"
  | "gitUnstaged"
  | "filesystemPreview"
  | "binary"
  | "redacted"
  | "metadataOnly";

export type WorkspaceEvidenceLimits = {
  snapshotEntryLimit: number;
  snapshotMillis: number;
  snapshotHashBytes: number;
  persistedChangeLimit: number;
  persistedPathBytes: number;
  perFileDetailBytes: number;
  aggregateDetailBytes: number;
  gitStatusBytes: number;
  issueLimit: number;
};

export type WorkspaceEvidenceIssue = {
  code: string;
  message: string;
  phase: string;
  path: string | null;
  blocksAgentApproval: boolean;
};

export type WorkspaceFileState = {
  kind: WorkspaceFileKind;
  sizeBytes: number | null;
  sha256: string | null;
  mode: number;
  binary: boolean | null;
  contentRedacted: boolean;
};

export type GitPathState = {
  indexStatus: string | null;
  worktreeStatus: string | null;
  untracked: boolean;
  conflicted: boolean;
  previousPath: string | null;
};

export type WorkspaceChange = {
  path: string;
  previousPath: string | null;
  changeKind: WorkspaceChangeKind;
  before: WorkspaceFileState | null;
  after: WorkspaceFileState | null;
  gitBefore: GitPathState | null;
  gitAfter: GitPathState | null;
  binary: boolean;
  contentRedacted: boolean;
  detailTruncated: boolean;
  humanReviewRequired: boolean;
};

export type WorkspaceEvidenceDetail = {
  path: string;
  kind: WorkspaceDetailKind;
  content: string | null;
  originalBytes: number;
  truncated: boolean;
  redacted: boolean;
};

export type WorkspaceChangeSummary = {
  totalChanges: number;
  retainedChanges: number;
  added: number;
  modified: number;
  deleted: number;
  renamed: number;
  typeChanged: number;
  statusChanged: number;
  staged: number;
  unstaged: number;
  untracked: number;
  binary: number;
  redacted: number;
};

export type WorkspaceChangeEvidence = {
  schemaVersion: 1;
  mode: WorkspaceEvidenceMode;
  status: WorkspaceEvidenceStatus;
  reviewability: WorkspaceEvidenceReviewability;
  consistency: "observedDuringRun";
  baselineGitHead: string | null;
  finalGitHead: string | null;
  changes: WorkspaceChange[];
  details: WorkspaceEvidenceDetail[];
  summary: WorkspaceChangeSummary;
  issues: WorkspaceEvidenceIssue[];
  issuesTruncated: boolean;
  beforeSnapshotTruncated: boolean;
  afterSnapshotTruncated: boolean;
  changesTruncated: boolean;
  detailsTruncated: boolean;
  limits: WorkspaceEvidenceLimits;
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isFileState(value: unknown): value is WorkspaceFileState {
  return (
    isObject(value) &&
    ["file", "directory", "blockedSymlink", "unsupported"].includes(
      String(value.kind),
    ) &&
    (value.sizeBytes === null || isNonNegativeNumber(value.sizeBytes)) &&
    isNullableString(value.sha256) &&
    isNonNegativeNumber(value.mode) &&
    (value.binary === null || typeof value.binary === "boolean") &&
    typeof value.contentRedacted === "boolean"
  );
}

function isGitPathState(value: unknown): value is GitPathState {
  return (
    isObject(value) &&
    isNullableString(value.indexStatus) &&
    isNullableString(value.worktreeStatus) &&
    typeof value.untracked === "boolean" &&
    typeof value.conflicted === "boolean" &&
    isNullableString(value.previousPath)
  );
}

function isWorkspaceChange(value: unknown): value is WorkspaceChange {
  return (
    isObject(value) &&
    typeof value.path === "string" &&
    isNullableString(value.previousPath) &&
    [
      "added",
      "modified",
      "deleted",
      "renamed",
      "typeChanged",
      "statusChanged",
    ].includes(String(value.changeKind)) &&
    (value.before === null || isFileState(value.before)) &&
    (value.after === null || isFileState(value.after)) &&
    (value.gitBefore === null || isGitPathState(value.gitBefore)) &&
    (value.gitAfter === null || isGitPathState(value.gitAfter)) &&
    typeof value.binary === "boolean" &&
    typeof value.contentRedacted === "boolean" &&
    typeof value.detailTruncated === "boolean" &&
    typeof value.humanReviewRequired === "boolean"
  );
}

function hasNumericFields(
  value: Record<string, unknown>,
  fields: readonly string[],
): boolean {
  return fields.every((field) => isNonNegativeNumber(value[field]));
}

export function normalizeWorkspaceEvidence(
  value: unknown,
): WorkspaceChangeEvidence | null {
  if (
    !isObject(value) ||
    value.schemaVersion !== 1 ||
    !Array.isArray(value.changes) ||
    !Array.isArray(value.details) ||
    !Array.isArray(value.issues) ||
    !isObject(value.summary) ||
    !isObject(value.limits) ||
    value.consistency !== "observedDuringRun" ||
    !isNullableString(value.baselineGitHead) ||
    !isNullableString(value.finalGitHead) ||
    !["git", "filesystem", "notCollected", "unavailable"].includes(
      String(value.mode),
    ) ||
    !["complete", "partial", "notCollected", "unavailable"].includes(
      String(value.status),
    ) ||
    ![
      "agentEligible",
      "humanReviewRequired",
      "unavailable",
    ].includes(String(value.reviewability))
  ) {
    return null;
  }

  if (
    !value.changes.every(isWorkspaceChange) ||
    !value.details.every(
      (detail) =>
        isObject(detail) &&
        typeof detail.path === "string" &&
        [
          "gitStaged",
          "gitUnstaged",
          "filesystemPreview",
          "binary",
          "redacted",
          "metadataOnly",
        ].includes(String(detail.kind)) &&
        isNullableString(detail.content) &&
        isNonNegativeNumber(detail.originalBytes) &&
        typeof detail.truncated === "boolean" &&
        typeof detail.redacted === "boolean",
    ) ||
    !value.issues.every(
      (issue) =>
        isObject(issue) &&
        typeof issue.code === "string" &&
        typeof issue.message === "string" &&
        typeof issue.phase === "string" &&
        isNullableString(issue.path) &&
        typeof issue.blocksAgentApproval === "boolean",
    ) ||
    !hasNumericFields(value.summary, [
      "totalChanges",
      "retainedChanges",
      "added",
      "modified",
      "deleted",
      "renamed",
      "typeChanged",
      "statusChanged",
      "staged",
      "unstaged",
      "untracked",
      "binary",
      "redacted",
    ]) ||
    !hasNumericFields(value.limits, [
      "snapshotEntryLimit",
      "snapshotMillis",
      "snapshotHashBytes",
      "persistedChangeLimit",
      "persistedPathBytes",
      "perFileDetailBytes",
      "aggregateDetailBytes",
      "gitStatusBytes",
      "issueLimit",
    ]) ||
    ![
      "issuesTruncated",
      "beforeSnapshotTruncated",
      "afterSnapshotTruncated",
      "changesTruncated",
      "detailsTruncated",
    ].every((field) => typeof value[field] === "boolean")
  ) {
    return null;
  }

  return value as unknown as WorkspaceChangeEvidence;
}

export function workspaceEvidenceStatusLabel(
  evidence: WorkspaceChangeEvidence,
): string {
  const mode =
    evidence.mode === "git"
      ? "Git"
      : evidence.mode === "filesystem"
        ? "Non-Git"
        : "Workspace";
  const status =
    evidence.status === "complete"
      ? "complete"
      : evidence.status === "partial"
        ? "partial"
        : evidence.status === "notCollected"
          ? "not collected"
          : "unavailable";
  return `${mode} evidence · ${status}`;
}

export function workspaceReviewabilityLabel(
  evidence: WorkspaceChangeEvidence,
): string {
  if (evidence.reviewability === "agentEligible") {
    return "Agent review eligible";
  }
  if (evidence.reviewability === "humanReviewRequired") {
    return "Human review required";
  }
  return "Review evidence unavailable";
}

export function workspaceChangeLabel(change: WorkspaceChange): string {
  return change.changeKind.replace(/([A-Z])/g, " $1").toLowerCase();
}

export function workspaceChangeCanOpen(change: WorkspaceChange): boolean {
  return (
    change.changeKind !== "deleted" &&
    change.after !== null &&
    (change.after.kind === "file" || change.after.kind === "directory")
  );
}

export function workspaceEvidenceHasVisibleLimit(
  evidence: WorkspaceChangeEvidence,
): boolean {
  return (
    evidence.status === "partial" ||
    evidence.issuesTruncated ||
    evidence.beforeSnapshotTruncated ||
    evidence.afterSnapshotTruncated ||
    evidence.changesTruncated ||
    evidence.detailsTruncated
  );
}

export function unavailableWorkspaceEvidence(
  message = "Structured workspace evidence is unavailable.",
): WorkspaceChangeEvidence {
  return {
    schemaVersion: 1,
    mode: "unavailable",
    status: "unavailable",
    reviewability: "unavailable",
    consistency: "observedDuringRun",
    baselineGitHead: null,
    finalGitHead: null,
    changes: [],
    details: [],
    summary: {
      totalChanges: 0,
      retainedChanges: 0,
      added: 0,
      modified: 0,
      deleted: 0,
      renamed: 0,
      typeChanged: 0,
      statusChanged: 0,
      staged: 0,
      unstaged: 0,
      untracked: 0,
      binary: 0,
      redacted: 0,
    },
    issues: [
      {
        code: "LEGACY_EVIDENCE_UNAVAILABLE",
        message,
        phase: "persistence",
        path: null,
        blocksAgentApproval: true,
      },
    ],
    issuesTruncated: false,
    beforeSnapshotTruncated: false,
    afterSnapshotTruncated: false,
    changesTruncated: false,
    detailsTruncated: false,
    limits: {
      snapshotEntryLimit: 20_000,
      snapshotMillis: 5_000,
      snapshotHashBytes: 512 * 1024 * 1024,
      persistedChangeLimit: 250,
      persistedPathBytes: 256 * 1024,
      perFileDetailBytes: 64 * 1024,
      aggregateDetailBytes: 512 * 1024,
      gitStatusBytes: 4 * 1024 * 1024,
      issueLimit: 64,
    },
  };
}
