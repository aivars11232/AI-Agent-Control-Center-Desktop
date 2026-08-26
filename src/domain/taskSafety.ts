import type {
  Agent,
  AgentTask,
  RiskLevel,
  SafetyMode,
  SafetyScope,
} from "../applicationState";

export type TaskSafetyAssessment = {
  riskLevel: RiskLevel;
  scopes: SafetyScope[];
  approvalScopes: SafetyScope[];
  requiresApproval: boolean;
  destructive: boolean;
  writesWorkspace: boolean;
  blockedReason: string | null;
  reason: string;
};

export const safetyScopeLabels: Record<SafetyScope, string> = {
  files: "workspace files",
  internet: "web access",
  clipboard: "clipboard",
  terminal: "terminal commands",
  system: "system control",
};

export function taskSafetyAssessment(
  task: AgentTask,
  agent: Agent,
  safetyMode: SafetyMode,
): TaskSafetyAssessment {
  const text = `${task.title} ${task.category}`.toLowerCase();
  const scopes = new Set<SafetyScope>();
  const mutatesWorkspace =
    /\b(create|write|edit|modify|change|update|refactor|fix|move|rename|replace|generate|add|implement)\b/i.test(
      text,
    );
  const destructive =
    /\b(delete|remove|erase|wipe|truncate|overwrite|reset\s+--hard|clean\s+-[a-z]*f|rm\s|rmdir|unlink)\b/i.test(
      text,
    );
  const terminal =
    /\b(command|terminal|shell|bash|execute|run\s+(?:the\s+)?command|npm|pnpm|yarn|cargo|rustc|git|python|pytest|sleep|build|compile|install)\b/i.test(
      text,
    );
  const internet =
    task.category === "Browsing" ||
    /\b(internet|website|web\s+search|browse|download|upload|curl|wget|url|online)\b/i.test(
      text,
    );
  const clipboard = /\bclipboard|copy\s+to|paste\s+from\b/i.test(text);
  const system =
    task.category === "System Control" ||
    /\b(systemctl|reboot|shutdown|power\s*off|desktop\s+control|computer\s+control|open\s+(?:an\s+)?app|close\s+(?:an\s+)?app)\b/i.test(
      text,
    );
  const privileged =
    /\b(sudo|doas|mkfs|chown|chmod|mount|umount|pacman|apt|dnf|account\s+management|package\s+removal)\b/i.test(
      text,
    );
  const writesWorkspace =
    mutatesWorkspace ||
    destructive ||
    /\b(build|compile|install|format)\b/i.test(text);

  if (writesWorkspace || task.category === "Development") {
    scopes.add("files");
  }
  if (terminal) scopes.add("terminal");
  if (internet) scopes.add("internet");
  if (clipboard) scopes.add("clipboard");
  if (system) scopes.add("system");

  const scopeList = Array.from(scopes);
  const missingCapabilities = scopeList.filter(
    (scope) => agent.capabilities[scope] === "none",
  );
  const deniedScopes = scopeList.filter(
    (scope) => agent.approvals[scope] === "deny",
  );
  const lockedAction =
    safetyMode === "locked" &&
    (mutatesWorkspace ||
      destructive ||
      terminal ||
      internet ||
      clipboard ||
      system);

  let blockedReason: string | null = null;
  if (privileged) {
    blockedReason =
      "Privileged and operating-system package commands are blocked by the desktop safety boundary.";
  } else if (scopes.has("terminal") && agent.capabilities.terminal === "admin") {
    blockedReason =
      "Administrator terminal access is blocked. Change this agent to Safe or User commands.";
  } else if (system) {
    blockedReason =
      "System-control tools are not enabled yet. This release confines Codex to the selected workspace.";
  } else if (lockedAction) {
    blockedReason =
      "Locked mode permits inspection only. Change the Safety mode in Settings to run this action.";
  } else if (missingCapabilities.length > 0) {
    blockedReason = `The agent lacks ${missingCapabilities
      .map((scope) => safetyScopeLabels[scope])
      .join(", ")} capability.`;
  } else if (deniedScopes.length > 0) {
    blockedReason = `${deniedScopes
      .map((scope) => safetyScopeLabels[scope])
      .join(", ")} is denied by this agent's approval policy.`;
  }

  const elevatedScopes = scopeList.filter(
    (scope) => agent.approvals[scope] === "ask",
  );
  const approvalScopes = Array.from(
    new Set<SafetyScope>([
      ...elevatedScopes,
      ...(destructive ? (["files"] as SafetyScope[]) : []),
      ...(task.priority === "Critical" ? scopeList : []),
      ...(safetyMode === "strict" ? scopeList : []),
    ]),
  );
  const riskLevel: RiskLevel =
    privileged || system
      ? "Critical"
      : destructive || task.priority === "Critical"
        ? "High"
        : terminal || internet || clipboard || mutatesWorkspace
          ? "Medium"
          : "Low";
  const requiresApproval = blockedReason === null && approvalScopes.length > 0;
  const reason = blockedReason
    ? blockedReason
    : requiresApproval
      ? `${riskLevel}-risk task requests ${approvalScopes
          .map((scope) => safetyScopeLabels[scope])
          .join(", ")}. Authorization applies to one run only.`
      : `${riskLevel}-risk task is permitted by the current agent and application policies.`;

  return {
    riskLevel,
    scopes: scopeList,
    approvalScopes,
    requiresApproval,
    destructive,
    writesWorkspace,
    blockedReason,
    reason,
  };
}
