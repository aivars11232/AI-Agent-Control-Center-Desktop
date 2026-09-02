# Security Model

> **Current status:** version 1.0.0 passed the sequential live acceptance and
> release gate under TASK-0030, including live proof that a high-risk action
> fails closed unless a trusted desktop confirmation outside the renderer is
> answered. This document still distinguishes checked-in controls from planned
> security invariants; treat anything marked planned as not yet enforced. TASK-0004 establishes the
> backend-authoritative approval, action-policy, privileged IPC, and WebView
> boundary. TASK-0005 establishes authoritative single-run coordination,
> lifecycle recovery, and bounded ledger evidence. TASK-0006 establishes exact
> provider/model identity and no-fallback registry dispatch. TASK-0007 adds
> capability-gated Codex isolation, bounded protocol handling, and deterministic
> Linux descendant cleanup. TASK-0008 adds fixed-loopback Ollama transport,
> bounded discovery and cancellation, and descriptor-confined conflict-safe
> workspace tools. TASK-0009 adds backend-authoritative agent identity,
> lifecycle, role-derived authority, and reporting validation. TASK-0010 adds
> backend-authoritative task routing, routing evidence, durable queue order,
> and execute-head admission. TASK-0011 adds backend-authoritative structured
> review requests/results, exact reporting-chain selection, bounded revisions,
> trusted human gates, and deterministic recovery. TASK-0012 adds bounded
> descriptor-confined Git/non-Git evidence, redaction, immutable persistence,
> and fail-closed review eligibility. TASK-0013 adds a typed renderer desktop
> client and accessible feature boundaries without moving any authorization or
> durable-state authority into the WebView. TASK-0014 adds strict portable
> backup, bounded retention, and revision-bound monitoring. TASK-0015 adds the
> canonical backend voice/system-action policy, exact-target, approval, and
> redacted-audit boundary. TASK-0016 adds pinned staged offline releases,
> bounded local-audio/listener handling, private XDG paths, exact install
> cancellation, and lifecycle-safe capability-bound KDE portal sessions;
> TASK-0017 adds strict typed core-specialist requests/results, exact stable-
> template identity, immutable run tool ceilings, fixed-point finance,
> disposable private scratch, and backend rejection of cross-role or external
> effects. TASK-0018 adds backend-owned passive schedule/delivery evidence,
> exactly scoped structured memory and immutable per-run bundles, and bounded
> sequential management handoffs; later tasks still own packaging and live
> acceptance.

## Security objective

The application must let a user delegate bounded work without allowing model
output, renderer state, imported data, or an ambiguous approval to expand
authority. The local backend must remain the decision point for durable state,
authorization, workspace scope, provider execution, and native system actions.

## Assets

- user-selected workspace files and Git state;
- task text, scoped memory, model output, review feedback, reminders/events,
  notification evidence, and management handoffs;
- agent definitions, capabilities, policies, approvals, and run records;
- local provider authentication or runtime state;
- clipboard, microphone, keyboard, pointer, applications, windows, and desktop;
- local backups, logs, settings, and installation artifacts.

Credentials, tokens, user content, and raw provider data are sensitive even
when stored locally.

## Trust boundaries

| Boundary | Trust treatment |
| --- | --- |
| User interaction | Intent source, but still subject to explicit scope and confirmation |
| React renderer/WebView | Untrusted for authorization; may be compromised or hold stale/tampered state |
| Imported backup and <code>localStorage</code> | Untrusted serialized input requiring validation and migration |
| Tauri IPC | Untrusted request boundary; backend must authenticate semantics, not just types |
| Rust backend | Authoritative agent registry, task routing/queue, specialist request/tool/result contracts, structured review/revision, workspace evidence, passive scheduler/delivery policy, structured-memory selection, sequential management handoffs, approval/action-policy, canonical system-action gateway/audit, provider/model dispatch, single-run lifecycle, ledger, and persistence boundary |
| Codex/Ollama output | Untrusted content and action proposals |
| Selected workspace | Sensitive bounded filesystem root |
| External/local provider process | Separate process/service with its own failure and trust model |
| Voice transcript | Untrusted and potentially misrecognized intent |
| KDE/Wayland/XDG interfaces | Privileged integration boundary requiring native consent and scoping |

## Current controls

Static inspection and deterministic tests found these implemented controls:

- strict typed action intents and a unified backend capability/policy evaluator;
- backend rejection of invalid, missing, paused, wrong-task, and ineligible
  review agents;
