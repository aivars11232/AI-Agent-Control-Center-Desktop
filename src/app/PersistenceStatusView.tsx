export type PersistencePhase =
  | "loading"
  | "mutating"
  | "hydrating"
  | "ready"
  | "error";

/**
 * Full-window persistence status shown before the authoritative application
 * state is ready. It deliberately renders no navigation, provider controls, or
 * feature content so a database load or migration failure can never be mistaken
 * for a working application screen.
 *
 * It owns its own centred layout rather than reusing the application shell
 * grid: the shell reserves its first column for the sidebar, so a single
 * `main` child would be squeezed into the sidebar column and the failure would
 * read as a broken screen instead of a report.
 *
 * `onRetry` is supplied only for a failure the renderer can actually recover
 * from - one raised after the application state had already loaded once. A
 * failure during startup is not offered a retry, because the backend opens the
 * database a single time when the process starts and a renderer retry would
 * replay the same stored error. Nothing here writes state or bypasses backend
 * validation.
 */
export function PersistenceStatusView({
  phase,
  message,
  onRetry,
}: {
  phase: Exclude<PersistencePhase, "ready">;
  message: string;
  onRetry?: () => void;
}) {
  const failed = phase === "error";
  return (
    <div className="status-shell">
      <main className="status-main">
        <section className="panel status-panel" role={failed ? "alert" : "status"}>
          <span className="eyebrow">APPLICATION STATE</span>
          <h1>
            {failed ? "Application data unavailable" : "Loading application data"}
          </h1>
          <p className="page-message">
            {failed
              ? "The application database could not be opened or validated, so no"
                + " application screen is shown. The reported cause is below."
              : phase === "mutating"
                ? "Updating the versioned local application database…"
                : "Opening the versioned local application database…"}
          </p>
          {failed && (
            <>
              <p className="status-detail">{message}</p>
              <p className="form-hint">
                No desktop data was written to browser storage, and nothing was
                modified.{" "}
                {onRetry
                  ? "Retrying re-reads the authoritative state from the backend."
                  : "The backend opens the application database once at startup,"
                    + " so this cannot be retried from here. Resolve the database"
                    + " error and restart the app."}
              </p>
              {onRetry && (
                <div className="status-actions">
                  <button
                    type="button"
                    className="primary-button"
                    onClick={onRetry}
                  >
                    Try again
                  </button>
                </div>
              )}
            </>
          )}
        </section>
      </main>
    </div>
  );
}
