import type {
  Agent,
  AgentCategory,
  AgentRegistryIssue,
  AgentRole,
  AgentTemplateKey,
  AuthorityLevel,
} from "./applicationState";

export type AgentGroup =
  | "All agents"
  | "Development"
  | "Finance and Events"
  | "Web and PC Control"
  | "General"
  | "Needs assignment";

export type AgentTemplateSummary = {
  templateKey: AgentTemplateKey;
  name: string;
  description: string;
  role: AgentRole;
  category: AgentCategory;
  authorityLevel: AuthorityLevel;
  activeAgentId: number | null;
  restorable: boolean;
};

export type AgentRegistrySnapshot = {
  revision: number;
  templates: AgentTemplateSummary[];
};

export type AgentHierarchyRow = {
  agent: Agent;
  depth: number;
  detached: boolean;
};

export type AgentGroupProjection = {
  group: AgentGroup;
  visibleAgents: Agent[];
  memberIds: Set<number>;
  rows: AgentHierarchyRow[];
};

const templateByLegacyIdentity: Record<number, {
  role: AgentRole;
  category: AgentCategory;
  templateKey: AgentTemplateKey;
}> = {
  1: { role: "Supervisor", category: "Management", templateKey: "supervisor" },
  2: { role: "Specialist", category: "Development", templateKey: "coding" },
  3: { role: "Senior Agent", category: "Development", templateKey: "debugging" },
  4: { role: "Specialist", category: "Browsing", templateKey: "browser" },
  5: { role: "Specialist", category: "Finance", templateKey: "financial" },
  6: {
    role: "Team Leader",
    category: "Management",
    templateKey: "development-team-leader",
  },
  7: { role: "Specialist", category: "System Control", templateKey: "pc-control" },
  8: { role: "Specialist", category: "Business", templateKey: "event-reminder" },
  9: {
    role: "Senior Agent",
    category: "Browsing",
    templateKey: "research-web-senior",
  },
  10: { role: "Senior Agent", category: "Finance", templateKey: "finance-senior" },
  11: { role: "Senior Agent", category: "Business", templateKey: "operations-senior" },
};

const templateKeys = new Set<AgentTemplateKey>(
  Object.values(templateByLegacyIdentity).map(({ templateKey }) => templateKey),
);
const registryIssues = new Set<AgentRegistryIssue>([
  "self-parent",
  "missing-manager",
  "manager-not-active",
  "manager-authority",
  "cycle",
]);

export function authorityForRole(role: AgentRole): AuthorityLevel {
  if (role === "Supervisor") return 4;
  if (role === "Team Leader") return 3;
  if (role === "Senior Agent") return 2;
  return 1;
}

export function activeRegistryAgents(agents: Agent[]): Agent[] {
  return agents.filter((agent) => agent.registryState === "active");
}

export function findActiveTemplateAgent(
  agents: Agent[],
  templateKey: AgentTemplateKey,
): Agent | null {
  return (
    agents.find(
      (agent) =>
        agent.registryState === "active" && agent.templateKey === templateKey,
    ) ?? null
  );
}

function baseGroupForCategory(category: AgentCategory): AgentGroup | null {
  if (category === "Development") return "Development";
  if (category === "Finance" || category === "Business") {
    return "Finance and Events";
  }
  if (
    category === "Browsing" ||
    category === "Research" ||
    category === "Communication" ||
    category === "System Control"
  ) {
    return "Web and PC Control";
  }
  if (category === "General") return "General";
  return null;
}

export function availableAgentGroups(agents: Agent[]): AgentGroup[] {
  const active = activeRegistryAgents(agents);
  const groups: AgentGroup[] = ["All agents"];
  for (const group of [
    "Development",
    "Finance and Events",
    "Web and PC Control",
    "General",
  ] as const) {
    if (active.some((agent) => baseGroupForCategory(agent.category) === group)) {
      groups.push(group);
    }
  }
  if (agents.some((agent) => agent.registryState === "unassigned")) {
    groups.push("Needs assignment");
  }
  return groups;
}

