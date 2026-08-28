# Implementation Plan

This is the authoritative repository roadmap for evolving the version 0.5.1
prototype into a release candidate. It preserves the current application and
uses twenty ordered, coherent tasks. Current implementation facts remain owned
by [CURRENT_STATE.md](CURRENT_STATE.md); execution state is tracked in
[planning/TASK_STATUS.md](planning/TASK_STATUS.md).

## Execution model

Every task follows the development contract in [AGENTS.md](AGENTS.md):

1. inspect the actual checkout and predecessor evidence;
2. complete read-only Phase A planning;
3. obtain the task's exact user approval;
4. implement the approved coherent change set in logical slices;
5. run focused verification after each slice and the full task gate;
6. report the exact diff and evidence without committing or pushing;
7. stop and let the user review, commit, push, and establish clean-tree closure;
8. let the successor's read-only preflight verify that closure from actual Git
   evidence and, during its approved Phase B, backfill any temporary tracker
   lag in the successor's normal implementation commit.

One foreground Codex workflow and one approved task are allowed at a time.
Within an approved task, every related file in a logical slice may be changed
together. The product also remains single-active-run and sequential unless a
later explicit decision changes that requirement.

## Dependency rule

The roadmap is a strict chain: each task after TASK-0001 depends on the
immediately preceding task. Do not skip a dependency because code for a later
feature already exists.

A successor may enter read-only Phase A when fresh preflight evidence verifies
every predecessor-closure condition below, even if the historical tracker still
temporarily says `PENDING USER`:

1. the predecessor tracker/evidence records Phase A `COMPLETE`, approval `YES`,
   Phase B `COMPLETE`, and verification `PASSED`;
2. the predecessor implementation commit is identified from actual Git history,
   not guessed from a commit message;
3. that commit is the checked-out `HEAD` or its ancestor;
4. that commit is reachable from `origin/main`;
5. the checked-out branch and `origin/main` have zero ahead/behind;
6. the working tree is clean; and
7. the commit scope matches the predecessor's reported task scope, with no
   unexplained intervening state that invalidates the evidence.

The successor Phase A records the exact commit and all fresh Git results but
changes no files. If any condition fails, it stops and reports the exact dirty,
divergent, unpushed, missing-verification, ambiguous-commit, or scope-conflict
evidence. It may not bypass predecessor implementation or verification, guess a
commit, overlap an active task, or authorize Phase B.

During the successor's approved Phase B, the ordinary status/documentation
slice backfills the predecessor's Git closure to `COMPLETE` and records the
verified historical evidence. That backfill belongs in the successor's normal
implementation commit; a separate closure-only commit is not normally required.
TASK-0020 has no successor, so its final Git/release closure is recorded in the
final release evidence and release commit.

## Ordered task index

| Task | Priority | Milestone | Depends on | Coherent outcome |
| --- | --- | --- | --- | --- |
| TASK-0001 | P0 | M0 | None | Authoritative project baseline and development contract |
| TASK-0002 | P0 | M0 | TASK-0001 | Reproducible verification and characterization baseline |
| TASK-0003 | P0 | M1 | TASK-0002 | Backend persistence and state-migration foundation |
| TASK-0004 | P0 | M1 | TASK-0003 | Authoritative approvals, capability policy, IPC, and CSP boundary |
| TASK-0005 | P0 | M1 | TASK-0004 | Single-run coordinator, task lifecycle, ledger, and bounded output |
| TASK-0006 | P0 | M2 | TASK-0005 | Truthful provider registry and common runtime contract |
| TASK-0007 | P0 | M2 | TASK-0006 | Codex runtime isolation, cancellation, and evidence hardening |
| TASK-0008 | P0 | M2 | TASK-0007 | Ollama discovery, transport, cancellation, and safe workspace tools |
| TASK-0009 | P1 | M3 | TASK-0008 | Dynamic agent registry and valid organizational hierarchy |
| TASK-0010 | P1 | M3 | TASK-0009 | Deterministic routing, queueing, workload, and sequential scheduling |
| TASK-0011 | P1 | M3 | TASK-0010 | Structured multi-level review, revisions, and recovery |
| TASK-0012 | P1 | M3 | TASK-0011 | Versioned, bounded, persisted Git/non-Git workspace evidence for execution and review |
| TASK-0013 | P1 | M4 | TASK-0012 | Frontend modularization, accessibility, and responsive operation |
| TASK-0014 | P1 | M4 | TASK-0013 | Data lifecycle, strict backup, retention, and truthful monitoring |
| TASK-0015 | P0 | M5 | TASK-0014 | Unified voice intent and system-action policy gateway |
| TASK-0016 | P0 | M5 | TASK-0015 | Offline voice runtime, KDE portal control, and XDG integration |
| TASK-0017 | P1 | M6 | TASK-0016 | Bounded Coding, Debugging, Browser, and Financial agent capabilities |
| TASK-0018 | P1 | M6 | TASK-0017 | Reminder scheduler, structured memory, and management handoff workspaces |
| TASK-0019 | P0 | M7 | TASK-0018 | Packaging, privacy-safe removal, release metadata, and CI security gates |
| TASK-0020 | P0 | M7 | TASK-0019 | Sequential live acceptance and version 1.0 release gate |

## Milestones

