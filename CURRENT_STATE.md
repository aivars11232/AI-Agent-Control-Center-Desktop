# Current State

> **Classification: Current static and fresh non-live evidence.** This snapshot
> was refreshed for TASK-0015 on 2026-08-28 from starting commit
> <code>2d19e7862d97c7f2c46080981b43c4cefc29c64b</code>
> (<code>task14</code>) on branch <code>main</code>. At the TASK-0015 preflight,
> checked-out <code>main</code> and <code>origin/main</code> both resolved to that
> commit, with zero ahead/behind and a clean working tree. Its 22-file actual
> scope matched retained TASK-0014 evidence. Reverify later implementation
> facts when they may have drifted.

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
TASK-0009 adds a backend-authoritative dynamic agent registry, schema-v4
lifecycle and template identity, validated role-derived reporting hierarchy,
logical deletion/explicit restoration, and registry-driven renderer
projections. TASK-0010 adds schema-v5 backend-authoritative task creation,
deterministic routing and evidence, workload overflow decisions, one durable
global execute queue, and queue-head admission into the existing single-run
coordinator. TASK-0011 adds schema-v6 structured review flows and attempts,
strict bound request/result protocols, exact sequential reporting-chain
reviewers, bounded revisions and review retries, trusted human decisions, and
deterministic restart recovery. TASK-0012 adds schema-v7 versioned Git/non-Git
workspace-change evidence, bounded before/after capture around every execution,
structured review binding, and authoritative task/run-ledger presentation.
TASK-0013 adds a thin renderer entry point, a stateful app controller, an
accessible app shell, feature-owned pages and styles, pure domain helpers, a
typed desktop client, native-dialog and tab semantics, keyboard-operable agent
cards, deterministic DOM accessibility checks, and narrow-screen provider
controls. It changes no backend command, schema, migration, policy, provider,
or durable-state authority.
TASK-0014 adds schema-v8 normalized lifecycle timestamps and durable bounded
maintenance evidence; strict sanitized portable backup v3 export, preview, and
atomic import; startup/periodic/mutation-triggered retention; revision-bound
backend monitoring pages; scoped local-activity deletion; truthful desktop
versus browser-preview labels; and explicit reset-versus-physical-purge
wording. Fresh deterministic checks establish these non-live behaviors; they
do not establish packaged or live runtime readiness.
TASK-0015 adds schema-v9 canonical voice intents, backend-owned agent/workspace
and exact Linux target resolution, action-specific capability/approval policy,
one-use destructive confirmation, a single renderer submission surface,
normal sequential coding-task routing, and a bounded redacted system-action
audit with idempotent restart-safe outcomes. Exact XDG desktop entries and user
directories plus exact KWin internal-window IDs replace fuzzy application,
caption, broad-process, and accidental active-window fallbacks.

