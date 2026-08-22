# Current State

> **Classification: Current static and fresh non-live evidence.** This snapshot
> was refreshed for TASK-0002 on 2026-08-22 from starting commit
> <code>91db386df910d91e488b5710ab65490963579475</code> on branch
> <code>main</code>. TASK-0002 changes remain in the working tree pending user
> review and Git closure. Reverify details that may drift before relying on
> them in a later task.

This document owns statements about what is implemented now. Planned behavior
belongs in [ARCHITECTURE.md](ARCHITECTURE.md) and
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Evidence boundary

TASK-0001 established the repository authority baseline. TASK-0002 then ran
fresh frontend and Rust characterization tests, TypeScript and Vite checks,
Rust formatting and Clippy, syntax checks, dependency-tree checks, and npm
audits. These checks establish the non-live verification baseline described
below; they do not establish live runtime readiness.

TASK-0002 did **not** run Codex, Ollama, or another model/provider; capture
microphone input; import or start the Python listener; authorize a KDE/XDG
portal; execute install/remove scripts; build a desktop package; or perform a
desktop/system-control action. Earlier audit observations remain historical
unless this document identifies a fresh TASK-0002 result.

## Product and release identity

| Fact | Current evidence |
| --- | --- |
| Product | AI Agent Control Center |
| Version | <code>0.5.1</code> in <code>package.json</code> and <code>src-tauri/tauri.conf.json</code> |
| Release state | Development/pre-production prototype |
| Primary platform direction | Arch Linux, KDE Plasma, and Wayland |
| Frontend | React 19, TypeScript, and Vite |
| Desktop backend | Tauri 2 and Rust |
| AI execution paths | Installed Codex CLI and local Ollama |
| Mandatory paid API key | None in the current Codex/Ollama execution design |
| Production gate | TASK-0020, not yet reached |

## Repository shape

| Path | Current responsibility |
| --- | --- |
| <code>src/App.tsx</code> | Main renderer UI, domain types, defaults, persistence, routing, review, approval workflow, and IPC calls |
| <code>src/App.css</code> | Main application styling and responsive behavior |
| <code>src/voiceCommand.ts</code> | Renderer voice-command interpretation |
| <code>src-tauri/src/lib.rs</code> | Tauri commands, provider execution, workspace operations, native desktop control, voice process management, and Rust tests |
| <code>src-tauri/src/main.rs</code> | Desktop entry point |
| <code>src-tauri/tauri.conf.json</code> | Tauri window, security, bundle, and resource configuration |
| <code>voice-runtime/</code> | Python offline listener plus setup scripts |
| <code>install-kde.sh</code> / <code>uninstall-kde.sh</code> | KDE-oriented local install and removal scripts |

The implementation is concentrated: at this snapshot, <code>src/App.tsx</code>
has 8,908 lines, <code>src/App.css</code> has 1,952 lines, and the formatted
<code>src-tauri/src/lib.rs</code> with its expanded test module has 3,205 lines.
These counts are observations, not architectural requirements.

## Renderer behavior

The current renderer exposes nine pages:

1. Dashboard
2. Agents
3. Voice Control
4. Tasks
5. Approvals
6. Reminders
7. Activity
8. Models
9. Settings

It contains renderer-side types and logic for agents, tasks, workspaces,
capabilities, approval policy, routing, review, reminders, activity, models,
preferences, and retention. It can export and import a JSON backup through the
UI.

### Persistence

Core product state is stored in the WebView's <code>localStorage</code>,
including:

- agents and their nested tasks, activity, memory, roles, and policies;
- approval requests;
- models;
- reminders;
- application preferences and workspace definitions;
- task/activity retention preferences and routing/review preferences.

Normalization functions attempt to make older or partial values usable, but
there is no backend-owned durable state schema or migration ledger yet.
TASK-0003 owns that transition.

### Default hierarchy

The default data contains eleven agents and preserves the intended reporting
shape:

| Level | Agent | Reports to |
| --- | --- | --- |
| Supervisor | Supervisor | — |
| Team Leader | Development Team Leader | Supervisor |
| Senior Agent | Debugging Agent | Development Team Leader |
| Senior Agent | Research and Web Senior | Development Team Leader |
| Senior Agent | Finance Senior | Development Team Leader |
| Senior Agent | Operations Senior | Development Team Leader |
| Specialist | Coding Agent | Debugging Agent |
| Specialist | Browser Agent | Research and Web Senior |
| Specialist | Financial Agent | Finance Senior |
| Specialist | PC Control Agent | Operations Senior |
| Specialist | Event and Reminder Agent | Operations Senior |

The hierarchy and role names exist as renderer data. They are not yet a
backend-authoritative dynamic agent registry; TASK-0009 owns that outcome.

## Provider behavior

### Codex

The Rust backend inspects the installed Codex CLI and login status. A run uses
<code>codex exec --ephemeral</code>, selects <code>read-only</code> or
<code>workspace-write</code> from the requested file policy, sets the working
directory to the selected workspace, streams progress, supports cancellation
and timeouts, and captures changed-file/diff evidence.

### Ollama

The Rust backend talks to a local Ollama endpoint, discovers installed models,
requires tool capability for coding runs, supplies bounded workspace tools,
limits tool turns, supports cancellation and timeouts, and captures workspace
change evidence. Its tool path checks reject absolute and parent-traversal
paths and prevent access to the selected workspace's <code>.git</code>
directory.

### Model labels versus integrations

