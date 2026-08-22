# Security Model

> **Current status:** version 0.5.1 is a development prototype, not a hardened
> or production-ready control plane. This document distinguishes checked-in
> controls from planned security invariants. TASK-0003 establishes backend
> persistence but does not implement TASK-0004 approval authority or IPC/CSP
> hardening.

## Security objective

The application must let a user delegate bounded work without allowing model
output, renderer state, imported data, or an ambiguous approval to expand
authority. The local backend must remain the decision point for durable state,
authorization, workspace scope, provider execution, and native system actions.

## Assets

- user-selected workspace files and Git state;
- task text, agent memory, model output, review feedback, and reminders;
- agent definitions, capabilities, policies, approvals, and run records;
- local provider authentication or runtime state;
- clipboard, microphone, keyboard, pointer, applications, windows, and desktop;
- local backups, logs, settings, and installation artifacts.

Credentials, tokens, user content, and raw provider data are sensitive even
when stored locally.

## Trust boundaries

| Boundary | Trust treatment |
| --- | --- |
| User interaction | Intent source, but still subject to explicit scope and confirmation |
| React renderer/WebView | Untrusted for authorization; may be compromised or hold stale/tampered state |
| Imported backup and <code>localStorage</code> | Untrusted serialized input requiring validation and migration |
| Tauri IPC | Untrusted request boundary; backend must authenticate semantics, not just types |
| Rust backend | Planned authoritative policy, state, and execution boundary |
| Codex/Ollama output | Untrusted content and action proposals |
| Selected workspace | Sensitive bounded filesystem root |
| External/local provider process | Separate process/service with its own failure and trust model |
| Voice transcript | Untrusted and potentially misrecognized intent |
| KDE/Wayland/XDG interfaces | Privileged integration boundary requiring native consent and scoping |

## Current controls

Static inspection found these implemented controls:

- backend validation of run mode, file-access, terminal-access, and scope names;
- rejection of administrator terminal access;
- a required approval identifier when the request contains authorized scopes;
- read-only/no-terminal/no-elevation constraints for review runs;
- rejection of a bounded list of privileged, package, power, permission, mount,
  and system-control patterns in task text;
- selected-workspace resolution;
- Codex <code>read-only</code> or <code>workspace-write</code> sandbox
  selection;
- Ollama workspace-tool containment, including absolute path, parent traversal,
  and <code>.git</code> rejection;
- per-run cancellation flags and bounded timeouts;
- changed-file and Git diff capture where available.
- typed, bounded backend validation for persisted application state;
- schema-versioned SQLite persistence with foreign keys, integrity checks,
  explicit migration evidence, atomic writes, and stale-revision rejection;
- fail-closed desktop startup/save behavior when persistence is unavailable;
- Unix application-data directory/file modes restricted to the current user;
- one-time legacy migration that commits before renderer cleanup and refuses
  malformed input without partial state;
- downgrade of pending/approved legacy approvals to expired records whose
  database rows are constrained to <code>authoritative = 0</code>.

These controls reduce risk but do not establish the target authorization model.

## Known current gaps

### Approval authority

Approval records are now durable backend data, but their lifecycle decisions
remain renderer-managed and every schema-v1 row is explicitly
non-authoritative. The renderer sends <code>approvalId</code>,
<code>authorizedScopes</code>, and <code>destructiveActionsApproved</code> to the
backend. The execution boundary validates their shape and some combinations,
but does not consult an authoritative approval record to prove:

- who or what issued the approval;
- exact task, agent, workspace, action, and scope matching;
- freshness and expiry;
- atomic one-use consumption;
- denial, cancellation, or replay state.

Therefore, the current approvals UI is not a complete security boundary.
TASK-0004 owns the authoritative approval/policy contract.

### State integrity and recovery

Desktop core domain state now uses a backend-owned SQLite transaction boundary,
schema/migration ledger, integrity check, and compare-and-swap revision. The
renderer cannot fall back to WebView storage after desktop persistence starts
or fails. Remaining lifecycle gaps include strict backup/export contracts,
continuous bounded retention, recovery UX, and integrated live upgrade
evidence. TASK-0014 owns those controls. The browser preview remains
non-authoritative.

### IPC and web content

<code>src-tauri/tauri.conf.json</code> currently sets Content Security Policy
to <code>null</code>. The invoke surface exposes provider, workspace, voice,
application, window, keyboard, text, pointer, and desktop-control commands.
Each backend command must enforce its own policy; renderer visibility is
insufficient. TASK-0004 owns CSP and IPC hardening, while TASK-0015 owns a
unified voice/system action policy gateway.

### Heuristic task-text checks

Substring checks for command terms can be bypassed, can misclassify benign
text, and do not authorize the eventual concrete tool invocation. They are a
prototype guardrail, not a parser or policy engine. Later enforcement must
authorize normalized operations and arguments at the point of use.

### Provider and run truth