TASK-0015 did **not** run a live Codex task, Ollama, or another model/provider.
It also did not capture microphone input; import or start the Python listener;
authorize a KDE/XDG portal; execute install/remove scripts; build a desktop
package; or perform a desktop/system-control action. The retained TASK-0008
Ollama tests use isolated numeric-loopback fake servers and temporary
workspaces; TASK-0007's
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
| <code>src/App.tsx</code> | Thin renderer compatibility entry and shared type re-exports |
| <code>src/app/AppController.tsx</code> / <code>src/app/AppShell.tsx</code> | State hydration, persistence/event coordination, feature composition, navigation, page focus, provider status, and global-run presentation |
| <code>src/features/</code> | Dashboard, Agents, Tasks, Approvals, Activity, Voice Control, Reminders, Models, and Settings page modules |
| <code>src/components/</code> | Native dialog, APG tabs, live status, and keyboard-action presentation primitives |
| <code>src/services/desktopClient.ts</code> / <code>src/services/authorization.ts</code> | Typed renderer command/event facade and shared approval-request presentation helpers; backend remains authoritative |
| <code>src/domain/</code> | Pure normalization, safety, model-default, and error helpers |
| <code>src/applicationState.ts</code> / <code>src/application-state-seed.json</code> | Shared renderer state types and the canonical fresh-state seed |
| <code>src/providerRegistry.ts</code> | Typed renderer projection of backend provider status, catalog bindings, and fail-closed model eligibility |
| <code>src/agentRegistry.ts</code> | Dynamic active-agent, category-group, ancestor, hierarchy-repair, stable-template, and compatible-manager projections |
| <code>src/taskOrchestration.ts</code> | Typed projection of backend queue order, positions, state, head admission, and routing evidence |
| <code>src/reviewOrchestration.ts</code> | Typed projection of backend review flows, levels, attempts, human fallback, and revision state |
| <code>src/runCoordinator.ts</code> | Typed renderer projection of authoritative run snapshots/events, stale-event rejection, and global stop state |
| <code>src/workspaceEvidence.ts</code> | Versioned renderer projection, defensive shape normalization, truthful labels, and safe final-state openability for workspace evidence |
| <code>src/dataLifecycle.ts</code> | Typed portable-backup, retention-evidence, monitoring-revision, task-page, and activity-page projections plus explicit browser-preview monitoring |
| <code>src/persistence.ts</code> | Typed desktop bootstrap, one-time legacy cleanup, serialized writes, strict backup preview/import, and reset adapter |
| <code>src/App.css</code> / <code>src/styles/</code> / feature CSS | Ordered style entry, tokens, shell/shared/workflow/evidence/responsive rules, and feature-owned Dashboard/Settings rules |
| <code>src/voiceCommand.ts</code> | Canonical renderer voice-intent interpretation with explicit active-window semantics |
| <code>src-tauri/src/lib.rs</code> | Tauri command/startup composition, provider tool-loop orchestration, unified voice/system-action gateway, workspace evidence, native adapters, and voice process management |
| <code>src-tauri/src/provider_runtime.rs</code> | Provider-neutral identity, capability, request, event, cancellation, result, error, adapter, registry, and fake-test contracts |
| <code>src-tauri/src/codex_runtime.rs</code> | Linux Codex compatibility probing, isolated command construction, bounded JSONL protocol handling, lifecycle containment, cancellation, timeout, and evidence capture |
| <code>src-tauri/src/ollama_runtime.rs</code> | Fixed-loopback Ollama discovery, per-model metadata, bounded async HTTP, task-deadline cancellation, and chat transport |
| <code>src-tauri/src/workspace_tools.rs</code> | Linux descriptor-confined listing, ranged reads, hashes, create-only writes, preconditioned patches, and atomic conflict handling |
| <code>src-tauri/src/workspace_evidence.rs</code> | Bounded descriptor-confined before/after snapshots, hardened Git inspection, change classification, detail redaction, and evidence validation |
| <code>src-tauri/src/app_state.rs</code> | Backend application-state types, validation, canonical seed loading, and legacy normalization |
| <code>src-tauri/src/agent_registry.rs</code> | Agent-registry DTOs, template catalog, role authority, legacy repair, and hierarchy validation |
| <code>src-tauri/src/task_orchestration.rs</code> | Deterministic routing eligibility/scoring, workload/overflow decisions, and queue DTOs |
| <code>src-tauri/src/review_orchestration.rs</code> | Versioned review schemas, exact reporting-chain selection, prompt/protocol validation, and review DTOs |
| <code>src-tauri/src/policy.rs</code> / <code>src-tauri/src/authorization.rs</code> | Normalized action intents, capability evaluation, native confirmation, and approval IPC contracts |
| <code>src-tauri/src/run_coordinator.rs</code> | Run states, legal transitions, ledger projections, and explicit evidence/retention bounds |
| <code>src-tauri/src/data_lifecycle.rs</code> | Strict duplicate-free bounded backup parsing, portable-state sanitization, backup/monitoring DTOs, and lifecycle constants |
| <code>src-tauri/src/system_actions.rs</code> | Closed canonical voice/system-action types, validation, exact-target authorization contract, risk classes, and redacted audit DTOs |
| <code>src-tauri/src/linux_desktop.rs</code> | Exact XDG desktop-entry/user-directory resolution and strict KWin target/action adapter |
| <code>src-tauri/src/persistence.rs</code> / <code>src-tauri/migrations/</code> | SQLite repository/service, schema migration, crash-safe transactions, and persistence tests |
| <code>src-tauri/src/main.rs</code> | Desktop entry point |
| <code>src-tauri/tauri.conf.json</code> | Tauri window, security, bundle, and resource configuration |
| <code>voice-runtime/</code> | Python offline listener plus setup scripts |
| <code>install-kde.sh</code> / <code>uninstall-kde.sh</code> | KDE-oriented local install and removal scripts |

