# Current State

> **Classification: Current static and fresh non-live evidence.** This snapshot
> was refreshed for TASK-0005 on 2026-08-23 from starting commit
> <code>0d7ade46ee7407d5feb0f43e3d52b6fe56abcba7</code> on branch
> <code>main</code>. TASK-0005 changes remain in the working tree pending user
> review and Git closure. Reverify details that may drift before relying on
> them in a later task.

This document owns statements about what is implemented now. Planned behavior
belongs in [ARCHITECTURE.md](ARCHITECTURE.md) and
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Evidence boundary

TASK-0001 established the repository authority baseline, TASK-0002 added the
reproducible verification routes, and TASK-0003 added backend persistence.
TASK-0004 adds schema-v2 authoritative approvals, unified action policy,
narrowed privileged IPC, protected security-setting changes, and WebView
hardening. TASK-0005 adds schema-v3 authoritative single-run coordination,
task/run lifecycle projection, a durable bounded ledger, cancellation, and
restart reconciliation. Fresh deterministic checks establish the non-live
verification baseline described below; they do not establish live runtime
readiness.

TASK-0005 did **not** run Codex, Ollama, or another model/provider; capture
microphone input; import or start the Python listener; authorize a KDE/XDG
portal; execute install/remove scripts; build a desktop package; or perform a
desktop/system-control action. Earlier audit observations remain historical
unless this document identifies a fresh TASK-0002 result.

## Product and release identity

| Fact | Current evidence |
| --- | --- |
| Product | AI Agent Control Center |
| Version | <code>0.5.1</code> in <code>package.json</code> and <code>src-tauri/tauri.conf.json</code> |
| Release state | Development/pre-production prototype |
| Primary platform direction | Arch Linux, KDE Plasma, and Wayland |
| Frontend | React 19, TypeScript, and Vite |
| Desktop backend | Tauri 2 and Rust |
| Persistence dependency | <code>rusqlite 0.32.1</code> using the platform SQLite library |
| AI execution paths | Installed Codex CLI and local Ollama |
| Mandatory paid API key | None in the current Codex/Ollama execution design |
| Production gate | TASK-0020, not yet reached |

## Repository shape

| Path | Current responsibility |
| --- | --- |
| <code>src/App.tsx</code> | Main renderer UI, browser-preview compatibility, routing/review presentation, approval display, and typed intent IPC callers |
| <code>src/applicationState.ts</code> / <code>src/application-state-seed.json</code> | Shared renderer state types and the canonical fresh-state seed |
| <code>src/runCoordinator.ts</code> | Typed renderer projection of authoritative run snapshots/events, stale-event rejection, and global stop state |
| <code>src/persistence.ts</code> | Typed desktop bootstrap, one-time legacy cleanup, serialized writes, backup import, and reset adapter |
| <code>src/App.css</code> | Main application styling and responsive behavior |
| <code>src/voiceCommand.ts</code> | Renderer voice-command interpretation |
| <code>src-tauri/src/lib.rs</code> | Tauri command/startup composition, provider execution, workspace operations, native desktop control, and voice process management |
| <code>src-tauri/src/app_state.rs</code> | Backend application-state types, validation, canonical seed loading, and legacy normalization |
| <code>src-tauri/src/policy.rs</code> / <code>src-tauri/src/authorization.rs</code> | Normalized action intents, capability evaluation, native confirmation, and approval IPC contracts |
| <code>src-tauri/src/run_coordinator.rs</code> | Run states, legal transitions, ledger projections, and explicit evidence/retention bounds |
| <code>src-tauri/src/persistence.rs</code> / <code>src-tauri/migrations/</code> | SQLite repository/service, schema migration, crash-safe transactions, and persistence tests |
| <code>src-tauri/src/main.rs</code> | Desktop entry point |
| <code>src-tauri/tauri.conf.json</code> | Tauri window, security, bundle, and resource configuration |
| <code>voice-runtime/</code> | Python offline listener plus setup scripts |
| <code>install-kde.sh</code> / <code>uninstall-kde.sh</code> | KDE-oriented local install and removal scripts |

The implementation remains concentrated, particularly in
<code>src/App.tsx</code> and <code>src-tauri/src/lib.rs</code>. TASK-0013 retains
ownership of frontend modularization; these file boundaries are observations,
not architectural requirements.