- backend derivation of run workspace, model/provider, capabilities, timeout,
  prompt context, scopes, and destructive classification from persisted state;
- exact backend resolution of one catalog model to one registered runtime,
  with missing, duplicate, unsupported, active-provider mismatch, and missing
  adapter cases rejected without fallback;
- a common typed provider request, event, cancellation, result, evidence,
  capability, and error contract implemented by the Codex and Ollama adapters;
- one provider-registry status projection whose catalog bindings explicitly
  mark Anthropic, Google, and Custom as non-executable;
- policy-v5/intent-v3 fingerprints that bind provider/model identity, canonical
  exact system actions and any
  exact review flow/stage/round/level/request context, invalidating older or
  mismatched approvals instead of inheriting authority across a decision;
- a schema-v2 request/approve/deny/expire/validate/consume lifecycle bound to
  exact agent, task, workspace, intent, and policy fingerprints;
- trusted native confirmation that identifies the exact normalized action or
  protected privilege increase before it is recorded;
- atomic one-use approval consumption with malformed, stale, expired,
  mismatched, non-authoritative, and replayed records rejected;
- bounded approval issuance that fails closed at 10,000 retained records;
- generic renderer saves that preserve backend approval rows and cannot mint or
  overwrite authorization;
- read-only/no-terminal/no-elevation constraints for review runs;
- versioned <code>ReviewRequestV1</code>/<code>ReviewResultV1</code> contracts
  bound to one flow, task, execution, round, level, stage, evidence set, and
  SHA-256 request fingerprint;
- duplicate-key, unknown-field, trailing-text, Markdown-fence, missing/both
  verdict, stale-binding, unknown-evidence, incomplete-approval, and malformed
  review output rejection before any verdict transition;
- backend-only sequential reviewer selection through the executor's exact
  active reporting chain, with distinct levels/identities, exact provider/model
  readiness, no substitution, and no renderer-scored reviewer authority;
- normalized schema-v6 review flow/stage state, immutable terminal attempts,
  three agent attempts per stage, three execution revisions, fresh queue and
  policy/approval evaluation, and conservative legacy/restart recovery;
- trusted native KDialog confirmation for human review decisions, rechecked
  against the current review revision before the transaction commits;
- rejection of a bounded list of privileged, package, power, permission, mount,
  and system-control patterns in task text;
- selected-workspace resolution;
- bounded Codex executable/version/flag/feature/login probes, sanitized
  readiness reporting, and executable-identity revalidation before launch;
- explicit ephemeral Codex configuration that ignores user config/rules,
  disables MCP, multi-agent, plugins/apps/hooks and shell network, streams the
  private prompt over standard input, and selects only <code>read-only</code> or
  <code>workspace-write</code> from backend policy;
- Linux Codex lifecycle containment with a Bubblewrap user/PID namespace,
  parent-death handling, dropped capabilities, private <code>/proc</code>,
  process-group signalling, bounded TERM/KILL escalation, and required cleanup
  before terminal completion;
- incremental bounded Codex JSONL parsing that accepts only completed agent
  messages as final output, derives response/usage evidence, curates progress,
  and assigns distinct protocol, output-limit, compatibility, and cleanup
  failures;
- fixed numeric-loopback Ollama transport with proxies, redirects, retries,
  referers, and connection reuse disabled; bounded request/decoded-response
  bodies; strict JSON content checks; and one task deadline;
- installed-model discovery through <code>/api/tags</code> plus bounded
  <code>/api/show</code> inspection, with normalized per-model capability,
  context-length, availability, and failure evidence;
- cancellation that aborts and awaits the active Ollama HTTP task before
  returning, preventing a detached request worker;
- Linux workspace tools anchored to one opened root descriptor, with
  <code>openat2</code> beneath/no-symlink/no-magic-link resolution and a
  component-wise <code>openat</code> fallback only when that syscall is absent;
- stable paginated listings, bounded ranged UTF-8 reads with full hashes,
  create-only files/directories, and ordered hash-preconditioned patches;
- same-directory synchronized temporary-file commits using no-replace or
  exchange rename semantics, with displaced-inode/content revalidation and
  rollback rather than silent overwrite on conflict;
- per-run cancellation flags and bounded timeouts;
- versioned before/after workspace evidence captured around every execute
  dispatch and persisted for success, cancellation, timeout, and failure;
