# Repository Development Contract

These instructions apply to every contributor and AI coding workflow operating
in this repository. [START_HERE.md](START_HERE.md) defines the authority order;
the current checkout remains the highest project evidence.

## Work boundary

- Use one foreground Codex workflow. Do not create background, parallel,
  delegated, recursive, or subagent AI workstreams.
- Work on one explicitly named and approved task at a time. Do not begin the
  next roadmap task automatically.
- Every task has a read-only Phase A plan followed by Phase B implementation
  only after the user provides the task's exact approval phrase.
- Phase B may update all approved related files in a coherent logical slice.
  Multi-file work is expected when it is required for one complete outcome.
- Do not add unrelated cleanup, speculative redesign, future-task behavior,
  dependency updates, generated output, or broad formatting.

## Before planning or editing

1. Resolve the actual repository root, branch, exact HEAD, working-tree state,
   and active task dependencies.
2. Read [START_HERE.md](START_HERE.md), the applicable authority documents, and
   the active task contract.
3. Inspect current code, tests, manifests, schemas, and relevant history. Never
   reconstruct unknown code from an audit, stale line number, memory, or chat.
4. Preserve all unexplained user changes. Stop before overwriting or discarding
   them.
5. Before changing code that interacts with KDE or other Linux components,
   research their native mechanisms and official constraints first. Check, as
   applicable, KWin APIs and window rules, KDE/Plasma integration points,
   XDG desktop portals, desktop-entry behavior, and XDG filesystem standards.
   Document viable native paths and bounded workarounds; do not assume a
   platform limitation has no workaround without evidence.

## Phase A: read-only planning

Phase A must state:

- current evidence and any conflicts;
- exact in-scope files or subsystems and explicit out-of-scope boundaries;
- preserved behavior and required decisions;
- coherent implementation slices and focused checks for each slice;
- the full verification gate, security/privacy effects, migrations, risks,
  failure handling, and recovery;
- the literal approval phrase needed to begin Phase B.

Phase A does not edit tracked files, install or update dependencies, stage,
commit, push, invoke providers, capture microphone input, authorize portals,
run install/remove scripts, or perform desktop/system-control actions.

## Phase B: approved implementation

- Recheck root, branch, HEAD, status, task dependency, and the approved plan
  before the first edit.
- Implement one approved logical slice at a time. Run its focused verification
  before continuing.
- Make one meaningful change at a time and correct only a clear in-scope cause
  if a check fails.
- Stop before a new dependency, migration class, subsystem, security boundary,
  materially broader behavior, or unresolved user decision.
- Stop later slices on the first unexplained failure. Perform one focused
  diagnosis; do not use blind retry loops.
- Run the full task gate and inspect the complete diff and status at the end.

## Security and data rules

- Treat renderer input, imported data, model output, provider output, workspace
  content, paths, shell arguments, and IPC payloads as untrusted.
- Keep authorization enforcement in an authoritative backend boundary. UI
  visibility, renderer state, or a prompt is not a security boundary.
- Default to least privilege, explicit workspace scope, deny-by-default system
  actions, and one-use approvals where an action requires approval.
- Never include credentials, tokens, private prompts, user file contents, or
  unredacted sensitive logs in documentation, fixtures, or reports.
- Do not run live providers, microphone capture, portal authorization,
  installers, uninstallers, or system-control actions unless the approved task
  owns a bounded case and the user explicitly authorizes it.

## Verification and evidence

- Use the narrowest deterministic checks that establish the slice, followed by
  the task's full gate. Do not substitute an unrelated passing command.
- Report exact commands, results, counts, skips, truncation, and environmental
  limitations. Separate fresh checks from historical or retained evidence.
- Do not claim behavior, safety, compatibility, or production readiness that
  was not demonstrated. Use **indeterminate** when required evidence is absent.
- Planned features must remain visibly planned in docs, tests, and reports.
- The version 1.0.0 release gate passed under TASK-0030. Do not describe the
  application as pre-production; do describe any specific behaviour that is
  still unproven as unproven, and keep recorded 1.0 limitations visible.

## Git and closure

- Do not commit, push, reset, clean, stash, rebase, force-push, or discard work
  unless the user explicitly requests that operation.
- End Phase B with the exact changed files, verification results, remaining
  limitations, `git status --short`, diff statistics, and a recommended commit
  message.
- The user performs final review, commit, and push. A successor Phase A may
  treat its predecessor as Git-closed only when fresh preflight evidence
  verifies all of the following:
  1. the predecessor tracker/evidence records Phase A `COMPLETE`, approval
     `YES`, Phase B `COMPLETE`, and verification `PASSED`;
  2. the predecessor implementation commit is identified from actual Git
     history rather than guessed from its message;
  3. that commit is the checked-out `HEAD` or an ancestor of it;
  4. that commit is reachable from `origin/main`;
  5. the checked-out branch and `origin/main` have zero ahead/behind;
  6. the working tree is clean; and
  7. the commit scope matches the predecessor's reported scope, with no
     unexplained intervening repository state that invalidates the evidence.
- When all seven conditions pass, a stale `PENDING USER` tracker value alone
  is not a Phase A blocker. The successor Phase A remains read-only and records
  the exact implementation commit, origin reachability, ahead/behind result,
  clean-tree result, and scope conclusion in its plan.
- During the successor's approved Phase B, its normal task-status/document
  update backfills the predecessor's Git closure to `COMPLETE` with the exact
  verified historical evidence. Include that backfill in the successor's
  normal implementation commit; do not require a separate closure-only commit.
- If any preflight condition fails, stop Phase A without changing files and
  report the exact dirty, divergent, unpushed, unverified, ambiguous-commit, or
  scope-conflict evidence. Never skip predecessor implementation or
  verification, guess a commit, overlap active tasks, or start Phase B without
  the successor's approved Phase A plan.
- TASK-0020 has no successor. Record its final Git and release closure through
  the final release evidence and release commit rather than a successor
  backfill.
