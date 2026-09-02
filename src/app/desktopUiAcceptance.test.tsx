// @vitest-environment jsdom

/**
 * Composed desktop UI and recovery acceptance (TASK-0028).
 *
 * These scenarios drive the real `AppController` over a scripted desktop-client
 * double rather than any single page in isolation, because the defects this
 * task owns only appear when the shell, the navigation, and one page's
 * projection of authoritative backend state are exercised together: a
 * first-run window, a populated window, a failed load that must still read as
 * a report, and a restart.
 *
 * The double answers the same read commands the installed app answers and
 * throws on any command a scenario is not supposed to reach, so an accidental
 * mutation is a failure rather than a silent pass.
 */
import axe from "axe-core";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createDefaultApplicationState,
  type ApplicationState,
  type StateEnvelope,
} from "../applicationState";
import { previewMonitoringSnapshot } from "../dataLifecycle";
import { EMPTY_RUN_COORDINATOR_SNAPSHOT } from "../runCoordinator";
import { emptyTaskOrchestrationSnapshot } from "../taskOrchestration";
import { emptyReviewOrchestrationSnapshot } from "../reviewOrchestration";
import { emptyReminderSchedulerSnapshot } from "../reminderScheduler";
import { emptyStructuredMemorySnapshot } from "../structuredMemory";
import { emptyManagementHandoffSnapshot } from "../managementHandoffs";
import { unknownProviderRegistrySnapshot } from "../providerRegistry";
import { PAGES, type Page } from "./navigation";

type Scenario = {
  envelope: StateEnvelope | null;
  loadError: string | null;
  saveError: string | null;
};

const scenario: Scenario = {
  envelope: null,
  loadError: null,
  saveError: null,
};

function envelopeFor(state: ApplicationState): StateEnvelope {
  return {
    schemaVersion: 4,
    revision: 12,
    state,
    migration: {
      sourceKind: "fresh",
      sourceVersion: null,
      migratedAtUnixMs: null,
      legacyCleanupAcknowledged: true,
    },
  };
}

function firstRunState(): ApplicationState {
  const state = createDefaultApplicationState();
  state.agents = [];
  state.approvalRequests = [];
  state.reminders = [];
  return state;
}

function populatedState(): ApplicationState {
  return createDefaultApplicationState();
}

function authoritativeMonitoring(state: ApplicationState) {
  return { ...previewMonitoringSnapshot(state, 0, 0), authoritative: true };
}

function currentState(): ApplicationState {
  if (!scenario.envelope) throw new Error("no scenario state loaded");
  return scenario.envelope.state;
}

function emptyMonitoringPage(state: ApplicationState) {
  return {
    authoritative: true,
    revision: authoritativeMonitoring(state).revision,
    offset: 0,
    limit: 100,
    total: 0,
    records: [],
  };
}

function unusedCommand(name: string) {
  return () => {
    throw new Error(`the scenario must not reach ${name}`);
  };
}

const noopListener = () => Promise.resolve(() => undefined);

