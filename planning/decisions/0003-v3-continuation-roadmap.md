# Decision 0003: Version 3.0 stabilization continuation roadmap

- **Status:** Accepted
- **Date:** 2026-08-30
- **Scope:** Roadmap and execution order (task numbering, milestones, release
  gate sequencing)
- **Established by:** Owner decision supplied at the start of TASK-0021, backed
  by the external package
  `AI_Agent_Control_Center_Autonomous_10_Task_Continuation_Package_v3.0`
- **Relation to [Decision 0001](0001-fixed-project-decisions.md) and
  [Decision 0002](0002-proprietary-license-and-business-model.md):** additive.
  No fixed project decision is superseded. The application is still repaired and
  completed rather than restarted; the backend stays authoritative; Codex and
  local Ollama remain the only execution paths; no paid service is introduced;
  one foreground workflow and one approved task at a time still apply.

## Context

The [IMPLEMENTATION_PLAN.md](../../IMPLEMENTATION_PLAN.md) roadmap of twenty
ordered tasks (TASK-0001 – TASK-0020) reached the following state:

- TASK-0001 – TASK-0019 completed and verified; TASK-0019 pushed at commit
  `588baa0` with in-scope CI/packaging-gate stabilization through `135e583`.
- TASK-0020 (sequential live acceptance and version 1.0 release gate) began on
  real Arch Linux / KDE Plasma / Wayland prototype data and reached:
  - S0 backup / preflight: PASS
  - S1 deterministic gate: PASS
  - S2 real build + clean install: PASS
  - S3 first-launch legacy migration: **FAIL** — the real legacy
    `localStorage` holds two agents with `id = 5` (`Finance Agent`,
    `Financial Agent`); legacy normalization did not repair duplicate agent
    identities before authoritative validation, so the migration rolled back
    and the UI did not load.

TASK-0020 explicitly returns a product defect to its owning subsystem as a
bounded correction before acceptance resumes. The S3 defect is owned by the
persistence / agent-registry legacy-migration path (original TASK-0003 and
TASK-0009 territory).

## Decision

The twenty-task roadmap is **not reopened or renumbered**. A version 3.0
continuation of ten ordered stabilization tasks is adopted, each depending on
the previous one:

| Task | Priority | Coherent outcome |
| --- | --- | --- |
| TASK-0021 | P0 | Legacy state migration repair and duplicate identity recovery |
| TASK-0022 | P0 | Startup persistence recovery and S3 migration completion |
| TASK-0023 | P0 | Agent hierarchy, routing, queue, approvals, and policy live stabilization |
| TASK-0024 | P0 | Codex / Ollama cancellation, workspace evidence, and review stabilization |
| TASK-0025 | P0 | Voice, KDE portal, PC control, and notification stabilization |
| TASK-0026 | P1 | Reminders, memory, backup/restore, and data-lifecycle stabilization |
| TASK-0027 | P0 | Install, upgrade, remove, purge, and Arch package stabilization |
| TASK-0028 | P1 | Integrated desktop UI/UX and recovery acceptance |
| TASK-0029 | P0 | Full regression, security, CI, and release-candidate hardening |
| TASK-0030 | P0 | Final version 1.0 acceptance, release evidence, and handoff |

Execution rules are unchanged: read-only Phase A, the exact approval phrase,
approved Phase B in coherent slices, focused checks plus a full task gate, no
commit or push by the coding workflow. Continuation tasks additionally allow
autonomous progression through routine in-scope defects instead of stopping
after every file, slice, or test failure; human pauses are reserved for
destructive real-data actions, secrets, sudo, microphone/KDE portal
interaction, or a material boundary decision.

The version 1.0 release gate in [IMPLEMENTATION_PLAN.md](../../IMPLEMENTATION_PLAN.md)
is unchanged in substance. TASK-0020's live-acceptance intent is fulfilled
through TASK-0030 once TASK-0021 – TASK-0029 are green; TASK-0020 is treated as
`IN PROGRESS (blocked at S3)` rather than complete.

## Consequences

- [IMPLEMENTATION_PLAN.md](../../IMPLEMENTATION_PLAN.md) gains a continuation
  section listing TASK-0021 – TASK-0030 and pointing at this record.
- [planning/TASK_STATUS.md](../TASK_STATUS.md) gains a continuation tracker and
  per-task evidence sections, starting with TASK-0021.
- The application stays development / pre-production until TASK-0030 records a
  clean version 1.0 acceptance.
- The external v3.0 package is reference material for task intent and order; the
  live checkout, these authority documents, and the approved Phase A plan remain
  the higher authority per [START_HERE.md](../../START_HERE.md).

## Supersession

The owner may change the continuation order, merge or split a continuation
task, or restore the original single release gate with a new numbered decision
record and a coordinated update of [IMPLEMENTATION_PLAN.md](../../IMPLEMENTATION_PLAN.md)
and [planning/TASK_STATUS.md](../TASK_STATUS.md) in the same approved task.
