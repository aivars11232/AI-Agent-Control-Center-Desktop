import type { Agent, AgentTask } from "../App";

type AgentFixtureOverrides = Omit<
  Partial<Agent>,
  "approvals" | "capabilities" | "performance"
> & {
  approvals?: Partial<Agent["approvals"]>;
  capabilities?: Partial<Agent["capabilities"]>;
  performance?: Partial<Agent["performance"]>;
};

export function taskFixture(overrides: Partial<AgentTask> = {}): AgentTask {
  return {
    id: 101,
    title: "Inspect the project status",
    category: "General",
    priority: "Normal",
    assignedAgentId: 1,
    status: "Pending",
    phase: "Assigned",
    createdAt: "2026-01-02T03:04:05.000Z",
    completedAt: null,
    result: null,
    responseId: null,
    runtimeModel: null,
    totalTokens: null,
    workspaceId: "workspace-1",
    changedFiles: [],
    diff: null,
    durationSeconds: null,
    routingMode: "selected",
    routedFromAgentId: null,
    routingReason: null,
    reviewAgentId: null,
    reviewStatus: "Not Requested",
    reviewResult: null,
    reviewModel: null,
    reviewDurationSeconds: null,
    reviewedAt: null,
    ...overrides,
  };
}

export function agentFixture(overrides: AgentFixtureOverrides = {}): Agent {
  const capabilities: Agent["capabilities"] = {
    files: "write",
    internet: "read",
    clipboard: "read",
    terminal: "user",
    system: "notifications",
    ...overrides.capabilities,
  };
  const approvals: Agent["approvals"] = {
    files: "allow",
    internet: "allow",
    clipboard: "allow",
    terminal: "allow",
    system: "allow",
    ...overrides.approvals,
  };
  const performance: Agent["performance"] = {
    strength: 5,
    focus: "balanced",
    cpuLimit: 70,
    gpuLimit: 50,
    overflowAction: "queue",
    redirectAgentId: null,
    ...overrides.performance,
  };

  return {
    id: 1,
    templateKey: null,
    registryState: "active",
    registryIssue: null,
    deletedAtUnixMs: null,
    name: "Fixture Agent",
    description: "Deterministic characterization fixture",
    status: "Waiting",
    role: "Specialist",
    category: "Development",
    reportsTo: null,
    authorityLevel: 1,
    model: "fixture-model",
    memory: "",
    tasks: [],
    activity: [],
    ...overrides,
    capabilities,
    approvals,
    performance,
  };
}
