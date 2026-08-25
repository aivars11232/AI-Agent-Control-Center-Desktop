import { describe, expect, it } from "vitest";
import {
  normalizeWorkspaceEvidence,
  unavailableWorkspaceEvidence,
  workspaceChangeCanOpen,
  workspaceEvidenceHasVisibleLimit,
  workspaceEvidenceStatusLabel,
  workspaceReviewabilityLabel,
  type WorkspaceChange,
} from "./workspaceEvidence";

function change(overrides: Partial<WorkspaceChange> = {}): WorkspaceChange {
  return {
    path: "src/main.ts",
    previousPath: null,
    changeKind: "modified",
    before: null,
    after: {
      kind: "file",
      sizeBytes: 12,
      sha256: "a".repeat(64),
      mode: 0o100644,
      binary: false,
      contentRedacted: false,
    },
    gitBefore: null,
    gitAfter: null,
    binary: false,
    contentRedacted: false,
    detailTruncated: false,
    humanReviewRequired: false,
    ...overrides,
  };
}

describe("workspace evidence projection", () => {
  it("accepts the complete versioned shape and rejects malformed nested data", () => {
    const valid = unavailableWorkspaceEvidence();
    expect(normalizeWorkspaceEvidence(valid)).toEqual(valid);
    expect(
      normalizeWorkspaceEvidence({
        ...valid,
        changes: [{ path: "missing-required-fields" }],
      }),
    ).toBeNull();
    expect(normalizeWorkspaceEvidence({ ...valid, schemaVersion: 2 })).toBeNull();
  });

  it("opens only final regular files or directories", () => {
    expect(workspaceChangeCanOpen(change())).toBe(true);
    expect(
      workspaceChangeCanOpen(
        change({ changeKind: "deleted", after: null }),
      ),
    ).toBe(false);
    expect(
      workspaceChangeCanOpen(
        change({
          after: {
            kind: "blockedSymlink",
            sizeBytes: null,
            sha256: null,
            mode: 0o120777,
            binary: null,
            contentRedacted: true,
          },
        }),
      ),
    ).toBe(false);
  });

  it("surfaces unavailable and bounded evidence without implying completeness", () => {
    const evidence = unavailableWorkspaceEvidence();
    expect(workspaceEvidenceStatusLabel(evidence)).toBe(
      "Workspace evidence · unavailable",
    );
    expect(workspaceReviewabilityLabel(evidence)).toBe(
      "Review evidence unavailable",
    );
    expect(workspaceEvidenceHasVisibleLimit(evidence)).toBe(false);
    expect(
      workspaceEvidenceHasVisibleLimit({
        ...evidence,
        status: "partial",
        reviewability: "humanReviewRequired",
      }),
    ).toBe(true);
  });
});
