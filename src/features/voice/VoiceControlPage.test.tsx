// @vitest-environment jsdom

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Dispatch, SetStateAction } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createDefaultApplicationState,
  type AppPreferences,
  type ApprovalRequest,
} from "../../applicationState";
import {
  desktopClient,
  type SystemActionAuditRecord,
  type VoiceIntentResult,
} from "../../services/desktopClient";
import { VoiceControlPage } from "./VoiceControlPage";

function audit(
  status: SystemActionAuditRecord["status"],
): SystemActionAuditRecord {
  return {
    id: 41,
    requestId: "voice:fixture",
    requestFingerprint: "voice-intent-v1|fixture",
    intentKind: "closeApplication",
    riskClass: "destructive",
    targetKind: "kwinWindow",
    targetId: "exact-window-id",
    agentId: 7,
    taskOwnerAgentId: null,
    taskId: null,
    approvalId: 42,
    authorizationKind:
      status === "approvalRequired"
        ? "approvalRequired"
        : "approvalConsumed",
    intentFingerprintSha256: "a".repeat(64),
    policyFingerprintSha256: "b".repeat(64),
    status,
    detailCode:
      status === "approvalRequired"
        ? "APPROVAL_REQUIRED"
        : "SYSTEM_ACTION_APPLIED",
    detailMessage:
      status === "approvalRequired"
        ? "The exact action is waiting for approval."
        : "The exact action was applied.",
    contentSha256: null,
    contentLength: null,
    createdAtUnixMs: 1,
    updatedAtUnixMs: 2,
  };
}

function approval(consumed: boolean): ApprovalRequest {
  return {
    id: 42,
    agentId: 7,
    taskId: null,
    title: "Close exact window",
    reason: "Destructive system action.",
    status: consumed ? "Approved" : "Pending",
    createdAt: "2026-08-28T12:00:00.000Z",
    resolvedAt: consumed ? "2026-08-28T12:01:00.000Z" : null,
    riskLevel: "High",
    scopes: ["system"],
    workspaceId: null,
    taskSnapshot: "",
    expiresAt: "2026-08-28T12:10:00.000Z",
    consumedAt: consumed ? "2026-08-28T12:01:01.000Z" : null,
  };
}

