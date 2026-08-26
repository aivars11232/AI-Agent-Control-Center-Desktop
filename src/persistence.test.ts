import { describe, expect, it, vi } from "vitest";
import { createDefaultApplicationState, type StateEnvelope } from "./applicationState";
import {
  ApplicationStateWriter,
  bootstrapDesktopApplicationState,
  collectLegacyRendererState,
  LEGACY_STORAGE_KEYS,
  type InvokeFunction,
  type StorageReader,
} from "./persistence";

class MemoryStorage implements StorageReader {
  readonly values = new Map<string, string>();
  readCount = 0;

  getItem(key: string): string | null {
    this.readCount += 1;
    return this.values.get(key) ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

function envelope(revision = 1): StateEnvelope {
  return {
    schemaVersion: 5,
    revision,
    state: createDefaultApplicationState(),
    migration: {
      sourceKind: "fresh",
      sourceVersion: null,
      migratedAtUnixMs: 1,
      legacyCleanupAcknowledged: false,
    },
  };
}

describe("desktop persistence bootstrap", () => {
  it("collects all seven legacy keys, commits once, then cleans and acknowledges", async () => {
    const storage = new MemoryStorage();
    for (const [index, key] of Object.values(LEGACY_STORAGE_KEYS).entries()) {
      storage.values.set(key, `legacy-${index}`);
    }
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const migrated = envelope();
    migrated.migration.sourceKind = "legacy_local_storage";
    const acknowledged = envelope();
    acknowledged.migration.sourceKind = "legacy_local_storage";
    acknowledged.migration.legacyCleanupAcknowledged = true;
    const invoke: InvokeFunction = async <T>(
      command: string,
      args?: Record<string, unknown>,
    ) => {
      calls.push({ command, args });
      if (command === "load_application_state") return null as T;
      if (command === "initialize_application_state") return migrated as T;
      return acknowledged as T;
    };

    const legacy = collectLegacyRendererState(storage);
    expect(Object.values(legacy)).toHaveLength(7);
    const result = await bootstrapDesktopApplicationState(invoke, storage);

    expect(result).toEqual({ envelope: acknowledged, cleanupWarning: null });
    expect(calls.map((call) => call.command)).toEqual([
      "load_application_state",
      "initialize_application_state",
      "acknowledge_legacy_cleanup",
    ]);
    expect(
      (calls[1].args?.request as { legacy: unknown }).legacy,
    ).toEqual(legacy);
    expect(storage.values.size).toBe(0);
  });

  it("does not remove legacy data when backend migration rejects it", async () => {
    const storage = new MemoryStorage();
    storage.values.set(LEGACY_STORAGE_KEYS.agents, "malformed");
    const invoke: InvokeFunction = async <T>(command: string) => {
      if (command === "load_application_state") return null as T;
      throw { code: "STATE_VALIDATION_FAILED", message: "Invalid state" };
    };

    await expect(
      bootstrapDesktopApplicationState(invoke, storage),
    ).rejects.toMatchObject({ code: "STATE_VALIDATION_FAILED" });
    expect(storage.getItem(LEGACY_STORAGE_KEYS.agents)).toBe("malformed");
  });

  it("loads an existing database without consulting or clearing legacy storage", async () => {
    const storage = new MemoryStorage();
    storage.values.set(LEGACY_STORAGE_KEYS.agents, "retained");
    const existing = envelope(8);
    const invoke: InvokeFunction = async <T>() => existing as T;

    const result = await bootstrapDesktopApplicationState(invoke, storage);

    expect(result.envelope).toBe(existing);
    expect(storage.values.get(LEGACY_STORAGE_KEYS.agents)).toBe("retained");
    expect(storage.readCount).toBe(0);
  });

  it("finishes cleanup after restart when migration committed before acknowledgement", async () => {
    const storage = new MemoryStorage();
    storage.values.set(LEGACY_STORAGE_KEYS.preferences, "committed-legacy");
    const committed = envelope(3);
    committed.migration.sourceKind = "legacy_local_storage";
    const acknowledged = envelope(3);
    acknowledged.migration.sourceKind = "legacy_local_storage";
    acknowledged.migration.legacyCleanupAcknowledged = true;
    const commands: string[] = [];
    const invoke: InvokeFunction = async <T>(command: string) => {
      commands.push(command);
      return (command === "load_application_state"
        ? committed
        : acknowledged) as T;
    };

    const result = await bootstrapDesktopApplicationState(invoke, storage);

    expect(commands).toEqual([
      "load_application_state",
      "acknowledge_legacy_cleanup",
    ]);
    expect(result.envelope.migration.legacyCleanupAcknowledged).toBe(true);
    expect(storage.values.size).toBe(0);
  });
});

describe("serialized application-state writes", () => {
  it("coalesces pending state and advances compare-and-swap revisions in order", async () => {
    type PendingSave = {
      args: Record<string, unknown> | undefined;
      resolve: (value: { schemaVersion: number; revision: number }) => void;
    };
    const saves: PendingSave[] = [];
    const invoke: InvokeFunction = <T>(
      command: string,
      args?: Record<string, unknown>,
    ) => {
      expect(command).toBe("save_application_state");
      return new Promise<T>((resolve) => {
        saves.push({
          args,
          resolve: (value) => resolve(value as T),
        });
      });
    };
    const failure = vi.fn();
    const commit = vi.fn();
    const writer = new ApplicationStateWriter(invoke, 1, failure, commit);
    const first = createDefaultApplicationState();
    first.preferences.theme = "light";
    const latest = createDefaultApplicationState();
    latest.preferences.theme = "system";

    writer.enqueue(first);
    writer.enqueue(latest);
    expect(saves).toHaveLength(1);
    expect(
      (saves[0].args?.request as { expectedRevision: number }).expectedRevision,
    ).toBe(1);
    saves[0].resolve({ schemaVersion: 5, revision: 2 });
    await Promise.resolve();
    await Promise.resolve();
    expect(saves).toHaveLength(2);
    const secondRequest = saves[1].args?.request as {
      expectedRevision: number;
      state: ReturnType<typeof createDefaultApplicationState>;
    };
    expect(secondRequest.expectedRevision).toBe(2);
    expect(secondRequest.state.preferences.theme).toBe("system");
    saves[1].resolve({ schemaVersion: 5, revision: 3 });

    await writer.flush();
    expect(failure).not.toHaveBeenCalled();
    expect(commit.mock.calls.map(([receipt]) => receipt.revision)).toEqual([
      2, 3,
    ]);
  });

  it("fails closed after a save error and does not issue later writes", async () => {
    const failure = vi.fn();
    const invoke = vi.fn(async () => {
      throw { code: "REVISION_CONFLICT", message: "stale revision" };
    }) as unknown as InvokeFunction;
    const writer = new ApplicationStateWriter(invoke, 4, failure);

    writer.enqueue(createDefaultApplicationState());
    await expect(writer.flush()).rejects.toMatchObject({
      code: "REVISION_CONFLICT",
    });
    expect(writer.hasFailed).toBe(true);
    await expect(writer.reset("RESET")).rejects.toMatchObject({
      code: "REVISION_CONFLICT",
    });
    writer.enqueue(createDefaultApplicationState());

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(failure).toHaveBeenCalledTimes(1);
  });

  it("flushes pending state before an authoritative registry mutation and advances its revision", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invoke: InvokeFunction = async <T>(
      command: string,
      args?: Record<string, unknown>,
    ) => {
      calls.push({ command, args });
      if (command === "save_application_state") {
        return { schemaVersion: 5, revision: 6 } as T;
      }
      return envelope(7) as T;
    };
    const writer = new ApplicationStateWriter(invoke, 5, vi.fn());
    writer.enqueue(createDefaultApplicationState());

    await writer.mutateAgentRegistry("create_agent", {
      name: "Custom Builder",
      description: "Builds custom workspace features",
      role: "Specialist",
      category: "Development",
      reportsTo: 3,
    });
    await writer.mutateAgentRegistry("delete_agent", {
      agentId: 12,
      replacementManagerId: null,
    });

    expect(calls.map((call) => call.command)).toEqual([
      "save_application_state",
      "create_agent",
      "delete_agent",
    ]);
    expect(calls[1].args?.request).toMatchObject({ expectedRevision: 6 });
    expect(calls[2].args?.request).toMatchObject({ expectedRevision: 7 });
  });

  it("serializes task-orchestration commands behind pending state and adopts their revisions", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invoke: InvokeFunction = async <T>(
      command: string,
      args?: Record<string, unknown>,
    ) => {
      calls.push({ command, args });
      if (command === "save_application_state") {
        return { schemaVersion: 5, revision: 12 } as T;
      }
      return envelope(command === "create_routed_task" ? 13 : 14) as T;
    };
    const writer = new ApplicationStateWriter(invoke, 11, vi.fn());
    writer.enqueue(createDefaultApplicationState());

    await writer.mutateTaskOrchestration("create_routed_task", {
      taskOwnerAgentId: 1,
      title: "Build the queue",
    });
    await writer.mutateTaskOrchestration("set_task_queue_disposition", {
      taskOwnerAgentId: 1,
      taskId: 101,
      disposition: "hold",
    });

    expect(calls.map((call) => call.command)).toEqual([
      "save_application_state",
      "create_routed_task",
      "set_task_queue_disposition",
    ]);
    expect(calls[1].args?.request).toMatchObject({ expectedRevision: 12 });
    expect(calls[2].args?.request).toMatchObject({ expectedRevision: 13 });
  });

  it("adopts a freshly loaded authoritative revision before the next write", async () => {
    const invoke = vi.fn(async () => envelope(22)) as unknown as InvokeFunction;
    const writer = new ApplicationStateWriter(invoke, 20, vi.fn());

    writer.adoptRevision(21);
    await writer.mutateTaskOrchestration("reroute_task", {
      taskOwnerAgentId: 1,
      taskId: 101,
    });

    expect(invoke).toHaveBeenCalledWith("reroute_task", {
      request: expect.objectContaining({ expectedRevision: 21 }),
    });
  });
});