Renderer model labels do not map to five distinct provider implementations.
Backend dispatch treats only Ollama specially and sends all other labels to
Codex. Active-run state is partly renderer-owned and the backend registry is
in-memory, without a durable system-wide lease or recovery ledger. TASK-0005
through TASK-0008 own these boundaries.

### Verification coverage

The checked-in non-live suite now includes 24 frontend tests and 23 Rust tests,
including persistence validation, migration, corruption, concurrency, and
rollback cases. These checks do not establish end-to-end, packaging, upgrade,
or live acceptance. Rust advisory status remains indeterminate when
<code>cargo-audit</code> is unavailable.

## Target security invariants

The following are **planned requirements**, not current guarantees:

1. The backend is the sole authority for durable domain state and action
   authorization.
2. Every IPC payload is parsed, normalized, bounded, and validated; unknown
   fields or values fail closed where compatibility permits.
3. Capabilities grant a maximum envelope. A task, approval, prompt, agent role,
   or provider may only narrow that envelope.
4. An approval binds one user-confirmed action or run to an exact subject,
   task, workspace, normalized operation, scopes, risk, and expiry.
5. Approval consumption is atomic, durable, one-use, and replay-resistant.
6. There is at most one active AI run system-wide, enforced by the backend and
   recoverable after cancellation, crash, or restart.
7. Workspace paths are canonicalized and remain below an approved root at every
   file operation; symlink and time-of-check/time-of-use risks are addressed.
8. Provider adapters receive only the capabilities required for the run and
   cannot silently substitute a different provider contract.
9. Model output and voice transcripts never execute native actions directly.
   They produce proposed, normalized intents for the policy gateway.
10. Native system actions default to deny and require action-specific policy,
    platform evidence, and user confirmation where warranted.
11. Logs and evidence are useful for review without exposing credentials,
    private prompt contents, or unrelated user data.
12. CSP, Tauri capabilities, IPC exposure, dependency posture, packaging, and
    install/remove behavior pass their owning release gates.

## Action policy direction

| Action class | Default direction | Required boundary |
| --- | --- | --- |
| Read workspace file | Deny outside selected root | Canonical backend path check |
| Write/delete workspace file | Explicit capability; approval when policy requires | Backend policy plus workspace containment |
| Web access | Off unless capability and task policy permit | Provider/tool-specific network gate |
| Terminal command | Deny by default; never infer from prose | Normalized command policy and bounded process execution |
| Clipboard | Deny by default | Backend action-specific grant |
| Package/power/privileged system action | Deny | Separate explicitly approved future decision, if ever supported |
| Application/window/input control | Deny by default | Voice/system policy gateway plus platform checks |
| Microphone capture | Off by default | Visible user state and portal/native consent |
| Reminder | Store/notify only | Must not trigger a model in the background |

## KDE, Wayland, and Linux mechanisms

Official platform mechanisms exist and must be assessed before inventing broad
desktop-control workarounds:

- KDE documents [KWin scripting APIs](https://develop.kde.org/docs/plasma/kwin/api/)
  and [KWin scripting](https://develop.kde.org/docs/plasma/kwin/) for bounded
  compositor/window integration.
- XDG Desktop Portal defines scoped
  [Session](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Session.html)
  and
  [RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)
  interfaces with explicit lifecycle and user-consent semantics.
- Tauri documents a configurable
  [Content Security Policy](https://v2.tauri.app/security/csp/).
- freedesktop.org defines the
  [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/)
  and
  [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/0.8/)
  for launch metadata and user-local data/config placement.

These references establish available native paths; they do not prove the
current app uses them correctly. TASK-0015, TASK-0016, and TASK-0019 own
implementation and acceptance. If a native mechanism cannot satisfy a bounded
requirement, the owning Phase A plan must document the constraint and compare
least-privilege workarounds before any code change.

## Failure and recovery direction

- Reject invalid or unauthorized input without starting a provider or native
  action.
- Persist terminal run/approval state before releasing authority.
- Make cancellation idempotent and distinguish user cancellation, timeout,
  provider failure, policy denial, and application restart.
- Never infer success from a missing process or UI state.
- Preserve approval records and workspace evidence needed for audit while
  applying explicit retention and redaction rules.
- On uncertain authorization or recovery state, fail closed and require a new
  user decision rather than replay an earlier action.

## Privacy rules

- Do not store provider credentials in ordinary renderer state or backups.
- Treat the SQLite application-state database as sensitive local user data;
  keep its path out of routine errors and preserve private filesystem modes.
- Do not include secrets, complete private prompts, workspace contents, or raw
  microphone audio in routine logs.
- Make retention and deletion behavior explicit and testable.
- Keep Ollama local by default and make any network boundary visible.
- Treat backup export as sensitive user data and validate imports before use.

## Ownership and release gate

[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) assigns each gap to a task.
Security-relevant work is not complete merely because its UI exists. TASK-0020
must validate the integrated system sequentially before documentation may call
the application production-ready.