The renderer is now split by domain, service, shell, shared component, and
feature responsibility. <code>src-tauri/src/lib.rs</code> remains a separate
backend concentration and TASK-0013 deliberately does not redesign it.

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

Feature modules contain renderer-side logic for agents, tasks, workspaces, capabilities,
approval policy, routing, review, reminders, activity, models, preferences,
and retention. The app controller composes those features and the typed desktop
client centralizes Tauri command/event calls. Shared persisted types live in
<code>src/applicationState.ts</code>. In the desktop runtime, Settings exports,
previews, and imports the backend-owned portable backup v3 format. Browser mode
retains a clearly labelled non-authoritative legacy version 2 preview path.

### Persistence

In the desktop runtime, core product state and the run ledger are stored in
<code>application-state.sqlite3</code> below Tauri's operating-system-provided
application data directory. Schema version 8 stores:

- agents, stable template identity, lifecycle/repair metadata, reporting
  structure, and their nested tasks, activity, memory, roles, and policies;
- approval requests;
- models;
- reminders;
- application preferences and workspace definitions;
- task/activity retention preferences and routing/review preferences;
- task routing inputs/evidence, executor assignment, queue state/order,
  thresholds, overflow policy, and orchestration allocators/revision;
- normalized review flows, revision rounds, structured stage attempts,
  request/result bindings, and review-orchestration revision;
- immutable terminal run attempts, bounded progress/evidence, versioned
  workspace changes on run attempts and task aggregates, approval reservations,
  and coordinator metadata.
- normalized Unix-millisecond task/activity/reminder lifecycle columns,
  retention indexes, maintenance revision/totals, and the latest 100 bounded
  maintenance-run evidence records.

The backend validates aggregate size, counts, identifiers, enums, numeric
ranges, text bounds, and selected relationships before an atomic replacement.
SQLite foreign keys, rollback-journal mode, full synchronous writes, integrity
checks, transactional aggregate reads, a migration ledger,
<code>PRAGMA user_version = 8</code>, and compare-and-swap revisions protect the
repository boundary. The database and its parent directory are restricted to
the current user on Unix.

Migration 0002 rebuilds approval storage with backend authority, normalized
intent JSON, exact intent/policy/workspace fingerprints, and authoritative
timestamps. Existing schema-v1 and imported pending/approved rows become
expired, non-authoritative history. Generic whole-state saves do not replace
approval tables. A backend-issued approval must be pending, current, exact,
natively confirmed, and unconsumed; consumption is atomic and one-use.
Issuance fails closed at the state contract's 10,000-record approval-history
limit. Schema-v8 retention prunes only resolved/consumed approval history and
protects active authority and reservations.

Migration 0003 adds run attempts, progress events, approval reservations, and
single-row coordinator metadata. Admission uses an immediate SQLite
transaction and a unique active-attempt reference, so concurrent connections
cannot both acquire the system-wide run slot. A separate coordinator revision
orders snapshots/events without creating false application-state revision
conflicts. Terminal attempt rows are immutable by database trigger. The
ledger retains at most 1,000 attempts and 256 MiB of counted evidence, prunes
oldest terminal attempts, and exposes prune/truncation metadata.

