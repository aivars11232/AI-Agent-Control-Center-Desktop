import { useState } from "react";
import type { Agent, ApprovalRequest } from "../../applicationState";
import type { RunCoordinatorUiState } from "../../runCoordinator";
import { availableAgentGroups, projectAgentGroup } from "../../agentRegistry";
import type { AgentGroup } from "../../agentRegistry";
import { queueStateLabel } from "../../taskOrchestration";
import type { TaskOrchestrationSnapshot } from "../../taskOrchestration";
import { workspaceEvidenceStatusLabel, workspaceReviewabilityLabel } from "../../workspaceEvidence";
import type { MonitoringSnapshot } from "../../dataLifecycle";


export function DashboardPage({
  agents,
  approvalRequests,
  taskOrchestration,
  runCoordinator,
  monitoringSnapshot,
  onOpenAgents,
  onOpenTasks,
  onOpenApprovals,
}: {
  agents: Agent[];
  approvalRequests: ApprovalRequest[];
  taskOrchestration: TaskOrchestrationSnapshot;
  runCoordinator: RunCoordinatorUiState;
  monitoringSnapshot?: MonitoringSnapshot | null;
  onOpenAgents: () => void;
  onOpenTasks: () => void;
  onOpenApprovals: () => void;
}) {
  const [activeAgentGroup, setActiveAgentGroup] =
    useState<AgentGroup>("Development");
  const activeAgentCount =
    monitoringSnapshot?.counts.activeAgents ??
    agents.filter((agent) => agent.status === "Working").length;

  const runningTaskCount =
    monitoringSnapshot?.counts.runningTasks ??
    agents.reduce(
      (total, agent) =>
        total +
        agent.tasks.filter((task) => task.status === "Running").length,
      0,
    );

  const waitingTaskCount = taskOrchestration.executeQueue.length;

  const supervisorQueue = [
    ...(taskOrchestration.activeExecute
      ? [taskOrchestration.activeExecute]
      : []),
    ...taskOrchestration.executeQueue,
    ...taskOrchestration.heldTasks,
  ]
    .flatMap((entry) => {
      const owner = agents.find(
        (agent) => agent.id === entry.taskOwnerAgentId,
      );
      const task = owner?.tasks.find((item) => item.id === entry.taskId);
      return owner && task ? [{ entry, owner, task }] : [];
    })
    .slice(0, 8);
  const pendingApprovalCount = approvalRequests.filter(
    (request) => request.status === "Pending",
  ).length;

  const configuredDashboardGroups = availableAgentGroups(agents).filter(
    (group) => group !== "All agents" && group !== "Needs assignment",
  );
  const dashboardGroups: AgentGroup[] =
    configuredDashboardGroups.length > 0
      ? configuredDashboardGroups
      : ["All agents"];
  const effectiveAgentGroup = dashboardGroups.includes(activeAgentGroup)
    ? activeAgentGroup
    : dashboardGroups[0];
  const groupProjection = projectAgentGroup(agents, effectiveAgentGroup);
  const groupedAgents = groupProjection.visibleAgents;

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">OVERVIEW</span>
          <h1>Dashboard</h1>
          <p className="page-message">
            {monitoringSnapshot?.authoritative
              ? "Transactional backend status across configured agents."
              : "Browser preview only; these counts are not authoritative backend evidence."}
          </p>
        </div>

        <button className="primary-button" onClick={onOpenAgents}>
          Manage agents
        </button>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Active agents</span>
          <strong>{activeAgentCount}</strong>
          <small>
            {monitoringSnapshot?.counts.configuredAgents ?? agents.length} configured
          </small>
        </article>

        <article className="summary-card">
          <span>Tasks running</span>
          <strong>{runningTaskCount}</strong>
          <small>{waitingTaskCount} pending</small>
        </article>

        <article className="summary-card">
          <span>Total tasks</span>
          <strong>
            {monitoringSnapshot?.counts.totalTasks ??
              agents.reduce(
                (total, agent) => total + agent.tasks.length,
                0,
              )}
          </strong>
          <small>Across all agents</small>
        </article>

        <article className="summary-card">
          <span>Retained runs</span>
          <strong>
            {monitoringSnapshot?.counts.retainedRunAttempts ??
              runCoordinator.snapshot.retainedAttemptCount}
          </strong>
          <small>Immutable evidence ledger</small>
        </article>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AUTHORITATIVE RUN EVIDENCE</span>
            <h2>Recent execution records</h2>
            <p className="page-message">
              Backend-owned outcomes and bounded workspace evidence; local activity messages are not used here.
            </p>
          </div>
        </div>
        {runCoordinator.snapshot.recentAttempts.length === 0 ? (
          <p className="page-message">No retained run attempts.</p>
        ) : (
          <div className="agent-list">
            {runCoordinator.snapshot.recentAttempts.slice(0, 6).map((attempt) => (
              <article className="agent-card" key={attempt.id}>
                <div>
                  <h3>{attempt.taskTitle}</h3>
                  <p>
                    {attempt.runMode === "review" ? "Review" : "Execution"} · {attempt.status.replace(/_/g, " ")}
                  </p>
                  <small>
                    {workspaceEvidenceStatusLabel(attempt.workspaceChanges)} · {attempt.workspaceChanges.summary.totalChanges} observed changes
                  </small>
                </div>
                <span className={`agent-status ${attempt.status === "succeeded" ? "waiting" : attempt.status === "running" ? "working" : "paused"}`}>
                  {workspaceReviewabilityLabel(attempt.workspaceChanges)}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">AUTHORITATIVE EXECUTE QUEUE</span>
            <h2>Sequential work</h2>
            <p className="page-message">
              Backend admission order, the active execute slot, and held work.
            </p>
          </div>

          <div className="topbar-actions">
            <button className="secondary-button" onClick={onOpenTasks}>
              Open task pipeline
            </button>
            {pendingApprovalCount > 0 && (
              <button className="danger-button" onClick={onOpenApprovals}>
                Review {pendingApprovalCount} approval{pendingApprovalCount === 1 ? "" : "s"}
              </button>
            )}
          </div>
        </div>

        {supervisorQueue.length === 0 ? (
          <p className="page-message">
            No work is active, queued, or held.
          </p>
        ) : (
          <div className="agent-list">
            {supervisorQueue.map(({ entry, task, owner }) => (
              <article className="agent-card" key={`${owner.id}-${task.id}`}>
                <div>
                  <h3>{task.title}</h3>
                  <p>
                    Owner: {owner.name} · Executor:{" "}
                    {agents.find(
                      (agent) => agent.id === entry.assignedAgentId,
                    )?.name ?? "Unknown agent"}
                  </p>
                  <small>
                    {task.priority} priority · {queueStateLabel(entry)}
                  </small>
                </div>
                <span
                  className={`agent-status ${
                    entry.queueState === "held"
                      ? "paused"
                      : entry.queueState === "running" ||
                          entry.queueState === "admitted"
                        ? "working"
                        : "waiting"
                  }`}
                >
                  {entry.queueState === "held"
                    ? "Held"
                    : entry.queueState === "running"
                      ? "Running"
                      : entry.queueState === "admitted"
                        ? "Admitted"
                        : `#${entry.queuePosition ?? "—"}`}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">YOUR TEAM</span>
            <h2>Agent workspaces</h2>
            <p className="page-message">
              Select a group to scan its hierarchy and active specialists.
            </p>
          </div>

          <button className="primary-button" onClick={onOpenAgents}>
            Manage groups
          </button>
        </div>

        <div
          className="dashboard-group-tabs"
          role="group"
          aria-label="Dashboard agent groups"
        >
          {dashboardGroups.map((group) => {
            const memberCount = projectAgentGroup(agents, group).memberIds.size;
            return (
              <button
                key={group}
                type="button"
                aria-pressed={effectiveAgentGroup === group}
                className={effectiveAgentGroup === group ? "dashboard-group-tab active" : "dashboard-group-tab"}
                onClick={() => setActiveAgentGroup(group)}
              >
                <strong>{group}</strong>
                <small>{memberCount} configured</small>
              </button>
            );
          })}
        </div>

        {groupedAgents.length === 0 ? (
          <p className="page-message">
            No agents configured yet.
          </p>
        ) : (
          <div className="dashboard-agent-grid">
            {groupedAgents.map((agent) => {
              const assignedTasks = agents.flatMap((owner) =>
                owner.tasks.filter(
                  (task) => task.assignedAgentId === agent.id,
                ),
              );
              const runningTask =
                assignedTasks.find((task) => task.status === "Running") ??
                null;

              const pendingTaskCount = assignedTasks.filter(
                (task) => task.queueState === "queued",
              ).length;

              const superior = agents.find((item) => item.id === agent.reportsTo);

              return (
                <article className={`dashboard-agent-tile role-${agent.role.toLowerCase().replace(/\s+/g, "-")}`} key={agent.id}>
                  <div className="dashboard-agent-tile-art">
                    <span>{agent.name.slice(0, 1)}</span>
                    <span className={`agent-status ${agent.status.toLowerCase()}`}>{agent.status}</span>
                  </div>
                  <div className="dashboard-agent-tile-body">
                    <h3>{agent.name}</h3>
                    <p>{agent.role} · {agent.category}</p>
                    <small>
                      {runningTask
                        ? `Running: ${runningTask.title}`
                        : pendingTaskCount > 0
                          ? `${pendingTaskCount} pending task${pendingTaskCount === 1 ? "" : "s"}`
                          : superior
                            ? `Reports to ${superior.name}`
                            : "No active tasks"}
                    </small>
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
