export type ScheduledItemKind = "reminder" | "event";
export type ScheduleStatus =
  | "scheduled"
  | "due"
  | "completed"
  | "dismissed"
  | "needs_attention";
export type RecurrenceKind = "none" | "daily" | "weekly" | "monthly";
export type DeliveryMode = "in_app" | "portal";
export type PrivacyMode = "generic" | "title";
export type DstResolution =
  | "exact"
  | "fold_earlier"
  | "gap_shifted_forward"
  | "unresolved";

export type RecurrenceRule = {
  kind: RecurrenceKind;
  interval: number;
  occurrenceLimit: number | null;
  untilUnixMs: number | null;
};

export type ScheduledItem = {
  id: number;
  position: number;
  revision: number;
  kind: ScheduledItemKind;
  title: string;
  notes: string;
  localDueAt: string;
  timeZone: string;
  dueAt: string;
  dueAtUnixMs: number | null;
  eventEndLocal: string | null;
  eventEndUnixMs: number | null;
  dstResolution: DstResolution;
  status: ScheduleStatus;
  recurrence: RecurrenceRule;
  nextOccurrenceSequence: number;
  missedOccurrenceCount: number;
  deliveryMode: DeliveryMode;
  privacyMode: PrivacyMode;
  scheduleFingerprint: string | null;
  subjectAgentId: number | null;
  workspaceId: string | null;
  taskOwnerAgentId: number | null;
  taskId: number | null;
  schedulerAgentId: number | null;
  scheduleIssueCode: string | null;
  scheduleIssueMessage: string | null;
  createdAt: string;
  createdAtUnixMs: number;
  resolvedAtUnixMs: number | null;
  updatedAtUnixMs: number;
};

export type ReminderOccurrence = {
  id: number;
  reminderId: number;
  scheduleRevision: number;
  occurrenceSequence: number;
  occurrenceKey: string;
  dueAtUnixMs: number;
  status: string;
  missedCount: number;
  firstMissedAtUnixMs: number | null;
  lastMissedAtUnixMs: number | null;
  detailCode: string | null;
  detailMessage: string | null;
  createdAtUnixMs: number;
  updatedAtUnixMs: number;
};

export type ReminderSchedulerSnapshot = {
  revision: number;
  applicationStateRevision: number;
  systemTimeZone: string | null;
  items: ScheduledItem[];
  recentOccurrences: ReminderOccurrence[];
};

export type ReminderSchedulerCommand =
  | "create_scheduled_item"
  | "update_scheduled_item"
  | "set_scheduled_item_status"
  | "delete_scheduled_item";

export type DueClassification =
  | "overdue"
  | "due_now"
  | "due_soon"
  | "future"
  | "inactive"
  | "needs_attention";

export const emptyReminderSchedulerSnapshot: ReminderSchedulerSnapshot = {
  revision: 0,
  applicationStateRevision: 0,
  systemTimeZone: null,
  items: [],
  recentOccurrences: [],
};

export function browserTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

export function defaultLocalDateTime(now = new Date()): string {
  const next = new Date(now.getTime() + 60 * 60 * 1000);
  next.setSeconds(0, 0);
  const offset = next.getTimezoneOffset() * 60_000;
  return new Date(next.getTime() - offset).toISOString().slice(0, 16);
}

export function classifyScheduledItem(
  item: ScheduledItem,
  nowUnixMs: number,
  dueWindowEndUnixMs: number,
): DueClassification {
  if (item.status === "needs_attention" || item.dueAtUnixMs === null) {
    return "needs_attention";
  }
  if (item.status !== "scheduled" && item.status !== "due") {
    return "inactive";
  }
  if (item.dueAtUnixMs < nowUnixMs) return "overdue";
  if (item.dueAtUnixMs === nowUnixMs) return "due_now";
  if (item.dueAtUnixMs <= dueWindowEndUnixMs) return "due_soon";
  return "future";
}

