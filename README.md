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
- a backend-owned, versioned SQLite store for desktop application state;
- execution paths for the installed Codex CLI and a local Ollama server;
- local workspace, task, agent, approval, reminder, and model-management UI;
- an included Python voice runtime and KDE-oriented install/remove scripts.

Important limits are documented in [CURRENT_STATE.md](CURRENT_STATE.md) and
[SECURITY_MODEL.md](SECURITY_MODEL.md). Desktop product state is now persisted
through typed Tauri commands and a schema-versioned backend database. Routing,
review orchestration, and durable run coordination are still prototype
boundaries. Approval issuance, matching, expiry, trusted resolution, and
single-use consumption are backend-authoritative; imported legacy approval
rows remain non-authoritative history.
Browser preview storage is retained only as a non-authoritative preview path.

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

The locked frontend toolchain requires Node.js <code>^20.19.0</code> or
<code>>=22.12.0</code>. The verification routes also require npm, Bash,
Python 3, Cargo, rustfmt, Clippy, <code>pkg-config</code>, and the platform
SQLite development files. Install JavaScript dependencies without package
lifecycle scripts:

```bash
npm ci --ignore-scripts
```

Run the deterministic fast characterization gate from the repository root:

```bash
npm run verify:fast
```

It runs 24 frontend characterization/persistence tests, the TypeScript check,
the Rust format check, and 41 locked/offline Rust tests. The full gate adds the
frontend build, Clippy, shell/Python/JSON checks, dependency-tree checks, and
production plus development npm audits:

```bash
npm run verify:full
```

Both routes print each command and stop on the first failure. They never start
Codex, Ollama, microphone capture, the Python listener, a KDE portal,
installation/removal, or desktop control. The full route accesses the npm
advisory service, so its audit result is a fresh time-dependent check rather
than offline evidence.

If `cargo-audit` is available, the full route audits
`src-tauri/Cargo.lock`. If it is unavailable, the route prints an explicit
skip and reports the Rust advisory result as **indeterminate**; absence of the
tool is never reported as a pass. Installing security tooling and making it a
mandatory CI gate belongs to TASK-0019.

Start or build the desktop application only when the active task explicitly
owns that runtime action:

```bash
npm run desktop
npm run desktop:build
```

The repository also contains `install-kde.sh` and `uninstall-kde.sh`. Those
scripts mutate the local desktop installation and must only be run as an
explicitly approved live action. TASK-0002 syntax-checks them but does not
execute them.

## Working on the project

Every change starts from the current checkout and follows [AGENTS.md](AGENTS.md):
one approved task at a time, Phase A planning before Phase B implementation,
coherent multi-file slices, focused verification, and no automatic commit or
push. Use [planning/TASK_TEMPLATE.md](planning/TASK_TEMPLATE.md) for new task
records and record any deliberate change to a fixed project decision under
[`planning/decisions/`](planning/decisions/0001-fixed-project-decisions.md).
