# Current State

> **Classification: Current static and fresh non-live evidence.** This snapshot
> was refreshed for TASK-0008 on 2026-08-23 from starting commit
> <code>4e06935bc9b4a7350e5ebca9970527f2f55cf2bd</code>
> (<code>task7</code>) on branch <code>main</code>. At the TASK-0008 preflight,
> checked-out <code>main</code> and <code>origin/main</code> both resolved to that
> commit, with zero ahead/behind and a clean working tree. Its actual scope
> matched the retained TASK-0007 implementation evidence. Reverify later
> implementation facts when they may have drifted.

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
restart reconciliation. TASK-0006 adds an explicit provider registry, a common
runtime contract, exact active-provider/model resolution, and truthful renderer
availability. TASK-0007 adds capability-gated Codex compatibility checks,
explicit command isolation, Linux process-lifecycle containment, bounded JSONL
evidence parsing, and deterministic descendant cleanup. TASK-0008 adds
contract-correct Ollama model inspection, bounded cancellable HTTP transport,
per-model readiness, and descriptor-confined conflict-safe workspace tools.
Fresh deterministic checks establish the non-live verification baseline
described below; they do not establish live runtime readiness.

TASK-0008 did **not** run a live Codex task, Ollama, or another model/provider.
It also did not capture microphone input; import or start the Python listener;
authorize a KDE/XDG portal; execute install/remove scripts; build a desktop
package; or perform a desktop/system-control action. Its Ollama tests use
isolated numeric-loopback fake servers and temporary workspaces; TASK-0007's
Codex subprocess tests use an isolated fake CLI. Earlier audit observations
remain historical unless this document identifies a fresh result.

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
| <code>src/providerRegistry.ts</code> | Typed renderer projection of backend provider status, catalog bindings, and fail-closed model eligibility |
| <code>src/runCoordinator.ts</code> | Typed renderer projection of authoritative run snapshots/events, stale-event rejection, and global stop state |
| <code>src/persistence.ts</code> | Typed desktop bootstrap, one-time legacy cleanup, serialized writes, backup import, and reset adapter |
| <code>src/App.css</code> | Main application styling and responsive behavior |
| <code>src/voiceCommand.ts</code> | Renderer voice-command interpretation |
| <code>src-tauri/src/lib.rs</code> | Tauri command/startup composition, provider tool-loop orchestration, workspace evidence, native desktop control, and voice process management |
| <code>src-tauri/src/provider_runtime.rs</code> | Provider-neutral identity, capability, request, event, cancellation, result, error, adapter, registry, and fake-test contracts |
| <code>src-tauri/src/codex_runtime.rs</code> | Linux Codex compatibility probing, isolated command construction, bounded JSONL protocol handling, lifecycle containment, cancellation, timeout, and evidence capture |
| <code>src-tauri/src/ollama_runtime.rs</code> | Fixed-loopback Ollama discovery, per-model metadata, bounded async HTTP, task-deadline cancellation, and chat transport |
| <code>src-tauri/src/workspace_tools.rs</code> | Linux descriptor-confined listing, ranged reads, hashes, create-only writes, preconditioned patches, and atomic conflict handling |
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

On Linux, the Rust backend resolves canonical Codex and Bubblewrap executables,
captures the Codex executable identity, and performs bounded compatibility
probes for versions, required flags, the disableable <code>multi_agent</code>
feature, and login status. It sanitizes readiness messages and revalidates the
executable identity immediately before a run. Missing or changed executables,
unsupported capabilities, unavailable containment, and non-Linux platforms
fail closed.

A run supplies the private prompt on standard input rather than the process
argument list. It uses ephemeral, JSONL, ignored user-config/rules, strict
configuration, no-MCP, no-plugin/app/hook, no-multi-agent, approval-never, and
backend-derived <code>read-only</code> or <code>workspace-write</code> settings.
Shell network remains disabled; hosted web search is enabled only by the
approved run capability. A requested file capability of <code>none</code> fails
because the inspected CLI cannot enforce it. Write/full file levels collapse
to workspace-write, and safe/user terminal levels collapse to the Codex
sandbox; administrator terminal access remains denied by backend policy.

The outer Bubblewrap process unshares user and PID namespaces, drops
capabilities, binds a private <code>/proc</code>, and dies with its parent. This
is process-lifecycle containment, not a second filesystem policy: it bind-mounts
the host tree, while the inner Codex sandbox remains the filesystem authority.
The backend uses nonblocking pipes and polling, sends termination to the process
group, escalates to kill after a bounded grace period, and requires namespace
cleanup before reporting cancellation, timeout, event-sink failure, or success.
Normal and session-detached descendants are covered by fake-process tests.

