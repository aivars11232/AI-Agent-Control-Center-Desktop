import applicationStateSeed from "./application-state-seed.json";
import type {
  RoutingEvidence,
  TaskQueueState,
} from "./taskOrchestration";
import type { WorkspaceChangeEvidence } from "./workspaceEvidence";
import type { SpecialistTaskRequest } from "./specialistCapabilities";

export type VoiceState = "VOICE_OFF" | "VOICE_PASSIVE" | "VOICE_ACTIVE";
export type AccessLevel = "none" | "read" | "write" | "full";
export type ApprovalMode = "allow" | "ask" | "deny";
export type TaskStatus =
  | "Pending"
  | "Running"
  | "Blocked"
  | "Under Review"
  | "Completed"
  | "Failed";
export type TaskCategory =
  | "Development"
  | "Research"
  | "Browsing"
  | "Finance"
  | "Business"
  | "Communication"
  | "System Control"
  | "General";
export type TaskPriority = "Low" | "Normal" | "High" | "Critical";
export type HistoryRetentionDays = 7 | 30 | 90 | "never";
export type AgentStatus = "Working" | "Waiting" | "Paused";
export type ThemeMode = "dark" | "light" | "system";
export type AccentColor = "violet" | "blue" | "cyan" | "green";
export type InterfaceDensity = "comfortable" | "compact";
export type ExecutionFocus = "speed" | "balanced" | "strength";
export type OverflowAction = "queue" | "redirect";
export type SafetyMode = "balanced" | "strict" | "locked";
export type RoutingMode = "selected" | "automatic";
export type ReviewMode = "off" | "manual" | "automatic";
export type ReviewStatus =
  | "Not Requested"
  | "Pending"
  | "Running"
  | "Approved"
  | "Changes Requested"
  | "Failed";
export type SafetyScope =
  | "files"
  | "internet"
  | "clipboard"
  | "terminal"
  | "system";
export type RiskLevel = "Low" | "Medium" | "High" | "Critical";

export type WorkspaceDefinition = {
  id: string;
  name: string;
  path: string;
};

export type AgentPerformance = {
  strength: number;
  focus: ExecutionFocus;
  cpuLimit: number;
  gpuLimit: number;
  queueThreshold: number;
  overflowAction: OverflowAction;
  redirectAgentId: number | null;
};

export type AppPreferences = {
  theme: ThemeMode;
  accentColor: AccentColor;
  density: InterfaceDensity;
  reducedMotion: boolean;
  defaultModel: string;
  activeAiProvider: RuntimeProviderId;
  defaultAgentStatus: AgentStatus;
  defaultTaskCategory: TaskCategory;
  defaultTaskPriority: TaskPriority;
  defaultPerformance: AgentPerformance;
  workspacePath: string;
  workspaces: WorkspaceDefinition[];
  activeWorkspaceId: string | null;
  agentTimeoutMinutes: number;
  safetyMode: SafetyMode;
  approvalExpiryMinutes: number;
  defaultRoutingMode: RoutingMode;
  reviewMode: ReviewMode;
  backgroundVoiceEnabled: boolean;
  voiceControlMasterEnabled: boolean;
  voiceWakePhrase: string;
  voiceDeactivatePhrase: string;
  voiceOpenPhrases: string;
  voiceClosePhrases: string;
  voiceCommandReplacements: string;
  voiceState: VoiceState;
};

export type ApprovalRequestStatus =
  | "Pending"
  | "Approved"
  | "Denied"
  | "Expired";

export type ApprovalRequest = {
  id: number;
  agentId: number;
  taskId: number | null;
  title: string;
  reason: string;
  status: ApprovalRequestStatus;
  createdAt: string;
  resolvedAt: string | null;
  riskLevel: RiskLevel;
  scopes: SafetyScope[];
  workspaceId: string | null;
  taskSnapshot: string;
  expiresAt: string;
  consumedAt: string | null;
};

export type TaskPhase =
  | "Assigned"
  | "Specialist Work"
  | "Senior Review"
  | "Team Leader Review"
  | "Supervisor Approval"
  | "Finished"
  | "Failed";

