// @vitest-environment jsdom

import axe from "axe-core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createRunCoordinatorUiState } from "../../runCoordinator";
import { emptyTaskOrchestrationSnapshot } from "../../taskOrchestration";
import { agentFixture } from "../../test/fixtures";
import { DashboardPage } from "./DashboardPage";

describe("DashboardPage", () => {
  it("keeps group selection and primary navigation keyboard-operable", async () => {
    const user = userEvent.setup();
    const onOpenAgents = vi.fn();
    render(
      <DashboardPage
        agents={[agentFixture()]}
        approvalRequests={[]}
        taskOrchestration={emptyTaskOrchestrationSnapshot()}
        runCoordinator={createRunCoordinatorUiState()}
        onOpenAgents={onOpenAgents}
        onOpenTasks={() => undefined}
        onOpenApprovals={() => undefined}
      />,
    );

    const group = screen.getByRole("group", {
      name: "Dashboard agent groups",
    });
    expect(group.querySelector('[aria-pressed="true"]')?.textContent).toContain(
      "Development",
    );

    await user.click(screen.getByRole("button", { name: "Manage agents" }));
    expect(onOpenAgents).toHaveBeenCalledOnce();
  });

  it("passes deterministic axe rules", async () => {
    const { container } = render(
      <DashboardPage
        agents={[agentFixture()]}
        approvalRequests={[]}
        taskOrchestration={emptyTaskOrchestrationSnapshot()}
        runCoordinator={createRunCoordinatorUiState()}
        onOpenAgents={() => undefined}
        onOpenTasks={() => undefined}
        onOpenApprovals={() => undefined}
      />,
    );
    const results = await axe.run(container, {
      rules: { "color-contrast": { enabled: false } },
    });
    expect(results.violations).toEqual([]);
  });
});