Migration 0004 adds optional unique template keys, active/unassigned/deleted
registry lifecycle, bounded repair reasons, deletion timestamps, and a
single-row monotonic ID allocator. It derives authority from role, maps the
eleven legacy default identities without depending on display names, and
quarantines invalid legacy reporting edges rather than hiding or dropping the
affected agents. Dedicated revision-checked IPC owns create, update, logical
delete, and explicit template restore. Generic renderer saves reject registry
structure changes, and policy/routing projections exclude non-active agents.

Migration 0005 adds task routing inputs and evidence, queue state and enqueue
sequence, queue-threshold/overflow preferences, indexes and validation
triggers, JavaScript-safe task/enqueue allocators, and an orchestration
revision. Dedicated compare-and-swap IPC creates and routes tasks, reroutes
queued/held tasks without changing their age, and owns hold/resume/reset queue
transitions. Generic renderer saves cannot create, remove, relocate, reroute,
or forge task queue/lifecycle/evidence state.

Migration 0006 adds normalized review flows and immutable terminal stage
attempts, binds review runs to one flow, round, stage, and request fingerprint,
and adds a separate review revision. A conservative upgrade converts ambiguous
legacy in-flight review state to <code>awaiting_human</code>; it does not infer a
provider verdict or resume dispatch. Dedicated backend commands start the exact
next stage and record natively confirmed human decisions. Generic renderer
saves preserve the normalized review ledger and cannot complete a review flow.

Migration 0007 adds checked structured workspace-evidence JSON to run attempts
and task aggregates. Terminal completion validates and writes the same bounded
record into the immutable run ledger and its backend-owned task projection in
one transaction, counts it toward ledger payload retention, and maps legacy
null rows to explicit unavailable evidence. Generic renderer saves preserve
the backend-owned task field and cannot forge or erase it.

Migration 0008 adds normalized retention timestamps, indexes, a lifecycle
revision/totals row, and bounded maintenance-run evidence. Maintenance runs at
desktop startup, every 15 minutes, after retention-setting changes, and after
imports; a bounded backlog retries after one minute. Each pass deletes at most
500 eligible rows per domain, protects active tasks/runs/reviews/approvals,
increments affected authority revisions, and records clock rollback instead
of performing age-based deletion. Task retention also governs terminal run and
review history; activity retention governs local activity, resolved/consumed
approval history, and resolved reminders. <code>never</code> disables the
age-based policy but does not remove the existing hard aggregate/ledger caps.

Migration 0009 adds a maximum-10,000 system-action audit, lifecycle retention
columns, policy-v5/intent-v3 evidence, and conservative expiry/redaction of
legacy desktop-text approval intents. Audit transitions bind one request ID to
one intent fingerprint, exact target, agent, risk class, authorization
evidence, and redacted content digest. Terminal records follow activity
retention; pending approval and dispatched records remain protected. Startup
turns an interrupted dispatched record into <code>uncertain</code> without
repeating the action.

Portable backup v3 is capped at 16 MiB and 128 JSON levels, rejects duplicate
keys, unknown fields, trailing content, unsupported versions, and future
schemas, and sanitizes active tasks/approvals/runtime evidence before export or
import. Import preview and apply use the same backend candidate, compare the
application revision, require an idle run boundary and trusted native
confirmation, clear mismatched run/review history, and commit atomically.
Provider credentials, authorization intents, run/review ledgers, portal
sessions, voice-runtime sessions, and the system-action audit are not portable
backup domains.

Dashboard, Tasks, Activity, and Settings consume backend monitoring snapshots
and pages bound to one application/task/run/review/lifecycle revision tuple.
Task and activity pages are capped at 100 rows; stale tuples return
<code>MONITORING_REVISION_CONFLICT</code>. Activity deletion changes only the
local configuration timeline; clearing it requires native confirmation and
cannot delete immutable run/review evidence. Browser projections state that
they are non-authoritative. Desktop monitoring refreshes after serialized state
commits and at a bounded one-minute interval; authoritative task/activity pages
do not substitute renderer records while an exact-tuple query is pending.

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

