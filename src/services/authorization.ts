import type { Dispatch, SetStateAction } from "react";
import type { ApprovalRequest } from "../applicationState";
import {
  desktopClient,
  type BackendActionIntent,
} from "./desktopClient";

export type AuthorizationReadiness = {
  ready: boolean;
  approval: ApprovalRequest | null;
};

export function upsertApprovalRequest(
  requests: ApprovalRequest[],
  approval: ApprovalRequest,
): ApprovalRequest[] {
  const existingIndex = requests.findIndex(
    (request) => request.id === approval.id,
  );
  if (existingIndex === -1) {
    return [approval, ...requests];
  }
  return requests.map((request) =>
    request.id === approval.id ? approval : request,
  );
}

export async function prepareBackendAuthorization(
  intent: BackendActionIntent,
  setApprovalRequests: Dispatch<SetStateAction<ApprovalRequest[]>>,
): Promise<AuthorizationReadiness> {
  const outcome = await desktopClient.requestAuthorization(intent);
  if (outcome.approval) {
    setApprovalRequests((requests) =>
      upsertApprovalRequest(requests, outcome.approval as ApprovalRequest),
    );
  }
  return {
    ready:
      outcome.decision === "allowed" || outcome.approval?.status === "Approved",
    approval: outcome.approval,
  };
}

export function markApprovalConsumed(
  setApprovalRequests: Dispatch<SetStateAction<ApprovalRequest[]>>,
  approval: ApprovalRequest | null,
): void {
  if (!approval || approval.status !== "Approved") {
    return;
  }
  setApprovalRequests((requests) =>
    requests.map((request) =>
      request.id === approval.id
        ? { ...request, consumedAt: new Date().toISOString() }
        : request,
    ),
  );
}
