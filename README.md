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
- a backend-authoritative agent registry with validated hierarchy, durable
  create/update/delete operations, and explicit default-template restoration;
- a backend-authoritative deterministic task router and one global sequential
  execute queue with durable workload, ordering, and routing evidence;
- a backend-authoritative sequential review pipeline with exact reporting-chain
  reviewers, structured verdicts, bounded revisions, and human fallback;
- a backend-authoritative single-run coordinator with a durable, bounded run
  ledger;
- versioned, bounded Git and non-Git workspace-change evidence captured around
  execution and persisted in the authoritative task/run ledger;
- a truthful Codex/Ollama provider registry with exact model/runtime dispatch
  and no silent fallback;
- a Linux Codex execution path with capability checks, explicit configuration
  isolation, bounded JSONL evidence, cancellation, and descendant cleanup;
- a bounded async local Ollama path with exact installed-model inspection,
  cancellable HTTP requests, and conflict-safe Linux workspace tools;
- schema-v10 typed Coding, Debugging, Browser Research, and Financial Analysis
  requests, immutable run tool contracts, backend-validated structured
  results, and exact stable-template routing;
- a schema-v11 passive reminder/event scheduler with IANA time-zone and DST
  resolution, deterministic recurrence/restart handling, app-owned timer/tray
  integration, and privacy-bounded XDG notification portal delivery;
- scoped agent/project/task/team memory with provenance, revision, retention,
  inspection/deletion, and an exact bounded per-run prompt bundle, plus
  explicit sequential task/run/review/human management-handoff evidence;
- a feature-modular React renderer with a typed desktop client, accessible
  navigation/dialog/tab/card primitives, and narrow-screen provider controls;
- schema-v8 lifecycle timestamps and durable bounded retention evidence,
  strict sanitized portable backup v4, and revision-bound backend monitoring
  with explicitly non-authoritative browser previews;
- one backend-owned canonical voice/system-action gateway with exact XDG/KWin
  target resolution, capability and one-use approval enforcement, normal
  sequential coding-task routing, and a bounded redacted action audit;
- a pinned, staged, cancellable offline voice runtime with a Vosk base path,
  optional hash-verified whisper.cpp support, bounded local audio handling,
  and explicit listener lifecycle projection;
- XDG-compliant localized desktop discovery and a backend-owned KDE
  RemoteDesktop session that monitors native closure, releases or closes after
  partial input, and follows the exact Full PC Control agent's authority;
- local workspace, task, agent, approval, reminder, and model-management UI;
- an included Python voice runtime and KDE-oriented install/remove scripts.

Important limits are documented in [CURRENT_STATE.md](CURRENT_STATE.md) and
[SECURITY_MODEL.md](SECURITY_MODEL.md). Desktop product state is now persisted
through typed Tauri commands and a schema-versioned backend database. Run
admission, task/run lifecycle projection, cancellation state, recovery, and
bounded evidence retention are backend-authoritative. Provider identity and
dispatch use a common backend contract. Codex process/protocol hardening is
implemented for the current Linux path; bounded Ollama transport and workspace
tools are implemented for the current Linux path. Deterministic task routing,
overflow handling, queue ordering, and execute-head admission are
backend-authoritative. Structured multi-level review/recovery and complete
workspace evidence are also backend-authoritative; live provider acceptance
remains a later prototype boundary.
Core specialist titles are not authority: the backend binds each typed task to
one stable template, derives an immutable role-specific tool ceiling at run
admission, and rejects cross-role routing, over-ceiling adapter behavior, or
malformed provider results. Coding
always needs one-use approval; Debugging is read-only; Browser Research uses
hosted search plus disposable private scratch; Financial Analysis has no web,
shell, user-workspace, account, credential, or transaction authority and uses
backend fixed-point calculations.
Due reminders are discovered by a non-AI backend timer and become in-app or
portal delivery evidence without creating a task or provider run. Portal
delivery is app-session behavior, not a persisted notification grant. The
backend resolves local civil time and recurrence, records missed/restart and
delivery outcomes, and exposes explicit needs-attention evidence when a
schedule cannot be resolved.
Structured memory is no longer granted by the legacy free-text agent field:
dedicated compare-and-swap commands own scoped records, and run admission
persists the exact bounded memory bundle and SHA-256 used by that attempt.
Management handoffs are derived transactionally from task, run, review, and
trusted human transitions; they remain sequential inspectable evidence rather
than an autonomous agent messaging channel.
Agent identity, lifecycle, role-derived authority, reporting relationships,
and template restoration are backend-authoritative; dashboard, agent, routing,
review, voice, reminder, approval, and settings projections consume active
registry entries instead of fixed IDs or display names.
Approval issuance, matching, expiry, trusted resolution, reservation, and
single-use consumption are backend-authoritative; imported legacy approval
rows remain non-authoritative history.
Browser preview storage is retained only as a non-authoritative preview path.
Renderer modularization does not move authorization, routing, review, run, or
provider authority out of the Rust backend.
Desktop backup export/import, retention, monitoring counts/pages, and local
activity deletion are also backend-owned. Activity-history controls do not
delete the immutable run/review ledger, and reset restores portable state
without claiming physical database/file erasure.
Voice interpretation submits one typed intent rather than invoking privileged
desktop commands. The backend resolves the active agent and exact target,
records authorization evidence before dispatch, refuses changed or ambiguous
targets, and never stores raw dictated/coding text in the system-action audit.
Voice setup uses private XDG data/config/cache/runtime roots and atomically
promotes only a fully validated staged release. High accuracy is optional: a
failed or absent whisper.cpp release does not disable the Vosk base listener.
No checked-in verification command downloads a model, captures audio, opens a
portal prompt, or sends desktop input.

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
| [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) | Third-party components and their licenses |