Incremental JSONL parsing accepts only completed agent-message output as the
final response, records the response/thread ID and usage, tolerates unknown
events, and emits only curated progress. Prompt input is limited to 256 KiB,
individual JSON lines to 64 KiB, stdout to 1 MiB, stderr to 512 KiB, and curated
progress to 64 events. Malformed/incomplete protocol, nonzero exit, output
overflow, compatibility failure, and cleanup failure use distinct stable error
classes with bounded evidence.

### Ollama

The Rust backend displays <code>http://localhost:11434</code> but connects only
to the fixed numeric loopback endpoint <code>127.0.0.1:11434</code>. Its async
HTTP client disables proxies, redirects, retries, referers, and connection
reuse; uses HTTP/1.1; decodes supported compression; enforces 2 MiB serialized
request and decoded-response bounds; checks declared lengths and JSON content;
and aborts and awaits the active request task on cancellation or the one task
deadline. No request worker thread is left detached.

Discovery obtains installed names from <code>/api/tags</code>, then inspects
bounded parallel <code>/api/show</code> responses for normalized capabilities
and architecture-specific context length. A failed show response leaves that
one model installed but explicitly unavailable with a reason. Coding runs
repeat exact tags/show resolution, require the selected installed model to be
tool-capable, use its bounded context metadata, and stop after at most 16 tool
turns.

On Linux, one opened workspace-root descriptor anchors tool access. Existing
paths use <code>openat2</code> beneath/no-symlink/no-magic-link resolution with a
component-by-component <code>openat</code> fallback when the syscall is absent.
Absolute paths, parent traversal, symlinks, non-directories, and
<code>.git</code> components fail closed. Listings are stable and paginated;
reads return bounded UTF-8 byte ranges plus the full-file SHA-256; new files and
directories are create-only; and ordered byte-range patches require the exact
current hash. Same-directory temporary files are synchronized and committed
with no-replace or exchange rename semantics. An exchange rechecks the
displaced inode and content and rolls back a conflict instead of silently
overwriting newer data. Ollama receives no terminal, web, delete, clipboard,
or system-control tool. Descriptor-confined tools currently fail closed on
non-Linux platforms.

### Registry, identity, and availability

The executable registry contains exactly two runtime identities:
<code>codex</code> and <code>ollama</code>. OpenAI catalog entries bind to the
Codex adapter and Ollama catalog entries bind to the Ollama adapter. Anthropic,
Google, and Custom remain valid persisted catalog labels, but their bindings
explicitly expose no executable adapter and the UI marks them unavailable.

The persisted <code>activeAiProvider</code> is an authoritative hard gate. The
backend resolves exactly one catalog row by model name, rejects missing or
duplicate identities, rejects unsupported provider labels, and rejects a model
whose adapter does not match the active provider. It does not silently switch
providers or fall back. The policy fingerprint now includes catalog model ID,
catalog provider, runtime provider, and active provider under
<code>policy-v3</code>; an older approval therefore fails exact matching rather
than authorizing a changed provider decision.

<code>provider_registry_status</code> returns common provider descriptors,
capabilities, readiness, versions, discovered runtime models with per-model
availability/reasons, and all catalog bindings. The renderer uses that one snapshot for the provider selector,
model catalog, assignments, defaults, automatic routing, and senior-review
selection. Codex catalog models are eligible only when Codex reports ready and
remain subject to CLI model validation at run start. Ollama models additionally
must be discovered locally, have readable show metadata, and report tool
support. Existing unavailable catalog entries and assignments are preserved
rather than rewritten.

Both adapters implement the same typed request, progress, cancellation,
result, evidence, and error contract. Orchestration dispatches through the
registry by the resolved runtime ID, stores canonical runtime IDs on new ledger
rows, and classifies terminal failures from typed error codes rather than
provider-specific message text. Historical ledger rows are retained unchanged.

No live provider connectivity, authentication, model execution, or model output
was checked in TASK-0008.

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
  and timeout from current state, then applies exact catalog/runtime/active
  provider resolution before dispatch. An exact approval is reserved at
  admission, consumed once only after the provider startup boundary succeeds,
  and released when cancellation or failure occurs before dispatch. Once
  dispatch may have occurred, recovery never restores that approval for replay.
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
Codex prompts/JSON lines/stdout/stderr are limited to 256 KiB/64 KiB/1 MiB/
512 KiB before the common ledger bounds apply; Ollama response and conversation
payloads are limited to 2 MiB each; summaries to 128 KiB; errors to 64 KiB;
diffs to 120,000 characters and 512 KiB; snapshots to 20,000 files or five
seconds; and changed-file evidence to 250 paths and 256 KiB. The ledger keeps
original counts/sizes and explicit truncation flags, which the renderer
surfaces instead of implying complete evidence.

