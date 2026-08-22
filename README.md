# AI Agent Control Center

AI Agent Control Center is a local Tauri desktop application for defining AI
agents, assigning work, reviewing results, and controlling access to local
workspaces. The checked-in application is a functional **version 0.5.1
development prototype**. It is not production-ready; production readiness is
reserved for the TASK-0020 release gate.

Start with [START_HERE.md](START_HERE.md). It identifies the authoritative
project documents, their ownership, and the rule used to resolve conflicting
claims.

## Current implementation

The current checkout contains:

- a React 19 and TypeScript renderer built with Vite;
- a Tauri 2 and Rust desktop backend;
- execution paths for the installed Codex CLI and a local Ollama server;
- local workspace, task, agent, approval, reminder, and model-management UI;
- an included Python voice runtime and KDE-oriented install/remove scripts.

Important limits are documented in [CURRENT_STATE.md](CURRENT_STATE.md) and
[SECURITY_MODEL.md](SECURITY_MODEL.md). In particular, most product state and
several orchestration and approval decisions are currently renderer-owned and
stored in browser `localStorage`; the planned backend-authoritative design has
not yet been implemented.

The supported product direction is Arch Linux, KDE Plasma, and Wayland first.
Other platforms are not current release targets.

## Project authorities

| Document | Purpose |
| --- | --- |
| [START_HERE.md](START_HERE.md) | Reading order, source precedence, and conflict resolution |
| [AGENTS.md](AGENTS.md) | Binding development and verification workflow |
| [CURRENT_STATE.md](CURRENT_STATE.md) | Evidence-backed snapshot of what exists now |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Current architecture and directional boundaries |
| [SECURITY_MODEL.md](SECURITY_MODEL.md) | Current trust boundaries, known gaps, and target invariants |
| [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) | Ordered roadmap, dependencies, milestones, and release gates |
| [planning/TASK_STATUS.md](planning/TASK_STATUS.md) | Status and closure evidence for every roadmap task |

## Development

Install JavaScript dependencies from the lockfile and start the Tauri
development application:

```bash
npm ci
npm run desktop
```

Build the frontend or desktop bundle with the checked-in scripts:

```bash
npm run build
npm run desktop:build
```

The repository also contains `install-kde.sh` and `uninstall-kde.sh`. Those
scripts mutate the local desktop installation and must only be run as an
explicitly approved live action. No build, provider, microphone, portal,
installer, uninstaller, or desktop-control action was run while establishing
this documentation baseline.

## Working on the project

Every change starts from the current checkout and follows [AGENTS.md](AGENTS.md):
one approved task at a time, Phase A planning before Phase B implementation,
coherent multi-file slices, focused verification, and no automatic commit or
push. Use [planning/TASK_TEMPLATE.md](planning/TASK_TEMPLATE.md) for new task
records and record any deliberate change to a fixed project decision under
[`planning/decisions/`](planning/decisions/0001-fixed-project-decisions.md).