The eleven records are initial templates, not an immortal fixed-ID roster.
Custom agents receive monotonically allocated IDs and remain visible through
dynamic category groups and ancestor projections. Display names may change
without breaking voice lookup because default capabilities use stable template
keys. Authority is derived from role (Supervisor 4, Team Leader 3, Senior Agent
2, Specialist 1), and every active non-supervisor must report to an active
higher-authority manager. Self-parenting, cycles, dangling managers, inactive
managers, and incompatible authority are rejected; invalid legacy edges are
paused in a visible Needs assignment state. Deletion is a durable logical
tombstone, direct reports require a valid reassignment, affected live task and
approval references are reconciled, and a default returns only through an
explicit template restore.

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
providers or fall back. The policy and intent fingerprints collectively bind
catalog model ID, catalog provider, runtime provider, active provider, and any
exact review-stage context under <code>policy-v5</code>/<code>intent-v3</code>;
an older or stale approval therefore fails exact matching rather than
authorizing a changed provider or review decision.

<code>provider_registry_status</code> returns common provider descriptors,
capabilities, readiness, versions, discovered runtime models with per-model
availability/reasons, and all catalog bindings. Backend routing consumes that
registry truth; the renderer projects the same snapshot for the provider
selector, model catalog, assignments, defaults, and routing evidence. The
backend consumes it when selecting the exact reporting-chain reviewer. Codex
catalog models are eligible only when Codex
reports ready and remain subject to CLI model validation at run start. Ollama
models additionally must be discovered locally, have readable show metadata,
and report tool support. Existing unavailable catalog entries and assignments
are preserved rather than rewritten.

Both adapters implement the same typed request, progress, cancellation,
result, evidence, and error contract. Orchestration dispatches through the
registry by the resolved runtime ID, stores canonical runtime IDs on new ledger
rows, and classifies terminal failures from typed error codes rather than
provider-specific message text. Historical ledger rows are retained unchanged.

No live provider connectivity, authentication, model execution, or model output
was checked in TASK-0008.

## Current run, routing, and review behavior

- The backend owns task creation, rerouting, candidate eligibility, scores,
  workload and overflow decisions, executor assignment, routing evidence,
  queue state/order, and execute-head admission. The renderer requests these
  decisions and displays their evidence. It also owns structured review flow,
  reviewer selection, revision state, and terminal review decisions.
- Hard routing eligibility requires an active, unpaused agent with compatible
  workspace, policy/capability, and exact provider/model availability; Ollama
  execution also requires tool support. Eligible candidates are ordered by
  deterministic score, then workload, then stable agent ID. Selected routing
  is an explicit recorded override but never bypasses a hard filter.
- Execute tasks share one durable global queue ordered by priority, enqueue
  sequence, owner ID, and task ID. Held tasks retain their age; rerouting does
  not reorder them; resetting a terminal task allocates a new sequence. Queue
  snapshots expose backend-computed positions and the active execute task.
- The renderer sends an <code>AgentRunRequest</code> containing a run ID, agent
  ID, task-owner ID, task ID, run mode, and—for review only—the exact
  backend-issued flow/stage/round/level/request-fingerprint context. Unknown
  legacy authorization or policy fields are rejected.
- The backend admits an execute attempt only for the current queue head, then
  acquires the one system-wide run slot through the existing immediate
  transaction. Review bypasses execute-queue ordering but shares that same run
  coordinator. Reuse of a bounded request ID is idempotent only when its
  normalized intent is identical.
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
  cancellation requests, task projections, events, outcomes, usage, structured
  workspace changes, bounded compatibility paths/diffs, errors, and recovery
  disposition are durable in SQLite.
- Execute runs take descriptor-confined before and after snapshots around the
  provider call, regardless of success, cancellation, timeout, or failure. Git
  roots additionally use hardened direct porcelain-v2 status plus staged and
  unstaged diff commands; non-Git roots compare filesystem metadata and bounded
  hashes. Review runs explicitly record that collection was not requested.
