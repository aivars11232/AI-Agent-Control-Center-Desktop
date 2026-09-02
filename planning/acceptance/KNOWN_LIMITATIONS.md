# Version 1.0.0 known limitations

These are recorded, not waived. Each is either a deliberate boundary of 1.0 or
something the acceptance run could not prove.

## Not proven by the acceptance run

- **Codex bounded-run completion.** The scenario that drives a real Codex turn to
  completion and asserts truthful zero-change workspace evidence could not run:
  the account's ChatGPT Codex usage limit was exhausted, resetting 2026-09-07.
  Verified outside the application. Codex containment, cancellation, identity,
  and typed failure were all observed live; only the completion path is
  outstanding. Re-run with:
  `cargo test --manifest-path src-tauri/Cargo.toml provider_review_acceptance::live -- --ignored --test-threads=1 --nocapture`
- **A spoken command through the full GUI voice pipeline.** Microphone capture,
  the offline listener, the intent gateway, and portal dispatch are each covered
  deterministically and, for the portal, live. The end-to-end path driven by an
  actual human voice was not executed because this workflow cannot speak.

## Deliberate boundaries of 1.0

- **Single active run.** The product is sequential by design; one run executes at
  a time. Changing that requires an explicit decision record.
- **No background AI.** Reminders and scheduling never start a model.
- **No paid API key required.** Execution is via the installed Codex CLI and a
  local Ollama; there is no hosted-key path.
- **Backend recovery from a startup database failure requires a restart.**
  `PersistenceService` holds a single `StateRepository` result for the process
  lifetime, so a database that fails to open or validate at startup is reported
  by a bounded recovery screen rather than retried in place. Restarting recovers,
  with no data loss and no security consequence. Examined and deliberately not
  changed under TASK-0029 and TASK-0030: it is a persistence-subsystem redesign,
  not an integration defect.
- **The persistent KDE screen-cast / remote-desktop grant is owned by KDE.** The
  application cannot revoke it; it is revoked in System Settings. Both the purge
  path and `--print-data-paths` say so explicitly.

## Usability notes

- While a trusted confirmation dialog is waiting, the window shows a full-screen
  "Loading application data" with no indication that a system dialog is pending,
  and the dialog does not raise itself above the application window. The
  application is not hung, but it can look that way.
- `namcap` reports the packaged `listener.py` as referencing uninstalled Python
  modules (`vosk.*`). That is correct and intended: the offline voice runtime is
  provisioned by `voice-runtime/setup.sh` on demand, not as a package dependency.
