// @vitest-environment jsdom

import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Tabs, tabId, tabPanelId } from "./Tabs";

const tabs = [
  { value: "Overview", label: "Overview" },
  { value: "Tasks", label: "Tasks" },
  { value: "Activity", label: "Activity" },
] as const;

function TabsHarness() {
  const [active, setActive] = useState<(typeof tabs)[number]["value"]>(
    "Overview",
  );
  return (
    <>
      <Tabs
        idPrefix="agent-workspace"
        label="Agent workspace"
        tabs={tabs}
        value={active}
        onChange={setActive}
      />
      <section
        id={tabPanelId("agent-workspace", active)}
        role="tabpanel"
        aria-labelledby={tabId("agent-workspace", active)}
      >
        {active} content
      </section>
    </>
  );
}

describe("Tabs", () => {
  it("automatically activates and focuses tabs with Arrow, Home, and End", async () => {
    const user = userEvent.setup();
    render(<TabsHarness />);
    const overview = screen.getByRole("tab", { name: "Overview" });
    overview.focus();

    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Tasks" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Tasks" }),
    );

    await user.keyboard("{End}");
    expect(screen.getByRole("tabpanel").textContent).toBe("Activity content");
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Activity" }),
    );

    await user.keyboard("{Home}");
    expect(screen.getByRole("tabpanel").textContent).toBe("Overview content");
    expect(document.activeElement).toBe(overview);
  });
});
