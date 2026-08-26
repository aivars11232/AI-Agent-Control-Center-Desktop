// @vitest-environment jsdom

import { useState } from "react";
import axe from "axe-core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

function InteractionProbe() {
  const [active, setActive] = useState(false);

  return (
    <main aria-labelledby="probe-title">
      <h1 id="probe-title">Interaction probe</h1>
      <button type="button" onClick={() => setActive(true)}>
        Activate
      </button>
      <p role="status">{active ? "Active" : "Inactive"}</p>
    </main>
  );
}

describe("DOM accessibility test foundation", () => {
  it("supports rendered interaction and axe checks", async () => {
    const user = userEvent.setup();
    const { container } = render(<InteractionProbe />);

    await user.click(screen.getByRole("button", { name: "Activate" }));

    expect(screen.getByRole("status").textContent).toBe("Active");
    const results = await axe.run(container, {
      rules: {
        // jsdom has no canvas-backed computed color contrast. Native/manual
        // verification owns visual contrast; DOM tests still exercise the
        // remaining deterministic axe rules.
        "color-contrast": { enabled: false },
      },
    });
    expect(results.violations).toEqual([]);
  });
});