## Development

The locked frontend toolchain requires Node.js <code>^22.22.2</code>,
<code>^24.15.0</code>, or <code>>=26.0.0</code> — the floor its own locked
development dependencies declare. <code>.nvmrc</code> pins the exact version CI
uses. The verification routes also require npm, Bash,
Python 3, Cargo, rustfmt, Clippy, <code>pkg-config</code>, and the platform
SQLite development files. Linux Codex execution additionally requires
<code>bwrap</code> (Bubblewrap); absence or incompatibility fails closed rather
than launching an uncontained Codex process. Install JavaScript dependencies
without package lifecycle scripts:

```bash
npm ci --ignore-scripts
```

Run the deterministic fast characterization gate from the repository root:

```bash
npm run verify:fast
```

It runs 69 frontend characterization, interaction, accessibility, persistence,
coordinator, provider, registry, and orchestration tests, the TypeScript check,
7 Python voice-runtime tests, the Rust format check, and 207 locked/offline Rust
tests.
The full gate adds the
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

If `cargo-audit` or `cargo-deny` is available, the full route runs the Rust
advisory check; if neither is, it prints an explicit skip and reports the
result as **indeterminate** — absence of the tool is never reported as a pass.
The full route also runs the packaging validation, the third-party license
gate, and the staged install/removal test. With `VERIFY_STRICT=1` (set by CI)
the advisory, license, `shellcheck`, and packaging gates become hard failures
when their tooling is missing. `.github/workflows/ci.yml` installs `cargo-deny`,
`gitleaks`, `shellcheck`, and the Arch build environment and enforces them.

Start or build the desktop application only when the active task explicitly
owns that runtime action:

```bash
npm run desktop
npm run desktop:build
```

## Packaging and removal

Two install paths, both under `$HOME` unless noted:

```bash
bash install-kde.sh                 # build + user-local install / upgrade
bash uninstall-kde.sh               # remove the app, keep the database + voice models
bash uninstall-kde.sh --purge       # also delete ALL local data (asks for a typed PURGE)
```

`packaging/PKGBUILD` builds the Arch system package (`cd packaging && makepkg`).
Data removal is delegated to the binary so paths never drift:

```bash
ai-agent-control-center --print-data-paths        # list every owned location
ai-agent-control-center --stop-runtime            # stop the tray + voice listener
ai-agent-control-center --uninstall               # keep-data removal
ai-agent-control-center --purge --confirm PURGE   # full, irreversible purge
```

Keep-data removal preserves only the SQLite database and downloaded voice
models. Purge additionally clears the stored provider key and the KDE portal
restore token. The persistent KDE screen-cast / remote-desktop *permission* is
revoked in KDE System Settings, not by the app. These scripts mutate the local
desktop installation and must only be run as an explicitly approved live action.

`.github/workflows/ci.yml` runs a single sequential gate — frontend, Rust
(`cargo-deny` mandatory), scripts, third-party licenses, secret scan, and an
Arch `makepkg` + staged install/removal job — with no live AI/microphone/portal
action and no release step.

## License

AI Agent Control Center is proprietary, commercial, subscription software. See
[LICENSE](LICENSE) (`LicenseRef-proprietary`, Copyright © 2026 Aivars Rocens,
all rights reserved). Visibility of this repository grants no license. Bundled
third-party components keep their own permissive licenses; see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Working on the project

Every change starts from the current checkout and follows [AGENTS.md](AGENTS.md):
one approved task at a time, Phase A planning before Phase B implementation,
coherent multi-file slices, focused verification, and no automatic commit or
push. Use [planning/TASK_TEMPLATE.md](planning/TASK_TEMPLATE.md) for new task
records and record any deliberate change to a fixed project decision under
[`planning/decisions/`](planning/decisions/0001-fixed-project-decisions.md).