- descriptor-confined traversal that never follows symlinks/special files,
  uses <code>.git</code> metadata directories as the sole unconditional name
  exclusion, hashes bounded regular files, and reports every timeout, size,
  entry, command, or unsupported-case limit;
- hardened direct Git porcelain-v2/staged/unstaged inspection with optional
  locks, lazy fetch, pagers, hooks, fsmonitor, external diffs, textconv, and
  configured clean/smudge/process filter helpers disabled;
- typed classification for staged, unstaged, untracked, deleted, renamed,
  binary, redacted, and non-Git cases, with sensitive hashes/content and binary
  raw data omitted from persistence;
- agent-review approval gated on a complete internally valid structured record
  whose redacted compatibility paths/diff match the run fields; every partial,
  unavailable, binary, redacted, conflicted, or inconsistent case requires
  human review;
- typed, bounded backend validation for persisted application state;
- dedicated revision-checked agent create/update/logical-delete/template-restore
  IPC with renderer attempts to mutate registry structure through generic saves
  rejected;
- role-derived authority and active-manager validation that rejects
  self-parenting, cycles, dangling/inactive managers, and incompatible
  reporting edges;
- migration quarantine that pauses and visibly detaches invalid legacy agents,
  plus durable tombstones that prevent deleted defaults from silently
  reappearing;
- legacy duplicate-identity repair that keeps the first occurrence canonical,
  re-keys and quarantines later duplicates as a duplicate-id registry issue, and
  moves only references owned by the re-keyed instance so a duplicate cannot
  inherit another agent's approvals, reviews, routing, or reporting authority;
- policy, routing, review, voice, approval, reminder, dashboard, and settings
  projections that exclude non-active agent identities;
- deterministic routing that hard-filters inactive/paused, workspace,
  capability, provider/model, and Ollama-tool-ineligible candidates; records
  score components, workload, disqualifications, winner, reason, and manual
  override; and never lets selected routing bypass a hard filter;
- dedicated revision-checked task creation/rerouting/hold/resume/reset IPC,
  with generic renderer saves unable to create, remove, relocate, reroute, or
  forge executor, queue, lifecycle, and routing-evidence state;
- schema-v10 duplicate-key-free, unknown-field-denying, size-bounded specialist
  requests, immutable admission contracts, canonical SHA-256 request binding,
  and structured result validation protected from generic renderer saves;
- exact stable-template/category routing for Coding, Debugging, Browser
  Research, and Financial Analysis; generic task text and manual selection
  cannot cross those identities or grant a core profile;
- forced one-use approval and selected-workspace-only authority for Coding,
  with destructive classification bound to declared delete/rename, optional
  hosted research bound to the exact request, Ollama create/modify tools
  filtered before execution, and all observed mutation classes checked before
  successful completion;
- read-only Debugging with exact requested-check result binding, zero observed
  workspace changes, and separately created Coding work for any fix; Coding's
  Senior review requires the exact stable Debugging template rather than a title;
- hosted-search-only Browser Research in a unique mode-0700 private scratch
  directory, HTTPS/domain/source-count validation, zero scratch changes, and
  backend rejection of form, authentication, upload/download, purchase,
  account, or other external-effect authority;
- Financial Analysis with no web, shell, user-workspace, account, credential,
  trading, transfer, purchase, or autonomous-decision authority; bounded
  decimal strings use checked fixed-point arithmetic and half-even rounding,
  and provider assumptions/values must exactly match the request/backend
  results;
- dedicated revision-bound schedule IPC with generic saves unable to create,
  edit, delete, or forge reminder/event, recurrence, portal-policy, occurrence,
  delivery, or issue evidence;
- IANA local-civil-time resolution with deterministic fold/gap policy,
  recurrence anchored to the original civil time, occurrence idempotency,
  restart reservation reconciliation, bounded missed-event accounting, and no
  provider/run invocation from the due path;
- privacy-bounded XDG notification delivery whose authority is derived by the
  backend from the current schedule, whose failure remains inspectable, and
  whose grant/delivery evidence is excluded from portable backup;
- exactly-one-of agent/project/task/team memory scope validation, compare-and-
  swap CRUD, provenance/revision/retention events, expiry filtering, and
  explicit deletion without generic-save authority;
- deterministic memory selection limited to the admitted agent, reporting
  team, workspace, and task, with a maximum 128-record/64 KiB canonical bundle
  and SHA-256 persisted immutably before the exact prompt is dispatched;
