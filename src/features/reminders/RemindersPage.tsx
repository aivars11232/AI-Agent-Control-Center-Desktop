import { useState } from "react";
import type { Agent, Reminder, ReminderStatus } from "../../applicationState";


export function RemindersPage({
  agents,
  reminders,
  setReminders,
}: {
  agents: Agent[];
  reminders: Reminder[];
  setReminders: React.Dispatch<React.SetStateAction<Reminder[]>>;
}) {
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [dueAt, setDueAt] = useState(() => {
    const date = new Date(Date.now() + 60 * 60 * 1000);
    date.setSeconds(0, 0);
    return date.toISOString().slice(0, 16);
  });
  const [agentId, setAgentId] = useState<number | null>(null);
  const [taskId, setTaskId] = useState<number | null>(null);
  const [statusFilter, setStatusFilter] = useState<ReminderStatus | "All">(
    "Upcoming",
  );

  const selectedAgent = agents.find((agent) => agent.id === agentId) ?? null;
  const selectedAgentTasks = selectedAgent?.tasks ?? [];
  const filteredReminders = reminders
    .filter((reminder) =>
      statusFilter === "All" ? true : reminder.status === statusFilter,
    )
    .sort(
      (left, right) =>
        new Date(left.dueAt).getTime() - new Date(right.dueAt).getTime(),
    );

  function addReminder() {
    const trimmedTitle = title.trim();
    if (!trimmedTitle || !dueAt) {
      return;
    }

    setReminders((current) => [
      {
        id: Date.now(),
        title: trimmedTitle,
        notes: notes.trim(),
        dueAt: new Date(dueAt).toISOString(),
        status: "Upcoming",
        agentId,
        taskId,
        createdAt: new Date().toISOString(),
      },
      ...current,
    ]);
    setTitle("");
    setNotes("");
    setTaskId(null);
  }

  function updateReminderStatus(id: number, status: ReminderStatus) {
    setReminders((current) =>
      current.map((reminder) =>
        reminder.id === id ? { ...reminder, status } : reminder,
      ),
    );
  }

  function deleteReminder(id: number) {
    setReminders((current) => current.filter((reminder) => reminder.id !== id));
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">TIME-BASED WORK</span>
          <h1>Reminders</h1>
          <p className="page-message">
            Keep deadlines and follow-up work visible alongside the agent workflow.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Upcoming</span>
          <strong>{reminders.filter((reminder) => reminder.status === "Upcoming").length}</strong>
          <small>Waiting for attention</small>
        </article>
        <article className="summary-card">
          <span>Due soon</span>
          <strong>
            {reminders.filter(
              (reminder) =>
                reminder.status === "Upcoming" &&
                new Date(reminder.dueAt).getTime() <= Date.now() + 24 * 60 * 60 * 1000,
            ).length}
          </strong>
          <small>Within the next 24 hours</small>
        </article>
        <article className="summary-card">
          <span>Completed</span>
          <strong>{reminders.filter((reminder) => reminder.status === "Completed").length}</strong>
          <small>Finished reminders</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">NEW REMINDER</span>
            <h2>Schedule follow-up</h2>
          </div>
        </div>

        <div className="task-composer reminder-composer">
          <label className="form-field">
            <span>Reminder title</span>
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") addReminder();
              }}
              placeholder="Send the project update"
            />
          </label>
          <label className="form-field">
            <span>Due</span>
            <input
              type="datetime-local"
              value={dueAt}
              onChange={(event) => setDueAt(event.target.value)}
            />
          </label>
          <label className="form-field">
            <span>Agent</span>
            <select
              value={agentId ?? ""}
              onChange={(event) => {
                setAgentId(event.target.value ? Number(event.target.value) : null);
                setTaskId(null);
              }}
            >
              <option value="">No agent</option>
              {agents.map((agent) => (
                <option value={agent.id} key={agent.id}>{agent.name}</option>
              ))}
            </select>
          </label>
          <label className="form-field">
            <span>Linked task</span>
            <select
              value={taskId ?? ""}
              disabled={!selectedAgent}
              onChange={(event) => setTaskId(event.target.value ? Number(event.target.value) : null)}
            >
              <option value="">No linked task</option>
              {selectedAgentTasks.map((task) => (
                <option value={task.id} key={task.id}>{task.title}</option>
              ))}
            </select>
          </label>
          <button className="primary-button" onClick={addReminder}>Add reminder</button>
        </div>

        <label className="form-field">
          <span>Notes</span>
          <textarea
            rows={3}
            value={notes}
            onChange={(event) => setNotes(event.target.value)}
            placeholder="Optional context for the reminder"
          />
        </label>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">REMINDER QUEUE</span>
            <h2>Scheduled follow-up</h2>
          </div>
          <label className="form-field filter-field">
            <span>Status</span>
            <select value={statusFilter} onChange={(event) => setStatusFilter(event.target.value as ReminderStatus | "All")}>
              <option value="Upcoming">Upcoming</option>
              <option value="Completed">Completed</option>
              <option value="Dismissed">Dismissed</option>
              <option value="All">All reminders</option>
            </select>
          </label>
        </div>

        {filteredReminders.length === 0 ? (
          <p className="page-message">No reminders match this filter.</p>
        ) : (
          <div className="agent-list">
            {filteredReminders.map((reminder) => {
              const linkedAgent = agents.find((agent) => agent.id === reminder.agentId);
              const linkedTask = linkedAgent?.tasks.find((task) => task.id === reminder.taskId);
              return (
                <article className="agent-card" key={reminder.id}>
                  <div>
                    <h3>{reminder.title}</h3>
                    <p>{new Date(reminder.dueAt).toLocaleString()}</p>
                    {reminder.notes && <small>{reminder.notes}</small>}
                    <small>
                      {linkedAgent ? `Agent: ${linkedAgent.name}` : "Unassigned"}
                      {linkedTask ? ` · Task: ${linkedTask.title}` : ""}
                    </small>
                  </div>
                  <div className="task-card-actions">
                    <span className={`agent-status ${reminder.status === "Completed" ? "working" : reminder.status === "Dismissed" ? "paused" : "waiting"}`}>
                      {reminder.status}
                    </span>
                    {reminder.status === "Upcoming" && (
                      <>
                        <button className="primary-button" onClick={() => updateReminderStatus(reminder.id, "Completed")}>Complete</button>
                        <button className="secondary-button" onClick={() => updateReminderStatus(reminder.id, "Dismissed")}>Dismiss</button>
                      </>
                    )}
                    {(reminder.status === "Completed" || reminder.status === "Dismissed") && (
                      <button className="secondary-button" onClick={() => updateReminderStatus(reminder.id, "Upcoming")}>Restore</button>
                    )}
                    <button className="danger-button" onClick={() => deleteReminder(reminder.id)}>Delete</button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}
