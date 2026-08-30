// @vitest-environment jsdom

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { createDefaultApplicationState } from "../../applicationState";
import type { Agent } from "../../applicationState";
import { emptyManagementHandoffSnapshot } from "../../managementHandoffs";
import { unknownProviderRegistrySnapshot } from "../../providerRegistry";
import { emptyReviewOrchestrationSnapshot } from "../../reviewOrchestration";
import { createRunCoordinatorUiState } from "../../runCoordinator";
import { emptyStructuredMemorySnapshot } from "../../structuredMemory";
import { emptyTaskOrchestrationSnapshot } from "../../taskOrchestration";
import { AgentsPage } from "./AgentsPage";

function Harness({ onRegistryMutation }: { onRegistryMutation: () => Promise<void> }) {
  const state = createDefaultApplicationState();
  const [agents, setAgents] = useState<Agent[]>(state.agents);
  return (
    <AgentsPage
      agents={agents}
      setAgents={setAgents}
      templates={[]}
      onRegistryMutation={onRegistryMutation}
      onTaskMutation={() => Promise.resolve()}
      authoritativeRegistry
      authoritativeTaskOrchestration
      taskOrchestration={emptyTaskOrchestrationSnapshot()}
      reviewOrchestration={emptyReviewOrchestrationSnapshot()}
      onReviewSnapshot={() => Promise.resolve()}
      models={state.models}
      providerRegistry={unknownProviderRegistrySnapshot("offline test")}
      preferences={state.preferences}
      runCoordinator={createRunCoordinatorUiState()}
      setRunCoordinator={() => undefined}
      approvalRequests={[]}
      setApprovalRequests={() => undefined}
      onOpenApprovals={() => undefined}
      structuredMemory={emptyStructuredMemorySnapshot}
      managementHandoffs={emptyManagementHandoffSnapshot}
      authoritativeMemory
      onMemoryMutation={() => Promise.resolve()}
    />
  );
}

describe("AgentsPage — editing after creation", () => {
  it("opens the agent editor from the agent workspace, prefilled for that agent", async () => {
    const user = userEvent.setup();
    const onRegistryMutation = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRegistryMutation={onRegistryMutation} />);

    // Enter the Coding Agent workspace.
    await user.click(
      screen.getAllByRole("button", { name: /Open Coding Agent/i })[0],
    );
    expect(
      screen.getByRole("heading", { name: "Agent details" }),
    ).toBeTruthy();

    // The workspace exposes an explicit editor entry point.
    await user.click(screen.getByRole("button", { name: "Edit agent" }));

    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).getByRole("heading", { name: "Edit agent" }),
    ).toBeTruthy();
    const nameField = within(dialog).getByLabelText("Agent name") as HTMLInputElement;
    expect(nameField.value).toBe("Coding Agent");

    // The editor exposes role and reporting-line controls that the read-only
    // workspace summary does not.
    expect(within(dialog).getByLabelText("Role")).toBeTruthy();
    expect(within(dialog).getByLabelText("Reports to")).toBeTruthy();
  });

  it("submits a reporting-line change through the authoritative registry", async () => {
    const user = userEvent.setup();
    const onRegistryMutation = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRegistryMutation={onRegistryMutation} />);

    await user.click(
      screen.getAllByRole("button", { name: /Open Coding Agent/i })[0],
    );
    await user.click(screen.getByRole("button", { name: "Edit agent" }));

    const dialog = screen.getByRole("dialog");
    await user.selectOptions(
      within(dialog).getByLabelText("Reports to"),
      within(dialog)
        .getByLabelText("Reports to")
        .querySelector("option:not([value=''])") as HTMLOptionElement,
    );
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    expect(onRegistryMutation).toHaveBeenCalledWith(
      "update_agent",
      expect.objectContaining({ agentId: 2, role: "Specialist" }),
    );
  });
});
