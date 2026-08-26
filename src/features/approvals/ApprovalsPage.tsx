import { useState } from "react";
import type { Agent, ApprovalRequest, ApprovalRequestStatus, WorkspaceDefinition } from "../../applicationState";
import { safetyScopeLabels } from "../../domain/taskSafety";
import { desktopClient } from "../../services/desktopClient";
import { errorMessage } from "../../domain/errors";
import { upsertApprovalRequest } from "../../services/authorization";

export function ApprovalsPage({
  agents,
  approvalRequests,
  setApprovalRequests,
  workspaces,
  onOpenAgents,
}: {
  agents: Agent[];
  approvalRequests: ApprovalRequest[];
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  workspaces: WorkspaceDefinition[];
  onOpenAgents: () => void;
}) {
  const [statusFilter, setStatusFilter] =
    useState<ApprovalRequestStatus | "All">("Pending");
  const [resolutionError, setResolutionError] = useState("");

  const filteredRequests = approvalRequests.filter(
    (request) =>
      statusFilter === "All" || request.status === statusFilter,
  );

  const pendingCount = approvalRequests.filter(
    (request) => request.status === "Pending",
  ).length;
  const approvedCount = approvalRequests.filter(
    (request) => request.status === "Approved",
  ).length;
  const deniedCount = approvalRequests.filter(
    (request) => request.status === "Denied",
  ).length;
  const expiredCount = approvalRequests.filter(
    (request) => request.status === "Expired",
  ).length;

  async function resolveApproval(
    requestId: number,
    status: "Approved" | "Denied",
  ) {
    const request = approvalRequests.find((item) => item.id === requestId);
    if (!request) {
      return;
    }
    setResolutionError("");
    let resolved: ApprovalRequest;
    try {
      resolved = await desktopClient.resolveApproval(
        requestId,
        status === "Approved" ? "approve" : "deny",
      );
    } catch (error) {
      setResolutionError(errorMessage(error));
      return;
    }
    setApprovalRequests((currentRequests) =>
      upsertApprovalRequest(currentRequests, resolved),
    );
  }

  return (
    <>
      <header className="topbar">
        <div>
          <span className="eyebrow">SAFETY GATE</span>
          <h1>Approvals</h1>
          <p className="page-message">
            Review and resolve actions that require human authorization.
            Approval history is managed by the backend.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>Pending</span>
          <strong>{pendingCount}</strong>
          <small>Needs a decision</small>
        </article>

        <article className="summary-card">
          <span>Approved</span>
          <strong>{approvedCount}</strong>
          <small>Authorized requests</small>
        </article>

        <article className="summary-card">
          <span>Denied</span>
          <strong>{deniedCount}</strong>
          <small>Rejected requests</small>
        </article>

        <article className="summary-card">
          <span>Expired</span>
          <strong>{expiredCount}</strong>
          <small>Authorization window closed</small>
        </article>
      </section>

      <section className="panel">
        {resolutionError && (
          <p className="page-message" role="alert">{resolutionError}</p>
        )}
        <div className="panel-heading">
          <div>
            <span className="eyebrow">REQUEST QUEUE</span>
            <h2>Approval requests</h2>
          </div>
        </div>

        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(220px, 320px)",
            marginBottom: "20px",
          }}
        >
          <label className="form-field">
            <span>Status</span>
            <select
              value={statusFilter}
              onChange={(event) =>
                setStatusFilter(
                  event.target.value as
                    | ApprovalRequestStatus
                    | "All",
                )
              }
            >
              <option value="Pending">Pending</option>
              <option value="Approved">Approved</option>
              <option value="Denied">Denied</option>
              <option value="Expired">Expired</option>
              <option value="All">All requests</option>
            </select>
          </label>
        </div>

        {filteredRequests.length === 0 ? (
          <p className="page-message">
            No approval requests match this filter.
          </p>
        ) : (
          <div className="agent-list">
            {filteredRequests.map((request) => {
              const executor =
                agents.find(
                  (item) => item.id === request.agentId,
                ) ?? null;
              const ownedTask = agents
                .flatMap((owner) =>
                  owner.tasks.map((task) => ({ owner, task })),
                )
                .find(({ task }) => task.id === request.taskId);
              const task = ownedTask?.task ?? null;

              return (
                <article
                  className="agent-card"
                  key={request.id}
                >
                  <div style={{ flex: 1 }}>
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
                        {request.title}
                      </h3>

                      <span
                        className={`agent-status ${
                          request.status === "Approved"
                            ? "working"
                            : request.status === "Denied" ||
                                request.status === "Expired"
                              ? "paused"
                              : "waiting"
                        }`}
                      >
                        {request.status}
                      </span>
                    </div>

                    <p>{request.reason}</p>
                    <div className="approval-detail-grid">
                      <span>
                        <strong>Risk</strong>
                        {request.riskLevel}
                      </span>
                      <span>
                        <strong>Permission</strong>
                        {request.scopes.length > 0
                          ? request.scopes
                              .map((scope) => safetyScopeLabels[scope])
                              .join(", ")
                          : "Manual review"}
                      </span>
                      <span>
                        <strong>Workspace</strong>
                        {workspaces.find(
                          (workspace) => workspace.id === request.workspaceId,
                        )?.name ?? "Unknown"}
                      </span>
                    </div>
                    <small>
                      Owner: {ownedTask?.owner.name ?? "Unknown"} · Executor:{" "}
                      {executor?.name ?? "Unknown"} · Role:{" "}
                      {executor?.role ?? "Unknown"}
                      {task
                        ? ` · Task phase: ${task.phase}`
                        : ""}
                    </small>
                    <br />
                    <small>
                      Requested:{" "}
                      {new Date(
                        request.createdAt,
                      ).toLocaleString()}
                    </small>
                    <br />
                    <small>
                      Expires: {new Date(request.expiresAt).toLocaleString()}
                      {request.consumedAt
                        ? ` · Used: ${new Date(request.consumedAt).toLocaleString()}`
                        : " · One run only"}
                    </small>
                  </div>

                  <div
                    style={{
                      display: "flex",
                      gap: "8px",
                      flexWrap: "wrap",
                      justifyContent: "flex-end",
                    }}
                  >
                    {request.status === "Pending" && (
                      <>
                        <button
                          className="primary-button"
                          onClick={() =>
                            resolveApproval(
                              request.id,
                              "Approved",
                            )
                          }
                        >
                          Approve
                        </button>

                        <button
                          className="danger-button"
                          onClick={() =>
                            resolveApproval(
                              request.id,
                              "Denied",
                            )
                          }
                        >
                          Deny
                        </button>
                      </>
                    )}

                    {request.status === "Approved" &&
                      request.consumedAt === null && (
                        <button
                          className="secondary-button"
                          onClick={onOpenAgents}
                        >
                          Open agent
                        </button>
                      )}
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
