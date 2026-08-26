import type { Agent, HistoryRetentionDays } from "../../applicationState";
import type { RunCoordinatorUiState } from "../../runCoordinator";
import { workspaceEvidenceStatusLabel, workspaceReviewabilityLabel } from "../../workspaceEvidence";


export function ActivityPage({
  agents,
  setAgents,
  runCoordinator,
  retentionDays,
  setRetentionDays,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  runCoordinator: RunCoordinatorUiState;
  retentionDays: HistoryRetentionDays;
  setRetentionDays: React.Dispatch<
    React.SetStateAction<HistoryRetentionDays>
  >;
}) {
  const allActivity = agents
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

  function deleteActivityEntry(agentId: number, entryId: number) {
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

  function clearAllActivity() {
    const shouldClear = window.confirm(
      "Delete all recorded activity from every agent?",
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
            Monitor active agents, workflow progress, and recent events.
          </p>
        </div>

        <button className="danger-button" onClick={clearAllActivity}>
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

      <section className="summary-grid">
        <article className="summary-card">
          <span>Active agents</span>
          <strong>{activeAgents.length}</strong>
          <small>Working or reviewing</small>
        </article>

        <article className="summary-card">
          <span>Waiting next</span>
          <strong>{nextAgents.length}</strong>
          <small>Agents with pending work</small>
        </article>

        <article className="summary-card">
          <span>Blocked agents</span>
          <strong>{blockedAgents.length}</strong>
          <small>Needs intervention</small>
        </article>

        <article className="summary-card">
          <span>Retained runs</span>
          <strong>{runCoordinator.snapshot.retainedAttemptCount}</strong>
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

        {allActivity.length === 0 ? (
          <p className="page-message">
            No activity has been recorded yet.
          </p>
        ) : (
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
                  onClick={() =>
                    deleteActivityEntry(entry.agentId, entry.id)
                  }
                >
                  Delete
                </button>
              </article>
            ))}
          </div>
        )}
      </section>
    </>
  );
}