- bounded idempotent management handoffs appended only by authoritative task,
  run, review, and trusted-human transactions, with source/owner/evidence
  bindings and sequence validation that prevents review evidence before an
  assignment/execution prefix;
- schema-versioned SQLite persistence with foreign keys, integrity checks,
  explicit migration evidence, atomic writes, and stale-revision rejection;
- one durable global execute queue ordered by priority, monotonic enqueue
  sequence, owner ID, and task ID, with queue age preserved across hold,
  reroute, and pre-dispatch failure;
- immediate-transaction admission of only the execute-queue head and at most
  one execute/review attempt across all renderer entry points;
- legal backend-only run transitions, immutable terminal attempts, bounded
  progress/output/evidence, explicit truncation metadata, and continuous
  terminal-history pruning;
- approval reservation at admission and one-use consumption only after the
  provider startup boundary, with pre-dispatch release and uncertain-dispatch
  replay prevention;
- durable cancellation state and startup reconciliation that distinguishes
  safe-to-retry from manual-review-required interrupted attempts;
- generic renderer saves that preserve run-owned task lifecycle/results and
  cannot manufacture or overwrite active-run truth;
- fail-closed desktop startup/save behavior when persistence is unavailable;
- Unix application-data directory/file modes restricted to the current user;
- one-time legacy migration that commits before renderer cleanup and refuses
  malformed input without partial state;
- downgrade of pending/approved legacy approvals to expired records whose
  database rows remain <code>authoritative = 0</code> after schema upgrade;
- one renderer voice submission surface whose backend resolves the active
  agent/workspace and exact XDG desktop-entry, XDG user-directory, or KWin
  internal-window target before policy evaluation;
- exact-target approval retry binding, forced one-use confirmation for Close,
  Cut, and Delete, and refusal of unknown, ambiguous, changed, or disappeared
  targets without fuzzy, caption, broad-process, or implicit active-window
  fallback;
- a maximum-10,000 schema-v9 action audit that records hashed intent/policy
  evidence and exact safe target identifiers before dispatch, records terminal
  or uncertain outcome, protects nonterminal rows, and stores dictated/coding
  content only as SHA-256 and length;
- authorization before provider, workspace-open, application/window,
  keyboard, clipboard-via-keyboard, pointer, text-input, microphone, portal,
  and voice-installer side effects;
- production CSP restricted to local application and Tauri IPC sources, frozen
  JavaScript prototypes, and a main-window capability containing only event
  listen/unlisten core permissions.

These controls establish the TASK-0004 authorization boundary, TASK-0005
run-coordination boundary, TASK-0006 provider-identity boundary, TASK-0007
Codex process/protocol boundary, TASK-0008 Ollama transport/workspace-tool
boundary, TASK-0009 agent-registry boundary, and TASK-0010 routing/queue
boundary, the TASK-0011 structured-review boundary, the TASK-0012 workspace
evidence boundary, the TASK-0014 backup/retention/monitoring boundary, and the
TASK-0015 voice/system-action boundary, TASK-0016 offline voice/KDE lifecycle
boundary, TASK-0017 specialist-contract boundary, and TASK-0018 passive-
scheduler/structured-memory/sequential-handoff boundary. They do not establish
production readiness or the later live provider/platform guarantees.

## Known current gaps

### State integrity and recovery

Desktop core domain state and agent registry now use a backend-owned SQLite
transaction boundary, schema/migration ledger, integrity check, and
compare-and-swap revision. The renderer cannot fall back to WebView storage
after desktop persistence starts or fails. Portable backup v4 rejects
oversized, deeply nested, duplicate-key, unknown-field, trailing, unsupported,
and future-schema input before mutation. Export/import strips active approval
authority and run/review/provider/portal/notification/handoff/voice-runtime
authority; apply is
revision checked, idle-run guarded, natively confirmed, and atomic. Legacy v2
and v3 imports cross the same sanitizer. The browser preview remains explicitly
non-authoritative.

Schema v10 persists portable typed task requests and non-portable immutable run
contracts/results. Generic saves cannot mutate them. Untouched legacy seed
profiles narrow during migration, while customized rows are preserved. A
pre-v10 core task without a typed request remains visible but fails closed at
routing/policy/run admission; the user must review it and create a new typed
task rather than receiving inferred authority from old prose.

