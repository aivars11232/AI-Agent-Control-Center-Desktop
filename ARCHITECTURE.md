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
    React renderer (src/App.tsx)
      |-- typed state/IPC adapter (src/applicationState.ts, src/persistence.ts)
      |-- provider availability/model projection (src/providerRegistry.ts)
      |-- run snapshot/event projection (src/runCoordinator.ts)
      |-- browser-preview localStorage (non-authoritative compatibility only)
      |-- renderer presentation: routing/reviewer selection and task view state
      |
      | Tauri invoke/events
      v
    Rust backend (src-tauri/src/lib.rs)
      |-- application state validation + SQLite repository/migrations
      |-- capability policy + authoritative one-use approvals
      |-- authoritative single-run coordinator + durable bounded ledger
      |-- provider registry/common contract (provider_runtime.rs)
      |     |-- Codex CLI adapter
      |     |-- local Ollama HTTP + workspace-tools adapter
      |-- filesystem/Git inspection
      |-- application/window/input control
      |-- Python voice listener process
      v
    Selected workspaces and local desktop environment

### Renderer

<code>src/App.tsx</code> currently combines page composition, import/export,
routing, review, approval presentation, run orchestration, and most
presentation logic.
Persisted renderer types and the canonical seed are separated into
<code>src/applicationState.ts</code> and
<code>src/application-state-seed.json</code>. The desktop renderer gates the UI
on a typed backend load, serializes whole-state saves, and uses revision-based
compare-and-swap. Browser preview persistence remains a non-authoritative
compatibility path. <code>src/runCoordinator.ts</code> projects ordered backend
snapshots/events, discards stale or cross-attempt events, and holds only
transient progress/stop presentation state. <code>src/providerRegistry.ts</code>
projects the common backend registry into truthful provider status and
fail-closed model eligibility for assignment, defaults, routing, and review;
it is presentation and preflight logic, not the dispatch authority.

### Rust backend

<code>src-tauri/src/lib.rs</code> owns native commands, provider
processes/transports, workspace resolution and tool access, run cancellation,
diff/change capture, desktop actions, and voice process management. It
also composes <code>app_state.rs</code>, <code>policy.rs</code>,
<code>authorization.rs</code>, <code>run_coordinator.rs</code>, and
<code>provider_runtime.rs</code>, and <code>persistence.rs</code>. Those modules
own the versioned state contract, normalized action intents, fail-closed
capability evaluation, authoritative approval lifecycle, legal run transitions
and evidence bounds, provider identity/contracts/dispatch, SQLite
schema/repository, legacy migration, and typed state IPC. The registry exposes
only Codex and Ollama adapters and rejects provider/model mismatch without
fallback. Non-run privileged command handlers consume backend authorization
before their first side effect; run approvals are reserved at admission and
consumed at successful provider startup.

### Persistence and migration

The desktop database is <code>application-state.sqlite3</code> below Tauri's
platform application-data directory. Migrations 0001 through 0003 establish
schema version 3 plus a migration ledger. Repository writes replace one validated
aggregate inside an immediate transaction and use a monotonically increasing
revision to reject stale writers. Startup refuses corrupt or unsupported newer
databases and retains the typed error for the renderer instead of silently
creating replacement state.

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
whole-state save. The current version 2 backup UI remains compatible through a bounded
backend import; strict backup lifecycle design remains owned by TASK-0014.

Schema v3 adds immutable-terminal run attempts, bounded progress events,
approval reservations, and coordinator metadata. One active-attempt foreign
key is acquired within an immediate transaction, while a coordinator-specific
revision orders run projections independently from aggregate-state saves.
Import/reset operations reject an active run; reset clears run-ledger data.
Startup reconciliation distinguishes attempts that are safe to retry from
ones that may have dispatched and require manual review.

### Voice runtime

<code>voice-runtime/listener.py</code> is a bundled local Python listener. Setup
scripts prepare its runtime and optional higher-accuracy support. Renderer
voice interpretation also exists. Installation, microphone, and portal
behavior were not exercised in TASK-0001.

### External and operating-system boundaries

- Codex is an installed CLI process authenticated outside application state.
- Ollama is a local HTTP service and model catalog.
- Workspaces are user-selected filesystem roots.
- KDE/Wayland integration crosses application, window, input, portal, desktop
  entry, and XDG data/config boundaries.
- Git evidence is read from a selected workspace when available.

## Current run flow

1. The user creates or selects a task in the renderer.
2. The renderer sends a typed run intent containing only the run locator,
   agent, task owner, task, and run mode.
3. The backend loads current state, rejects invalid or paused subjects, derives
   policy, and atomically admits exactly one execute/review attempt or returns
   a deterministic busy/pending-approval result. There is no run queue.
4. An exact approved record is reserved by admission. It is consumed once only
   after successful provider startup; cancellation or startup failure before
   dispatch releases it, while uncertain post-dispatch recovery prevents
   replay.
