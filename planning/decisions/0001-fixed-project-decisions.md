# Decision 0001: Fixed Project Decisions

- **Status:** Accepted
- **Date:** 2026-08-22
- **Scope:** Project-wide
- **Established by:** AI Agent Control Center Full Codex Task Package v2.0,
  reconciled with the current checkout in TASK-0001

## Context

The version 0.5.1 prototype, audit, reconstructed context, and roadmap contained
both current facts and future intent. Later tasks need durable constraints so
they do not restart the application, silently broaden authority, introduce paid
requirements, or reinterpret the intended hierarchy and execution model.

These decisions are binding direction. They do not claim that planned backend,
security, orchestration, voice, packaging, or release guarantees already exist.
[CURRENT_STATE.md](../../CURRENT_STATE.md) remains authoritative for present
implementation.

## Product decisions

1. Repair and complete the current application; do not restart or replace it
   with a new project.
2. Target Arch Linux, KDE Plasma, and Wayland first.
3. Preserve the organizational shape:
   Supervisor → Team Leader → Senior Agent → Specialist.
4. Preserve the named core agents and make their roles real and bounded:
   Supervisor, Development Team Leader, Debugging Agent, Research and Web
   Senior, Finance Senior, Operations Senior, Coding Agent, Browser Agent,
   Financial Agent, PC Control Agent, and Event and Reminder Agent.
5. Preserve both the installed Codex CLI and local Ollama execution paths.
6. Do not require a paid API key or paid external service.
7. Make backend state, policy, authorization, and run lifecycle authoritative.
   This is a target requirement; the 0.5.1 renderer still owns substantial
   state and approval workflow.
8. Keep one active product AI run system-wide and execute work sequentially
   until an explicit superseding decision changes that model.
9. Reminders and scheduling do not run AI models in the background.
10. Do not couple this project to unfinished Context for AI code.
11. Do not call the application version 1.0 or production-ready before
    TASK-0020 passes its complete sequential live acceptance gate.

## Development workflow decisions

1. Use one foreground Codex workflow for one approved task at a time.
2. Do not use background, parallel, delegated, recursive, worker, or subagent
   AI development workflows. Normal build/test tool parallelism is allowed.
3. Every task has read-only Phase A planning and Phase B implementation only
   after the exact task-specific approval phrase.
4. Phase B may edit all approved related files in coherent logical slices.
   Multi-file implementation is not multiple tasks when it delivers one
   approved outcome.
5. Verify after logical slice checkpoints and at the full task gate, not after
   every individual file edit.
6. Stop on the first unexplained failure, perform one focused diagnosis, and do
   not use blind retry loops.
7. Codex does not commit or push by default. The user owns final Git review,
   commit, push, and clean-tree closure.
8. Stop after the approved task; do not begin its successor automatically.

## Platform implementation decision

Before code changes involving KDE or other Linux components, inspect official
native mechanisms and their constraints. Depending on the feature, this
includes KWin APIs and window rules, KDE/Plasma integration, XDG portals,
desktop entries, and XDG base directories. If native behavior is insufficient,
the owning Phase A plan must compare bounded least-privilege workarounds rather
than assuming no workaround exists or adopting unrestricted desktop control.

## Consequences

- [ARCHITECTURE.md](../../ARCHITECTURE.md) describes a backend-authoritative
  modular monolith as direction but creates no placeholder modules.
- [SECURITY_MODEL.md](../../SECURITY_MODEL.md) treats the renderer, imports,
  provider output, voice transcripts, and IPC requests as untrusted.
- [IMPLEMENTATION_PLAN.md](../../IMPLEMENTATION_PLAN.md) keeps the twenty-task
  dependency chain and reserves production readiness for TASK-0020.
- Provider, system action, voice, packaging, and live acceptance changes remain
  in their owning tasks; their existence in the prototype is not permission to
  broaden TASK-0001.
- A convenience improvement cannot override a fixed safety or workflow
  decision implicitly.

## Supersession

The user may explicitly change any decision. A material change must:

1. identify the decision being changed and the evidence or goal motivating it;
2. receive approval before implementation;
3. add a numbered decision record that marks this record wholly or partly
   superseded;
4. update every directly affected authority and task dependency in the same
   approved change set.

Chat history, memory, an audit suggestion, or an implementation accident does
not supersede this record.