Schema v11 persists portable schedules and unexpired scoped memory but keeps
portal delivery in-app after import and omits portal grants/delivery evidence,
management handoffs, and immutable per-attempt bundles with the run ledger.
Legacy reminder/memory migration is one-time and inspectable; invalid schedules
become needs-attention evidence rather than executable input. Generic renderer
saves cannot recreate the old free-text memory authority or mutate scheduler,
memory, handoff, or run-bundle records.

Schema-v8 retention uses backend time, normalized timestamps, active-record
protection, 500-row per-domain passes, latest-100 evidence, a 15-minute timer,
and a one-minute backlog retry. Backward clock movement skips age deletion and
is recorded. Monitoring queries bind application/task/run/review/lifecycle
revisions transactionally and stale tuples fail closed. Local activity clear
requires native confirmation and cannot clear the authoritative run/review
ledger.

Privacy-safe removal (TASK-0019) is backend-authoritative:
`src-tauri/src/lifecycle_removal.rs` enumerates every owned Linux location
across both the `com.aivarsrocens.aiagentcontrolcenter` bundle identifier
(database, WebKitGTK data/cache/cookies, debug logs) and the
`ai-agent-control-center` namespace (voice models/venv, voice config and the
KDE portal restore token, voice download cache, compositor/voice runtime).
`--stop-runtime` escalates `SIGTERM` then `SIGKILL` over the tray process and an
orphaned voice listener; `--uninstall` keeps only the database and voice models;
`--purge --confirm PURGE` removes every location, clears the stored provider key
from the OS keyring, deletes the portal restore token, and is idempotent. A
safety check refuses any path that is not namespaced and strictly below a known
XDG root. The persistent KDE screen-cast / remote-desktop *permission* is owned
by KDE System Settings and every removal path states that it cannot revoke it.
Live installed removal, purge, and packaged recovery acceptance remain with
TASK-0020.

Exact approval binding stores normalized intent JSON in the local database.
Canonical text-input and coding-task approvals contain SHA-256 plus byte length,
not the raw content; migration 0009 expires and redacts earlier desktop-text
approval intents. Raw typed/coding content remains transiently present in the
validated submission/task-creation path, and a successfully created coding
task intentionally persists its title in normal task state. Unix database
permissions restrict local state to the current user. Resolved/consumed
approval and terminal action-audit history follow activity retention, while
pending/current/dispatched authority is protected. Portable backup omits
authorization intents and the system-action audit.
Configured standard-folder and coding-workspace paths are represented in that
audit only by SHA-256 target bindings, not raw local paths.

### Residual IPC and web content work

The current privileged invoke surface is policy-gated and production CSP plus
Tauri core permissions are narrowed. TASK-0013 centralizes renderer invokes and
event listeners behind a typed desktop client, but the backend remains the
authority and persistence keeps its revision-aware injected invoke boundary.
TASK-0015 removes the direct renderer application/window/input command surface
and exposes one typed gateway plus a read-only redacted audit query. TASK-0016
implements deterministic offline listener/portal integration reliability.
TASK-0019 adds the sequential `.github/workflows/ci.yml` gate: `cargo-deny`
(advisories, licenses, bans, sources), `scripts/check-licenses.sh` (fails on any
non-permissive dependency license), `gitleaks`, `shellcheck`, packaging
validation, and a containerised Arch `makepkg` + staged install/removal test,
all with no live AI/microphone/portal/system action and no release step.
`scripts/verify-full.sh` runs the same gates locally and marks the Rust advisory
result **indeterminate** when the tooling is absent rather than passing.
TASK-0020 owns live installed and packaged platform acceptance. Current source
tests do not replace those later live gates.

### Heuristic task-text checks

Substring checks for command terms can be bypassed, can misclassify benign
text, and do not authorize the eventual concrete tool invocation. They are a
prototype guardrail, not a parser or policy engine. Later enforcement must
authorize normalized operations and arguments at the point of use.

### Provider and run truth