function result(
  status: "approvalRequired" | "applied",
): VoiceIntentResult {
  return {
    requestId: "voice:fixture",
    status,
    message:
      status === "approvalRequired"
        ? "Approve the exact action."
        : "The exact action was applied.",
    approval: approval(status === "applied"),
    taskOwnerAgentId: null,
    taskId: null,
    audit: audit(status),
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

describe("VoiceControlPage unified gateway", () => {
  it("submits one named-close intent and reuses its request ID after approval", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    const state = createDefaultApplicationState();
    state.preferences.backgroundVoiceEnabled = false;
    state.preferences.voiceControlMasterEnabled = true;
    const pcAgent = state.agents.find(
      (agent) => agent.templateKey === "pc-control",
    );
    if (!pcAgent) throw new Error("PC Control fixture missing");
    pcAgent.capabilities.system = "full";

    vi.spyOn(desktopClient, "voiceRuntimeStatus").mockResolvedValue({
      installed: true,
      listening: false,
      highAccuracyAvailable: true,
      installState: "ready",
      listenerState: "stopped",
      operationId: null,
      canCancel: false,
      message: "Voice runtime ready.",
    });
    vi.spyOn(desktopClient, "desktopControlStatus").mockResolvedValue({
      enabled: false,
      state: "disabled",
      message: "Desktop input is disabled.",
    });
    vi.spyOn(desktopClient, "querySystemActionAudits").mockResolvedValue({
      records: [],
      limit: 50,
    });
    vi.spyOn(desktopClient, "onVoiceRuntimeStatus").mockResolvedValue(
      () => undefined,
    );
    vi.spyOn(desktopClient, "onDesktopControlStatus").mockResolvedValue(
      () => undefined,
    );
    vi.spyOn(desktopClient, "onVoiceTranscript").mockResolvedValue(
      () => undefined,
    );
    const submit = vi
      .spyOn(desktopClient, "submitVoiceIntent")
      .mockResolvedValueOnce(result("approvalRequired"))
      .mockResolvedValueOnce(result("applied"));
    const enableDesktop = vi.spyOn(
      desktopClient,
      "enableDesktopControl",
    );
    const onGatewayMutation = vi.fn(async () => undefined);
    const setApprovalRequests = vi.fn() as Dispatch<
      SetStateAction<ApprovalRequest[]>
    >;
    const setPreferences = vi.fn() as Dispatch<
      SetStateAction<AppPreferences>
    >;
    const user = userEvent.setup();

    render(
      <VoiceControlPage
        agents={state.agents}
        onGatewayMutation={onGatewayMutation}
        setApprovalRequests={setApprovalRequests}
        preferences={state.preferences}
        setPreferences={setPreferences}
      />,
    );

    await waitFor(() =>
      expect(desktopClient.querySystemActionAudits).toHaveBeenCalled(),
    );
    expect(enableDesktop).not.toHaveBeenCalled();

    await user.type(
      screen.getByRole("textbox", { name: "Command" }),
      "close imaginary editor",
    );
    await user.click(screen.getByRole("button", { name: "Run command" }));

    await waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
    const firstRequest = submit.mock.calls[0][0];
    expect(firstRequest.intent).toEqual({
      kind: "closeApplication",
      application: "imaginary editor",
    });
    expect(
      screen.getByRole("button", { name: "Retry approved action" }),
    ).toBeTruthy();

    await user.click(
      screen.getByRole("button", { name: "Retry approved action" }),
    );
    await waitFor(() => expect(submit).toHaveBeenCalledTimes(2));
    expect(submit.mock.calls[1][0]).toEqual(firstRequest);
    expect(onGatewayMutation).toHaveBeenCalledOnce();
  });

  it("cancels only the reported install operation and explicitly disables KDE input", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    const state = createDefaultApplicationState();
    state.preferences.backgroundVoiceEnabled = false;
    const pcAgent = state.agents.find(
      (agent) => agent.templateKey === "pc-control",
    );
    if (!pcAgent) throw new Error("PC Control fixture missing");
    pcAgent.capabilities.system = "full";
    const installing = {
      installed: true,
      listening: false,
      highAccuracyAvailable: false,
      installState: "installing" as const,
      listenerState: "stopped" as const,
      operationId: "install-high-fixture",
      canCancel: true,
      message: "Installing optional high accuracy.",
    };
    vi.spyOn(desktopClient, "voiceRuntimeStatus").mockResolvedValue(
      installing,
    );
    vi.spyOn(desktopClient, "desktopControlStatus").mockResolvedValue({
      enabled: true,
      state: "enabled",
      message: "Desktop input is active.",
    });
    vi.spyOn(desktopClient, "querySystemActionAudits").mockResolvedValue({
      records: [],
      limit: 50,
    });
    vi.spyOn(desktopClient, "onVoiceRuntimeStatus").mockResolvedValue(
      () => undefined,
    );
    vi.spyOn(desktopClient, "onDesktopControlStatus").mockResolvedValue(
      () => undefined,
    );
    vi.spyOn(desktopClient, "onVoiceTranscript").mockResolvedValue(
      () => undefined,
    );
    const cancel = vi
      .spyOn(desktopClient, "cancelVoiceRuntimeInstall")
      .mockResolvedValue({
        ...installing,
        installState: "cancelling",
        message: "Cancelling installation.",
      });
    const disable = vi
      .spyOn(desktopClient, "disableDesktopControl")
      .mockResolvedValue({
        enabled: false,
        state: "disabled",
        message: "Desktop input is disabled.",
      });
    const user = userEvent.setup();

    render(
      <VoiceControlPage
        agents={state.agents}
        onGatewayMutation={vi.fn(async () => undefined)}
        setApprovalRequests={vi.fn()}
        preferences={state.preferences}
        setPreferences={vi.fn()}
      />,
    );

    await user.click(
      await screen.findByRole("button", {
        name: "Cancel voice installation",
      }),
    );
    expect(cancel).toHaveBeenCalledWith("install-high-fixture");
    await user.click(
      screen.getByRole("button", { name: "Disable KDE desktop input" }),
    );
    expect(disable).toHaveBeenCalledOnce();
  });
});