- Evidence classifies staged, unstaged, untracked, added, modified, deleted,
  renamed, type/status-changed, binary, redacted, and truncated cases. It never
  follows symlinks or special files, uses <code>.git</code> metadata directories
  as the sole unconditional name exclusion, and records every collection or limit
  issue. Partial, unavailable, binary, redacted, conflicted, or inconsistent
  evidence requires human review; legacy flat paths/diffs remain redacted
  compatibility projections rather than review authority.
- A Specialist result follows Senior → Team Leader → Supervisor; a Senior result
  follows Team Leader → Supervisor; a Team Leader result follows Supervisor;
  and a Supervisor result requires a trusted human decision. Each agent stage
  uses the exact active <code>reportsTo</code> identity, requires a distinct
  active/unpaused read-capable reviewer with a ready exact provider/model, and
  never substitutes another candidate.
- <code>ReviewRequestV1</code> binds task, executor, execution attempt, revision
  round, level, stage, bounded evidence, and a SHA-256 request fingerprint.
  <code>ReviewResultV1</code> must be one duplicate-key-free, unknown-field-free
  JSON object with an exact bound <code>approved</code> or
  <code>changesRequested</code> verdict and requirements, correctness,
  verification, security, and scope checks. Markdown fences, trailing text,
  missing or colliding verdicts, unknown evidence, incomplete agent evidence,
  and stale bindings cannot approve.
- Agent review stages are read-only, have no terminal or elevated scopes, and
  cannot grant authorization. At most three agent attempts may address one
  stage. A changes verdict queues a fresh execution with a fresh sequence and
  fresh policy/approval evaluation; at most three revision executions are
  allowed before explicit human adjudication.
- Human approve/request-changes decisions are revalidated against the current
  flow revision and shown through the trusted native KDialog confirmation
  boundary. Missing confirmation, unavailable reviewers, exhausted attempts,
  incomplete legacy bindings, or uncertain dispatch fail closed to
  <code>awaiting_human</code>.
- Startup reconciliation marks pre-dispatch attempts interrupted and safe to
  retry; dispatching/running/cancel-requested attempts become interrupted and
  require manual review. A safe pre-dispatch review interruption returns to the
  exact pending stage without dispatch; uncertain review execution moves to
  human adjudication. Legacy tasks found in a running/reviewing state get a
  synthetic interrupted ledger record and an unbound review migrates to human
  review instead of being silently reset or resumed.
- Pre-dispatch/startup failure returns an execute task to its original queue
  age. A post-dispatch uncertain recovery holds the task for explicit user
  action. Terminal completion removes it from the queue.
- Global run/stop UI remains visible across navigation, stale or cross-attempt
  events are ignored, queue/run snapshots refresh after authoritative changes,
  and all execute/review controls observe the same active attempt regardless of
  how the task was created.

Progress is limited to 256 events, 8 KiB per message, and 512 KiB per attempt.
Codex prompts/JSON lines/stdout/stderr are limited to 256 KiB/64 KiB/1 MiB/
512 KiB before the common ledger bounds apply; Ollama response and conversation
payloads are limited to 2 MiB each; summaries to 128 KiB; errors to 64 KiB;
diffs to 120,000 characters and 512 KiB; each workspace snapshot to 20,000
entries, five seconds, and 512 MiB of hashing; and retained change evidence to
250 entries and 256 KiB of path text. Structured evidence additionally limits
each detail to 64 KiB, aggregate details to 512 KiB, Git status output to
4 MiB, issues to 64, and its persisted JSON to 2 MiB. The ledger keeps original
counts/sizes and explicit truncation flags, which the renderer surfaces instead
of implying complete evidence.

Ollama workspace listings expose at most 200 entries per page and reads expose
at most 64 KiB per range. Full-file hashing and patches are limited to 8 MiB,
new-file content and aggregate replacement text to 512 KiB, and a patch to 64
ordered non-overlapping edits. Every partial list/read result includes an
explicit continuation cursor/offset and truncation flag.

## Native desktop and voice behavior