The executable registry contains Codex and Ollama only. OpenAI catalog entries
map to Codex, Ollama entries map to Ollama, and Anthropic, Google, and Custom
are explicitly unavailable. Exact backend identity and active-provider
matching now prevent silent substitution. Codex compatibility, JSONL evidence,
cancellation/timeout escalation, and normal/session-detached descendant cleanup
are covered by a fake CLI under Linux Bubblewrap. The outer namespace
bind-mounts the host tree and therefore supplies lifecycle containment rather
than filesystem authority; the inner Codex sandbox supplies that authority.
The inspected CLI cannot enforce a zero-file-access mode or separate safe from
user terminal access, write/full file levels both use workspace-write, and an
absolute alternate Codex executable cannot be universally excluded. Live Codex
authentication/model behavior and non-Linux support remain unverified for
TASK-0020. Deterministic numeric-loopback fake-server and temporary-workspace
tests cover Ollama discovery, transport, cancellation, large/conflicting edits,
path escape, and bounded tool turns. They do not establish live Ollama or
packaged-platform behavior, which remains with TASK-0020. Descriptor-confined
workspace tools fail closed outside Linux.
Browser Research is Codex-only because its contract requires hosted search;
Financial Analysis may use Ollama without tools or Codex in a disposable
private scratch directory. The zero-user-workspace profiles therefore do not
claim the current Codex CLI can enforce a literal no-filesystem mode. No live
specialist provider run, live source retrieval, transaction, or consequential
external action was exercised by TASK-0017's deterministic checks.
Codex Coding's workspace-write sandbox also cannot pre-restrict create,
modify, delete, and rename as separate operation classes, and safe terminal is
not a backend command allowlist. Authoritative post-run evidence rejects an
undeclared observed mutation from successful completion but does not undo it;
unexpected changes require user review/recovery. Ollama is narrower: typed
create/modify tools are filtered before execution, hidden calls fail, and
contracts requiring shell checks or delete/rename are rejected at routing.

### Verification coverage

The checked-in non-live suite contains 69 frontend tests and 207 Rust tests. It
covers frontend characterization, typed IPC mapping, keyboard/dialog/tab
interaction, deterministic axe checks, responsive-style contracts, plus backend
policy, authorization, run
coordination/recovery/bounds, provider identity/fake-adapter dispatch, Codex
compatibility/command/protocol/descendant-process cases, strict IPC,
CSP/capability, Ollama fake-server transport/discovery/cancellation, safe
workspace operations, agent CRUD/reopen/template/lifecycle/hierarchy behavior,
routing eligibility/scoring/overflow evidence, global queue ordering/head
admission/restart/recovery, strict bound review protocols, exact reporting-chain
selection, revision/attempt caps, human fallback, review cancellation/restart,
schema-v5 review migration, persistence validation, migration, corruption,
concurrency, rollback, Git/non-Git add/modify/delete/rename/binary/staging,
explicit collection limits, symlink containment, redaction, disabled Git
helpers, structured persistence, and review eligibility
cases. Nineteen TASK-0017 Rust tests additionally cover strict specialist
schemas/hashes/ceilings/results, fixed-point arithmetic, exact
checks/assumptions, cross-role routing/review, schema-v10 persistence and
reroute, scratch cleanup, and hidden Ollama tool rejection; three frontend
tests cover composer/profile contracts. Twenty-one TASK-0018 Rust tests cover
local time/DST/due windows/anchored recurrence, restart and delivery evidence,
passive model behavior, schema-v11 migration, backup-v4 sanitization, scoped
memory/provenance/exact bundles, sequential handoffs, bounded retention, and a
fake notification sink; three frontend tests cover due-window and management-
handoff presentation. Ten TASK-0019 Rust tests cover the owned-data-location
inventory, the keep-data / purge scope split, idempotent re-purge, the
hostile-<code>XDG</code> guard, and `SIGTERM → SIGKILL` process escalation;
`scripts/staged-install-test.sh` and `scripts/check-licenses.sh` run in the full
gate. These checks do not establish live end-to-end, packaging, upgrade,
installed removal, or live acceptance. Rust advisory status is **indeterminate**
locally when <code>cargo-audit</code>/<code>cargo-deny</code> is unavailable;
CI installs and requires them.

## Target security invariants

The agent-registry, task-orchestration, and voice/system-action subsets of
invariant 1, the
approval/action subset of invariants 2 through 5, the typed-routing-input
subset of invariant 2, the queue/coordinator subset
of invariant 6, the structured-review binding and recovery subsets of
invariants 2, 3, 6, and 9, the Ollama workspace and TASK-0012 evidence subsets
of invariants 7 and 11, and the
provider-identity/no-substitution plus Codex/Ollama adapter subsets of invariant
8, and the canonical intent/exact-target/redacted-audit subsets of invariants
9 through 11 are implemented for the current command surface. The full integrated
invariants remain the release target:

