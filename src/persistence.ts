import type {
  ApplicationState,
  LegacyRendererState,
  PersistenceError,
  SaveReceipt,
  StateEnvelope,
} from "./applicationState";
import type { BackupImportPreview } from "./dataLifecycle";

export const LEGACY_STORAGE_KEYS = {
  agents: "ai-agent-control-center-agents",
  models: "ai-agent-control-center-models",
  approvalRequests: "ai-agent-control-center-approval-requests",
  reminders: "ai-agent-control-center-reminders",
  taskRetentionDays: "ai-agent-control-center-task-retention",
  activityRetentionDays: "ai-agent-control-center-activity-retention",
  preferences: "ai-agent-control-center-preferences",
} as const;

export type InvokeFunction = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type StorageReader = Pick<Storage, "getItem" | "removeItem">;

export type BootstrapResult = {
  envelope: StateEnvelope;
  cleanupWarning: string | null;
};

export type TaskOrchestrationCommand =
  | "create_routed_task"
  | "reroute_task"
  | "set_task_queue_disposition";

export function collectLegacyRendererState(
  storage: StorageReader,
): LegacyRendererState {
  return {
    agents: storage.getItem(LEGACY_STORAGE_KEYS.agents),
    models: storage.getItem(LEGACY_STORAGE_KEYS.models),
    approvalRequests: storage.getItem(LEGACY_STORAGE_KEYS.approvalRequests),
    reminders: storage.getItem(LEGACY_STORAGE_KEYS.reminders),
    taskRetentionDays: storage.getItem(
      LEGACY_STORAGE_KEYS.taskRetentionDays,
    ),
    activityRetentionDays: storage.getItem(
      LEGACY_STORAGE_KEYS.activityRetentionDays,
    ),
    preferences: storage.getItem(LEGACY_STORAGE_KEYS.preferences),
  };
}

export function clearLegacyRendererState(storage: StorageReader): void {
  for (const key of Object.values(LEGACY_STORAGE_KEYS)) {
    storage.removeItem(key);
  }
}

export async function bootstrapDesktopApplicationState(
  invoke: InvokeFunction,
  storage: StorageReader,
): Promise<BootstrapResult> {
  const existing = await invoke<StateEnvelope | null>(
    "load_application_state",
  );
  let envelope =
    existing ??
    (await invoke<StateEnvelope>("initialize_application_state", {
      request: { legacy: collectLegacyRendererState(storage) },
    }));
  if (
    envelope.migration.sourceKind !== "legacy_local_storage" ||
    envelope.migration.legacyCleanupAcknowledged
  ) {
    return { envelope, cleanupWarning: null };
  }

  try {
    clearLegacyRendererState(storage);
  } catch {
    return {
      envelope,
      cleanupWarning:
        "State migration committed, but legacy browser storage could not be fully removed.",
    };
  }

  try {
    envelope = await invoke<StateEnvelope>("acknowledge_legacy_cleanup", {
      request: { expectedRevision: envelope.revision },
    });
    return { envelope, cleanupWarning: null };
  } catch {
    return {
      envelope,
      cleanupWarning:
        "State migration committed and legacy data was removed, but cleanup acknowledgement could not be recorded.",
    };
  }
}

export class ApplicationStateWriter {
  private revision: number;
  private pendingState: ApplicationState | null = null;
  private drainPromise: Promise<void> | null = null;
  private failed = false;
  private failure: unknown;

  constructor(
    private readonly invoke: InvokeFunction,
    initialRevision: number,
    private readonly onFailure: (error: unknown) => void,
    private readonly onCommit: (receipt: SaveReceipt) => void = () => {},
  ) {
    this.revision = initialRevision;
  }

  get hasFailed(): boolean {
    return this.failed;
  }

  enqueue(state: ApplicationState): void {
    if (this.failed) {
      return;
    }
    this.pendingState = state;
    if (!this.drainPromise) {
      this.drainPromise = this.drain();
    }
  }

  async flush(): Promise<void> {
    await this.drainPromise;
    if (this.failed) {
      throw this.failure;
    }
  }

  async importLegacyBackup(backupJson: string): Promise<StateEnvelope> {
    await this.flush();
    const envelope = await this.invoke<StateEnvelope>("import_legacy_backup", {
      request: {
        expectedRevision: this.revision,
        backupJson,
      },
    });
    this.revision = envelope.revision;
    return envelope;
  }

  async previewBackupImport(backupJson: string): Promise<BackupImportPreview> {
    await this.flush();
    return this.invoke<BackupImportPreview>("preview_backup_import", {
      request: {
        expectedRevision: this.revision,
        backupJson,
      },
    });
  }

  async applyBackupImport(backupJson: string): Promise<StateEnvelope> {
    await this.flush();
    const envelope = await this.invoke<StateEnvelope>("apply_backup_import", {
      request: {
        expectedRevision: this.revision,
        backupJson,
      },
    });
    this.revision = envelope.revision;
    return envelope;
  }

  async reset(confirmation: string): Promise<StateEnvelope> {
    await this.flush();
    const envelope = await this.invoke<StateEnvelope>("reset_application_state", {
      request: {
        expectedRevision: this.revision,
        confirmation,
      },
    });
    this.revision = envelope.revision;
    return envelope;
  }

  async mutateAgentRegistry(
    command: "create_agent" | "update_agent" | "delete_agent" | "restore_agent_template",
    request: Record<string, unknown>,
  ): Promise<StateEnvelope> {
    await this.flush();
    const envelope = await this.invoke<StateEnvelope>(command, {
      request: {
        ...request,
        expectedRevision: this.revision,
      },
    });
    this.revision = envelope.revision;
    return envelope;
  }

  async mutateTaskOrchestration(
    command: TaskOrchestrationCommand,
    request: Record<string, unknown>,
  ): Promise<StateEnvelope> {
    await this.flush();
    const envelope = await this.invoke<StateEnvelope>(command, {
      request: {
        ...request,
        expectedRevision: this.revision,
      },
    });
    this.revision = envelope.revision;
    return envelope;
  }

  adoptRevision(revision: number): void {
    if (this.drainPromise || this.pendingState) {
      throw new Error(
        "Cannot adopt an authoritative revision while a state write is pending.",
      );
    }
    this.revision = revision;
  }

  private async drain(): Promise<void> {
    try {
      while (this.pendingState) {
        const state = this.pendingState;
        this.pendingState = null;
        const receipt = await this.invoke<SaveReceipt>(
          "save_application_state",
          {
            request: {
              expectedRevision: this.revision,
              state,
            },
          },
        );
        this.revision = receipt.revision;
        this.onCommit(receipt);
      }
    } catch (error) {
      this.failed = true;
      this.failure = error;
      this.onFailure(error);
    } finally {
      this.drainPromise = null;
      if (this.pendingState && !this.failed) {
        this.enqueue(this.pendingState);
      }
    }
  }
}

export function persistenceErrorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as Partial<PersistenceError>).message === "string"
  ) {
    const persistenceError = error as Partial<PersistenceError>;
    return persistenceError.code
      ? `${persistenceError.code}: ${persistenceError.message}`
      : persistenceError.message ?? "Application persistence failed.";
  }
  return error instanceof Error
    ? error.message
    : "Application persistence failed.";
}
