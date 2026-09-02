# Final acceptance matrix — version 1.0.0

Run by TASK-0030 on 2026-09-02, on the real machine: Arch Linux, KDE Plasma 6.7.4,
Wayland, single 1920x1080 output at scale 1.

Every live case ran against an **isolated scratch data home** holding a
`sqlite3 .backup` copy of the real prototype database. The real user database was
verified unchanged throughout: 430080 bytes, mtime 2026-08-30 18:38:07.

## Deterministic gates

| Criterion | Result | Evidence |
| --- | --- | --- |
| Frontend suite | **PASS** | 26 files, 97 tests |
| Rust suite | **PASS** | 313 total: 305 passed, 8 `#[ignore]`d live, 0 failed |
| Typecheck | **PASS** | `tsc --noEmit` clean |
| Build | **PASS** | 71 modules transformed; `makepkg -f` exit 0, `ai-agent-control-center 1.0.0-1` |
| Packaging validation | **PASS** | `VERIFY_STRICT=1 scripts/check-packaging.sh` → `packaging validation: passed`; version parity 1.0.0 across all five manifests; namcap on PKGBUILD and package reports only justified findings |
| Staged install / upgrade / remove / keep-data / purge | **PASS** | `scripts/staged-install-test.sh` → passed, purge idempotent |
| Namespaced pacman transaction | **PASS** | `scripts/pacman-transaction-test.sh` → passed |
| Full strict gate | **PASS** | `PATH=$HOME/.cargo/bin:$PATH VERIFY_STRICT=1 npm run verify:full` exit **0**, zero skips: 26 frontend files / 97 tests, 7 Python tests, rustfmt, 305 Rust tests, Clippy `-D warnings`, both npm audits 0 vulnerabilities, 541 crates all permissive, 89 script-gate `ok` checks, shellcheck, and `cargo deny check advisories` → `advisories ok`. **Rust advisory status: PASSED**, not indeterminate. |
| Branch CI | See RELEASE_EVIDENCE | Requires the push; the candidate's prior run was red and the cause was fixed by this task |

## Live acceptance

| Case | Result | Evidence |
| --- | --- | --- |
| S3 startup and legacy migration | **PASS** | The migrated real-data clone opens the normal shell: `user_version` 11, `state_revision` 11, `source_kind = legacy_local_storage`, 13 agent rows / 12 configured. Startup restored a voice-listener intent as a **pending approval** and started no microphone listener — deny-by-default held. |
| S4 approvals and policy | **PASS, fail-closed proven** | Approve raises a trusted `kdialog` "Confirm one-time authorization" outside the renderer, naming action, agent, risk High, scopes, expiry, and "Approve exactly one use?". The request stays Pending while it is open. Answering **Yes** → Approved. **Escape** → `NATIVE_CONFIRMATION_UNAVAILABLE`, still Pending. **No** → `NATIVE_CONFIRMATION_DENIED`, still Pending. The only production path to Approved is the Tauri `resolve_approval` command gated on the dialog's exit status, so a renderer alone can never grant. |
| S5 Codex identity and containment | **PASS** | `codex-cli 0.144.5`, installed / authenticated / compatible; GUI reports "installed, compatible, contained, and signed in with ChatGPT". |
| S5 Codex cancellation | **PASS** | Cancelled run leaves no owned process. |
| S5 Codex bounded read run | **BLOCKED (external)** | `PROVIDER_EXECUTION_FAILED / Codex reported a failed turn.` Root cause verified **outside** the application by invoking `codex` directly: `You've hit your usage limit … try again at Sep 7th, 2026 12:44 PM.` Not an application defect — the app failed closed and fabricated nothing. Re-runnable after 2026-09-07. |
| S5 Ollama discovery | **PASS** | Ollama 0.32.3, 5 inspected models; `qwen2.5-coder:7b` Ready, capabilities `[completion, insert, tools]`, context 32768. |
| S5 Ollama bounded run | **PASS** | Terminated with typed evidence in 64.7 s. |
| S5 Ollama cancellation | **PASS** | Settled 21.98 ms after the flag was set. |
| S6 XDG notification portal | **PASS** | Reminder notification delivered and withdrawn through the real portal. |
| S6 portal negotiation | **PASS** | RemoteDesktop session negotiated. |
| S6 portal grant and bounded input | **PASS** | The real KDE consent dialog (`org.freedesktop.impl.portal.desktop.kde`, "Remote Control") was raised and answered by the operator; session negotiated, bounded input injected, session released; 5.54 s. |
| S6 spoken command through the GUI pipeline | **NOT EXECUTED** | Requires a human voice, which this workflow cannot supply. The offline listener and the portal dispatch are each covered above and by the deterministic S6 matrix. |
| S7 backup export through the GUI | **PASS** | Writes a real 12,987-byte portable-backup v4 file to an operator-chosen path and reports that path back. Content carries `format: ai-agent-control-center-portable-backup`, `version: 4`, and contains no credential-shaped strings. |
| S7 reset through the GUI | **PASS** | Gated by a second trusted dialog "Confirm application reset". Applied: agents 13 → 11 defaults, approvals 7 → 0, revision 11 → 12, workspaces 1 → 0. |
| S7 backup import through the GUI | **PASS** | The backend validates and previews before applying ("Validated backup v4 … 7 approval-history records … will be cleared"), then a third trusted dialog "Confirm portable backup import" lists exact record counts and reports `Security: No protected security configuration increase was detected.` Applied; revision 11 → 12. |
| S8 live system-database install | **PASS** | `pacman -U` against the **live** system package database, exit 0. All 10 owned files present; packaged binary reports `ai-agent-control-center 1.0.0`; desktop entry and AppStream validate. |
| S8 live system-database removal | **PASS** | `pacman -R` exit 0; every owned file removed; **per-user data survived exactly as the install hook promises** — database intact with 13 agents, voice models intact; the user-local install was unaffected; PlasmaShell was not restarted. |
| S9 desktop UI and recovery | **PASS** | 1280x820 window; Dashboard, Approvals, Tasks, Models and Settings rendered and driven by keyboard alone; sliders named; provider status truthful. |
| S10 release gate itself | **PASS** | 10 deterministic scenarios over the shipped gate scripts and CI workflow. |

## Defects found and fixed by this task

| ID | Defect | Status |
| --- | --- | --- |
| CI-1 | The axe scenario inherited Vitest's implicit 5000 ms default and took 5168 ms on the pinned runner, so the deterministic gate's verdict depended on host speed. The candidate had never had a green CI run. | **Fixed** — both page-walking scenarios declare an explicit budget; new S10 scenario forbids inheriting the default. |
| D1 | Backup export saved through a `Blob` + `<a download>` click. WebKitGTK performs that download into the process's working directory; the desktop entry sets no `Path=`, so the destination depended on how the app was launched and was never reported. | **Fixed** — a backend `save_backup_file` command driven by `kdialog --getsavefilename`, reporting the real path. |
| D2 | "Import backup" was a `<label>` wrapping a `display: none` file input. A label is not focusable and the hidden input is outside the tab order, so a keyboard-only operator could never restore a backup. | **Fixed** — a real button owning the tab stop; live re-confirmed. |
| D3 | On `turn.failed` the Codex runtime discarded the event's own message and substituted a fixed string, so a usage-limit stop read exactly like a crash. | **Fixed** — the provider's reason is surfaced, bounded and stripped of control characters. |

Each fix was confirmed to **fail** against the pre-fix file before being accepted.