5. The backend projects the task into its active lifecycle, derives workspace,
   capability limits, timeout, prompt, and sandbox from persisted state. It
   resolves exactly one catalog model, maps only OpenAI to Codex and Ollama to
   Ollama, requires that adapter to equal the persisted active provider, and
   dispatches exactly that registry adapter. Unsupported, missing, ambiguous,
   inactive, or unavailable identities fail closed without fallback.
6. The backend persists bounded ordered events, cancellation state, terminal
   outcome, output summary, usage, and workspace evidence. Terminal attempts
   cannot be updated.
7. The renderer displays authoritative snapshots/events and a global Stop
   control across navigation; generic state saves cannot overwrite run-owned
   task fields.

## Directional architecture

The target is a modular monolith with the Tauri backend as the authoritative
state and policy boundary. Later tasks should extract real modules only when
they carry behavior; TASK-0001 intentionally creates no empty placeholders.

### Directional renderer boundaries

- **App shell and navigation** — window-level layout and page routing.
- **Feature UI** — dashboard, agents, tasks, approvals, reminders, activity,
  models, voice, and settings.
- **Typed IPC adapter** — one renderer boundary for backend commands/events.
- **View state** — transient selection, form, display, and accessibility state.
- **Presentation components** — reusable controls without policy authority.

The renderer may request actions and display decisions. It must not mint
authorization, decide durable run truth, or become the sole owner of domain
state.

### Directional backend boundaries

- **Domain/state** — versioned agents, tasks, approvals, reminders, settings,
  runs, and audit records.
- **Persistence/migrations** — atomic storage, schema versions, backup import,
  recovery, retention, and migration evidence.
- **Policy/authorization** — normalized capabilities, approval matching,
  expiry, one-use consumption, replay resistance, and denial reasons.
- **Run coordinator** — one active run, deterministic no-queue admission,
  lifecycle transitions, cancellation, timeout, recovery, and a durable
  bounded ledger. This boundary is implemented by TASK-0005; later tasks may
  extract adapters without moving authority back to the renderer.
- **Provider registry** — provider identity, model capability, readiness, and
  truthful dispatch contracts. The common registry and contract are
  implemented by TASK-0006; later tasks harden adapter internals.
- **Codex adapter** — isolated CLI invocation and evidence parsing.
- **Ollama adapter** — local discovery, transport, tool loop, and workspace
  tools.
- **Workspace service** — canonical roots, path containment, snapshots, diffs,
  and change evidence.
- **Voice/system gateway** — intent normalization and policy-checked desktop
  actions.
- **Platform integration** — KDE, Wayland, XDG portals, desktop entries, and
  install/remove lifecycle.

These are responsibility boundaries, not prescribed filenames. Each owning
task must choose the smallest structure supported by the code at that time.

## Target domain ownership

| Data or decision | Current owner | Planned authoritative owner |
| --- | --- | --- |
| Agents and hierarchy | Backend SQLite aggregate; renderer manages semantics | Validated backend agent registry (TASK-0009) |
| Tasks and results | Backend SQLite aggregate plus run-ledger-owned lifecycle/results; renderer still manages broader task semantics | Backend domain store and run ledger |
| Approval records | Backend SQLite; backend-issued rows are authoritative and imported rows are expired history | Backend policy/approval store |
| Approval match/consume | Backend exact-match transaction | Backend exact-match transaction |
| Routing and review | Renderer | Backend scheduler/orchestrator |
| Active run | Backend system-wide coordinator and SQLite ledger; only live cancellation handles are in memory | Backend system-wide coordinator |
| Provider/model truth | Backend registry and exact active-provider/catalog-model resolution; renderer projects availability | Backend provider registry |
| Workspace evidence | Backend bounded evidence persisted per attempt | Backend durable evidence record |
| Reminders | Backend SQLite; renderer manages behavior | Backend passive reminder service |
| UI preferences | Backend SQLite for desktop; preview storage in browsers | Local settings store, with UI ownership where safe |

## Target hierarchy and flow

    Supervisor
      └── Team Leader
            ├── Senior Agent
            │     └── Specialist
            └── Senior Agent
                  └── Specialist

Specialists perform bounded work. Senior agents review specialist results.
Team leaders coordinate workload and escalation. The supervisor owns final
cross-team coordination. Titles alone grant no filesystem, terminal, network,
or system authority; capability and policy records do.

The planned system-wide execution flow is sequential:

1. accept and persist intent;
2. normalize and route it;
3. assess policy and obtain any exact approval;
4. atomically acquire the single-run slot with deterministic no-queue
   admission;
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
TASK-0006 establishes provider identity and the common contract; TASK-0007 and
TASK-0008 harden the Codex and Ollama adapters. TASK-0009 through
TASK-0012 establish hierarchy and orchestration. TASK-0013 and TASK-0014
modularize UI/data lifecycle. TASK-0015 and TASK-0016 own system actions and
KDE/voice integration. TASK-0017 through TASK-0020 complete bounded roles,
packaging, acceptance, and release.

Exact dependencies and gates are authoritative in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).