function hierarchyRows(agents: Agent[], visibleIds: Set<number>): AgentHierarchyRow[] {
  const visible = agents.filter((agent) => visibleIds.has(agent.id));
  const byManager = new Map<number | null, Agent[]>();
  for (const agent of visible) {
    const manager =
      agent.reportsTo !== null && visibleIds.has(agent.reportsTo)
        ? agent.reportsTo
        : null;
    const reports = byManager.get(manager) ?? [];
    reports.push(agent);
    byManager.set(manager, reports);
  }

  const rows: AgentHierarchyRow[] = [];
  const visited = new Set<number>();
  const append = (agent: Agent, depth: number, detached: boolean) => {
    if (visited.has(agent.id)) return;
    visited.add(agent.id);
    rows.push({ agent, depth, detached });
    for (const report of byManager.get(agent.id) ?? []) {
      append(report, Math.min(depth + 1, 4), detached);
    }
  };
  for (const root of byManager.get(null) ?? []) append(root, 0, false);
  for (const agent of visible) {
    if (!visited.has(agent.id)) append(agent, 0, true);
  }
  return rows;
}

export function projectAgentGroup(
  agents: Agent[],
  group: AgentGroup,
): AgentGroupProjection {
  const memberIds = new Set<number>();
  if (group === "Needs assignment") {
    for (const agent of agents) {
      if (agent.registryState === "unassigned") memberIds.add(agent.id);
    }
  } else {
    for (const agent of activeRegistryAgents(agents)) {
      if (
        group === "All agents" ||
        baseGroupForCategory(agent.category) === group
      ) {
        memberIds.add(agent.id);
      }
    }
  }

  const visibleIds = new Set(memberIds);
  const byId = new Map(agents.map((agent) => [agent.id, agent]));
  if (group !== "All agents" && group !== "Needs assignment") {
    for (const memberId of memberIds) {
      const chain = new Set<number>();
      let current = byId.get(memberId)?.reportsTo ?? null;
      while (current !== null && !chain.has(current)) {
        chain.add(current);
        const manager = byId.get(current);
        if (!manager || manager.registryState !== "active") break;
        visibleIds.add(manager.id);
        current = manager.reportsTo;
      }
    }
  }

  const visibleAgents = agents.filter((agent) => visibleIds.has(agent.id));
  return {
    group,
    visibleAgents,
    memberIds,
    rows: hierarchyRows(agents, visibleIds),
  };
}

export function validManagerCandidates(
  agents: Agent[],
  role: AgentRole,
  editingAgentId: number | null,
): Agent[] {
  const authority = authorityForRole(role);
  const descendantIds = new Set<number>();
  if (editingAgentId !== null) {
    const pendingManagerIds = [editingAgentId];
    while (pendingManagerIds.length > 0) {
      const managerId = pendingManagerIds.pop();
      for (const agent of agents) {
        if (
          agent.reportsTo === managerId &&
          !descendantIds.has(agent.id)
        ) {
          descendantIds.add(agent.id);
          pendingManagerIds.push(agent.id);
        }
      }
    }
  }
  return agents.filter(
    (agent) =>
      agent.registryState === "active" &&
      agent.id !== editingAgentId &&
      !descendantIds.has(agent.id) &&
      agent.authorityLevel > authority,
  );
}

