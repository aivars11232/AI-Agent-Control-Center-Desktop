export type MemoryScopeKind = "agent" | "project" | "task" | "team";
export type MemoryRecordKind = "instruction" | "fact" | "decision" | "summary";
export type MemoryProvenance =
  | "user"
  | "legacy_agent_memory"
  | "handoff_promotion"
  | "backup_import";
export type MemoryRetention = "manual" | "7d" | "30d" | "90d" | "task_lifetime";

export type MemoryScope = {
  kind: MemoryScopeKind;
  agentId: number | null;
  workspaceId: string | null;
  taskOwnerAgentId: number | null;
  taskId: number | null;
  teamLeaderAgentId: number | null;
};

export type MemoryRecord = {
  id: number;
  scope: MemoryScope;
  kind: MemoryRecordKind;
  content: string;
  provenance: MemoryProvenance;
  provenanceRef: string | null;
  revision: number;
  retention: MemoryRetention;
  expiresAtUnixMs: number | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
};

export type MemoryEvent = {
  id: number;
  recordId: number | null;
  action: string;
  actorKind: string;
  recordRevision: number;
  createdAtUnixMs: number;
};

export type StructuredMemorySnapshot = {
  revision: number;
  applicationStateRevision: number;
  records: MemoryRecord[];
  recentEvents: MemoryEvent[];
};

export type StructuredMemoryCommand =
  | "create_memory_record"
  | "update_memory_record"
  | "delete_memory_record";

export const emptyStructuredMemorySnapshot: StructuredMemorySnapshot = {
  revision: 0,
  applicationStateRevision: 0,
  records: [],
  recentEvents: [],
};

export function describeMemoryScope(scope: MemoryScope): string {
  switch (scope.kind) {
    case "agent":
      return `Agent ${scope.agentId}`;
    case "project":
      return `Project ${scope.workspaceId}`;
    case "task":
      return `Task ${scope.taskOwnerAgentId}/${scope.taskId}`;
    case "team":
      return `Team led by agent ${scope.teamLeaderAgentId}`;
  }
}

