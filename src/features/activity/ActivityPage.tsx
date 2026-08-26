import { useEffect, useState } from "react";
import type { Agent, HistoryRetentionDays } from "../../applicationState";
import type { RunCoordinatorUiState } from "../../runCoordinator";
import { workspaceEvidenceStatusLabel, workspaceReviewabilityLabel } from "../../workspaceEvidence";
import type { MonitoringActivityPage, MonitoringSnapshot } from "../../dataLifecycle";
import { desktopClient } from "../../services/desktopClient";
import { persistenceErrorMessage } from "../../persistence";


export function ActivityPage({
  agents,
  setAgents,
  runCoordinator,
  retentionDays,
  setRetentionDays,
  monitoringSnapshot,
  onMonitoringStale,
  onDeleteActivity,
  onClearActivity,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  runCoordinator: RunCoordinatorUiState;
  retentionDays: HistoryRetentionDays;
  setRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
  monitoringSnapshot: MonitoringSnapshot | null;
  onMonitoringStale: () => Promise<unknown>;
  onDeleteActivity: (ownerAgentId: number, entryId: number) => Promise<void>;
  onClearActivity: () => Promise<void>;
}) {
  const [monitoringPage, setMonitoringPage] =
    useState<MonitoringActivityPage | null>(null);
  const [activityMessage, setActivityMessage] = useState("");
  const [activityBusy, setActivityBusy] = useState(false);
  const monitoringRevisionKey = monitoringSnapshot
    ? Object.values(monitoringSnapshot.revision).join(":")
    : "preview";

  useEffect(() => {
    if (!monitoringSnapshot?.authoritative) {
      setMonitoringPage(null);
      return;
    }
    let active = true;
    void desktopClient
      .queryMonitoringActivity({
        expectedRevision: monitoringSnapshot.revision,
        offset: 0,
        limit: 100,
      })
      .then((page) => {
        if (active) setMonitoringPage(page);
      })
      .catch((error) => {
        if (!active) return;
        setActivityMessage(persistenceErrorMessage(error));
        if (
          typeof error === "object" &&
          error !== null &&
          "code" in error &&
          error.code === "MONITORING_REVISION_CONFLICT"
        ) {
          void onMonitoringStale();
        }
      });
    return () => {
      active = false;
    };
  }, [monitoringRevisionKey, monitoringSnapshot?.authoritative, onMonitoringStale]);

  const localActivity = agents
    .flatMap((agent) =>
      agent.activity.map((entry) => ({
        ...entry,
        agentId: agent.id,
        agentName: agent.name,
        agentRole: agent.role,
      })),
    )
    .sort(
      (a, b) =>
        new Date(b.createdAt).getTime() -
        new Date(a.createdAt).getTime(),
    );
  const allActivity = monitoringSnapshot?.authoritative
    ? (monitoringPage?.records ?? []).map((entry) => ({
        id: entry.entryId,
        message: entry.message,
        createdAt: entry.createdAt,
        agentId: entry.ownerAgentId,
        agentName: entry.ownerName,
        agentRole: entry.ownerRole,
      }))
    : localActivity;

  const activeAgents = agents.filter((agent) =>
    agent.tasks.some(
      (task) =>
        task.status === "Running" ||
        task.status === "Under Review",
    ),
  );

  const blockedAgents = agents.filter((agent) =>
    agent.tasks.some((task) => task.status === "Blocked"),
  );

  const recentlyFinished = agents
    .flatMap((agent) =>
      agent.tasks
        .filter(
          (task) =>
            task.status === "Completed" ||
            task.status === "Failed",
        )
        .map((task) => ({
          agent,
          task,
        })),
    )
    .slice(0, 8);

  const nextAgents = agents.filter((agent) =>
    agent.tasks.some((task) => task.status === "Pending"),
  );

  async function deleteActivityEntry(agentId: number, entryId: number) {
    if (monitoringSnapshot?.authoritative) {
      setActivityBusy(true);
      setActivityMessage("");
      try {
        await onDeleteActivity(agentId, entryId);
      } catch (error) {
        setActivityMessage(persistenceErrorMessage(error));
      } finally {
        setActivityBusy(false);
      }
      return;
    }
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === agentId
          ? {
              ...agent,
              activity: agent.activity.filter(
                (entry) => entry.id !== entryId,
              ),
            }
          : agent,
      ),
    );
  }

  async function clearAllActivity() {
    if (monitoringSnapshot?.authoritative) {
      setActivityBusy(true);
      setActivityMessage("");
      try {
        await onClearActivity();
      } catch (error) {
        setActivityMessage(persistenceErrorMessage(error));
      } finally {
        setActivityBusy(false);
      }
      return;
    }
    const shouldClear = window.confirm(
      "Delete all browser-preview activity from every agent?",
    );

    if (!shouldClear) {
      return;
    }

    setAgents((currentAgents) =>
      currentAgents.map((agent) => ({
        ...agent,
        activity: [],
      })),
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">SYSTEM ACTIVITY</span>
          <h1>Activity</h1>
          <p className="page-message">
            {monitoringSnapshot?.authoritative
              ? "Revision-consistent backend monitoring and local configuration events."
              : "Browser preview only; this timeline is not authoritative backend evidence."}
          </p>
        </div>

        <button
          className="danger-button"
          disabled={activityBusy}
          onClick={() => void clearAllActivity()}
        >
          Clear activity history
        </button>
      </header>

      <section className="panel">
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(220px, 320px)",
          }}
        >
          <label className="form-field">
            <span>Activity retention</span>
            <select
              value={retentionDays}
              onChange={(event) =>
                setRetentionDays(
                  event.target.value === "never"
                    ? "never"
                    : (Number(event.target.value) as 7 | 30 | 90),
                )
              }
            >
              <option value={7}>Delete after 7 days</option>
              <option value={30}>Delete after 30 days</option>
              <option value={90}>Delete after 90 days</option>
              <option value="never">Never delete automatically</option>
            </select>
          </label>
        </div>
      </section>

      {activityMessage && (
        <div className="runtime-message" role="status">
          {activityMessage}
        </div>
      )}

      <section className="summary-grid">
        <article className="summary-card">
          <span>Active agents</span>
          <strong>
            {monitoringSnapshot?.counts.activeAgents ?? activeAgents.length}
          </strong>
          <small>Working or reviewing</small>
        </article>

        <article className="summary-card">
          <span>Waiting next</span>
          <strong>
            {monitoringSnapshot?.counts.pendingTasks ?? nextAgents.length}
          </strong>
          <small>
            {monitoringSnapshot?.authoritative
              ? "Backend pending task records"
              : "Agents with pending work"}
          </small>
        </article>

        <article className="summary-card">
          <span>Blocked agents</span>
          <strong>
            {monitoringSnapshot?.counts.blockedTasks ?? blockedAgents.length}
          </strong>
          <small>Needs intervention</small>
        </article>

        <article className="summary-card">
          <span>Retained runs</span>
          <strong>
            {monitoringSnapshot?.counts.retainedRunAttempts ??
              runCoordinator.snapshot.retainedAttemptCount}
          </strong>
          <small>Backend-owned evidence</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AUTHORITATIVE RUN LEDGER</span>
            <h2>Execution and review evidence</h2>
            <p className="page-message">
              These records come from the immutable backend run ledger and cannot be cleared through the local activity controls.
            </p>
          </div>
        </div>
        {runCoordinator.snapshot.recentAttempts.length === 0 ? (
          <p className="page-message">No retained run attempts.</p>
        ) : (
          <div className="agent-list">
            {runCoordinator.snapshot.recentAttempts.map((attempt) => (
              <article className="agent-card" key={attempt.id}>
                <div>
                  <h3>{attempt.taskTitle}</h3>
                  <p>
                    {attempt.runMode === "review" ? "Review" : "Execution"} · {attempt.status.replace(/_/g, " ")}
                  </p>
                  <small>
                    {workspaceEvidenceStatusLabel(attempt.workspaceChanges)} · {attempt.workspaceChanges.summary.totalChanges} observed changes · {workspaceReviewabilityLabel(attempt.workspaceChanges)}
                  </small>
                </div>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">CURRENTLY ACTIVE</span>
            <h2>Agents in progress</h2>
          </div>
        </div>

        {activeAgents.length === 0 ? (
          <p className="page-message">
            No agents are currently working or reviewing.
          </p>
        ) : (
          <div className="agent-list">
            {activeAgents.map((agent) => {
              const activeTask =
                agent.tasks.find(
                  (task) => task.status === "Running",
                ) ??
                agent.tasks.find(
                  (task) => task.status === "Under Review",
                ) ??
                null;

              return (
                <article className="agent-card" key={agent.id}>
                  <div>
                    <h3>{agent.name}</h3>
                    <p>
                      {activeTask
                        ? activeTask.title
                        : "Active without assigned task"}
                    </p>
                    <small>
                      {agent.role} ·{" "}
                      {activeTask
                        ? `${activeTask.phase} · ${activeTask.category}`
                        : agent.category}
                    </small>
                  </div>

                  <span
                    className={`agent-status ${agent.status.toLowerCase()}`}
                  >
                    {agent.status}
                  </span>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">WAITING NEXT</span>
            <h2>Queued agents</h2>
          </div>
        </div>

        {nextAgents.length === 0 ? (
          <p className="page-message">
            No agents are waiting for pending work.
          </p>
        ) : (
          <div className="agent-list">
            {nextAgents.map((agent) => {
              const nextTask =
                agent.tasks.find(
                  (task) => task.status === "Pending",
                ) ?? null;

              return (
                <article className="agent-card" key={agent.id}>
                  <div>
                    <h3>{agent.name}</h3>
                    <p>
                      {nextTask
                        ? nextTask.title
                        : "Pending work available"}
                    </p>
                    <small>
                      {agent.role}
                      {nextTask
                        ? ` · Next phase: ${nextTask.phase}`
                        : ""}
                    </small>
                  </div>

                  <span className="agent-status waiting">
                    Waiting
                  </span>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">RECENT OUTCOMES</span>
            <h2>Finished and failed work</h2>
          </div>
        </div>

        {recentlyFinished.length === 0 ? (
          <p className="page-message">
            No completed or failed tasks yet.
          </p>
        ) : (
          <div className="agent-list">
            {recentlyFinished.map(({ agent, task }) => (
              <article
                className="agent-card"
                key={`${agent.id}-${task.id}`}
              >
                <div>
                  <h3>{task.title}</h3>
                  <p>
                    {agent.name} · {agent.role}
                  </p>
                  <small>
                    {task.category} · {task.phase}
                  </small>
                </div>

                <span
                  className={`agent-status ${
                    task.status === "Completed"
                      ? "waiting"
                      : "paused"
                  }`}
                >
                  {task.status}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">TIMELINE</span>
            <h2>Local configuration timeline</h2>
          </div>
        </div>

        {monitoringSnapshot?.authoritative && !monitoringPage && !activityMessage ? (
          <p className="page-message">Loading authoritative activity records…</p>
        ) : allActivity.length === 0 ? (
          <p className="page-message">
            No activity has been recorded yet.
          </p>
        ) : (
          <>
            {monitoringPage && (
              <p className="page-message">
                Showing {monitoringPage.records.length} of {monitoringPage.total}{" "}
                backend activity entries. Pages are capped at 100.
              </p>
            )}
            <div className="agent-list">
            {allActivity.map((entry) => (
              <article
                className="agent-card"
                key={`${entry.agentId}-${entry.id}`}
              >
                <div>
                  <h3>{entry.message}</h3>
                  <p>
                    {entry.agentName} · {entry.agentRole}
                  </p>
                  <small>
                    {new Date(entry.createdAt).toLocaleString()}
                  </small>
                </div>

                <button
                  className="danger-button"
                  disabled={activityBusy}
                  onClick={() =>
                    void deleteActivityEntry(entry.agentId, entry.id)
                  }
                >
                  Delete
                </button>
              </article>
            ))}
            </div>
          </>
        )}
      </section>
    </>
  );
}