1. The backend is the sole authority for durable domain state and action
   authorization.
2. Every IPC payload is parsed, normalized, bounded, and validated; unknown
   fields or values fail closed where compatibility permits.
3. Capabilities grant a maximum envelope. A task, approval, prompt, agent role,
   or provider may only narrow that envelope.
4. An approval binds one user-confirmed action or run to an exact subject,
   task, workspace, normalized operation, scopes, risk, and expiry.
5. Approval consumption is atomic, durable, one-use, and replay-resistant.
6. There is at most one active AI run system-wide, enforced by the backend and
   recoverable after cancellation, crash, or restart.
7. Workspace paths are canonicalized and remain below an approved root at every
   file operation; symlink and time-of-check/time-of-use risks are addressed.
8. Provider adapters receive only the capabilities required for the run and
   cannot silently substitute a different provider contract.
9. Model output and voice transcripts never execute native actions directly.
   They produce proposed, normalized intents for the policy gateway.
10. Native system actions default to deny and require action-specific policy,
    platform evidence, and user confirmation where warranted.
11. Logs and evidence are useful for review without exposing credentials,
    private prompt contents, or unrelated user data.
12. CSP, Tauri capabilities, IPC exposure, dependency posture, packaging, and
    install/remove behavior pass their owning release gates.

## Action policy direction

| Action class | Default direction | Required boundary |
| --- | --- | --- |
| Read workspace file | Deny outside selected root | Descriptor-confined backend path resolution on the current Linux path |
| Write/delete workspace file | Explicit capability; approval when policy requires | Backend policy plus workspace containment; Ollama filters create/modify and exposes no delete, while Codex mutation classes are post-evidence rather than pre-operation |
| Web access | Off unless capability and task policy permit | Provider/tool-specific network gate |
| Browser research | Hosted read-only search only; no interactive effects | Exact Browser request, HTTPS/domain/source validation, private scratch, and Codex hosted-search contract |
| Financial calculation/report | Local inputs only; user retains decisions | Checked fixed-point backend results, zero tools/external effects, and strict structured result |
| Terminal command | Deny by default; never infer from prose | Normalized command policy and bounded process execution |
| Clipboard | Deny by default | Backend action-specific grant |
| Package/power/privileged system action | Deny | Separate explicitly approved future decision, if ever supported |
| Application/window/input control | Deny by default | Voice/system policy gateway plus platform checks |
| Microphone capture | Off by default | Visible user state and portal/native consent |
| Reminder | Store/notify only | Must not trigger a model in the background |

## KDE, Wayland, and Linux mechanisms

Official platform mechanisms exist and must be assessed before inventing broad
desktop-control workarounds:

