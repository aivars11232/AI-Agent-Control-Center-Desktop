// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PersistenceStatusView } from "./PersistenceStatusView";

describe("PersistenceStatusView", () => {
  it("reports a migration failure as an alert without any application chrome", () => {
    render(
      <PersistenceStatusView
        phase="error"
        message={
          "STATE_VALIDATION_FAILED: Stored application state is invalid at " +
          "agents[5].id: agent id must be unique"
        }
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("STATE_VALIDATION_FAILED");
    expect(alert.textContent).toContain("agent id must be unique");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(
      "Application data unavailable",
    );
    expect(
      screen.getByText(/No desktop data was written to browser storage/),
    ).toBeTruthy();

    // The failure screen must never masquerade as a working application screen.
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.queryByRole("combobox")).toBeNull();
  });

  it("shows a non-alert loading status while the database opens", () => {
    render(<PersistenceStatusView phase="loading" message="" />);

    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("status").textContent).toContain(
      "Opening the versioned local application database",
    );
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe(
      "Loading application data",
    );
    expect(
      screen.queryByText(/No desktop data was written to browser storage/),
    ).toBeNull();
  });

  it("distinguishes an in-progress database update from a first open", () => {
    render(<PersistenceStatusView phase="mutating" message="" />);

    expect(screen.getByRole("status").textContent).toContain(
      "Updating the versioned local application database",
    );
  });
});