## Renderer behavior

The current renderer exposes nine pages:

1. Dashboard
2. Agents
3. Voice Control
4. Tasks
5. Approvals
6. Reminders
7. Activity
8. Models
9. Settings

It contains renderer-side logic for agents, tasks, workspaces, capabilities,
approval policy, routing, review, reminders, activity, models, preferences,
and retention. Shared persisted types now live in
<code>src/applicationState.ts</code>. It can export and import a version 2 JSON
backup through the UI; TASK-0014 still owns the strict long-term backup format.

### Persistence

In the desktop runtime, core product state and the run ledger are stored in
<code>application-state.sqlite3</code> below Tauri's operating-system-provided
application data directory. Schema version 3 stores:

- agents and their nested tasks, activity, memory, roles, and policies;
- approval requests;
- models;
- reminders;
- application preferences and workspace definitions;
- task/activity retention preferences and routing/review preferences;
- immutable terminal run attempts, bounded progress/evidence, approval
  reservations, and coordinator metadata.

The backend validates aggregate size, counts, identifiers, enums, numeric
ranges, text bounds, and selected relationships before an atomic replacement.
SQLite foreign keys, rollback-journal mode, full synchronous writes, integrity
checks, transactional aggregate reads, a migration ledger,
<code>PRAGMA user_version = 3</code>, and compare-and-swap revisions protect the
repository boundary. The database and its parent directory are restricted to
the current user on Unix.

Migration 0002 rebuilds approval storage with backend authority, normalized
intent JSON, exact intent/policy/workspace fingerprints, and authoritative
timestamps. Existing schema-v1 and imported pending/approved rows become
expired, non-authoritative history. Generic whole-state saves do not replace
approval tables. A backend-issued approval must be pending, current, exact,
natively confirmed, and unconsumed; consumption is atomic and one-use.
Issuance fails closed at the state contract's 10,000-record approval-history
limit; TASK-0014 retains ownership of automated retention and deletion policy.

Migration 0003 adds run attempts, progress events, approval reservations, and
single-row coordinator metadata. Admission uses an immediate SQLite
transaction and a unique active-attempt reference, so concurrent connections
cannot both acquire the system-wide run slot. A separate coordinator revision
orders snapshots/events without creating false application-state revision
conflicts. Terminal attempt rows are immutable by database trigger. The
ledger retains at most 1,000 attempts and 256 MiB of counted evidence, prunes
oldest terminal attempts, and exposes prune/truncation metadata.

Privilege-increasing workspace-root, capability, approval-policy, review-role,
safety-mode, approval-lifetime, and microphone changes require a trusted native
dialog that names the exact elevation and are rechecked in the save
transaction. Reductions can commit directly. The workspace path editor is
picker-only in the desktop UI.

On first desktop startup, the renderer supplies the seven legacy
<code>localStorage</code> values as untrusted strings. The backend parses and
validates the complete candidate before one transaction commits. Pending or
approved legacy approvals become expired, remain marked non-authoritative,
and cannot inherit authority from migration. Legacy keys are removed only
after commit, and cleanup acknowledgement is restart-safe. Malformed legacy
data remains available for recovery and does not partially initialize state.

Desktop startup and saves fail closed on database errors; they do not fall
back to browser storage. A browser-only preview continues to use
<code>localStorage</code> and is explicitly non-authoritative.

### Default hierarchy

The default data contains eleven agents and preserves the intended reporting
shape:

| Level | Agent | Reports to |
| --- | --- | --- |
| Supervisor | Supervisor | — |
| Team Leader | Development Team Leader | Supervisor |
| Senior Agent | Debugging Agent | Development Team Leader |
| Senior Agent | Research and Web Senior | Development Team Leader |
| Senior Agent | Finance Senior | Development Team Leader |
| Senior Agent | Operations Senior | Development Team Leader |
| Specialist | Coding Agent | Debugging Agent |
| Specialist | Browser Agent | Research and Web Senior |
| Specialist | Financial Agent | Finance Senior |
| Specialist | PC Control Agent | Operations Senior |
| Specialist | Event and Reminder Agent | Operations Senior |

The hierarchy and role names exist as renderer data. They are not yet a
backend-authoritative dynamic agent registry; TASK-0009 owns that outcome.

## Provider behavior

### Codex

