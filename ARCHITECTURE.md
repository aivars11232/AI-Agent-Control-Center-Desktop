# Architecture

This document separates the architecture that exists in version 0.5.1 from the
approved direction for later tasks. A **planned** component name is not evidence
that a corresponding module or guarantee exists today.

## Architectural goals

AI Agent Control Center is intended to be a local-first desktop control plane
for a bounded hierarchy of AI agents. The design must keep user intent,
authorization, workspace scope, provider execution, run evidence, and recovery
under an authoritative local backend while retaining an understandable
renderer UI.

Fixed direction:

- repair and evolve the existing Tauri application rather than restart it;
- target Arch Linux, KDE Plasma, and Wayland first;
- preserve Supervisor → Team Leader → Senior Agent → Specialist;
- preserve Codex and local Ollama without requiring a paid API key;
- use one active AI run system-wide until a later recorded decision changes it;
- keep reminders passive; they do not run models in the background.

The binding decision record is
[planning/decisions/0001-fixed-project-decisions.md](planning/decisions/0001-fixed-project-decisions.md).

## Current architecture

    User
      |
      v
    React renderer (src/App.tsx -> src/app/AppController.tsx)
      |-- app shell/navigation/status (src/app/AppShell.tsx)
      |-- feature pages (src/features/*)
      |-- shared accessible controls (src/components/*)
      |-- typed desktop client (src/services/desktopClient.ts)
      |-- state/persistence adapter (src/applicationState.ts, src/persistence.ts)
      |-- provider availability/model projection (src/providerRegistry.ts)
      |-- dynamic agent/group/hierarchy projection (src/agentRegistry.ts)
      |-- task queue/routing evidence projection (src/taskOrchestration.ts)
      |-- structured review-flow projection (src/reviewOrchestration.ts)
      |-- run snapshot/event projection (src/runCoordinator.ts)
      |-- specialist request/result presentation (src/specialistCapabilities.ts)
      |-- structured workspace-evidence projection (src/workspaceEvidence.ts)
      |-- passive schedule, scoped memory, and handoff projections
      |-- canonical voice-intent interpretation (src/voiceCommand.ts)
      |-- browser-preview localStorage (non-authoritative compatibility only)
      |-- renderer presentation: review controls and task view state
      |
      | Tauri invoke/events
      v
    Rust backend (src-tauri/src/lib.rs)
      |-- application state validation + SQLite repository/migrations
      |-- authoritative agent registry/hierarchy (agent_registry.rs)
      |-- authoritative task routing/queue contract (task_orchestration.rs)
      |-- authoritative review protocol/pipeline (review_orchestration.rs)
      |-- strict specialist request/result/tool contracts (specialist_capabilities.rs)
      |-- passive reminder/event contracts (reminder_scheduler.rs)
      |-- scoped memory contracts (structured_memory.rs)
      |-- sequential handoff contracts (management_handoffs.rs)
      |-- capability policy + authoritative one-use approvals
      |-- authoritative single-run coordinator + durable bounded ledger
      |-- provider registry/common contract (provider_runtime.rs)
      |     |-- isolated Codex CLI runtime (codex_runtime.rs)
      |     |-- bounded Ollama HTTP runtime (ollama_runtime.rs)
      |     |-- descriptor-confined workspace tools (workspace_tools.rs)
      |-- versioned filesystem/Git evidence collector (workspace_evidence.rs)
      |-- canonical system-action contract/audit (system_actions.rs)
      |-- exact XDG/KWin adapter (linux_desktop.rs)
      |-- Python voice listener process
      v
    Selected workspaces and local desktop environment

### Renderer

<code>src/App.tsx</code> is a thin compatibility entry point.
<code>src/app/AppController.tsx</code> owns state hydration, persistence
coordination, event subscriptions, and feature composition, while
<code>src/app/AppShell.tsx</code> owns window-level navigation, provider status,
skip navigation, page-focus transfer, and the global-run banner. Page UI lives
under <code>src/features/</code>; accessible dialog, tabs, live status, and
keyboard action primitives live under <code>src/components/</code>. The typed
<code>src/services/desktopClient.ts</code> facade centralizes Tauri commands,
payloads, and event channels. Voice Control exposes one canonical submission
and one read-only audit query rather than direct privileged desktop methods.
Persisted renderer types and the canonical seed are separated into
<code>src/applicationState.ts</code> and
<code>src/application-state-seed.json</code>. The desktop renderer gates the UI
on a typed backend load, serializes whole-state saves, and uses revision-based
compare-and-swap. Browser preview persistence remains a non-authoritative
compatibility path. <code>src/runCoordinator.ts</code> projects ordered backend
snapshots/events, discards stale or cross-attempt events, and holds only
transient progress/stop presentation state. <code>src/providerRegistry.ts</code>
projects the common backend registry into truthful provider status and
fail-closed model eligibility for assignment, defaults, and routing;
it is presentation and preflight logic, not the dispatch authority.
<code>src/agentRegistry.ts</code> projects active agents, dynamic category
groups, ancestor context, repair rows, role-derived authority, compatible
manager choices, and stable template identities. It does not authorize or
persist registry mutations. <code>src/taskOrchestration.ts</code> projects the
backend queue snapshot, positions, states, and routing evidence; it does not
score candidates or decide admission. <code>src/reviewOrchestration.ts</code>
projects backend flow/stage history and human-fallback status; it does not pick
reviewers, parse provider verdicts, or decide transitions.
<code>src/workspaceEvidence.ts</code> defensively projects the backend's
versioned evidence into task, dashboard, and activity views and only offers a
file-open action for an existing final regular file or directory.
<code>src/specialistCapabilities.ts</code> builds typed role-specific task
requests, describes effective ceilings, and projects backend-validated results.
Its Debugging-to-Coding action only pre-populates a visible Coding draft; it
does not create, approve, queue, or dispatch work by itself.
<code>src/reminderScheduler.ts</code>, <code>src/structuredMemory.ts</code>, and
<code>src/managementHandoffs.ts</code> defensively project backend-owned
schedules, scoped records, and sequential evidence. Their forms request
revision-bound mutations and their views expose failures and provenance; they
do not resolve civil time, authorize notification delivery, select run memory,
or create management transitions.

### Rust backend

<code>src-tauri/src/lib.rs</code> owns native commands, provider
processes/transports, workspace resolution and tool access, run cancellation,
diff/change capture, desktop actions, and voice process management. It
also composes <code>app_state.rs</code>, <code>policy.rs</code>,
<code>agent_registry.rs</code>,
<code>task_orchestration.rs</code>,
<code>review_orchestration.rs</code>,
<code>authorization.rs</code>, <code>run_coordinator.rs</code>,
<code>provider_runtime.rs</code>, <code>codex_runtime.rs</code>,
<code>ollama_runtime.rs</code>, <code>workspace_tools.rs</code>,
<code>workspace_evidence.rs</code>, <code>data_lifecycle.rs</code>,
<code>specialist_capabilities.rs</code>,
<code>reminder_scheduler.rs</code>, <code>structured_memory.rs</code>,
<code>management_handoffs.rs</code>,
<code>system_actions.rs</code>, <code>linux_paths.rs</code>,
<code>linux_desktop.rs</code>, <code>voice_runtime.rs</code>,
<code>desktop_control.rs</code>, and
<code>persistence.rs</code>. Those modules
own the versioned state contract, normalized action intents, fail-closed
capability evaluation, authoritative approval lifecycle, legal run transitions
and evidence bounds, provider identity/contracts/dispatch, isolated Codex
compatibility/process/protocol handling, bounded fixed-loopback Ollama
transport/discovery, descriptor-confined conflict-safe workspace edits and
before/after evidence capture, hardened Git inspection,
validated agent identity/lifecycle/hierarchy operations, deterministic routing
and queue evidence, structured reporting-chain review and verdict validation,
passive schedule/time-zone resolution, scoped-memory selection and exact run
bundles, sequential management-handoff validation,
canonical voice/system-action contracts, exact Linux desktop target
resolution, bounded redacted action audit, SQLite schema/repository, legacy
migration, and typed state IPC. The provider registry
exposes only Codex and Ollama adapters and rejects provider/model mismatch
without fallback. Non-run
privileged command handlers consume backend authorization before their first
side effect; run approvals are reserved at admission and consumed at successful
provider startup.

### Persistence and migration

The desktop database is <code>application-state.sqlite3</code> below Tauri's
platform application-data directory. Migrations 0001 through 0011 establish
schema version 11 plus a migration ledger. Repository writes replace one
validated aggregate inside an immediate transaction and use a monotonically
increasing revision to reject stale writers. Startup refuses corrupt or
unsupported newer databases and retains the typed error for the renderer
instead of silently creating replacement state.

Schema v2 adds authoritative approval intent, policy, and workspace bindings
plus backend timestamps. Generic renderer state saves cannot insert, approve,
consume, or overwrite approval rows. Capability, approval-policy, review-role,
microphone, safety, approval-lifetime, and workspace-root privilege increases
require a trusted native confirmation that names the exact elevation and are
rechecked inside the save transaction.

The one-time legacy path treats seven WebView storage values as untrusted.
Only a fully parsed and validated candidate commits. Legacy pending/approved
approvals are downgraded to expired non-authoritative history, and browser keys
are deleted only after the transaction commits. Backend-issued records use a
separate authoritative origin and cannot be manufactured by migration or a
whole-state save. Portable backup v4 export, preview, and apply are backend
owned, strict, bounded, sanitized, revision checked, idle-run guarded, natively
confirmed, and atomic. Legacy version 2 and 3 imports pass through the same
sanitization boundary; browser-only version 2 behavior is explicitly a
non-authoritative preview.

Schema v3 adds immutable-terminal run attempts, bounded progress events,
approval reservations, and coordinator metadata. One active-attempt foreign
key is acquired within an immediate transaction, while a coordinator-specific
revision orders run projections independently from aggregate-state saves.
Import/reset operations reject an active run; reset clears run-ledger data.
Startup reconciliation distinguishes attempts that are safe to retry from
ones that may have dispatched and require manual review.

Schema v4 adds stable optional template keys, explicit active/unassigned/deleted
lifecycle state, migration issues, deletion timestamps, and a durable monotonic
agent-ID allocator. Create, update, logical delete, and template restore use
dedicated compare-and-swap IPC transactions. Generic whole-state saves cannot
change registry identity, lifecycle, name, role, category, authority, or
reporting structure. Roles derive authority; active non-supervisors require an
active higher-authority manager; self-parenting, cycles, dangling managers, and
incompatible authority fail validation. Legacy invalid relationships are
paused and detached for explicit repair, legacy agents that share an id are
made unique by keeping the first occurrence and quarantining re-keyed
duplicates, and absent/deleted defaults are not silently recreated.

Schema v5 adds bounded routing inputs and evidence, queue state and enqueue
sequence on tasks, per-agent queue thresholds and overflow policy, plus durable
JavaScript-safe task/enqueue allocators and an orchestration revision. Dedicated
compare-and-swap commands create, reroute, hold, resume, or reset tasks. Generic
whole-state saves cannot create, remove, or relocate tasks; change routing
inputs or the assigned executor; or forge queue, lifecycle, or routing
evidence. Queue order is global and deterministic by priority, enqueue
sequence, owner ID, and task ID.

Schema v6 adds normalized review flows and stage-attempt history, one active
flow per task, explicit revision rounds, exact request fingerprints, and review
bindings on run attempts. Backend policy binds review admission to the current
flow, stage, level, round, reviewer, and request fingerprint. Terminal stage
records are immutable; generic aggregate saves preserve normalized review
state. Migration treats legacy unbound in-flight review as human-required, and
startup reconciliation retries only a provably pre-dispatch stage without
automatically invoking a provider.

Schema v7 adds one nullable, JSON-checked versioned workspace-evidence record
to each run attempt and task aggregate. A null legacy value becomes explicit
unavailable evidence when projected. Completion validates and persists the
record transactionally, accounts it against run retention, and relies on the
existing terminal-attempt trigger plus generic-save field preservation to keep
the evidence backend-owned and immutable after completion.

Schema v8 adds normalized Unix-millisecond lifecycle columns, retention
indexes, durable lifecycle metadata, and a latest-100 maintenance evidence
ledger. A synchronous bounded pass runs at startup and after retention-setting
or import mutations; a non-AI timer runs every 15 minutes and retries bounded
backlog after one minute. Each domain deletes at most 500 eligible rows per
pass, active authority/work remains protected, and backward clock movement is
recorded while age deletion is skipped. Maintenance advances the application,
task, run, review, and lifecycle revisions that its exact deletions affect.
Normalized timestamps survive unrelated aggregate-state saves, so an inferred
legacy age cannot silently move forward on each renderer mutation.

Schema v9 adds an immutable-binding system-action audit capped at 10,000 rows,
plus lifecycle totals/run counts for terminal audit retention. Request IDs bind
canonical intent fingerprints to exact targets and authorization evidence.
Restart reconciliation changes a pre-crash <code>dispatched</code> action to
<code>uncertain</code> and never repeats it. Portable backup v4 intentionally
omits this local authority/evidence domain.

Schema v10 adds bounded JSON-checked specialist requests to tasks and immutable
specialist run contracts/results to attempts. Strict duplicate-free parsing,
unknown/future-field rejection, canonical request hashing, task-template-kind
matching, and generic-save preservation keep those records backend-owned.
Untouched legacy seed profiles are narrowed to truthful defaults; customized
profile rows are preserved. Older tasks without a typed request remain visible
but cannot acquire core-specialist authority until the user creates a new typed
task. Portable backup v4 carries portable task requests but excludes immutable
run contracts/results with the rest of the run ledger.

Schema v11 replaces renderer-owned reminder behavior with normalized scheduled
items, occurrence/delivery evidence, scheduler authority metadata, and a
dedicated revision. Local civil time is resolved through IANA zones with an
earlier-offset fold policy and forward-shift gap policy; daily, weekly, and
monthly recurrence stays anchored to the original civil time. A non-AI desktop
timer scans after startup and every 30 seconds, records missed or uncertain
occurrences idempotently, and optionally sends a privacy-bounded XDG
notification. Invalid legacy schedules and recurrence limits become explicit
needs-attention evidence rather than repeated dispatch.

Schema v11 also introduces exactly scoped agent/project/task/team memory,
revision/provenance/retention events, and immutable per-attempt prompt bundles.
Run admission selects only records visible to the exact agent, reporting team,
workspace, and task; caps the canonical bundle at 128 records/64 KiB; stores
its JSON and SHA-256 before dispatch; and passes that exact bundle to the
provider prompt. Legacy agent free text migrates once as inspectable agent
memory and generic aggregate saves cannot recreate memory authority.

Management handoffs in schema v11 are append-only, idempotent, bounded records
for plans, assignments, execution/failure evidence, review decisions,
revisions, human overrides, and recovery. The task, run, review, and trusted
human transactions create them in required sequence and bind their source,
owner role, identities, revision round, and evidence IDs. They are an
inspectable workspace projection, not a provider-dispatched message channel.
Portable backup v4 accepts prior v2/v3 input, carries schedules and unexpired
structured memory, converts portal delivery to in-app behavior, and omits
portal grants/delivery evidence, handoffs, and run/review authority.

Monitoring reads the application, task-orchestration, run-coordinator,
review-orchestration, and lifecycle revisions in one transaction. Task and
activity page queries require that exact tuple, reject stale callers, and cap
pages at 100 records. Local activity deletion advances application/lifecycle
revisions but does not address run/review tables. Portable backup and
monitoring DTOs live in the data-lifecycle domain; persistence remains the
transaction authority and the typed desktop client remains only a renderer
adapter. The renderer refreshes monitoring after state commits and at a bounded
one-minute interval; authoritative pages remain blank while their exact-tuple
query is loading rather than falling back to renderer estimates.

### Voice runtime

<code>voice-runtime/listener.py</code> is a bundled local Python listener. Its
base path imports only pinned Vosk dependencies, consumes exact 20 ms PipeWire
PCM frames, retains a bounded pre-roll and 20-second utterance, reloads valid
configuration at most once per second, and keeps audio in memory. Optional
whisper.cpp transcription uses a private mode-0600 temporary WAV and falls
back to the base transcript on failure. NDJSON and subprocess diagnostics are
bounded and invalid listener messages fail closed.

The setup scripts accept backend-created XDG staging/cache paths, use pinned
artifact versions and SHA-256 verification, resume download caches, write the
manifest last, and never replace an active release directly. The backend owns
one install operation ID, rejects overlap, cancels its process group, validates
the staged release, and atomically promotes or preserves the previous release.
Base and optional high-accuracy releases remain independent.
<code>src/voiceCommand.ts</code> maps local transcripts or typed commands to a
closed canonical intent union. The backend—not the renderer—selects the current
agent/workspace, resolves exact XDG/KWin targets, derives risk and scopes,
issues/consumes approval, writes audit transitions, and dispatches.
Voice-created coding tasks use the existing routed-task transaction and global
sequential queue and now carry the same typed Coding request as renderer-created
work. Raw transcripts are not sent to the gateway; dictated and
coding content are represented in the action audit only by SHA-256 and length,
while configured workspace paths are bound by SHA-256 without storing the raw
path.

The listener, microphone, and RemoteDesktop session remain separate native
boundaries. The portal session is bound to the exact active Full PC Control
agent, subscribes to native <code>Closed</code>, supports explicit close, closes
partial grants, and is reconciled after state import/reset or agent authority
change. Failed pressed-input cleanup closes the session. TASK-0016 establishes
these checked-in non-live contracts; TASK-0020 owns sequential live and
packaged acceptance.

### Current voice/system-action flow

1. The renderer interprets untrusted local input into one bounded typed intent
   and assigns an idempotency key.
2. The backend loads current state and resolves one active Coding or PC Control
   template plus the selected workspace or exact native target.
3. Policy derives scopes/risk from backend state. Close, Cut, and Delete force
   one-use approval; other actions follow the persisted approval mode.
4. Approval-required retries retain the same request and target binding.
   Target drift terminates the request without dispatch.
5. The backend records <code>dispatched</code> before the platform/task side
   effect, then records <code>applied</code>, <code>taskCreated</code>,
   <code>failed</code>, or <code>uncertain</code>.
6. XDG launch/folder actions use precedence-aware localized native metadata,
   visibility/TryExec rules, tombstones, configured user directories, and
   absolute base-directory values. <code>gtk-launch</code> remains primary and
   GIO launches the already-resolved desktop file as a native fallback. KWin runs only its returned
   <code>/Scripting/Script{id}</code> object and reports through a token-bound
   callback authenticated to KWin's current D-Bus owner. Portal input rechecks
   the active window, and coding work enters the normal queue.

### External and operating-system boundaries

- Codex is an installed CLI process authenticated outside application state.
- Linux Codex execution requires Bubblewrap for user/PID namespaces,
  parent-death handling, capability drop, and descendant-lifecycle cleanup;
  the inner Codex sandbox remains the filesystem policy boundary.
- Ollama is a local HTTP service and model catalog.
- Workspaces are user-selected filesystem roots.
- KDE/Wayland integration crosses application, window, input, portal, desktop
  entry, and XDG data/config boundaries.
- Reminder notification delivery crosses the XDG notification portal; no
  notification grant or restore token is treated as portable application
  authority.
- Git evidence is read through direct hardened Git commands in a selected
  workspace; descriptor-confined filesystem evidence is the explicit fallback
  for non-Git roots or unusable Git state.
- Removal is backend-authoritative: `src-tauri/src/lifecycle_removal.rs` owns
  the inventory of every on-disk location across both the
  `com.aivarsrocens.aiagentcontrolcenter` bundle identifier and the
  `ai-agent-control-center` namespace, and the `--stop-runtime`, `--uninstall`
  (keep-data), and `--purge --confirm PURGE` subcommands. The shell installers
  never hard-code data paths; the persistent KDE portal permission is revoked
  only in KDE System Settings.

### Packaging and CI

- Two install paths: `install-kde.sh` (user-local `~/.local`, idempotent
  upgrade with rollback, no PlasmaShell restart) and `packaging/PKGBUILD` (Arch
  system package, `/usr/lib` payload with a `/usr/bin` symlink, pacman hooks).
- Release metadata: proprietary root `LICENSE` (`LicenseRef-proprietary`),
  `THIRD-PARTY-NOTICES.md`, repository/license/engines fields in
  `package.json` and `Cargo.toml`, a single-main-category desktop entry, and a
  validated AppStream `metainfo.xml`.
- `.github/workflows/ci.yml` runs one strictly sequential job chain
  (`frontend → rust → scripts → licenses → secrets → packaging`) with no
  release step and no live AI/microphone/portal/system action. `cargo-deny`,
  `scripts/check-licenses.sh` (permissive-only), and `gitleaks` are mandatory;
  the Arch `packaging` job exercises `makepkg`, `namcap`, and the staged
  install/removal test in a container.

## Current run flow

1. The renderer requests task creation or rerouting through typed IPC. The
   backend filters candidates by active state, workspace, capability, and exact
   provider/model readiness, applies deterministic scores, workload, and
   overflow policy, and persists candidate evidence plus the selected executor.
   A core specialist request must match exactly one stable template and category;
   generic or manually selected cross-specialist routing cannot bypass this gate.
2. Execute tasks enter one durable global queue. The backend snapshot orders
   them by priority, enqueue sequence, owner ID, and task ID; held tasks retain
   their queue age, while a terminal task reset receives a new sequence.
3. The renderer sends a typed run intent containing the run locator, agent,
   task owner, task, and run mode. A review intent additionally echoes the
   backend-issued flow, stage, round, level, and request fingerprint.
4. For execute mode, the backend admits only the current queue head and
   atomically acquires the shared single-run slot. Review bypasses execute-queue
   order but uses the same run coordinator. Invalid, busy, non-head, or
   pending-approval requests fail deterministically.
5. An exact approved record is reserved by admission. It is consumed once only
   after successful provider startup; cancellation or startup failure before
   dispatch releases it, while uncertain post-dispatch recovery prevents
   replay.
6. The backend projects the task into its active lifecycle, derives workspace,
   capability limits, timeout, prompt, and sandbox from persisted state. It
   resolves exactly one catalog model, maps only OpenAI to Codex and Ollama to
   Ollama, requires that adapter to equal the persisted active provider, and
   dispatches exactly that registry adapter. Unsupported, missing, ambiguous,
   inactive, or unavailable identities fail closed without fallback.
   For core work it also hashes the typed request and persists an immutable
   role/tool/provider/model/approval contract before dispatch. It selects the
   exact visible structured-memory records, persists their canonical bounded
   bundle and SHA-256 on the attempt, and renders that same bundle into the
   provider prompt.
7. Codex dispatch revalidates its executable, streams the prompt on standard
   input, runs in an outer lifecycle-only Bubblewrap namespace plus an explicit
   inner Codex sandbox, parses bounded JSONL incrementally, and refuses a
   terminal outcome until descendant cleanup is established.
8. Ollama dispatch resolves names through <code>/api/tags</code> and metadata
   through <code>/api/show</code>, holds one cancellable async session and task
   deadline across its bounded tool loop, and exposes only descriptor-confined
   paginated/read/hash/create/patch workspace operations.
9. For execution, the backend brackets provider dispatch with bounded
   descriptor-confined snapshots and hardened Git status/diff inspection. It
   persists the validated structured result with ordered events, cancellation
   state, terminal outcome, output summary, and usage. Terminal attempts cannot
   be updated. A successful execution starts or resumes one normalized review
   flow when review is required.
   A successful specialist run additionally requires one strict kind-matched
   structured result. Debugging, Browser Research, and Financial Analysis must
   have zero observed scratch/workspace changes; Browser sources must be HTTPS
   and domain-bounded; requested checks must be reported exactly; Financial
   assumptions and calculation values must exactly match the request and
   backend fixed-point results.
   The same completion transaction appends bounded execution or failure
   handoff evidence linked to the full immutable run record rather than
   duplicating unbounded workspace details.
10. The next stage is derived from the executor role and walks the exact active
    reporting chain sequentially: Senior, Team Leader, and Supervisor as
    applicable. Missing, repeated, inactive, or provider-unready identities
    move the flow to explicit human review without substitution.
11. Each agent stage receives one versioned, fingerprinted, bounded evidence
    request under read-only policy. Agent approval additionally requires a
    complete matching structured workspace record; partial, unavailable,
    redacted, binary, conflicted, or inconsistent evidence requires human
    review. Only one exact structured result with all required checks and
    matching identifiers can transition the flow; provider prose and embedded
    workspace evidence remain untrusted data.
12. Approval advances to the next level or completes the task. Requested
    changes enqueue a fresh policy-evaluated execution, capped at three
    revisions. Three invalid/failed stage attempts, an uncertain dispatch, the
    revision cap, or a Supervisor executor requires a natively confirmed human
    decision.
13. The renderer displays authoritative queue/run/review snapshots, routing and
    workspace evidence, explicit partial/unavailable states, and a global Stop
    control across navigation; generic state saves cannot overwrite run- or
    review-owned task fields.

## Directional architecture

The target is a modular monolith with the Tauri backend as the authoritative
state and policy boundary. Later tasks should extract real modules only when
they carry behavior; TASK-0001 intentionally creates no empty placeholders.

### Renderer boundaries

- **App shell and navigation** — window-level layout, page routing, skip
  navigation, focus transfer, provider status, and global-run presentation.
- **Feature UI** — dashboard, agents, tasks, approvals, reminders, activity,
  models, voice, and settings.
- **Typed IPC adapter** — one renderer boundary for backend commands/events.
- **View state** — transient selection, form, display, and accessibility state.
- **Presentation components** — reusable dialog, tabs, status, and keyboard
  controls without policy authority.

TASK-0013 implements the modular renderer boundaries. TASK-0014 connects its
Dashboard, Tasks, Activity, and Settings projections to revision-bound backend
monitoring and data-lifecycle commands without moving authority into the UI.
TASK-0015 connects Voice Control to the typed backend gateway and audit without
moving agent selection, capability decisions, target resolution, or dispatch
authority into the UI.
TASK-0017 connects the Agents task composer and run ledger to typed specialist
requests, visible effective ceilings, immutable run contracts, and validated
structured results without making renderer fields an authorization boundary.
TASK-0018 connects Reminders and Agents to revision-bound schedule, structured-
memory, and management-handoff commands/events without moving time-zone,
delivery, memory-selection, or transition authority into the renderer.

The renderer may request actions and display decisions. It must not mint
authorization, decide durable run truth, or become the sole owner of domain
state.

### Directional backend boundaries

- **Domain/state** — versioned agents, tasks, approvals, reminders/events,
  structured memory, management handoffs, settings, runs, and audit records.
- **Persistence/migrations** — atomic storage, schema versions, backup import,
  recovery, retention, and migration evidence.
- **Policy/authorization** — normalized capabilities, approval matching,
  expiry, one-use consumption, replay resistance, and denial reasons.
- **Specialist contracts** — strict request/result schemas, exact stable-template
  identity, immutable per-run tool ceilings, fixed-point calculations, and
  cross-role/external-effect rejection. This boundary is implemented by
  TASK-0017.
- **Reminder scheduler** — IANA civil-time resolution, anchored recurrence,
  restart/missed-event reconciliation, occurrence idempotency, and optional
  XDG notification delivery. This boundary is implemented by TASK-0018.
- **Structured memory** — exact agent/project/task/team scope, provenance,
  revision/retention, retrieval, and immutable per-run bundle evidence. This
  boundary is implemented by TASK-0018.
- **Management handoffs** — bounded sequential task/run/review/human transition
  evidence and management visibility. This boundary is implemented by
  TASK-0018.
- **Task orchestrator** — deterministic hard eligibility, scoring, workload and
  overflow decisions, durable global execute-queue ordering, and backend queue
  admission. This boundary is implemented by TASK-0010.
- **Review orchestrator** — versioned evidence/result protocol, exact sequential
  reporting-chain stages, bounded revision/retry state, human gates, and
  restart reconciliation. This boundary is implemented by TASK-0011.
- **Run coordinator** — one active run, atomic admission,
  lifecycle transitions, cancellation, timeout, recovery, and a durable
  bounded ledger. This boundary is implemented by TASK-0005; later tasks may
  extract adapters without moving authority back to the renderer.
- **Provider registry** — provider identity, model capability, readiness, and
  truthful dispatch contracts. The common registry and contract are
  implemented by TASK-0006; TASK-0007 hardens the Codex adapter and TASK-0008
  hardens the Ollama adapter.
- **Codex adapter** — capability-based compatibility checks, explicit CLI
  isolation, Linux descendant-lifecycle containment, bounded JSONL protocol,
  cancellation/timeout escalation, and evidence parsing. This boundary is
  implemented by TASK-0007; live acceptance remains with TASK-0020.
- **Ollama adapter** — local discovery, transport, tool loop, and workspace
  tools. Bounded transport/discovery, cancellable requests, per-model metadata,
  and conflict-safe workspace tools are implemented by TASK-0008; live
  acceptance remains with TASK-0020.
- **Workspace service** — canonical roots, descriptor-confined path containment,
  bounded before/after snapshots, hardened Git status/diffs, redaction, and
  versioned change evidence. This boundary is implemented by TASK-0012.
- **Voice/system gateway** — intent normalization and policy-checked desktop
  actions.
- **Platform integration** — KDE, Wayland, XDG portals, desktop entries, and
  install/remove lifecycle.

These are responsibility boundaries, not prescribed filenames. Each owning
task must choose the smallest structure supported by the code at that time.

## Target domain ownership

| Data or decision | Current owner | Planned authoritative owner |
| --- | --- | --- |
| Agents and hierarchy | Validated backend registry with transactional CRUD, lifecycle, role-derived authority, and reporting constraints; renderer projects views | Backend agent registry |
| Tasks and results | Backend SQLite aggregate plus authoritative task creation/rerouting/queue state, typed core-specialist requests, and run-ledger-owned lifecycle/contracts/results | Backend domain store and run ledger |
| Approval records | Backend SQLite; backend-issued rows are authoritative and imported rows are expired history | Backend policy/approval store |
| Approval match/consume | Backend exact-match transaction | Backend exact-match transaction |
| Routing and review | Backend owns routing, execute-queue admission, exact reviewer selection, structured stage transitions, revisions, human fallback, and recovery; renderer projects state and requests commands | Backend scheduler/orchestrator |
| Active run | Backend system-wide coordinator and SQLite ledger; only live cancellation handles are in memory | Backend system-wide coordinator |
| Provider/model truth | Backend registry and exact active-provider/catalog-model resolution; renderer projects availability | Backend provider registry |
| Workspace evidence | Backend captures and validates bounded Git/non-Git evidence around execution, persists it per attempt/task, and supplies only complete matching records to agent review | Backend durable evidence record |
| Reminders/events | Backend schedule/occurrence store, non-AI timer, and optional XDG notification delivery; renderer requests mutations and projects evidence | Backend passive reminder service |
| Structured memory | Backend scoped records/events and immutable exact per-attempt bundles; renderer requests revision-bound CRUD and projects provenance | Backend memory service |
| Management handoffs | Backend task/run/review/human transactions append bounded sequential evidence; renderer projects workspaces | Backend scheduler/orchestrator |
| UI preferences | Backend SQLite for desktop; preview storage in browsers | Local settings store, with UI ownership where safe |

## Target hierarchy and flow

    Supervisor
      └── Team Leader
            ├── Senior Agent
            │     └── Specialist
            └── Senior Agent
                  └── Specialist

Specialists perform bounded work. The implemented review pipeline traverses the
executor's exact reporting chain through Senior, Team Leader, and Supervisor as
applicable; Supervisor execution requires a human gate. Titles alone grant no
filesystem, terminal, network, or system authority; capability and policy
records do.

The planned system-wide execution flow is sequential:

1. accept and persist intent;
2. normalize and route it;
3. assess policy and obtain any exact approval;
4. persist the execute task in the deterministic global queue and admit only
   its head to the single-run slot;
5. execute through one provider adapter;
6. capture output and workspace evidence;
7. review or request bounded revision;
8. persist terminal state and release the lease;
9. recover deterministically after cancellation, crash, or restart.

## Evolution rules

- Preserve the implemented legacy migration and do not reintroduce desktop
  <code>localStorage</code> fallback.
- Move one authority boundary at a time and characterize current behavior
  before changing it.
- Define shared IPC data explicitly; reject unknown or invalid values at the
  backend.
- Keep providers behind one contract without pretending unsupported labels are
  integrations.
- Keep KDE/Wayland-specific behavior behind a platform boundary and research
  native mechanisms before workarounds.
- Do not couple this application to the unfinished Context for AI project.

## Roadmap ownership

TASK-0003 through TASK-0005 establish backend state, policy, and coordination.
TASK-0006 establishes provider identity and the common contract; TASK-0007
hardens the Codex adapter and TASK-0008 hardens the Ollama adapter.
TASK-0009 establishes the dynamic agent registry and valid hierarchy;
TASK-0010 establishes deterministic routing and sequential queueing; TASK-0011
establishes structured review/revision/recovery; TASK-0012 establishes complete
bounded workspace evidence orchestration; TASK-0013 establishes the modular,
accessible, responsive renderer; TASK-0014 establishes strict portable backup,
bounded continuous retention, and truthful revision-bound monitoring.
TASK-0015 establishes the unified system-action policy/audit gateway;
TASK-0016 establishes deterministic offline voice plus non-live
KDE/portal/XDG integration contracts. TASK-0017 establishes strict bounded
Coding, Debugging, Browser Research, and Financial Analysis profiles.
TASK-0018 establishes passive reminders/events, scoped structured memory, and
sequential management handoffs. TASK-0019 establishes reproducible user-local
and Arch packaging, backend-authoritative keep-data / purge removal, proprietary
release metadata with a permissive third-party inventory, and a sequential
CI/security gate. TASK-0020 retains all sequential live and packaged acceptance
and is the only production-readiness gate.

Exact dependencies and gates are authoritative in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).
