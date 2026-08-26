import { useEffect, useRef, type ReactNode } from "react";
import type { RuntimeProviderId } from "../applicationState";
import type { RunAttempt } from "../runCoordinator";
import { LiveStatus } from "../components/LiveStatus";
import logoUrl from "../../AI-Agents.png";
import { PAGES, type Page } from "./navigation";

export type ProviderShellState = {
  activeProvider: RuntimeProviderId;
  busy: boolean;
  connected: boolean;
  disabled: boolean;
  hint: string;
  message: string;
  name: string;
};

export function AppShell({
  activePage,
  activeRun,
  children,
  latestRunProgress,
  onNavigate,
  onProviderChange,
  onStopRun,
  pendingApprovalCount,
  persistenceMessage,
  provider,
  stopRequested,
}: {
  activePage: Page;
  activeRun: RunAttempt | null;
  children: ReactNode;
  latestRunProgress?: string;
  onNavigate: (page: Page) => void;
  onProviderChange: (provider: RuntimeProviderId) => void;
  onStopRun: () => void;
  pendingApprovalCount: number;
  persistenceMessage: string;
  provider: ProviderShellState;
  stopRequested: boolean;
}) {
  const mainRef = useRef<HTMLElement>(null);
  const previousPageRef = useRef(activePage);

  useEffect(() => {
    if (previousPageRef.current === activePage) return;
    previousPageRef.current = activePage;
    const heading = mainRef.current?.querySelector<HTMLElement>("h1");
    if (heading) {
      heading.tabIndex = -1;
      heading.focus();
    } else {
      mainRef.current?.focus();
    }
  }, [activePage]);

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>

      <aside className="sidebar" aria-label="Application sidebar">
        <div className="brand">
          <div className="brand-icon">
            <img src={logoUrl} alt="" />
          </div>
          <div>
            <strong>AI Agent</strong>
            <span>Control Center</span>
          </div>
        </div>

        <nav aria-label="Primary navigation">
          {PAGES.map((page) => (
            <button
              key={page}
              type="button"
              className={`nav-item ${activePage === page ? "active" : ""}`}
              aria-current={activePage === page ? "page" : undefined}
              onClick={() => onNavigate(page)}
            >
              <span>{page}</span>
              {page === "Approvals" && pendingApprovalCount > 0 && (
                <span
                  className="nav-count"
                  aria-label={`${pendingApprovalCount} pending approvals`}
                >
                  {pendingApprovalCount}
                </span>
              )}
            </button>
          ))}
        </nav>

        <section className="system-status" aria-label="AI provider status">
          <span
            className={`status-dot ${provider.connected ? "" : "offline"}`}
            aria-hidden="true"
          />
          <div>
            <strong>
              {provider.name} {provider.connected ? "connected" : "unavailable"}
            </strong>
            <small>{provider.hint}</small>
            <label className="system-provider-select">
              <span>AI provider</span>
              <select
                value={provider.activeProvider}
                disabled={provider.disabled || provider.busy}
                onChange={(event) =>
                  onProviderChange(event.target.value as RuntimeProviderId)
                }
              >
                <option value="codex">Codex</option>
                <option value="ollama">Ollama</option>
              </select>
            </label>
            <LiveStatus
              className="system-provider-message"
              kind={provider.connected ? "status" : "error"}
            >
              {provider.message}
            </LiveStatus>
          </div>
        </section>
      </aside>

      <main
        ref={mainRef}
        id="main-content"
        className="main-content"
        tabIndex={-1}
      >
        <LiveStatus className="page-message">
          {persistenceMessage}
        </LiveStatus>
        {activeRun && (
          <section className="global-run-banner" aria-live="polite">
            <div>
              <span className="eyebrow">ONE ACTIVE AI RUN</span>
              <strong>{activeRun.taskTitle}</strong>
              <small>
                {activeRun.runMode === "review"
                  ? "Structured review"
                  : "Task execution"}
                {` · ${activeRun.status.replace(/_/g, " ")}`}
                {activeRun.provider ? ` · ${activeRun.provider}` : ""}
                {activeRun.model ? ` · ${activeRun.model}` : ""}
              </small>
              {latestRunProgress && (
                <span className="global-run-progress">
                  {latestRunProgress}
                </span>
              )}
            </div>
            <button
              type="button"
              className="danger-button"
              disabled={stopRequested}
              onClick={onStopRun}
            >
              {stopRequested ? "Stopping…" : "Stop active run"}
            </button>
          </section>
        )}
        {children}
      </main>
    </div>
  );
}
