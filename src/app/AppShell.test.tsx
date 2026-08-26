// @vitest-environment jsdom

import { useState } from "react";
import axe from "axe-core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { AppShell } from "./AppShell";
import type { Page } from "./navigation";

function ShellHarness() {
  const [page, setPage] = useState<Page>("Dashboard");
  return (
    <AppShell
      activePage={page}
      activeRun={null}
      onNavigate={setPage}
      onProviderChange={() => undefined}
      onStopRun={() => undefined}
      pendingApprovalCount={2}
      persistenceMessage=""
      provider={{
        activeProvider: "codex",
        busy: false,
        connected: true,
        disabled: false,
        hint: "Provider ready",
        message: "",
        name: "Codex",
      }}
      stopRequested={false}
    >
      <h1>{page}</h1>
      <p>{page} content</p>
    </AppShell>
  );
}

describe("AppShell", () => {
  it("exposes navigation state, skip navigation, provider selection, and page focus", async () => {
    const user = userEvent.setup();
    render(<ShellHarness />);

    expect(screen.getByRole("link", { name: "Skip to main content" }).getAttribute("href")).toBe(
      "#main-content",
    );
    expect(screen.getByRole("button", { name: "Dashboard" }).getAttribute("aria-current")).toBe(
      "page",
    );
    expect(screen.getByRole("combobox", { name: "AI provider" })).toBeTruthy();
    expect(
      screen.getByLabelText("2 pending approvals").textContent,
    ).toBe("2");

    await user.click(screen.getByRole("button", { name: "Settings" }));

    const settingsHeading = screen.getByRole("heading", {
      level: 1,
      name: "Settings",
    });
    expect(document.activeElement).toBe(settingsHeading);
    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("passes deterministic axe rules", async () => {
    const { container } = render(<ShellHarness />);
    const results = await axe.run(container, {
      rules: { "color-contrast": { enabled: false } },
    });
    expect(results.violations).toEqual([]);
  });
});