vi.mock("../services/desktopClient", () => {
  const client = {
    isDesktopRuntime: () => true,
    invokeApplicationState: (command: string) => {
      if (command === "save_application_state") {
        if (scenario.saveError) {
          return Promise.reject(new Error(scenario.saveError));
        }
        const envelope = scenario.envelope as StateEnvelope;
        return Promise.resolve({
          schemaVersion: envelope.schemaVersion,
          revision: envelope.revision + 1,
        });
      }
      if (scenario.loadError) return Promise.reject(new Error(scenario.loadError));
      if (command === "load_application_state" || command === "initialize_application_state") {
        return Promise.resolve(scenario.envelope);
      }
      return Promise.reject(new Error(`unexpected state command ${command}`));
    },
    loadApplicationState: () => {
      if (scenario.loadError) return Promise.reject(new Error(scenario.loadError));
      return Promise.resolve(scenario.envelope);
    },
    agentRegistrySnapshot: () => Promise.resolve({ revision: 1, templates: [] }),
    taskOrchestrationSnapshot: () => Promise.resolve(emptyTaskOrchestrationSnapshot()),
    reviewOrchestrationSnapshot: () => Promise.resolve(emptyReviewOrchestrationSnapshot()),
    runCoordinatorSnapshot: () => Promise.resolve(EMPTY_RUN_COORDINATOR_SNAPSHOT),
    monitoringSnapshot: () => Promise.resolve(authoritativeMonitoring(currentState())),
    reminderSchedulerSnapshot: () =>
      Promise.resolve({
        ...emptyReminderSchedulerSnapshot,
        systemTimeZone: "Europe/Amsterdam",
      }),
    structuredMemorySnapshot: () => Promise.resolve(emptyStructuredMemorySnapshot),
    managementHandoffSnapshot: () => Promise.resolve(emptyManagementHandoffSnapshot),
    providerRegistryStatus: () =>
      Promise.resolve(unknownProviderRegistrySnapshot("Provider readiness inspected.")),
    querySystemActionAudits: () => Promise.resolve({ records: [], nextCursor: null }),
    queryMonitoringActivity: () =>
      Promise.resolve(emptyMonitoringPage(currentState())),
    queryMonitoringTasks: () =>
      Promise.resolve(emptyMonitoringPage(currentState())),
    voiceRuntimeStatus: () =>
      Promise.resolve({
        installed: false,
        listening: false,
        highAccuracyAvailable: false,
        installState: "missing" as const,
        listenerState: "stopped" as const,
        operationId: null,
        canCancel: false,
        message: "The offline voice engine is not installed.",
      }),
    desktopControlStatus: () =>
      Promise.resolve({
        enabled: false,
        sessionActive: false,
        restoreTokenPresent: false,
        message: "Desktop input control is disabled.",
      }),
    onRunCoordinatorEvent: noopListener,
    onRunCoordinatorSnapshot: noopListener,
    onReminderSchedulerSnapshot: noopListener,
    onVoiceControlOpen: noopListener,
    onRemindersOpen: noopListener,
    onVoiceRuntimeStatus: noopListener,
    onDesktopControlStatus: noopListener,
    onVoiceTranscript: noopListener,
    cancelAgentRun: unusedCommand("cancelAgentRun"),
    clearMonitoringActivity: unusedCommand("clearMonitoringActivity"),
    deleteMonitoringActivity: unusedCommand("deleteMonitoringActivity"),
    exportBackup: unusedCommand("exportBackup"),
    mutateReminderScheduler: unusedCommand("mutateReminderScheduler"),
    mutateStructuredMemory: unusedCommand("mutateStructuredMemory"),
    resolveApproval: unusedCommand("resolveApproval"),
    runAgentTask: unusedCommand("runAgentTask"),
    startReviewStage: unusedCommand("startReviewStage"),
    submitVoiceIntent: unusedCommand("submitVoiceIntent"),
    startVoiceListener: unusedCommand("startVoiceListener"),
    stopVoiceListener: () => Promise.resolve(undefined),
    requestAuthorization: unusedCommand("requestAuthorization"),
    recordHumanReviewDecision: unusedCommand("recordHumanReviewDecision"),
    chooseWorkspaceFolder: unusedCommand("chooseWorkspaceFolder"),
    openWorkspaceItem: unusedCommand("openWorkspaceItem"),
    enableDesktopControl: unusedCommand("enableDesktopControl"),
    disableDesktopControl: unusedCommand("disableDesktopControl"),
    installVoiceRuntime: unusedCommand("installVoiceRuntime"),
    installHighAccuracyVoiceRuntime: unusedCommand("installHighAccuracyVoiceRuntime"),
    cancelVoiceRuntimeInstall: unusedCommand("cancelVoiceRuntimeInstall"),
  };
  return {
    desktopClient: client,
    isDesktopRuntime: client.isDesktopRuntime,
  };
});

