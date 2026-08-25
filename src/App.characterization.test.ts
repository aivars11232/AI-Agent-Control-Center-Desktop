import { describe, expect, it } from "vitest";
import {
  normalizeApprovalRequest,
  normalizePerformance,
  normalizePreferences,
  taskSafetyAssessment,
  type ApprovalRequest,
} from "./App";
import { agentFixture, taskFixture } from "./test/fixtures";

describe("task safety characterization", () => {
  it("requires one-run approval for requested write and terminal scopes", () => {
    const assessment = taskSafetyAssessment(
      taskFixture({
        title: "Implement the parser and run cargo build",
        category: "Development",
      }),
      agentFixture({ approvals: { files: "ask", terminal: "ask" } }),
      "balanced",
    );

    expect(assessment).toMatchObject({
      riskLevel: "Medium",
      scopes: ["files", "terminal"],
      approvalScopes: ["files", "terminal"],
      requiresApproval: true,
      destructive: false,
      writesWorkspace: true,
      blockedReason: null,
    });
  });

  it("requires files approval for destructive workspace language", () => {
    const assessment = taskSafetyAssessment(
      taskFixture({
        title: "Delete the generated cache",
        category: "Development",
      }),
      agentFixture(),
      "balanced",
    );

    expect(assessment).toMatchObject({
      riskLevel: "High",
      scopes: ["files"],
      approvalScopes: ["files"],
      requiresApproval: true,
      destructive: true,
      blockedReason: null,
    });
  });

  it("blocks privileged, system-control, and locked-mode actions", () => {
    const privileged = taskSafetyAssessment(
      taskFixture({ title: "Run sudo pacman to install a package" }),
      agentFixture(),
      "balanced",
    );
    const systemControl = taskSafetyAssessment(
      taskFixture({ title: "Open an app", category: "System Control" }),
      agentFixture(),
      "balanced",
    );
    const locked = taskSafetyAssessment(
      taskFixture({ title: "Run cargo test", category: "Development" }),
      agentFixture(),
      "locked",
    );

    expect(privileged).toMatchObject({
      riskLevel: "Critical",
      requiresApproval: false,
      blockedReason:
        "Privileged and operating-system package commands are blocked by the desktop safety boundary.",
    });
    expect(systemControl).toMatchObject({
      riskLevel: "Critical",
      requiresApproval: false,
      blockedReason:
        "System-control tools are not enabled yet. This release confines Codex to the selected workspace.",
    });
    expect(locked).toMatchObject({
      requiresApproval: false,
      blockedReason:
        "Locked mode permits inspection only. Change the Safety mode in Settings to run this action.",
    });
  });
});

describe("approval normalization characterization", () => {
  it("rejects records without the required identity fields", () => {
    expect(normalizeApprovalRequest({ id: 1, agentId: 2 })).toBeNull();
  });

  it("normalizes legacy defaults and filters unknown scopes", () => {
    const normalized = normalizeApprovalRequest({
      id: 11,
      agentId: 3,
      title: "Approval",
      reason: "Characterization fixture",
      createdAt: "2026-01-02T03:04:05.000Z",
      status: "Unknown",
      riskLevel: "Unknown",
      scopes: ["files", "unknown", "system"],
    } as unknown as Partial<ApprovalRequest>);

    expect(normalized).toEqual({
      id: 11,
      agentId: 3,
      taskId: null,
      title: "Approval",
      reason: "Characterization fixture",
      status: "Pending",
      createdAt: "2026-01-02T03:04:05.000Z",
      resolvedAt: null,
      riskLevel: "Low",
      scopes: ["files", "system"],
      workspaceId: null,
      taskSnapshot: "",
      expiresAt: "2026-01-02T03:34:05.000Z",
      consumedAt: null,
    });
  });
});

describe("stored-value normalization characterization", () => {
  it("clamps performance values and falls back from invalid enums", () => {
    expect(
      normalizePerformance({
        strength: 99,
        focus: "unsupported" as never,
        cpuLimit: 1,
        gpuLimit: 101,
        overflowAction: "redirect",
        redirectAgentId: 7,
      }),
    ).toEqual({
      strength: 10,
      focus: "balanced",
      cpuLimit: 10,
      gpuLimit: 100,
      queueThreshold: 10,
      overflowAction: "redirect",
      redirectAgentId: 7,
    });
  });

  it("migrates a legacy workspace path and normalizes voice preferences", () => {
    const preferences = normalizePreferences({
      workspacePath: " /tmp/example-project/ ",
      workspaces: [],
      activeWorkspaceId: "missing-workspace",
      agentTimeoutMinutes: 0,
      approvalExpiryMinutes: 999,
      voiceWakePhrase: " Lucy Activate, On ",
      voiceDeactivatePhrase: "  END LUCY  ",
      voiceOpenPhrases: " Launch, GO ",
      voiceClosePhrases: " CLOSE ",
      voiceCommandReplacements: "Fire Fox = Firefox",
      voiceState: "unexpected" as never,
    });

    expect(preferences.workspacePath).toBe("/tmp/example-project/");
    expect(preferences.workspaces).toEqual([
      {
        id: "migrated-workspace",
        name: "example-project",
        path: "/tmp/example-project/",
      },
    ]);
    expect(preferences.activeWorkspaceId).toBe("migrated-workspace");
    expect(preferences.agentTimeoutMinutes).toBe(1);
    expect(preferences.approvalExpiryMinutes).toBe(120);
    expect(preferences.voiceWakePhrase).toBe("lucy");
    expect(preferences.voiceDeactivatePhrase).toBe("end lucy");
    expect(preferences.voiceOpenPhrases).toBe("launch, go");
    expect(preferences.voiceClosePhrases).toBe("close");
    expect(preferences.voiceCommandReplacements).toBe(
      "fire fox = firefox",
    );
    expect(preferences.voiceState).toBe("VOICE_PASSIVE");
  });
});
