import { useEffect, useMemo, useState } from "react";
import type { Agent } from "../../applicationState";
import { persistenceErrorMessage } from "../../persistence";
import {
  browserTimeZone,
  classifyScheduledItem,
  defaultLocalDateTime,
  type DeliveryMode,
  type PrivacyMode,
  type RecurrenceKind,
  type ReminderSchedulerCommand,
  type ReminderSchedulerSnapshot,
  type ScheduleStatus,
  type ScheduledItem,
  type ScheduledItemKind,
} from "../../reminderScheduler";

function requestId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function statusLabel(status: ScheduleStatus): string {
  return status.replace("_", " ").replace(/^./, (letter) => letter.toUpperCase());
}

export function RemindersPage({
  agents,
  snapshot,
  authoritative,
  onMutation,
}: {
  agents: Agent[];
  snapshot: ReminderSchedulerSnapshot;
  authoritative: boolean;
  onMutation: (
    command: ReminderSchedulerCommand,
    request: Record<string, unknown>,
  ) => Promise<void>;
}) {
  const [kind, setKind] = useState<ScheduledItemKind>("reminder");
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [localDueAt, setLocalDueAt] = useState(defaultLocalDateTime);
  const [eventEndLocal, setEventEndLocal] = useState("");
  const [timeZone, setTimeZone] = useState(
    snapshot.systemTimeZone ?? browserTimeZone(),
  );
  const [recurrenceKind, setRecurrenceKind] = useState<RecurrenceKind>("none");
  const [recurrenceInterval, setRecurrenceInterval] = useState(1);
  const [occurrenceLimit, setOccurrenceLimit] = useState("");
  const [deliveryMode, setDeliveryMode] = useState<DeliveryMode>("in_app");
  const [privacyMode, setPrivacyMode] = useState<PrivacyMode>("generic");
  const [agentId, setAgentId] = useState<number | null>(null);
  const [taskId, setTaskId] = useState<number | null>(null);
  const [statusFilter, setStatusFilter] = useState<ScheduleStatus | "all">("scheduled");
  const [editingId, setEditingId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [nowUnixMs, setNowUnixMs] = useState(Date.now());

  useEffect(() => {
    if (snapshot.systemTimeZone && editingId === null) {
      setTimeZone(snapshot.systemTimeZone);
    }
  }, [snapshot.systemTimeZone, editingId]);

  useEffect(() => {
    const interval = window.setInterval(() => setNowUnixMs(Date.now()), 30_000);
    return () => window.clearInterval(interval);
  }, []);

  const selectedAgent = agents.find((agent) => agent.id === agentId) ?? null;
  const selectedTasks = selectedAgent?.tasks ?? [];
  const items = useMemo(
    () =>
      snapshot.items
        .filter((item) => statusFilter === "all" || item.status === statusFilter)
        .sort(
          (left, right) =>
            (left.dueAtUnixMs ?? Number.MAX_SAFE_INTEGER) -
            (right.dueAtUnixMs ?? Number.MAX_SAFE_INTEGER),
        ),
    [snapshot.items, statusFilter],
  );
  const dueWindowEnd = nowUnixMs + 24 * 60 * 60 * 1000;
  const overdue = snapshot.items.filter(
    (item) => classifyScheduledItem(item, nowUnixMs, dueWindowEnd) === "overdue",
  ).length;
  const dueSoon = snapshot.items.filter((item) => {
    const classification = classifyScheduledItem(item, nowUnixMs, dueWindowEnd);
    return classification === "due_now" || classification === "due_soon";
  }).length;

  function resetComposer() {
    setEditingId(null);
    setKind("reminder");
    setTitle("");
    setNotes("");
    setLocalDueAt(defaultLocalDateTime());
    setEventEndLocal("");
    setRecurrenceKind("none");
    setRecurrenceInterval(1);
    setOccurrenceLimit("");
    setDeliveryMode("in_app");
    setPrivacyMode("generic");
    setAgentId(null);
    setTaskId(null);
  }

  function editItem(item: ScheduledItem) {
    setEditingId(item.id);
    setKind(item.kind);
    setTitle(item.title);
    setNotes(item.notes);
    setLocalDueAt(item.localDueAt.slice(0, 16));
    setEventEndLocal(item.eventEndLocal?.slice(0, 16) ?? "");
    setTimeZone(item.timeZone);
    setRecurrenceKind(item.recurrence.kind);
    setRecurrenceInterval(item.recurrence.interval);
    setOccurrenceLimit(item.recurrence.occurrenceLimit?.toString() ?? "");
    setDeliveryMode(item.deliveryMode);
    setPrivacyMode(item.privacyMode);
    setAgentId(item.subjectAgentId);
    setTaskId(item.taskId);
    setMessage("");
  }

  async function saveItem() {
    const trimmedTitle = title.trim();
    if (!trimmedTitle || !localDueAt || !timeZone.trim()) {
      setMessage("Title, local due time, and IANA time zone are required.");
      return;
    }
    const current = editingId === null
      ? null
      : snapshot.items.find((item) => item.id === editingId) ?? null;
    if (editingId !== null && !current) {
      setMessage("This scheduled item changed. Refresh and try again.");
      return;
    }
    const linkedTask = selectedTasks.find((task) => task.id === taskId) ?? null;
    const common = {
      expectedRevision: snapshot.revision,
      requestId: requestId(editingId === null ? "schedule-create" : "schedule-update"),
      title: trimmedTitle,
      notes: notes.trim(),
      localDueAt: localDueAt.length === 16 ? `${localDueAt}:00` : localDueAt,
      timeZone: timeZone.trim(),
      eventEndLocal:
        kind === "event" && eventEndLocal
          ? eventEndLocal.length === 16
            ? `${eventEndLocal}:00`
            : eventEndLocal
          : null,
      recurrence: {
        kind: recurrenceKind,
        interval: recurrenceInterval,
        occurrenceLimit: occurrenceLimit ? Number(occurrenceLimit) : null,
        untilUnixMs: null,
      },
      deliveryMode,
      privacyMode,
      subjectAgentId: agentId,
      workspaceId: linkedTask?.workspaceId ?? null,
      taskOwnerAgentId: linkedTask ? agentId : null,
      taskId: linkedTask?.id ?? null,
    };
    setBusy(true);
    setMessage("");
    try {
      if (current) {
        await onMutation("update_scheduled_item", {
          ...common,
          itemId: current.id,
          expectedItemRevision: current.revision,
        });
      } else {
        await onMutation("create_scheduled_item", { ...common, kind });
      }
      resetComposer();
    } catch (error) {
      setMessage(persistenceErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function changeStatus(item: ScheduledItem, status: ScheduleStatus) {
    setBusy(true);
    setMessage("");
    try {
      await onMutation("set_scheduled_item_status", {
        expectedRevision: snapshot.revision,
        expectedItemRevision: item.revision,
        requestId: requestId("schedule-status"),
        itemId: item.id,
        status,
      });
    } catch (error) {
      setMessage(persistenceErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function deleteItem(item: ScheduledItem) {
    if (!window.confirm(`Delete "${item.title}" and its local delivery history?`)) return;
    setBusy(true);
    setMessage("");
    try {
      await onMutation("delete_scheduled_item", {
        expectedRevision: snapshot.revision,
        expectedItemRevision: item.revision,
        requestId: requestId("schedule-delete"),
        itemId: item.id,
      });
      if (editingId === item.id) resetComposer();
    } catch (error) {
      setMessage(persistenceErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">LOCAL EVENT / REMINDER AGENT</span>
          <h1>Reminders &amp; events</h1>
          <p className="page-message">
            The app-owned local scheduler records due and missed occurrences. Becoming due never starts an AI model.
          </p>
        </div>
      </header>

      {!authoritative && (
        <p className="inline-notice">Scheduling is read-only in the browser preview. Open the installed desktop app to create or deliver items.</p>
      )}
      {message && <p className="inline-error" role="alert">{message}</p>}

      <section className="summary-grid">
        <article className="summary-card"><span>Scheduled</span><strong>{snapshot.items.filter((item) => item.status === "scheduled").length}</strong><small>Waiting on local time</small></article>
        <article className="summary-card"><span>Due in 24 hours</span><strong>{dueSoon}</strong><small>Excludes overdue items</small></article>
        <article className="summary-card"><span>Overdue</span><strong>{overdue}</strong><small>Missed occurrences stay explicit</small></article>
        <article className="summary-card"><span>Time zone</span><strong className="summary-value-text">{snapshot.systemTimeZone ?? "Unavailable"}</strong><small>Each item retains its IANA zone</small></article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div><span className="eyebrow">{editingId === null ? "NEW SCHEDULE" : "EDIT SCHEDULE"}</span><h2>{editingId === null ? "Schedule local follow-up" : "Revise exact schedule"}</h2></div>
          {editingId !== null && <button className="secondary-button" onClick={resetComposer}>Cancel edit</button>}
        </div>
        <div className="form-grid reminder-form-grid">
          <label className="form-field"><span>Type</span><select value={kind} disabled={editingId !== null} onChange={(event) => setKind(event.target.value as ScheduledItemKind)}><option value="reminder">Reminder</option><option value="event">Event</option></select></label>
          <label className="form-field"><span>Title</span><input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="Send the project update" /></label>
          <label className="form-field"><span>Local due time</span><input type="datetime-local" value={localDueAt} onChange={(event) => setLocalDueAt(event.target.value)} /></label>
          {kind === "event" && <label className="form-field"><span>Local end time</span><input type="datetime-local" value={eventEndLocal} onChange={(event) => setEventEndLocal(event.target.value)} /></label>}
          <label className="form-field"><span>IANA time zone</span><input value={timeZone} onChange={(event) => setTimeZone(event.target.value)} placeholder="Europe/Amsterdam" /></label>
          <label className="form-field"><span>Recurrence</span><select value={recurrenceKind} onChange={(event) => setRecurrenceKind(event.target.value as RecurrenceKind)}><option value="none">Does not repeat</option><option value="daily">Daily</option><option value="weekly">Weekly</option><option value="monthly">Monthly</option></select></label>
          {recurrenceKind !== "none" && <><label className="form-field"><span>Repeat every</span><input type="number" min={1} max={366} value={recurrenceInterval} onChange={(event) => setRecurrenceInterval(Number(event.target.value))} /></label><label className="form-field"><span>Occurrence limit · optional</span><input type="number" min={1} max={10000} value={occurrenceLimit} onChange={(event) => setOccurrenceLimit(event.target.value)} /></label></>}
          <label className="form-field"><span>Delivery</span><select value={deliveryMode} onChange={(event) => setDeliveryMode(event.target.value as DeliveryMode)}><option value="in_app">In-app only</option><option value="portal">Desktop notification portal</option></select></label>
          <label className="form-field"><span>Notification privacy</span><select value={privacyMode} onChange={(event) => setPrivacyMode(event.target.value as PrivacyMode)}><option value="generic">Generic notification</option><option value="title">Show title</option></select></label>
          <label className="form-field"><span>Agent</span><select value={agentId ?? ""} onChange={(event) => { setAgentId(event.target.value ? Number(event.target.value) : null); setTaskId(null); }}><option value="">No linked agent</option>{agents.map((agent) => <option value={agent.id} key={agent.id}>{agent.name}</option>)}</select></label>
          <label className="form-field"><span>Linked task</span><select value={taskId ?? ""} disabled={!selectedAgent} onChange={(event) => setTaskId(event.target.value ? Number(event.target.value) : null)}><option value="">No linked task</option>{selectedTasks.map((task) => <option value={task.id} key={task.id}>{task.title}</option>)}</select></label>
        </div>
        <label className="form-field"><span>Notes</span><textarea rows={3} value={notes} onChange={(event) => setNotes(event.target.value)} placeholder="Optional local context" /></label>
        <div className="task-card-actions"><button className="primary-button" disabled={busy || !authoritative} onClick={() => void saveItem()}>{editingId === null ? "Add schedule" : "Save revision"}</button></div>
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">AUTHORITATIVE QUEUE</span><h2>Scheduled work</h2></div><label className="form-field filter-field"><span>Status</span><select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value as ScheduleStatus | "all")}><option value="scheduled">Scheduled</option><option value="due">Due</option><option value="needs_attention">Needs attention</option><option value="completed">Completed</option><option value="dismissed">Dismissed</option><option value="all">All</option></select></label></div>
        {items.length === 0 ? <p className="page-message">No schedules match this filter.</p> : <div className="agent-list">{items.map((item) => {
          const classification = classifyScheduledItem(item, nowUnixMs, dueWindowEnd);
          const linkedAgent = agents.find((agent) => agent.id === item.subjectAgentId);
          const linkedTask = linkedAgent?.tasks.find((task) => task.id === item.taskId);
          return <article className="agent-card" key={item.id}><div><div className="agent-result-heading"><h3>{item.title}</h3><span className={`agent-status ${item.status === "completed" ? "working" : item.status === "dismissed" || item.status === "needs_attention" ? "paused" : "waiting"}`}>{statusLabel(item.status)}</span></div><p>{item.dueAtUnixMs === null ? "Unresolved local time" : new Date(item.dueAtUnixMs).toLocaleString()} · {item.timeZone}</p><small>{classification.replace("_", " ")} · DST: {item.dstResolution.split("_").join(" ")} · {item.recurrence.kind === "none" ? "one time" : `every ${item.recurrence.interval} ${item.recurrence.kind}`}</small>{item.missedOccurrenceCount > 0 && <small>{item.missedOccurrenceCount} missed occurrence(s) recorded</small>}{item.scheduleIssueMessage && <small className="inline-error">{item.scheduleIssueCode}: {item.scheduleIssueMessage}</small>}{item.notes && <p>{item.notes}</p>}<small>{linkedAgent ? `Agent: ${linkedAgent.name}` : "Unassigned"}{linkedTask ? ` · Task: ${linkedTask.title}` : ""} · Delivery: {item.deliveryMode === "portal" ? "desktop portal" : "in app"}</small></div><div className="task-card-actions"><button className="secondary-button" disabled={busy || !authoritative} onClick={() => editItem(item)}>Edit</button>{item.status === "scheduled" || item.status === "due" ? <><button className="primary-button" disabled={busy || !authoritative} onClick={() => void changeStatus(item, "completed")}>Complete</button><button className="secondary-button" disabled={busy || !authoritative} onClick={() => void changeStatus(item, "dismissed")}>Dismiss</button></> : <button className="secondary-button" disabled={busy || !authoritative || item.status === "needs_attention"} onClick={() => void changeStatus(item, "scheduled")}>Restore</button>}<button className="danger-button" disabled={busy || !authoritative} onClick={() => void deleteItem(item)}>Delete</button></div></article>;
        })}</div>}
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">DELIVERY EVIDENCE</span><h2>Recent occurrences</h2></div></div>
        {snapshot.recentOccurrences.length === 0 ? <p className="page-message">No due occurrences have been recorded.</p> : <div className="activity-list">{snapshot.recentOccurrences.slice(0, 25).map((occurrence) => <article className="activity-item" key={occurrence.id}><div><strong>Occurrence {occurrence.occurrenceSequence + 1}</strong><p>{occurrence.status.split("_").join(" ")}</p><small>{new Date(occurrence.dueAtUnixMs).toLocaleString()}{occurrence.missedCount > 0 ? " · missed" : ""}</small>{occurrence.detailMessage && <small>{occurrence.detailCode}: {occurrence.detailMessage}</small>}</div></article>)}</div>}
      </section>
    </>
  );
}
