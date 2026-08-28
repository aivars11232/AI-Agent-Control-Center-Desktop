import { describe, expect, it, vi } from "vitest";
import type { InvokeFunction } from "../persistence";
import {
  createDesktopClient,
  type DesktopControlStatus,
  type DesktopListenFunction,
  type VoiceRuntimeStatus,
  type VoiceTranscriptEvent,
} from "./desktopClient";

type InvokeCall = {
  command: string;
  args?: Record<string, unknown>;
};

function clientHarness() {
  const calls: InvokeCall[] = [];
  const listeners = new Map<string, (payload: unknown) => void>();
  const stoppedEvents: string[] = [];
  const invokeFn: InvokeFunction = async <T,>(
    command: string,
    args?: Record<string, unknown>,
  ) => {
    calls.push({ command, args });
    return undefined as T;
  };
  const listenFn: DesktopListenFunction = async <T,>(
    eventName: string,
    handler: (payload: T) => void,
  ) => {
    listeners.set(eventName, handler as (payload: unknown) => void);
    return () => stoppedEvents.push(eventName);
  };

  return {
    calls,
    listeners,
    stoppedEvents,
    client: createDesktopClient(invokeFn, listenFn),
  };
}

describe("typed desktop client command contracts", () => {
  it("maps authorization, run, review, workspace, and the unified gateway exactly", async () => {
    const { calls, client } = clientHarness();
    const reviewContext = {
      flowId: 31,
      stageAttemptId: 32,
      revisionRound: 2,
      level: "senior" as const,
      requestFingerprint: "review-fingerprint",
    };

    await client.requestAuthorization({
      kind: "runTask",
      agentId: 4,
      taskOwnerAgentId: 5,
      taskId: 6,
      runMode: "review",
      reviewContext,
    });
    await client.resolveApproval(7, "approve");
    await client.startReviewStage({
      expectedRevision: 8,
      taskOwnerAgentId: 5,
      taskId: 6,
    });
    await client.recordHumanReviewDecision({
      expectedRevision: 9,
      taskOwnerAgentId: 5,
      taskId: 6,
      flowId: 31,
      verdict: "changesRequested",
      feedback: "Revise the bounded change.",
    });
    await client.runAgentTask({
      runId: "run-10",
      runMode: "review",
      agentId: 4,
      taskOwnerAgentId: 5,
      taskId: 6,
      reviewContext,
    });
    await client.cancelAgentRun("run-10");
    await client.openWorkspaceItem({
      agentId: 4,
      workspaceId: "workspace-11",
      itemPath: "src/main.ts",
    });
    await client.enableDesktopControl(4);
    await client.submitVoiceIntent({
      requestId: "voice:gateway-12",
      intent: {
        kind: "closeApplication",
        application: "firefox.desktop",
      },
    });
    await client.querySystemActionAudits(25);

    expect(calls).toEqual([
      {
        command: "request_authorization",
        args: {
          intent: {
            kind: "runTask",
            agentId: 4,
            taskOwnerAgentId: 5,
            taskId: 6,
            runMode: "review",
            reviewContext,
          },
        },
      },
      {
        command: "resolve_approval",
        args: { request: { approvalId: 7, resolution: "approve" } },
      },
      {
        command: "start_review_stage",
        args: {
          request: {
            expectedRevision: 8,
            taskOwnerAgentId: 5,
            taskId: 6,
          },
        },
      },
      {
        command: "record_human_review_decision",
        args: {
          request: {
            expectedRevision: 9,
            taskOwnerAgentId: 5,
            taskId: 6,
            flowId: 31,
            verdict: "changesRequested",
            feedback: "Revise the bounded change.",
          },
        },
      },
      {
        command: "run_agent_task",
        args: {
          request: {
            runId: "run-10",
            runMode: "review",
            agentId: 4,
            taskOwnerAgentId: 5,
            taskId: 6,
            reviewContext,
          },
        },
      },
      { command: "cancel_agent_run", args: { runId: "run-10" } },
      {
        command: "open_workspace_item",
        args: {
          request: {
            agentId: 4,
            workspaceId: "workspace-11",
            itemPath: "src/main.ts",
          },
        },
      },
      { command: "enable_desktop_control", args: { agentId: 4 } },
      {
        command: "submit_voice_intent",
        args: {
          request: {
            requestId: "voice:gateway-12",
            intent: {
              kind: "closeApplication",
              application: "firefox.desktop",
            },
          },
        },
      },
      { command: "query_system_action_audits", args: { limit: 25 } },
    ]);
  });

  it("maps snapshots, voice lifecycle, and persistence commands without payload drift", async () => {
    const { calls, client } = clientHarness();

    await client.desktopControlStatus();
    await client.disableDesktopControl();
    await client.reviewOrchestrationSnapshot();
    await client.voiceRuntimeStatus();
    await client.installVoiceRuntime(12);
    await client.installHighAccuracyVoiceRuntime(12);
    await client.cancelVoiceRuntimeInstall("install-base-12");
    await client.startVoiceListener(12);
    await client.stopVoiceListener();
    await client.chooseWorkspaceFolder();
    await client.agentRegistrySnapshot();
    await client.taskOrchestrationSnapshot();
    await client.loadApplicationState();
    await client.runCoordinatorSnapshot();
    await client.providerRegistryStatus();
    await client.invokeApplicationState("save_application_state", {
      expectedRevision: 13,
    });

    expect(calls).toEqual([
      { command: "desktop_control_status", args: undefined },
      { command: "disable_desktop_control", args: undefined },
      { command: "review_orchestration_snapshot", args: undefined },
      { command: "voice_runtime_status", args: undefined },
      { command: "install_voice_runtime", args: { agentId: 12 } },
      {
        command: "install_high_accuracy_voice_runtime",
        args: { agentId: 12 },
      },
      {
        command: "cancel_voice_runtime_install",
        args: { operationId: "install-base-12" },
      },
      { command: "start_voice_listener", args: { agentId: 12 } },
      { command: "stop_voice_listener", args: undefined },
      { command: "choose_workspace_folder", args: undefined },
      { command: "agent_registry_snapshot", args: undefined },
      { command: "task_orchestration_snapshot", args: undefined },
      { command: "load_application_state", args: undefined },
      { command: "run_coordinator_snapshot", args: undefined },
      { command: "provider_registry_status", args: undefined },
      {
        command: "save_application_state",
        args: { expectedRevision: 13 },
      },
    ]);
  });

  it("uses stable event names, forwards payloads, and exposes listener cleanup", async () => {
    const { client, listeners, stoppedEvents } = clientHarness();
    const transcriptHandler = vi.fn<(event: VoiceTranscriptEvent) => void>();
    const runtimeHandler = vi.fn<(status: VoiceRuntimeStatus) => void>();
    const desktopHandler = vi.fn<(status: DesktopControlStatus) => void>();
    const openHandler = vi.fn<() => void>();

    const stopTranscript = await client.onVoiceTranscript(transcriptHandler);
    const stopRuntime = await client.onVoiceRuntimeStatus(runtimeHandler);
    const stopDesktop = await client.onDesktopControlStatus(desktopHandler);
    const stopOpen = await client.onVoiceControlOpen(openHandler);
    listeners.get("voice-transcript")?.({
      kind: "command",
      transcript: "inspect the build",
    });
    listeners.get("voice-control-open")?.(undefined);
    listeners.get("voice-runtime-status")?.({ installState: "ready" });
    listeners.get("desktop-control-status")?.({ state: "closed" });
    stopTranscript();
    stopRuntime();
    stopDesktop();
    stopOpen();

    expect(transcriptHandler).toHaveBeenCalledWith({
      kind: "command",
      transcript: "inspect the build",
    });
    expect(openHandler).toHaveBeenCalledOnce();
    expect(runtimeHandler).toHaveBeenCalledWith({ installState: "ready" });
    expect(desktopHandler).toHaveBeenCalledWith({ state: "closed" });
    expect(stoppedEvents).toEqual([
      "voice-transcript",
      "voice-runtime-status",
      "desktop-control-status",
      "voice-control-open",
    ]);
  });

  it("maps lifecycle backup and revision-bound monitoring commands exactly", async () => {
    const { calls, client } = clientHarness();
    const revision = {
      applicationState: 1,
      taskOrchestration: 2,
      runCoordinator: 3,
      reviewOrchestration: 4,
      dataLifecycle: 5,
    };

    await client.monitoringSnapshot();
    await client.queryMonitoringTasks({
      expectedRevision: revision,
      status: "Completed",
      category: null,
      offset: 0,
      limit: 100,
    });
    await client.queryMonitoringActivity({
      expectedRevision: revision,
      offset: 0,
      limit: 100,
    });
    await client.deleteMonitoringActivity({
      expectedRevision: revision,
      ownerAgentId: 7,
      entryId: 8,
    });
    await client.clearMonitoringActivity(revision);
    await client.exportBackup();
    await client.previewBackupImport(9, "{\"version\":3}");
    await client.applyBackupImport(9, "{\"version\":3}");

    expect(calls).toEqual([
      { command: "monitoring_snapshot", args: undefined },
      {
        command: "query_monitoring_tasks",
        args: {
          request: {
            expectedRevision: revision,
            status: "Completed",
            category: null,
            offset: 0,
            limit: 100,
          },
        },
      },
      {
        command: "query_monitoring_activity",
        args: {
          request: { expectedRevision: revision, offset: 0, limit: 100 },
        },
      },
      {
        command: "delete_monitoring_activity",
        args: {
          request: {
            expectedRevision: revision,
            ownerAgentId: 7,
            entryId: 8,
          },
        },
      },
      {
        command: "clear_monitoring_activity",
        args: { request: { expectedRevision: revision } },
      },
      { command: "export_backup", args: undefined },
      {
        command: "preview_backup_import",
        args: {
          request: { expectedRevision: 9, backupJson: "{\"version\":3}" },
        },
      },
      {
        command: "apply_backup_import",
        args: {
          request: { expectedRevision: 9, backupJson: "{\"version\":3}" },
        },
      },
    ]);
  });
});
