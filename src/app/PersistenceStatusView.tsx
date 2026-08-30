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
 * for a working application screen. Recovery affordances beyond truthful
 * reporting are owned by the later integrated UI/UX task.
 */
export function PersistenceStatusView({
  phase,
  message,
}: {
  phase: Exclude<PersistencePhase, "ready">;
  message: string;
}) {
  const failed = phase === "error";
  return (
    <div className="app-shell">
      <main className="main-content">
        <section className="panel" role={failed ? "alert" : "status"}>
          <span className="eyebrow">APPLICATION STATE</span>
          <h1>
            {failed ? "Application data unavailable" : "Loading application data"}
          </h1>
          <p className="page-message">
            {failed
              ? message
              : phase === "mutating"
                ? "Updating the versioned local application database…"
                : "Opening the versioned local application database…"}
          </p>
          {failed && (
            <p className="form-hint">
              No desktop data was written to browser storage. Resolve the
              database error and restart the app.
            </p>
          )}
        </section>
      </main>
    </div>
  );
}
