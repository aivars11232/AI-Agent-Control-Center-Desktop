import { useEffect, useState } from "react";
import type { Agent, AgentTask, ApprovalRequest, TaskCategory, TaskStatus } from "../../applicationState";
import { persistenceErrorMessage } from "../../persistence";
import { queueEntryForTask, queueStateLabel } from "../../taskOrchestration";
import type { TaskOrchestrationSnapshot } from "../../taskOrchestration";
import { errorMessage } from "../../domain/errors";
import type { TaskOrchestrationMutation } from "../contracts";
import { prepareBackendAuthorization, type AuthorizationReadiness } from "../../services/authorization";
import { desktopClient } from "../../services/desktopClient";
import type { MonitoringSnapshot, MonitoringTaskPage } from "../../dataLifecycle";

export function TasksPage({
  agents,
  taskOrchestration,
  onTaskMutation,
  runActive,
  setApprovalRequests,
  monitoringSnapshot,
  onMonitoringStale,
}: {
  agents: Agent[];
  taskOrchestration: TaskOrchestrationSnapshot;
  onTaskMutation: TaskOrchestrationMutation;
  runActive: boolean;
  monitoringSnapshot: MonitoringSnapshot | null;
  onMonitoringStale: () => Promise<unknown>;
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
}) {
  const [statusFilter, setStatusFilter] =
    useState<TaskStatus | "All">("All");
  const [categoryFilter, setCategoryFilter] =
    useState<TaskCategory | "All">("All");
  const [taskMutationKey, setTaskMutationKey] = useState("");
  const [taskMessage, setTaskMessage] = useState("");
  const [monitoringPage, setMonitoringPage] =
    useState<MonitoringTaskPage | null>(null);

  const monitoringRevisionKey = monitoringSnapshot
    ? Object.values(monitoringSnapshot.revision).join(":")
    : "preview";

  useEffect(() => {
    if (!monitoringSnapshot?.authoritative) {
      setMonitoringPage(null);
      return;
    }
    let active = true;
    setMonitoringPage(null);
    void desktopClient
      .queryMonitoringTasks({
        expectedRevision: monitoringSnapshot.revision,
        status: statusFilter === "All" ? null : statusFilter,
        category: categoryFilter === "All" ? null : categoryFilter,
        offset: 0,
        limit: 100,
      })
      .then((page) => {
        if (active) setMonitoringPage(page);
      })
      .catch((error) => {
        if (!active) return;
        setTaskMessage(persistenceErrorMessage(error));
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
  }, [
    monitoringRevisionKey,
    monitoringSnapshot?.authoritative,
    statusFilter,
    categoryFilter,
    onMonitoringStale,
  ]);

  const localTasks = agents.flatMap((owner) =>
    owner.tasks.map((task) => ({
      task,
      ownerId: owner.id,
      ownerName: owner.name,
      executorName:
        agents.find((agent) => agent.id === task.assignedAgentId)?.name ?? null,
      entry: queueEntryForTask(taskOrchestration, owner.id, task.id),
    })),
  );
  const allTasks = monitoringSnapshot?.authoritative
    ? (monitoringPage?.records ?? []).map((record) => ({
        task: record.task,
        ownerId: record.ownerAgentId,
        ownerName: record.ownerName,
        executorName: record.executorName,
        entry: queueEntryForTask(
          taskOrchestration,
          record.ownerAgentId,
          record.task.id,
        ),
      }))
    : localTasks;

  const authoritativeEntries = [
    ...(taskOrchestration.activeExecute
      ? [taskOrchestration.activeExecute]
      : []),
    ...taskOrchestration.executeQueue,
    ...taskOrchestration.heldTasks,
  ];
  const authoritativeOrder = new Map(
    authoritativeEntries.map((entry, index) => [
      `${entry.taskOwnerAgentId}:${entry.taskId}`,
      index,
    ]),
  );

  const filteredTasks = (monitoringSnapshot?.authoritative ? allTasks : allTasks
    .filter(({ task }) => {
      const matchesStatus =
        statusFilter === "All" || task.status === statusFilter;
      const matchesCategory =
        categoryFilter === "All" || task.category === categoryFilter;
      return matchesStatus && matchesCategory;
    })
    .sort((left, right) => {
      const leftOrder =
        authoritativeOrder.get(`${left.ownerId}:${left.task.id}`) ??
        Number.MAX_SAFE_INTEGER;
      const rightOrder =
        authoritativeOrder.get(`${right.ownerId}:${right.task.id}`) ??
        Number.MAX_SAFE_INTEGER;
      return (
        leftOrder - rightOrder ||
        right.task.createdAt.localeCompare(left.task.createdAt) ||
        left.task.id - right.task.id
      );
    }));

  const summary = {
    total: monitoringSnapshot?.counts.totalTasks ?? allTasks.length,
    active:
      monitoringSnapshot?.counts.activeRunAttempts ??
      (taskOrchestration.activeExecute ? 1 : 0),
    pending:
      monitoringSnapshot?.counts.pendingTasks ??
      taskOrchestration.executeQueue.length,
    blocked:
      monitoringSnapshot?.counts.blockedTasks ??
      taskOrchestration.heldTasks.length,
  };

  async function setQueueDisposition(
    ownerAgentId: number,
    task: AgentTask,
    disposition: "hold" | "resume" | "resetTerminal",
  ) {
    const mutationKey = `${ownerAgentId}:${task.id}`;
    setTaskMutationKey(mutationKey);
    setTaskMessage("");
    try {
      await onTaskMutation("set_task_queue_disposition", {
        taskOwnerAgentId: ownerAgentId,
        taskId: task.id,
        disposition,
      });
    } catch (error) {
      setTaskMessage(persistenceErrorMessage(error));
    } finally {
      setTaskMutationKey("");
    }
  }

  async function requestTaskApproval(ownerAgentId: number, task: AgentTask) {
    const executor =
      agents.find((agent) => agent.id === task.assignedAgentId) ?? null;
    if (!executor) {
      setTaskMessage("The backend-selected task executor is unavailable.");
      return;
    }
    setTaskMessage("");
    let authorization: AuthorizationReadiness;
    try {
      authorization = await prepareBackendAuthorization(
        {
          kind: "runTask",
          agentId: executor.id,
          taskOwnerAgentId: ownerAgentId,
          taskId: task.id,
          runMode: "execute",
        },
        setApprovalRequests,
      );
    } catch (error) {
      setTaskMessage(errorMessage(error));
      return;
    }
    if (authorization.ready && !authorization.approval) {
      setTaskMessage(
        "Current backend policy allows this task without an approval record.",
      );
      return;
    }
    setTaskMessage(
      authorization.ready
        ? "A one-use backend authorization is ready."
        : "A one-use backend authorization is waiting for trusted approval.",
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">GLOBAL WORKFLOW</span>
          <h1>Tasks</h1>
          <p className="page-message">
            {monitoringSnapshot?.authoritative
              ? "Inspect a revision-consistent backend task page, routing, and queue state."
              : "Browser preview only; task counts and ordering are not authoritative backend evidence."}
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Total tasks</span>
          <strong>{summary.total}</strong>
          <small>Across all agents</small>
        </article>

        <article className="summary-card">
          <span>Active</span>
          <strong>{summary.active}</strong>
          <small>Single execute slot</small>
        </article>

        <article className="summary-card">
          <span>Pending</span>
          <strong>{summary.pending}</strong>
          <small>Authoritative queue</small>
        </article>

        <article className="summary-card">
          <span>Held</span>
          <strong>{summary.blocked}</strong>
          <small>Outside admission</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">FILTERS</span>
            <h2>Authoritative task queue</h2>
          </div>
        </div>

        <div
          style={{
            display: "grid",
            gridTemplateColumns:
              "repeat(auto-fit, minmax(220px, 1fr))",
            gap: "14px",
            marginBottom: "20px",
          }}
        >
          <label className="form-field">
            <span>Status</span>
            <select
              value={statusFilter}
              onChange={(event) =>
                setStatusFilter(
                  event.target.value as TaskStatus | "All",
                )
              }
            >
              <option value="All">All statuses</option>
              <option value="Pending">Pending</option>
              <option value="Running">Running</option>
              <option value="Blocked">Blocked</option>
              <option value="Under Review">Under Review</option>
              <option value="Completed">Completed</option>
              <option value="Failed">Failed</option>
            </select>
          </label>

          <label className="form-field">
            <span>Category</span>
            <select
              value={categoryFilter}
              onChange={(event) =>
                setCategoryFilter(
                  event.target.value as TaskCategory | "All",
                )
              }
            >
              <option value="All">All categories</option>
              <option value="Development">Development</option>
              <option value="Research">Research</option>
              <option value="Browsing">Browsing</option>
              <option value="Finance">Finance</option>
              <option value="Business">Business</option>
              <option value="Communication">Communication</option>
              <option value="System Control">System Control</option>
              <option value="General">General</option>
            </select>
          </label>
        </div>

        {taskMessage && (
          <div className="runtime-message" role="status">
            {taskMessage}
          </div>
        )}

        {monitoringPage && (
          <p className="page-message">
            Showing {monitoringPage.records.length} of {monitoringPage.total}{" "}
            revision-consistent backend task records. Pages are capped at 100.
          </p>
        )}

        {monitoringSnapshot?.authoritative && !monitoringPage && !taskMessage && (
          <p className="page-message">Loading authoritative task records…</p>
        )}

        {filteredTasks.length === 0 ? (
          <p className="page-message">
            No tasks match the selected filters.
          </p>
        ) : (
          <div className="agent-list">
                    {filteredTasks.map(({ task, ownerId, ownerName, executorName, entry }) => (
                      <article
                        className="agent-card task-card"
                        key={`${ownerId}-${task.id}`}
                      >
                        <div className="task-card-content">
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "10px",
                              flexWrap: "wrap",
                              marginBottom: "8px",
                            }}
                          >
                            <h3 style={{ margin: 0 }}>
                              {task.title}
                            </h3>

                            <span
                              className={`agent-status ${
                                task.status === "Running"
                                  ? "working"
                                  : task.status === "Completed"
                                    ? "waiting"
                                    : task.status === "Failed"
                                      ? "paused"
                                      : "waiting"
                              }`}
                            >
                              {task.status}
                            </span>
                          </div>

                          <p>
                            {task.category} · {task.priority} priority
                          </p>
                          <small>
                            Owner: {ownerName} · Executor:{" "}
                            {executorName ?? "Unavailable"}
                            {task.routingMode === "automatic"
                              ? " · Automatically routed"
                              : ""}
                            {task.reviewStatus !== "Not Requested"
                              ? ` · Review: ${task.reviewStatus}`
                              : ""}
                          </small>
                          <div className="routing-note">
                            <strong>{queueStateLabel(entry)}</strong>
                            <small>
                              Phase: {task.phase}
                              {task.enqueueSequence !== null
                                ? ` · Enqueue sequence ${task.enqueueSequence}`
                                : ""}
                            </small>
                            {task.routingReason && (
                              <small>{task.routingReason}</small>
                            )}
                          </div>
                        </div>

                        <div className="task-card-actions">
                          {task.queueState === "queued" && (
                              <button
                                className="secondary-button"
                                disabled={
                                  runActive ||
                                  taskMutationKey === `${ownerId}:${task.id}`
                                }
                                onClick={() =>
                                  void setQueueDisposition(
                                    ownerId,
                                    task,
                                    "hold",
                                  )
                                }
                              >
                                Hold
                              </button>
                            )}

                          {task.queueState === "held" && (
                              <button
                                className="secondary-button"
                                disabled={
                                  runActive ||
                                  taskMutationKey === `${ownerId}:${task.id}`
                                }
                                onClick={() =>
                                  void setQueueDisposition(
                                    ownerId,
                                    task,
                                    "resume",
                                  )
                                }
                              >
                                Resume
                              </button>
                            )}

                          {task.queueState === "notQueued" &&
                            (task.status === "Completed" ||
                              task.status === "Failed") && (
                            <button
                              className="secondary-button"
                              disabled={
                                runActive ||
                                taskMutationKey === `${ownerId}:${task.id}`
                              }
                              onClick={() =>
                                void setQueueDisposition(
                                  ownerId,
                                  task,
                                  "resetTerminal",
                                )
                              }
                            >
                              Reset with new queue age
                            </button>
                          )}

                          {(task.queueState === "queued" ||
                            task.queueState === "held") && (
                              <button
                                className="secondary-button"
                                disabled={runActive}
                                onClick={() =>
                                  void requestTaskApproval(ownerId, task)
                                }
                              >
                                Request approval
                              </button>
                            )}
                        </div>
                      </article>
                    ))}
          </div>
        )}
      </section>
    </>
  );
}