Ollama workspace listings expose at most 200 entries per page and reads expose
at most 64 KiB per range. Full-file hashing and patches are limited to 8 MiB,
new-file content and aggregate replacement text to 512 KiB, and a patch to 64
ordered non-overlapping edits. Every partial list/read result includes an
explicit continuation cursor/offset and truncation flag.

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
- rejects missing, ambiguous, unsupported, inactive, or unavailable provider
  and model identities without fallback;
- admits at most one execute or review attempt system-wide and protects
  backend-owned task lifecycle fields from renderer overwrites;
- derives maximum capabilities and approval modes from backend state;
- rejects paused, missing, wrong-task, and ineligible review agents;
- rejects administrator terminal access and forces review runs to be
  read-only with no elevated authorization;
- blocks task text containing a bounded list of privileged, package, power,
  mount, permission, and system-control command patterns;
- resolves the selected workspace and constrains Codex with an explicit Codex
  sandbox plus Linux process-lifecycle containment;
- anchors Ollama tools to a selected-workspace descriptor, refuses symlink and
  <code>.git</code> traversal, and requires hash-preconditioned atomic edits.

These controls include backend-issued exact approvals with native resolution,
expiry, policy/workspace invalidation, and atomic one-use consumption. Imported
or renderer-origin records cannot authorize actions. Heuristic task-text
classification and the limits of the current Codex CLI sandbox projection
remain prototype limits.
[SECURITY_MODEL.md](SECURITY_MODEL.md) owns the full boundary and gap list.

Production CSP permits local application resources and Tauri IPC only, blocks
objects/base/form/frame embedding, and excludes <code>unsafe-eval</code>. Tauri
prototype freezing is enabled. The main-window capability grants only core
event listen/unlisten APIs; a separate development CSP adds the Vite localhost
and WebSocket endpoints.

## Verification inventory

Five Vitest files contain 33 deterministic frontend tests for renderer, voice,
legacy migration, serialization, revision, fail-closed writer behavior,
authoritative run projection, provider binding, readiness, and model
eligibility.

The Rust library contains 87 passing tests. They add Codex compatibility,
command-isolation, bounded protocol, fake-process descendant cleanup, provider
registry, fake-adapter dispatch, exact identity, typed failure, run-state,
concurrent admission, idempotency, approval-boundary, cancellation, timeout,
crash/restart, stale completion/event, truncation, and retention coverage to
the earlier provider, workspace, run-safety, voice, state-validation,
persistence, authorization, corruption, concurrency, and rollback tests.
Sixteen TASK-0008 tests cover fake-server discovery/transport/cancellation,
large and conflicting workspace edits, path escape, and bounded tool turns;
they do not contact a live provider.

The repository-root entry points are:

- <code>npm run verify:fast</code> — Vitest, TypeScript, rustfmt, and locked
  offline Rust tests;
- <code>npm run verify:full</code> — the fast route plus the Vite build,
  Clippy, shell/Python/strict-JSON syntax checks, npm/Cargo dependency trees,
  and production plus full npm audits.

TASK-0008 focused checks passed on 2026-08-23: 16 task-specific Rust tests, 6
provider-registry frontend tests, TypeScript, rustfmt, and Clippy with warnings
denied. The complete <code>npm run verify:fast</code> and
<code>npm run verify:full</code> routes passed with 33 frontend tests, 87
locked/offline Rust tests, a 37-module production build, Clippy with warnings
denied, shell/Python/JSON checks, dependency trees, and both npm audits reporting
zero vulnerabilities. Exact task evidence is recorded in
[planning/TASK_STATUS.md](planning/TASK_STATUS.md).

<code>cargo-audit</code> is not installed in the inspected environment. The
full route therefore reports the Rust advisory result as **indeterminate** and
does not represent the skip as a pass. Mandatory installed/CI security tooling
belongs to TASK-0019.

## Known gaps and roadmap ownership

| Gap | Owning task |
| --- | --- |
| Mandatory installed/CI Rust advisory tooling | TASK-0019 |
| Live Codex compatibility, authentication, model, and packaged-platform acceptance | TASK-0020 |
| Live Ollama connectivity, installed-model behavior, cancellation, and packaged-platform acceptance | TASK-0020 |
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
