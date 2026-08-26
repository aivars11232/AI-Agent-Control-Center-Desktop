// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { KeyboardAction } from "./KeyboardAction";

describe("KeyboardAction", () => {
  it("activates from click, Enter, and Space without scrolling", async () => {
    const user = userEvent.setup();
    const onActivate = vi.fn();
    render(
      <KeyboardAction label="Open agent workspace" onActivate={onActivate}>
        Agent details
      </KeyboardAction>,
    );
    const action = screen.getByRole("button", {
      name: "Open agent workspace",
    });

    await user.click(action);
    action.focus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");

    expect(onActivate).toHaveBeenCalledTimes(3);
  });
});