The renderer submits task, application, standard-folder, explicit active/named
window, pointer, keyboard/clipboard, and bounded typing requests through
<code>submit_voice_intent</code>. It cannot invoke the former direct privileged
application/window/input commands. The backend resolves the one active Coding
or PC Control template and active workspace, evaluates current persisted
capability/approval policy, records an exact audit transition before dispatch,
and returns one user-visible outcome. Close, Cut, and Delete always require a
one-use trusted approval even when policy otherwise allows system actions.
Approval retry reuses the same request ID and refuses a changed exact target.
Voice-created coding work also binds the active workspace ID and a SHA-256 of
its configured path without writing the raw workspace path to the action audit.

Application launch uses an exact XDG desktop-entry ID or unique exact desktop
name. Standard folders use <code>user-dirs.dirs</code> rather than guessed
paths. Named KWin actions require one exact desktop-entry match and one exact
normal window; active-window actions require explicit wording and recheck the
KWin internal ID before portal input. Broad <code>pkill</code>, caption/
substring matching, and implicit Alt+F4 fallback are absent. KWin operations
use create-new private temporary scripts, execute only the returned per-script
D-Bus object, and accept strict token-bound acknowledgements only from KWin's
current D-Bus owner. Relative XDG base-directory and PATH entries are ignored,
registry/config traversal is bounded, and configured folder-path drift is
detected through a SHA-256 binding without persisting the raw path. Timeout
after dispatch is recorded as <code>uncertain</code>.

The Python voice runtime and setup scripts remain bundled. Voice-runtime
installation, microphone behavior, restored portal sessions, and live
KDE/Wayland/XDG compatibility were not exercised; TASK-0016 and TASK-0020 own
those later runtime and acceptance gates.

## Current safety enforcement

The backend currently:

- validates typed action intents and rejects unknown run IPC fields;
- rejects missing, ambiguous, unsupported, inactive, or unavailable provider
  and model identities without fallback;
- routes only across hard-eligible active agents, records deterministic
  candidate evidence and manual overrides, and rejects renderer attempts to
  forge routing, executor, queue, or lifecycle state;
- admits only the global execute-queue head, admits at most one execute or
  review attempt system-wide, and protects backend-owned task lifecycle fields
  from renderer overwrites;
- derives maximum capabilities and approval modes from backend state;
- rejects paused, missing, wrong-task, and ineligible review agents;
- resolves voice coding and PC-control identities only from one active backend
  template, and routes coding requests through the normal global queue;
- refuses unknown/ambiguous XDG or KWin targets and exact-target drift, forces
  one-use approval for destructive actions, and audits dispatch/outcome without
  raw transcript, dictated text, coding request, caption, or user path;
- rejects administrator terminal access and forces review runs to be
  read-only with no elevated authorization;
- requires an exact backend-issued review flow/stage/round/fingerprint binding,
  strict structured verdict checks, sequential reporting-chain identity, and
  bounded retries before any review transition;
- requires a current trusted native confirmation for human review decisions and
  never treats renderer visibility or provider text as approval authority;
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

Nineteen Vitest files contain 62 deterministic frontend tests for renderer
domain characterization, voice, persistence, revision/fail-closed writer
behavior, authoritative run/provider/registry/queue/review projections, exact
desktop command/event mappings, native-dialog focus and cancellation, APG tab
keyboard behavior, keyboard agent-card activation, skip navigation, page-focus
transfer, deterministic axe checks, and responsive provider/reduced-motion
style contracts.