The Rust backend inspects the installed Codex CLI and login status. A run uses
<code>codex exec --ephemeral</code>, selects <code>read-only</code> or
<code>workspace-write</code> from backend-derived task/capability policy, sets the working
directory to the selected workspace, streams progress, supports cancellation
and timeouts, and captures changed-file/diff evidence.

### Ollama

The Rust backend talks to a local Ollama endpoint, discovers installed models,
requires tool capability for coding runs, supplies bounded workspace tools,
limits tool turns, supports cancellation and timeouts, and captures workspace
change evidence. Its tool path checks reject absolute and parent-traversal
paths and prevent access to the selected workspace's <code>.git</code>
directory.

### Model labels versus integrations

The persisted model catalog can label models as OpenAI, Anthropic,
Google, Ollama, or Custom. That catalog is not evidence of five provider
integrations. The backend derives the selected agent's model/provider and
selects Ollama only when that stored provider is <code>Ollama</code>; every
other label currently uses the Codex execution
path. TASK-0006 owns a truthful provider registry and runtime contract.

No live provider connectivity or model output was checked in TASK-0005.

## Current run, routing, and review behavior

- The renderer still chooses routing and reviewer selection, but task/run
  lifecycle, progress, stop state, and terminal outcomes are projections of
  authoritative backend snapshots and events.
- The renderer sends an <code>AgentRunRequest</code> containing only a run ID,
  agent ID, task-owner ID, task ID, and run mode. Unknown legacy authorization
  or policy fields are rejected.
- The backend admits execute and review attempts through one immediate
  transaction. There is no queue: the first valid attempt acquires the one
  system-wide slot and another intent receives a deterministic busy result.
  Reuse of a bounded request ID is idempotent only when its normalized intent
  is identical.
- Legal active states are <code>admitted</code>, <code>starting</code>,
  <code>dispatching</code>, <code>running</code>, and
  <code>cancel_requested</code>. Terminal states are
  <code>succeeded</code>, <code>cancelled</code>, <code>timed_out</code>,
  <code>startup_failed</code>, <code>failed</code>, and
  <code>interrupted</code>.
- The backend derives task, workspace, model/provider, capabilities, prompt,
  and timeout from current state. An exact approval is reserved at admission,
  consumed once only after the provider startup boundary succeeds, and
  released when cancellation or failure occurs before dispatch. Once dispatch
  may have occurred, recovery never restores that approval for replay.
- The backend keeps only the live cancellation handle in memory. Admission,
  cancellation requests, task projections, events, outcomes, usage, changed
  paths, diffs, errors, and recovery disposition are durable in SQLite.
- Review requests are backend-checked for read-only files, no terminal access,
  no elevated scopes, and no destructive approval.
- Startup reconciliation marks pre-dispatch attempts interrupted and safe to
  retry; dispatching/running/cancel-requested attempts become interrupted and
  require manual review. Legacy tasks found in a running/reviewing state get a
  synthetic interrupted ledger record instead of being silently reset.
- Global run/stop UI remains visible across navigation, stale or cross-attempt
  events are ignored, and all execute/review controls observe the same active
  attempt regardless of how the task was created.

Progress is limited to 256 events, 8 KiB per message, and 512 KiB per attempt.
Codex stdout/stderr capture is limited to 1 MiB/512 KiB; Ollama response and
conversation payloads to 2 MiB each; summaries to 128 KiB; errors to 64 KiB;
diffs to 120,000 characters and 512 KiB; snapshots to 20,000 files or five
seconds; and changed-file evidence to 250 paths and 256 KiB. The ledger keeps
original counts/sizes and explicit truncation flags, which the renderer
surfaces instead of implying complete evidence.

## Native desktop and voice behavior

The Tauri invoke surface includes workspace selection/opening, application and
window control, keyboard/pointer/text actions, desktop-control status,
Codex/Ollama execution and cancellation, and voice-runtime setup/listener
commands. The Python voice runtime and its setup scripts are bundled as a
resource.

Every current privileged workspace-open, application/window,
keyboard/clipboard, pointer/text, voice-install, microphone-start, and portal
command constructs a typed backend intent and consumes authorization before
its first side effect. Provider runs reserve authorization during atomic
admission and consume it at the successful startup boundary described above.
Voice listener configuration is derived from persisted backend preferences
rather than a renderer-supplied config IPC. Safe stop and status commands
remain directly available.