- KDE documents [KWin scripting APIs](https://develop.kde.org/docs/plasma/kwin/api/)
  and [KWin scripting](https://develop.kde.org/docs/plasma/kwin/) for bounded
  compositor/window integration.
- XDG Desktop Portal defines scoped
  [Session](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Session.html)
  and
  [RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)
  interfaces with explicit lifecycle and user-consent semantics, plus a native
  [Notification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Notification.html)
  interface for app-owned desktop notifications.
- Tauri documents a configurable
  [Content Security Policy](https://v2.tauri.app/security/csp/).
- freedesktop.org defines the
  [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/)
  and
  [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/0.8/)
  for launch metadata and user-local data/config placement.

TASK-0015 and TASK-0016 use XDG desktop-entry/base-directory data, configured
<code>user-dirs.dirs</code>, KWin scripts with exact IDs, and a backend-owned
RemoteDesktop portal session for bounded input. Relative XDG/PATH inputs are
ignored; localized names, precedence/tombstones, visibility, and
<code>TryExec</code> are enforced; configured folder paths are target-bound
only by SHA-256; KWin
executes only the returned per-script object and reports through
a token-bound callback authenticated to KWin's current D-Bus owner, rather
than the broad all-scripts start method or journal/log parsing. The portal
observes native session closure, offers explicit close, binds its generation
to the exact active Full PC Control agent, and closes after partial grants or
unconfirmed pressed-input cleanup. Its restore token and listener
configuration use private atomic files. Base audio stays in memory; optional
high-accuracy WAV files are mode 0600 and removed on every tested exit path.
TASK-0018 uses the notification portal through the Rust backend for passive
reminder delivery, while app-owned timers and the tray route provide bounded
in-process scheduling/navigation. It does not add KWin rules, a systemd user
unit, KAlarm integration, or a background AI process; live portal behavior
remains a TASK-0020 acceptance case.
Deterministic tests establish the checked-in fail-closed contracts, not live
KDE compatibility. TASK-0020 retains runtime and packaged acceptance. If a
native mechanism cannot satisfy a bounded requirement, the
owning Phase A plan must document the constraint and compare least-privilege
workarounds before any code change.

## Failure and recovery direction

- Reject invalid or unauthorized input without starting a provider or native
  action.
- Record an exact system-action dispatch before its side effect; after an
  acknowledgement gap or restart, mark the request uncertain and never replay
  it automatically.
- Persist terminal run/approval state before releasing authority.
- Make cancellation idempotent and distinguish user cancellation, timeout,
  provider failure, policy denial, and application restart.
- Do not report Codex cancellation, timeout, or success until bounded process
  and namespace cleanup is established; cleanup uncertainty is an interrupted
  outcome requiring review.
- Abort and await an active Ollama request before reporting cancellation or
  timeout; never detach the request worker from the authoritative run.
- Retry a review stage only when interruption occurred before dispatch; require
  human adjudication after uncertain dispatch, missing exact reviewer state,
  exhausted stage attempts, the revision cap, or ambiguous legacy binding.
- Never infer or replay a review verdict from provider prose, renderer state,
  task phase, or a prior request fingerprint.
- Reject a malformed, wrong-kind, check/assumption/calculation-mismatched,
  undeclared-mutation, external-effect, or workspace-changing specialist result
  as a typed protocol failure. Remove the private scratch directory on every
  normal/error path; report cleanup failure instead of claiming clean completion.
- Commit each due occurrence idempotently before delivery, reconcile a reserved
  pre-crash delivery as uncertain without replay, and hold an unresolvable next
  recurrence with inspectable issue evidence while preserving the current due
  occurrence.
- Reject cross-scope or expired memory before run admission and retain the exact
  canonical bundle/hash on the attempt; a later memory edit cannot rewrite what
  an earlier run received.
- Keep task/run/review/human handoff creation in the owning transaction, bound
  summaries/payloads, link large evidence to the immutable source record, and
  preserve a valid sequence prefix under bounded retention.
- Never infer success from a missing process or UI state.
- Preserve approval records and workspace evidence needed for audit while
  applying explicit retention and redaction rules.
- On uncertain authorization or recovery state, fail closed and require a new
  user decision rather than replay an earlier action.

## Privacy rules

- Do not store provider credentials in ordinary renderer state or backups.
- Treat the SQLite application-state database as sensitive local user data;
  keep its path out of routine errors and preserve private filesystem modes.
- Treat run summaries, progress, stderr excerpts, changed paths, diffs, hashes,
  and structured workspace records in the bounded local ledger as sensitive
  workspace evidence. TASK-0012 redacts detected secret lines and sensitive
  paths and omits binary bodies, but those bounded controls are not a guarantee
  that all sensitive content can be recognized; retention remains required.
- Treat typed specialist questions, symptoms, assumptions, constraints,
  calculations, sources, and structured results as sensitive task/run data;
  keep them out of routine logs and non-portable authority records.
- Treat reminder titles/notes, local schedules, scoped memory content,
  management summaries, and prompt-bundle evidence as sensitive local data;
  use generic notification text unless the user explicitly selects title
  disclosure, and never place those records in routine logs.
- Do not include secrets, complete private prompts, workspace contents, or raw
  microphone audio in routine logs.
- Keep age retention bounded and revision visible; protect active authority and
  record maintenance errors or clock rollback instead of claiming deletion.
- Keep Ollama local by default and make any network boundary visible.
- Treat backup export as sensitive user data; sanitize portable authority,
  bound/strictly validate imports, preview the exact candidate, and require
  trusted confirmation before atomic replacement.
- Preserve deletion tombstones and explicit restore intent in lifecycle data;
  do not resurrect absent defaults during normalization.

## Ownership and release gate

[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) assigns each gap to a task.
Security-relevant work is not complete merely because its UI exists. That
sequential validation was completed through the TASK-0021 - TASK-0030
continuation and is recorded in <code>planning/acceptance/</code>, including live
evidence that approving a high-risk action requires a trusted desktop
confirmation outside the renderer and fails closed when it is dismissed.