| Milestone | Tasks | Required exit result |
| --- | --- | --- |
| M0 — Controlled baseline | TASK-0001–TASK-0002 | Authoritative project truth, deterministic tests, formatting, and dependency baseline |
| M1 — Authoritative state and safety | TASK-0003–TASK-0005 | Backend state, fail-closed policy, one active run, and durable ledger |
| M2 — Reliable providers | TASK-0006–TASK-0008 | Truthful registry and hardened Codex/Ollama runtimes |
| M3 — Real orchestration | TASK-0009–TASK-0012 | Dynamic hierarchy, routing, review/recovery, and versioned bounded change evidence |
| M4 — Maintainable application and data lifecycle | TASK-0013–TASK-0014 | Modular accessible frontend and safe, truthful data lifecycle |
| M5 — Voice and KDE safety | TASK-0015–TASK-0016 | Unified voice policy and reliable privacy-safe KDE/Wayland integration |
| M6 — Core agent completion | TASK-0017–TASK-0018 | Bounded specialists, reminders, memory, and management handoffs |
| M7 — Packaging and release | TASK-0019–TASK-0020 | Reproducible packaging/CI and complete sequential live release evidence |

A milestone closes only when every task in its range has green required
verification, recorded commit/push evidence, and a clean worktree. A task may
define stricter gates; it may not weaken these gates.

## Delivery sequence

### M0 — Control the baseline

TASK-0001 installs repository authority without changing runtime behavior.
TASK-0002 then creates deterministic non-live verification and characterizes
the prototype before security or architecture changes.

### M1 — Move authority to the backend

TASK-0003 introduces versioned backend persistence and safe migration.
TASK-0004 makes approvals, policy, IPC, and CSP backend-authoritative.
TASK-0005 adds the system-wide single-run coordinator, lifecycle, durable
ledger, cancellation/recovery behavior, and bounded output.

### M2 — Make providers truthful and reliable

TASK-0006 defines one provider/runtime contract and exposes only executable
providers. TASK-0007 hardens Codex process isolation and evidence. TASK-0008
repairs Ollama discovery, transport, cancellation, and conflict-safe workspace
tools.

### M3 — Implement real orchestration

TASK-0009 makes the agent registry and hierarchy dynamic. TASK-0010 adds
deterministic routing and sequential scheduling. TASK-0011 implements
multi-level review, bounded revisions, and recovery. TASK-0012 brackets every
execution with bounded workspace capture, persists versioned Git/non-Git
evidence, and requires explicit human review whenever that evidence is not
complete and internally consistent.

### M4 — Make the application maintainable

TASK-0013 modularizes the renderer and completes its checked-in accessibility
and responsive contracts. TASK-0014 establishes strict portable backup/import,
bounded continuous retention with durable evidence, revision-bound monitoring,
and truthful reset/data-lifecycle behavior. Installed and packaged
platform acceptance remains part of TASK-0020.

### M5 — Make voice and KDE safe

TASK-0015 now routes canonical voice/system intents through one authoritative
policy, approval, exact-target, dispatch, and redacted-audit gateway; coding
requests enter the existing sequential queue. TASK-0016 now establishes pinned
and staged offline runtime reliability, bounded local listener behavior,
standards-aware XDG discovery, and lifecycle-safe native KDE RemoteDesktop
integration with bounded workarounds. TASK-0020 retains sequential live
microphone, portal, compositor, restored-session, installed, and packaged
acceptance.

### M6 — Complete bounded core agents

TASK-0017 now makes Coding, Debugging, Browser Research, and Financial Analysis
distinct backend-enforced profiles: typed requests and results, exact stable-
template routing, immutable per-run tool ceilings, forced one-use Coding
approval, read-only Debugging, hosted-search-only Browser Research, and local
fixed-point Financial Analysis with no external effects. TASK-0018 adds passive
reminders, structured memory, and management handoff workspaces without
background model execution.

### M7 — Package and release

TASK-0019 owns reproducible packaging, release metadata, CI security gates, and
privacy-safe removal. TASK-0020 runs every mandatory live case sequentially and
is the only production-readiness gate.

## Cross-task requirements

- The backend becomes authoritative for state, policy, authorization, and run
  lifecycle; renderer state never grants security authority.
- Codex and local Ollama remain supported. No mandatory paid service or API key
  may be introduced without a superseding user decision.
- The named hierarchy and core agents remain, but roles and capabilities must
  become real, bounded, dynamic, and testable.
- Native KDE, KWin, Wayland, XDG portal, desktop-entry, and filesystem
  mechanisms must be researched before implementing platform workarounds.
- Reminders never launch an AI model in the background.
- The project remains independent of unfinished Context for AI code.
- Planned behavior must never be presented as current evidence.

## Version 1.0 release gate

The application may be called version 1.0 or production-ready only when all of
the following are recorded:

- TASK-0001 through TASK-0019 are complete and manually closed;
- deterministic CI, security, and package gates are green;
- TASK-0020 ran every mandatory live acceptance case sequentially;
- approval replay, stale state, and renderer-forgery cases fail closed;
- Codex and Ollama execute only inside their approved boundaries;
- voice, KDE/portal, reminder, backup, remove, and purge cases have real
  evidence;
- no mandatory test is failed, silently skipped, or inferred from inspection;
- the exact tested commit and package are identified;
- release evidence and the final worktree are clean.

Until then, every project authority must call the application development or
pre-production software.

## Changing this roadmap

An explicit user decision may change scope, order, or a fixed decision. A
material change requires a new record under
[planning/decisions/](planning/decisions/0001-fixed-project-decisions.md) and
an update to this plan and the status tracker in the same approved task. Chat
history alone is not a durable roadmap change.