These paths exist in source, but TASK-0005 did not exercise them. TASK-0015
retains structured voice-intent behavior and TASK-0016 retains offline voice
and KDE/XDG live integration acceptance.

## Current safety enforcement

The backend currently:

- validates typed action intents and rejects unknown run IPC fields;
- admits at most one execute or review attempt system-wide and protects
  backend-owned task lifecycle fields from renderer overwrites;
- derives maximum capabilities and approval modes from backend state;
- rejects paused, missing, wrong-task, and ineligible review agents;
- rejects administrator terminal access and forces review runs to be
  read-only with no elevated authorization;
- blocks task text containing a bounded list of privileged, package, power,
  mount, permission, and system-control command patterns;
- resolves the selected workspace and constrains Codex with a Codex sandbox;
- constrains Ollama tools to paths below the selected workspace.

These controls include backend-issued exact approvals with native resolution,
expiry, policy/workspace invalidation, and atomic one-use consumption. Imported
or renderer-origin records cannot authorize actions. Heuristic task-text
classification and current provider adapters remain prototype limits.
[SECURITY_MODEL.md](SECURITY_MODEL.md) owns the full boundary and gap list.

Production CSP permits local application resources and Tauri IPC only, blocks
objects/base/form/frame embedding, and excludes <code>unsafe-eval</code>. Tauri
prototype freezing is enabled. The main-window capability grants only core
event listen/unlisten APIs; a separate development CSP adds the Vite localhost
and WebSocket endpoints.

## Verification inventory

Four Vitest files contain 27 deterministic frontend tests for renderer, voice,
legacy migration, serialization, revision, fail-closed writer behavior, and
authoritative run projection.

The Rust library contains 55 passing tests. They add run-state, concurrent
admission, idempotency, approval-boundary, cancellation, timeout,
crash/restart, stale completion/event, truncation, and retention coverage to
the earlier provider, workspace, run-safety, voice, state-validation,
persistence, authorization, corruption, concurrency, and rollback tests.
The Ollama connection test uses an isolated loopback server; it does not contact
a live provider.

The repository-root entry points are:

- <code>npm run verify:fast</code> — Vitest, TypeScript, rustfmt, and locked
  offline Rust tests;
- <code>npm run verify:full</code> — the fast route plus the Vite build,
  Clippy, shell/Python/strict-JSON syntax checks, npm/Cargo dependency trees,
  and production plus full npm audits.

TASK-0005 focused checks passed on 2026-08-23: 14 task-specific Rust tests,
all 27 frontend tests, TypeScript, rustfmt, and Clippy. The complete
<code>npm run verify:full</code> route passed with 27 frontend tests, 55 Rust
tests, a 36-module production build, Clippy with warnings denied,
shell/Python/JSON checks, dependency trees, and both npm audits reporting zero
vulnerabilities. Dependency-tree output exceeded the captured console view,
but the command completed with exit status 0.

<code>cargo-audit</code> is not installed in the inspected environment. The
full route therefore reports the Rust advisory result as **indeterminate** and
does not represent the skip as a pass. Mandatory installed/CI security tooling
belongs to TASK-0019.

## Known gaps and roadmap ownership

| Gap | Owning task |
| --- | --- |
| Mandatory installed/CI Rust advisory tooling | TASK-0019 |
| Truthful provider registry and hardened Codex/Ollama paths | TASK-0006–TASK-0008 |
| Dynamic hierarchy, routing, review, and workspace evidence | TASK-0009–TASK-0012 |
| Frontend modularity, strict backup, and full data lifecycle | TASK-0013–TASK-0014 |
| Voice/system policy and KDE/XDG integration | TASK-0015–TASK-0016 |
| Bounded specialist capabilities and management handoffs | TASK-0017–TASK-0018 |
| Packaging, CI, live acceptance, and production gate | TASK-0019–TASK-0020 |

See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for exact sequencing.

## Historical reconciliation

The original audit predates Git initialization. The current repository is a Git
checkout whose initial public baseline is commit <code>9805c71</code>; the
audit's no-Git observation is historical, not current. Current inspection also
corrected stale references such as <code>uninstall.sh</code> (the repository
contains <code>uninstall-kde.sh</code>) and older page lists that omitted Voice
Control and Reminders. Current source evidence wins if those historical
artifacts conflict.
