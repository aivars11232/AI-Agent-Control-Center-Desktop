# Start Here

This is the authority index for AI Agent Control Center. Read this file before
planning or implementing repository work.

## Release status

The current checkout is a functional version 0.5.1 development prototype. It
is **not production-ready**. Only successful completion of the TASK-0020 live
acceptance and release gate may change that statement.

## Required reading order

1. [START_HERE.md](START_HERE.md) — authority and conflict resolution.
2. [AGENTS.md](AGENTS.md) — development contract and approval gates.
3. [CURRENT_STATE.md](CURRENT_STATE.md) — verified current implementation.
4. [ARCHITECTURE.md](ARCHITECTURE.md) and
   [SECURITY_MODEL.md](SECURITY_MODEL.md) — present boundaries and intended
   direction.
5. [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) and
   [planning/TASK_STATUS.md](planning/TASK_STATUS.md) — task order and closure
   state.
6. The active task record or an instance of
   [planning/TASK_TEMPLATE.md](planning/TASK_TEMPLATE.md).
7. Applicable decisions under [`planning/decisions/`](planning/decisions/0001-fixed-project-decisions.md):
   [0001 fixed project decisions](planning/decisions/0001-fixed-project-decisions.md)
   and [0002 proprietary license and business model](planning/decisions/0002-proprietary-license-and-business-model.md).

## Source precedence

When two sources disagree, use this order from highest to lowest authority:

1. Observable evidence in the current repository checkout: code, tests,
   schemas, manifests, Git state, and newly captured runtime evidence.
2. Current authoritative repository documents listed in this file.
3. The active task's explicitly approved Phase A plan.
4. The AI Agent Control Center Full Codex Task Package v2.0 and its master
   roadmap.
5. The original audit, together with later verified Git corrections.
6. Reconstructed project context.
7. Memory, chat recollection, or unsupported assumption.

An explicit new user decision may supersede an existing project decision. If
it materially changes scope, architecture, security, dependencies, or task
order, stop and record the supersession in a decision document before
implementation. Approval of one task does not approve another task.

## Document ownership

| Concern | Authority | Content rule |
| --- | --- | --- |
| Human entry point | [README.md](README.md) | Concise product and contributor orientation |
| Current facts | [CURRENT_STATE.md](CURRENT_STATE.md) | Only evidence-backed, present-tense implementation claims |
| Architecture | [ARCHITECTURE.md](ARCHITECTURE.md) | Clearly separated current and directional structures |
| Security | [SECURITY_MODEL.md](SECURITY_MODEL.md) | Current enforcement, gaps, target invariants, and ownership |
| Roadmap | [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) | Task dependencies, milestones, and release gates |
| Execution state | [planning/TASK_STATUS.md](planning/TASK_STATUS.md) | Task status and evidence needed for closure |
| Development workflow | [AGENTS.md](AGENTS.md) | Binding planning, implementation, verification, and Git rules |
| Fixed decisions | [planning/decisions/0001-fixed-project-decisions.md](planning/decisions/0001-fixed-project-decisions.md) | Durable choices and their supersession rule |

Do not make a second document authoritative for the same concern. Link to the
owning document instead of copying details that can drift.

## Claim labels

Authority documents use these meanings even when a section does not repeat the
label on every sentence:

- **Current** — supported by the present checkout or fresh evidence.
- **Planned** — approved direction that is not yet implemented.
- **Historical** — retained evidence from an earlier audit or run.
- **Unverified** — plausible but not established by current evidence.

Never rewrite a planned or historical claim as current behavior. If current
evidence is missing, say so.

## Conflict procedure

1. Preserve the current checkout and any unexplained user changes.
2. Identify the conflicting claims and their evidence.
3. Apply the source-precedence order above.
4. Correct the lowest-authority source within an approved task, or mark the
   issue unresolved if the active task does not own it.
5. Record a material decision change and update every directly affected
   authority in the same approved slice.

Line numbers, file sizes, test counts, and runtime observations are evidence
snapshots, not durable interfaces. Reverify them when a later task depends on
them.
