// @vitest-environment jsdom

import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Dialog } from "./Dialog";

function DialogHarness() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Create agent
      </button>
      <Dialog
        open={open}
        labelledBy="dialog-title"
        onClose={() => setOpen(false)}
      >
        <h2 id="dialog-title">Create agent</h2>
        <button type="button" aria-label="Close agent editor" onClick={() => setOpen(false)}>
          ×
        </button>
        <label>
          Agent name
          <input data-dialog-initial-focus />
        </label>
      </Dialog>
    </>
  );
}

describe("Dialog", () => {
  it("opens modally, moves focus inside, handles cancel, and restores focus", async () => {
    const user = userEvent.setup();
    render(<DialogHarness />);
    const trigger = screen.getByRole("button", { name: "Create agent" });

    await user.click(trigger);

    const dialog = screen.getByRole("dialog", { name: "Create agent" });
    expect(dialog.hasAttribute("open")).toBe(true);
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("textbox", { name: "Agent name" }),
      ),
    );

    fireEvent(dialog, new Event("cancel", { cancelable: true }));

    await waitFor(() => expect(dialog.hasAttribute("open")).toBe(false));
    expect(document.activeElement).toBe(trigger);
  });

  it("provides a named close control", async () => {
    const user = userEvent.setup();
    render(<DialogHarness />);
    const trigger = screen.getByRole("button", { name: "Create agent" });

    await user.click(trigger);
    await user.click(
      screen.getByRole("button", { name: "Close agent editor" }),
    );

    await waitFor(() =>
      expect(screen.getByRole("dialog", { hidden: true }).hasAttribute("open")).toBe(
        false,
      ),
    );
    expect(document.activeElement).toBe(trigger);
  });
});
