import type { TaskOrchestrationCommand } from "../persistence";

export type TaskOrchestrationMutation = (
  command: TaskOrchestrationCommand,
  request: Record<string, unknown>,
) => Promise<void>;