export function normalizeLegacyAgentRegistry(agent: Partial<Agent>): Agent {
  const role: AgentRole =
    agent.role === "Supervisor" ||
    agent.role === "Team Leader" ||
    agent.role === "Senior Agent" ||
    agent.role === "Specialist"
      ? agent.role
      : "Specialist";
  const registryState =
    agent.registryState === "unassigned" || agent.registryState === "deleted"
      ? agent.registryState
      : "active";
  const identity = typeof agent.id === "number" ? templateByLegacyIdentity[agent.id] : undefined;
  const legacyTemplateKey =
    identity?.role === role && identity.category === agent.category
      ? identity.templateKey
      : null;
  const templateKey =
    agent.templateKey === null
      ? null
      : agent.templateKey !== undefined && templateKeys.has(agent.templateKey)
        ? agent.templateKey
        : legacyTemplateKey;
  const registryIssue =
    registryState === "unassigned" &&
    agent.registryIssue !== null &&
    agent.registryIssue !== undefined &&
    registryIssues.has(agent.registryIssue)
      ? agent.registryIssue
      : registryState === "unassigned"
        ? "missing-manager"
        : null;
  const deletedAtUnixMs =
    registryState === "deleted" &&
    typeof agent.deletedAtUnixMs === "number" &&
    Number.isSafeInteger(agent.deletedAtUnixMs) &&
    agent.deletedAtUnixMs >= 0
      ? agent.deletedAtUnixMs
      : registryState === "deleted"
        ? 0
        : null;
  return {
    ...(agent as Agent),
    templateKey,
    registryState,
    registryIssue,
    deletedAtUnixMs,
    status: registryState === "active" ? agent.status ?? "Waiting" : "Paused",
    reportsTo: registryState === "active" ? agent.reportsTo ?? null : null,
    role,
    authorityLevel: authorityForRole(role),
  };
}

export function normalizeLegacyAgentRegistrySet(agents: Agent[]): Agent[] {
  let normalized = agents.map((agent) => normalizeLegacyAgentRegistry(agent));
  const activeById = new Map(
    normalized
      .filter((agent) => agent.registryState === "active")
      .map((agent) => [agent.id, agent]),
  );
  const cycleIds = new Set<number>();

  for (const startingAgent of activeById.values()) {
    const path: number[] = [];
    const positions = new Map<number, number>();
    let current: Agent | undefined = startingAgent;
    while (current && current.reportsTo !== null) {
      if (current.reportsTo === current.id) break;
      const priorPosition = positions.get(current.id);
      if (priorPosition !== undefined) {
        path.slice(priorPosition).forEach((id) => cycleIds.add(id));
        break;
      }
      positions.set(current.id, path.length);
      path.push(current.id);
      current = activeById.get(current.reportsTo);
    }
  }

  normalized = normalized.map((agent) => {
    if (agent.registryState !== "active") return agent;
    if (agent.role === "Supervisor") return { ...agent, reportsTo: null };
    if (!cycleIds.has(agent.id)) return agent;
    return {
      ...agent,
      registryState: "unassigned",
      registryIssue: "cycle",
      status: "Paused",
      reportsTo: null,
    };
  });

  let changed = true;
  while (changed) {
    changed = false;
    const byId = new Map(normalized.map((agent) => [agent.id, agent]));
    normalized = normalized.map((agent) => {
      if (agent.registryState !== "active" || agent.role === "Supervisor") {
        return agent;
      }
      const manager =
        agent.reportsTo === null ? undefined : byId.get(agent.reportsTo);
      const issue: AgentRegistryIssue | null =
        agent.reportsTo === agent.id
          ? "self-parent"
          : !manager
            ? "missing-manager"
            : manager.registryState !== "active"
              ? "manager-not-active"
              : manager.authorityLevel <= agent.authorityLevel
                ? "manager-authority"
                : null;
      if (!issue) return agent;
      changed = true;
      return {
        ...agent,
        registryState: "unassigned",
        registryIssue: issue,
        status: "Paused",
        reportsTo: null,
      };
    });
  }

  return normalized;
}

export function registryIssueMessage(issue: AgentRegistryIssue | null): string {
  if (issue === "self-parent") return "This agent previously reported to itself.";
  if (issue === "missing-manager") return "This agent needs an existing manager.";
  if (issue === "manager-not-active") return "This agent's manager is not active.";
  if (issue === "manager-authority") return "This agent needs a manager with greater authority.";
  if (issue === "cycle") return "This agent was part of a reporting cycle.";
  return "This agent needs a valid reporting assignment.";
}
