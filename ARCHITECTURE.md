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
      |-- dynamic agent/group/hierarchy projection (src/agentRegistry.ts)
      |-- task queue/routing evidence projection (src/taskOrchestration.ts)
      |-- structured review-flow projection (src/reviewOrchestration.ts)
      |-- run snapshot/event projection (src/runCoordinator.ts)
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
      |-- capability policy + authoritative one-use approvals
      |-- authoritative single-run coordinator + durable bounded ledger
      |-- provider registry/common contract (provider_runtime.rs)
      |     |-- isolated Codex CLI runtime (codex_runtime.rs)
      |     |-- bounded Ollama HTTP runtime (ollama_runtime.rs)
      |     |-- descriptor-confined workspace tools (workspace_tools.rs)
      |-- filesystem/Git inspection
      |-- application/window/input control
      |-- Python voice listener process
      v
    Selected workspaces and local desktop environment

### Renderer

<code>src/App.tsx</code> currently combines page composition, import/export,
routing requests/evidence presentation, review, approval presentation, run
orchestration, and most
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
<code>ollama_runtime.rs</code>, <code>workspace_tools.rs</code>, and
<code>persistence.rs</code>. Those modules
own the versioned state contract, normalized action intents, fail-closed
capability evaluation, authoritative approval lifecycle, legal run transitions
and evidence bounds, provider identity/contracts/dispatch, isolated Codex
compatibility/process/protocol handling, bounded fixed-loopback Ollama
transport/discovery, descriptor-confined conflict-safe workspace edits,
validated agent identity/lifecycle/hierarchy operations, deterministic routing
and queue evidence, structured reporting-chain review and verdict validation,
SQLite
schema/repository, legacy migration, and typed state IPC. The provider registry
exposes only Codex and Ollama adapters and rejects provider/model mismatch
without fallback. Non-run
privileged command handlers consume backend authorization before their first
side effect; run approvals are reserved at admission and consumed at successful
provider startup.

### Persistence and migration

The desktop database is <code>application-state.sqlite3</code> below Tauri's
platform application-data directory. Migrations 0001 through 0006 establish
schema version 6 plus a migration ledger. Repository writes replace one
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
whole-state save. The current version 2 backup UI remains compatible through a bounded
backend import; strict backup lifecycle design remains owned by TASK-0014.

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
paused and detached for explicit repair, and absent/deleted defaults are not
silently recreated.

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

### Voice runtime

<code>voice-runtime/listener.py</code> is a bundled local Python listener. Setup
scripts prepare its runtime and optional higher-accuracy support. Renderer
voice interpretation also exists. Installation, microphone, and portal
behavior were not exercised in TASK-0001.

### External and operating-system boundaries

- Codex is an installed CLI process authenticated outside application state.
- Linux Codex execution requires Bubblewrap for user/PID namespaces,
  parent-death handling, capability drop, and descendant-lifecycle cleanup;
  the inner Codex sandbox remains the filesystem policy boundary.
- Ollama is a local HTTP service and model catalog.
- Workspaces are user-selected filesystem roots.
- KDE/Wayland integration crosses application, window, input, portal, desktop
  entry, and XDG data/config boundaries.
- Git evidence is read from a selected workspace when available.

## Current run flow

1. The renderer requests task creation or rerouting through typed IPC. The
   backend filters candidates by active state, workspace, capability, and exact
   provider/model readiness, applies deterministic scores, workload, and
   overflow policy, and persists candidate evidence plus the selected executor.
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
7. Codex dispatch revalidates its executable, streams the prompt on standard
   input, runs in an outer lifecycle-only Bubblewrap namespace plus an explicit
   inner Codex sandbox, parses bounded JSONL incrementally, and refuses a
   terminal outcome until descendant cleanup is established.
8. Ollama dispatch resolves names through <code>/api/tags</code> and metadata
   through <code>/api/show</code>, holds one cancellable async session and task
   deadline across its bounded tool loop, and exposes only descriptor-confined
   paginated/read/hash/create/patch workspace operations.
9. The backend persists bounded ordered events, cancellation state, terminal
   outcome, output summary, usage, and workspace evidence. Terminal attempts
   cannot be updated. A successful execution starts or resumes one normalized
   review flow when review is required.
10. The next stage is derived from the executor role and walks the exact active
    reporting chain sequentially: Senior, Team Leader, and Supervisor as
    applicable. Missing, repeated, inactive, or provider-unready identities
    move the flow to explicit human review without substitution.
11. Each agent stage receives one versioned, fingerprinted, bounded evidence
    request under read-only policy. Only one exact structured result with all
    required checks and matching identifiers can transition the flow; provider
    prose and embedded workspace evidence remain untrusted data.
12. Approval advances to the next level or completes the task. Requested
    changes enqueue a fresh policy-evaluated execution, capped at three
    revisions. Three invalid/failed stage attempts, an uncertain dispatch, the
    revision cap, or a Supervisor executor requires a natively confirmed human
    decision.
13. The renderer displays authoritative queue/run/review snapshots, routing
    evidence, and a global Stop control across navigation; generic state saves
    cannot overwrite run- or review-owned task fields.

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
| Agents and hierarchy | Validated backend registry with transactional CRUD, lifecycle, role-derived authority, and reporting constraints; renderer projects views | Backend agent registry |
| Tasks and results | Backend SQLite aggregate plus authoritative task creation/rerouting/queue state and run-ledger-owned lifecycle/results | Backend domain store and run ledger |
| Approval records | Backend SQLite; backend-issued rows are authoritative and imported rows are expired history | Backend policy/approval store |
| Approval match/consume | Backend exact-match transaction | Backend exact-match transaction |
| Routing and review | Backend owns routing, execute-queue admission, exact reviewer selection, structured stage transitions, revisions, human fallback, and recovery; renderer projects state and requests commands | Backend scheduler/orchestrator |
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
establishes structured review/revision/recovery, and TASK-0012 completes
workspace evidence orchestration. TASK-0013
and TASK-0014 modularize UI/data lifecycle. TASK-0015 and TASK-0016 own system actions and
KDE/voice integration. TASK-0017 through TASK-0020 complete bounded roles,
packaging, acceptance, and release.

Exact dependencies and gates are authoritative in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).