The renderer's editable model catalog can label models as OpenAI, Anthropic,
Google, Ollama, or Custom. That catalog is not evidence of five provider
integrations. The backend selects Ollama only when <code>modelProvider</code>
is <code>Ollama</code>; every other label currently uses the Codex execution
path. TASK-0006 owns a truthful provider registry and runtime contract.

No live provider connectivity or model output was checked in TASK-0001.

## Current run, routing, and review behavior

- The renderer assesses tasks, creates and resolves approval records, chooses
  routing, and manages specialist/reviewer state.
- The renderer sends an <code>AgentRunRequest</code> over Tauri IPC with run
  mode, workspace, model/provider, access levels, approval identifier,
  authorized scopes, destructive-action flag, and timeout.
- The backend maintains an in-memory active-run map for cancellation and
  dispatches the request to Ollama or Codex.
- Review requests are backend-checked for read-only files, no terminal access,
  no elevated scopes, and no destructive approval.
- Results, review outcomes, routing reasons, changed files, and diffs return to
  renderer-owned task state.

There is no durable backend run ledger or authoritative system-wide single-run
coordinator yet. TASK-0005 owns those guarantees. The current renderer behavior
must not be described as a complete authorization boundary.

## Native desktop and voice behavior

The Tauri invoke surface includes workspace selection/opening, application and
window control, keyboard/pointer/text actions, desktop-control status,
Codex/Ollama execution and cancellation, and voice-runtime setup/listener
commands. The Python voice runtime and its setup scripts are bundled as a
resource.

These paths exist in source, but TASK-0001 did not exercise them. Backend policy
consolidation for voice/system actions belongs to TASK-0015; offline voice and
KDE/XDG integration acceptance belongs to TASK-0016.

## Current safety enforcement

The backend currently:

- validates run mode, file-access and terminal-access enum values;
- rejects administrator terminal access;
- rejects unknown authorization scope strings;
- requires an approval identifier when authorized scopes are supplied;
- forces review runs to be read-only with no elevated authorization;
- blocks task text containing a bounded list of privileged, package, power,
  mount, permission, and system-control command patterns;
- resolves the selected workspace and constrains Codex with a Codex sandbox;
- constrains Ollama tools to paths below the selected workspace.

These are useful prototype controls, not a complete backend-authoritative
approval system. The renderer supplies the approval identifier, scopes, and
destructive flag, while the backend has no durable approval repository against
which to verify their issue, expiry, match, consumption, or replay state.
[SECURITY_MODEL.md](SECURITY_MODEL.md) owns the full boundary and gap list.

The Tauri configuration currently sets Content Security Policy to
<code>null</code>. TASK-0004 owns IPC/policy hardening and CSP.

## Verification inventory

TASK-0002 adds two Vitest files containing 18 deterministic frontend tests.
They characterize the current renderer safety assessment, approval
normalization, execution and review routing, stored preference/performance
normalization, and voice-command interpretation. The production functions are
exported as minimal test seams; their bodies and callers are unchanged.

The Rust module now contains nine passing unit tests: the four earlier tests
covering the Ollama connection, Qwen fallback parsing, workspace path
containment, and read-only tool exposure, plus five tests characterizing run
safety validation and voice configuration normalization. The Ollama connection
test uses an isolated loopback test server; it does not contact a live
provider.

The repository-root entry points are:

- <code>npm run verify:fast</code> — Vitest, TypeScript, rustfmt, and locked
  offline Rust tests;
- <code>npm run verify:full</code> — the fast route plus the Vite build,
  Clippy, shell/Python/strict-JSON syntax checks, npm/Cargo dependency trees,
  and production plus full npm audits.

Both fresh routes passed on 2026-08-22. The frontend build transformed 34
modules; both npm audits reported zero vulnerabilities. The full route produced
large dependency-tree output that was truncated in the captured console view,
but the process completed with exit status 0 and its final audit/status lines
were captured.

<code>cargo-audit</code> is not installed in the inspected environment. The
full route therefore reports the Rust advisory result as **indeterminate** and
does not represent the skip as a pass. Mandatory installed/CI security tooling
belongs to TASK-0019.

## Known gaps and roadmap ownership

| Gap | Owning task |
| --- | --- |
| Mandatory installed/CI Rust advisory tooling | TASK-0019 |
| Backend persistence and migrations | TASK-0003 |
| Authoritative approval/policy boundary and CSP | TASK-0004 |
| Single-run coordinator, lifecycle, ledger, and output contract | TASK-0005 |
| Truthful provider registry and hardened Codex/Ollama paths | TASK-0006–TASK-0008 |
| Dynamic hierarchy, routing, review, and workspace evidence | TASK-0009–TASK-0012 |
| Frontend modularity and data lifecycle | TASK-0013–TASK-0014 |
| Voice/system policy and KDE/XDG integration | TASK-0015–TASK-0016 |
| Bounded specialist capabilities and management handoffs | TASK-0017–TASK-0018 |
| Packaging, CI, live acceptance, and production gate | TASK-0019–TASK-0020 |

See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for exact sequencing.

## Historical reconciliation

The original audit predates Git initialization. The current repository is a Git
checkout whose initial public baseline is commit <code>9805c71</code>; the
audit's no-Git observation is historical, not current. Current inspection also
corrected stale references such as <code>uninstall.sh</code> (the repository
contains <code>uninstall-kde.sh</code>) and older page lists that omitted Voice
Control and Reminders. Current source evidence wins if those historical
artifacts conflict.