The Rust library contains 153 passing tests. They add Codex compatibility,
command-isolation, bounded protocol, fake-process descendant cleanup, provider
registry, fake-adapter dispatch, exact identity, typed failure, run-state,
concurrent admission, idempotency, approval-boundary, cancellation, timeout,
crash/restart, stale completion/event, truncation, and retention coverage to
the earlier provider, workspace, run-safety, voice, state-validation,
persistence, authorization, corruption, concurrency, and rollback tests.
Sixteen TASK-0008 tests cover fake-server discovery/transport/cancellation,
large and conflicting workspace edits, path escape, and bounded tool turns;
they do not contact a live provider. Twelve TASK-0010 Rust tests cover routing
eligibility/scoring/overflow, global ordering, queue-head admission,
concurrency, restart, allocator rollback, reroute age, reset, and failure
recovery.
Fifteen TASK-0011 Rust tests cover strict/duplicate-free protocol parsing,
bound evidence and verdicts, exact role pipelines and reporting-chain identity,
schema-v5 migration, complete sequential approval, stale revisions, invalid
output, cancellation, unavailable reviewers, revision/attempt caps, trusted
human adjudication, and safe versus uncertain restart recovery. Two frontend
tests cover authoritative flow projection and human-fallback presentation. The
review retention case proves that generic ledger pruning preserves execution
evidence referenced by an active flow.

The repository-root entry points are:

- <code>npm run verify:fast</code> — Vitest, TypeScript, rustfmt, and locked
  offline Rust tests;
- <code>npm run verify:full</code> — the fast route plus the Vite build,
  Clippy, shell/Python/strict-JSON syntax checks, npm/Cargo dependency trees,
  and production plus full npm audits.

TASK-0012 focused checks passed on 2026-08-25: 8 Git/non-Git collection tests,
2 persistence/review binding tests, 6 renderer workspace/run projection tests,
TypeScript, and the complete 130-test Rust suite across the focused runs.
<code>npm run verify:fast</code> and <code>npm run verify:full</code> passed with
9 frontend files/44 tests, TypeScript, rustfmt, 130 locked/offline Rust tests, a
43-module production build, Clippy with warnings denied, shell/Python/JSON
checks, dependency trees, and both npm audits reporting zero vulnerabilities.
Exact task evidence is recorded in
[planning/TASK_STATUS.md](planning/TASK_STATUS.md).

TASK-0013 focused checks passed on 2026-08-26 for each extraction and
accessibility slice. Native WebKitGTK 2.52.6 browser-preview acceptance used no
desktop IPC and verified keyboard skip/navigation/page focus, keyboard agent
card and Arrow-key tab activation, named modal focus/inertness/Escape/return
focus, visible provider controls at 1280/900/680/520 CSS pixels, and equivalent
360/320-pixel reflow measured through WebKit page zoom when MiniBrowser/KWin
enforced a 405-pixel native client minimum. A 508-pixel outer window contained
the 462-by-650-pixel dialog. An isolated per-process GTK reduced-motion signal
made the product media query true and reduced computed transition/animation
durations to 0.000001 seconds. No desktop setting, provider, microphone, portal,
installer, or application desktop-control path was invoked.

TASK-0014 focused checks passed on 2026-08-26: 4 strict portable-backup tests,
3 bounded-retention/timestamp-stability/clock/active-work tests, 1 transactional
monitoring and activity-scope test, 3 focused frontend files/14 persistence-
writer/browser-preview/typed-client tests, TypeScript, and a 66-module
production build. The complete fast and full gate results are recorded in
[planning/TASK_STATUS.md](planning/TASK_STATUS.md).

TASK-0015 focused checks passed on 2026-08-28: 15 Rust tests cover canonical
redaction/bounds, exact XDG/KWin targets, policy, destructive approval,
paused-agent rejection, schema/audit transitions, restart uncertainty,
wrong-target retry refusal, and the narrowed IPC surface. Three frontend
files/11 tests cover canonical parsing, unknown-close safety, exact typed
client payloads, one backend submission, explicit portal enablement, and
same-ID approval retry. TypeScript passed, and the complete fast gate passed
with 19 frontend files/62 tests and 153 locked/offline Rust tests. The complete
full gate also passed with a 66-module production build, Clippy warnings denied,
shell/Python/strict-JSON checks, npm/Cargo dependency trees, and production plus
full npm audits reporting zero vulnerabilities.

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
| Physical database/file purge and installed removal evidence | TASK-0019 |
| Installed WebView, packaged accessibility, and live platform acceptance | TASK-0020 |
| Offline voice-runtime reliability and live KDE/portal/XDG acceptance | TASK-0016 and TASK-0020 |
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