const { AppController } = await import("./AppController");

async function renderReadyApp() {
  const view = render(<AppController />);
  await screen.findByRole("navigation", { name: "Primary navigation" });
  return view;
}

async function navigateTo(user: ReturnType<typeof userEvent.setup>, page: Page) {
  await user.click(screen.getByRole("button", { name: page }));
  return screen.findByRole("heading", { level: 1 });
}

beforeEach(() => {
  scenario.envelope = envelopeFor(populatedState());
  scenario.loadError = null;
  scenario.saveError = null;
  window.localStorage?.clear();
});

// A scenario that walks all nine pages is the slowest work in the deterministic
// frontend gate, and Vitest's implicit 5 s default gave it no headroom: the axe
// walk measured 5168 ms on the pinned CI runner and 2.5 s on the development
// machine, so the gate's verdict turned on which machine ran it. These two
// scenarios declare their own budget instead of inheriting a default that
// encodes an assumption about host speed.
const PAGE_WALK_TIMEOUT_MS = 30_000;

describe("integrated desktop UI acceptance", () => {
  it("navigates every core page from a first-run window and projects a truthful state", async () => {
    scenario.envelope = envelopeFor(firstRunState());
    const user = userEvent.setup();
    await renderReadyApp();

    // Every hop is a real page change, so the shell's focus move is asserted
    // on each one. The window opens on Dashboard, so it is visited last.
    const order: Page[] = [
      ...PAGES.filter((page) => page !== "Dashboard"),
      "Dashboard",
    ];
    for (const page of order) {
      const heading = await navigateTo(user, page);

      // The page rendered its own content, not the unfinished-page placeholder.
      expect(
        screen.queryByText("This page is ready for its controls and content."),
      ).toBeNull();
      expect(heading.textContent?.trim().length ?? 0).toBeGreaterThan(0);

      // Navigation state and keyboard focus both followed the click.
      expect(
        screen.getByRole("button", { name: page }).getAttribute("aria-current"),
      ).toBe("page");
      expect(document.activeElement).toBe(heading);
    }
  }, PAGE_WALK_TIMEOUT_MS);

  it("keeps the Activity page consistent with the authoritative active-agent count", async () => {
    const state = populatedState();
    scenario.envelope = envelopeFor(state);
    const activeAgents = state.agents.filter((agent) => agent.status === "Working");
    expect(activeAgents.length).toBeGreaterThan(0);
    // The seeded agents that count as active own no running task, which is the
    // exact shape that used to make the page contradict its own headline.
    expect(
      activeAgents.every((agent) =>
        agent.tasks.every(
          (task) => task.status !== "Running" && task.status !== "Under Review",
        ),
      ),
    ).toBe(true);

    const user = userEvent.setup();
    await renderReadyApp();
    await navigateTo(user, "Activity");

    const activeCard = screen.getByText("Active agents").closest("article");
    expect(activeCard).not.toBeNull();
    expect(
      within(activeCard as HTMLElement).getByText(String(activeAgents.length)),
    ).toBeTruthy();

    expect(
      screen.queryByText(/No agent is marked working and no task is running/),
    ).toBeNull();
    for (const agent of activeAgents) {
      expect(screen.getAllByText(agent.name).length).toBeGreaterThan(0);
    }
  });

  it("reports a failed load as a bounded recovery screen, not a broken application screen", async () => {
    scenario.loadError =
      "DATABASE_CORRUPT: The application database could not be validated and was not modified.";
    render(<AppController />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("DATABASE_CORRUPT");
    expect(
      screen.getByRole("heading", { level: 1 }).textContent,
    ).toBe("Application data unavailable");

    // No application chrome may appear behind a failure.
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(screen.queryByRole("combobox")).toBeNull();

    // The screen owns a full-width centred layout instead of the shell grid,
    // whose first column is reserved for the sidebar.
    expect(document.querySelector(".app-shell")).toBeNull();
    expect(document.querySelector(".status-shell")).not.toBeNull();

    // A startup failure offers no retry: the backend opens the database once
    // when the process starts, so a renderer retry would replay the same error.
    // The screen says so instead of offering an action that cannot work.
    expect(screen.queryByRole("button")).toBeNull();
    expect(
      screen.getByText(/opens the application database once at startup/),
    ).toBeTruthy();

    const results = await axe.run(document.body, {
      rules: { "color-contrast": { enabled: false } },
    });
    expect(results.violations).toEqual([]);
  });

  it("recovers into the working application when a post-load failure is retried", async () => {
    const user = userEvent.setup();
    await renderReadyApp();
    await navigateTo(user, "Activity");

    // A write that fails after the window is already usable drops the whole
    // window to the recovery screen; that failure class is retryable, so the
    // screen offers the action and the retry restores the application.
    scenario.saveError = "DATABASE_LOCKED: The application database is locked.";
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Activity retention" }),
      "7",
    );

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("DATABASE_LOCKED");
    expect(screen.queryByRole("navigation")).toBeNull();

    scenario.saveError = null;
    await user.click(screen.getByRole("button", { name: "Try again" }));

    await screen.findByRole("navigation", { name: "Primary navigation" });
    expect(screen.queryByRole("alert")).toBeNull();
    // The recovered window returns to the page the operator was on rather than
    // resetting their position.
    expect(
      screen.getByRole("heading", { level: 1, name: "Activity" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Activity" }).getAttribute("aria-current"),
    ).toBe("page");
  });

  it("re-reads authoritative state after a restart instead of reusing renderer state", async () => {
    const user = userEvent.setup();
    const first = await renderReadyApp();
    await navigateTo(user, "Agents");
    const agentName = currentState().agents[0].name;
    expect(screen.getAllByText(agentName).length).toBeGreaterThan(0);
    first.unmount();

    // A restart against a database that lost its agents must show the empty
    // window, never the previous render's projection.
    scenario.envelope = envelopeFor(firstRunState());
    await renderReadyApp();
    await waitFor(() =>
      expect(screen.queryByText(agentName)).toBeNull(),
    );
    expect(
      screen.getByRole("heading", { level: 1, name: "Dashboard" }),
    ).toBeTruthy();
  });

  // TASK-0030 live acceptance: tabbing from "Export backup" landed on "Reset
  // portable state", skipping the import control entirely. It was a `<label>`
  // wrapping a `display: none` file input — a label is not focusable and the
  // hidden input is outside the tab order, so restoring a backup was impossible
  // without a pointer. axe did not catch it: a hidden input raises no violation,
  // and the label had no interactive role to check.
  it("keeps both backup controls reachable from the keyboard", async () => {
    const user = userEvent.setup();
    await renderReadyApp();
    await navigateTo(user, "Settings");

    const exportButton = screen.getByRole("button", { name: "Export backup" });
    const importButton = screen.getByRole("button", { name: "Import backup" });

    exportButton.focus();
    expect(document.activeElement).toBe(exportButton);

    // The very next tab stop must be the import control, not whatever follows
    // it in the panel.
    await user.tab();
    expect(document.activeElement).toBe(importButton);
  });

  it("passes deterministic axe rules on every core page", async () => {
    const user = userEvent.setup();
    const { container } = await renderReadyApp();

    for (const page of PAGES) {
      await navigateTo(user, page);
      const results = await axe.run(container, {
        rules: { "color-contrast": { enabled: false } },
      });
      expect(
        results.violations.map(
          (violation) =>
            `${page}: ${violation.id}: ${violation.nodes
              .map((node) => node.html)
              .join(" | ")}`,
        ),
      ).toEqual([]);
    }
  }, PAGE_WALK_TIMEOUT_MS);
});