export type AgentTask = {
  id: number;
  title: string;
  category: TaskCategory;
  priority: TaskPriority;
  assignedAgentId: number;
  status: TaskStatus;
  phase: TaskPhase;
  createdAt: string;
  completedAt: string | null;
  result: string | null;
  responseId: string | null;
  runtimeModel: string | null;
  totalTokens: number | null;
  workspaceId: string | null;
  changedFiles: string[];
  diff: string | null;
  workspaceChanges: WorkspaceChangeEvidence | null;
  durationSeconds: number | null;
  routingMode: RoutingMode;
  routedFromAgentId: number | null;
  routingReason: string | null;
  queueState: TaskQueueState;
  enqueueSequence: number | null;
  routingEvidence: RoutingEvidence | null;
  specialistRequest: SpecialistTaskRequest | null;
  reviewAgentId: number | null;
  reviewStatus: ReviewStatus;
  reviewResult: string | null;
  reviewModel: string | null;
  reviewDurationSeconds: number | null;
  reviewedAt: string | null;
};

export type ModelProvider =
  | "OpenAI"
  | "Anthropic"
  | "Google"
  | "Ollama"
  | "Custom";
export type RuntimeProviderId = "codex" | "ollama";

export type ModelDefinition = {
  id: number;
  name: string;
  provider: ModelProvider;
};

export type ActivityEntry = {
  id: number;
  message: string;
  createdAt: string;
};

export type ReminderStatus = "Upcoming" | "Completed" | "Dismissed";

export type Reminder = {
  id: number;
  title: string;
  notes: string;
  dueAt: string;
  status: ReminderStatus;
  agentId: number | null;
  taskId: number | null;
  createdAt: string;
};

export type AgentRole =
  | "Supervisor"
  | "Team Leader"
  | "Senior Agent"
  | "Specialist";
export type AgentCategory =
  | "Management"
  | "Development"
  | "Research"
  | "Browsing"
  | "Finance"
  | "Business"
  | "Communication"
  | "System Control"
  | "General";
export type AuthorityLevel = 1 | 2 | 3 | 4;
export type AgentTemplateKey =
  | "supervisor"
  | "coding"
  | "debugging"
  | "browser"
  | "financial"
  | "development-team-leader"
  | "pc-control"
  | "event-reminder"
  | "research-web-senior"
  | "finance-senior"
  | "operations-senior";
export type AgentRegistryState = "active" | "unassigned" | "deleted";
export type AgentRegistryIssue =
  | "self-parent"
  | "missing-manager"
  | "manager-not-active"
  | "manager-authority"
  | "cycle";

export type Agent = {
  id: number;
  templateKey: AgentTemplateKey | null;
  registryState: AgentRegistryState;
  registryIssue: AgentRegistryIssue | null;
  deletedAtUnixMs: number | null;
  name: string;
  description: string;
  status: AgentStatus;
  role: AgentRole;
  category: AgentCategory;
  reportsTo: number | null;
  authorityLevel: AuthorityLevel;
  model: string;
  memory: string;
  tasks: AgentTask[];
  activity: ActivityEntry[];
  performance: AgentPerformance;
  capabilities: {
    files: AccessLevel;
    internet: AccessLevel;
    clipboard: AccessLevel;
    terminal: "none" | "safe" | "user" | "admin";
    system: "none" | "notifications" | "power" | "full";
  };
  approvals: {
    files: ApprovalMode;
    internet: ApprovalMode;
    clipboard: ApprovalMode;
    terminal: ApprovalMode;
    system: ApprovalMode;
  };
};

export type ApplicationState = {
  agents: Agent[];
  models: ModelDefinition[];
  approvalRequests: ApprovalRequest[];
  reminders: Reminder[];
  taskRetentionDays: HistoryRetentionDays;
  activityRetentionDays: HistoryRetentionDays;
  preferences: AppPreferences;
};

export type MigrationInfo = {
  sourceKind: string | null;
  sourceVersion: number | null;
  migratedAtUnixMs: number | null;
  legacyCleanupAcknowledged: boolean;
};

export type StateEnvelope = {
  schemaVersion: number;
  revision: number;
  state: ApplicationState;
  migration: MigrationInfo;
};

export type SaveReceipt = {
  schemaVersion: number;
  revision: number;
};

export type PersistenceError = {
  code: string;
  message: string;
  recoverable: boolean;
};

export type LegacyRendererState = {
  agents: string | null;
  models: string | null;
  approvalRequests: string | null;
  reminders: string | null;
  taskRetentionDays: string | null;
  activityRetentionDays: string | null;
  preferences: string | null;
};

export function createDefaultApplicationState(): ApplicationState {
  return JSON.parse(JSON.stringify(applicationStateSeed)) as ApplicationState;
}
