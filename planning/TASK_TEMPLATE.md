# TASK-[NNNN] — [Title]

Use this template for repository task records. Replace bracketed prompts with
verified task-specific content. The active task package or recorded repository
task contract remains authoritative if it requires stricter wording or gates.

## Metadata

| Field | Value |
| --- | --- |
| Task | TASK-[NNNN] |
| Priority | P0 / P1 / P2 |
| Milestone | M[NUMBER] — [Name] |
| Depends on | TASK-[NNNN] / None |
| Phase | A — Read-only planning / B — Approved implementation |
| Status | NOT STARTED / IN PROGRESS / READY / COMPLETE / BLOCKED / DECISION REQUIRED |
| Starting branch | [Branch] |
| Starting HEAD | [Full commit] |
| Suggested commit | [Imperative message] |

## 1. Repository preflight

- Resolved root:
- Branch and exact HEAD:
- Working-tree status:
- Predecessor Phase A/approval/Phase B/verification status:
- Exact predecessor implementation commit and how Git history identified it:
- Implementation commit is `HEAD` or an ancestor:
- Implementation commit is reachable from `origin/main`:
- Checked-out branch versus `origin/main` ahead/behind:
- Clean working-tree result:
- Commit-scope consistency and unexplained intervening state:
- Predecessor tracker closure value and successor Phase B backfill required:
- Unexplained user changes:
- Relevant repository authorities read:
- Active task contract/package evidence read:

For a successor task, record every predecessor-closure result above from fresh
Git evidence. A stale `PENDING USER` tracker value alone is not a blocker when
all seven closure conditions in [AGENTS.md](../AGENTS.md) pass. Stop without
changing files if the root is wrong, predecessor implementation or verification
is incomplete, the implementation commit is ambiguous or unpushed, branch and
`origin/main` diverge, the tree is dirty, commit scope conflicts with the
reported task, unexplained intervening state exists, the approved plan is
stale, or an unexplained change overlaps proposed work.

## 2. Current implementation evidence

Separate evidence by class:

### Current

- Files, symbols, schemas, manifests, tests, and history inspected:
- Current behavior:
- Fresh commands and exact results:

### Historical

- Earlier audit or retained runtime evidence:
- Why it is still relevant:

### Planned

- Required outcome that does not exist yet:

### Unverified

- Missing evidence and consequence:

List every corrected package, audit, line-number, or memory assumption. Current
checkout evidence has precedence under [START_HERE.md](../START_HERE.md).

## 3. Task boundary

### Goal

[One coherent outcome.]

### In scope

- [Approved behavior, subsystem, or document.]

### Out of scope

- [Explicit future-task or live/external boundary.]

### Behavior to preserve

- [Compatibility, user data, safety, or workflow invariant.]

### Allowed files or subsystems

- [Exact path or bounded module.]

## 4. Platform and native-mechanism research

Complete this section before code that interacts with KDE or another Linux
component:

- Native mechanism and official source:
- KWin/KDE/Wayland/XDG constraints:
- Permission or user-consent boundary:
- Viable native implementation:
- Bounded workaround if native behavior is insufficient:
- Rejected broad or unsafe workarounds:

Use **not applicable** with a reason for tasks that do not touch platform code.

## 5. Coherent change set

| Logical slice | Related files/modules | Purpose | Focused completion checkpoint |
| --- | --- | --- | --- |
| 1 | [Paths] | [Outcome] | [Command/evidence] |
| 2 | [Paths] | [Outcome] | [Command/evidence] |

Do not split one required behavior into artificial one-file tasks. Do not add a
file merely to fill a planned architecture shape.

## 6. Contracts and compatibility

- Types/domain invariants:
- APIs/IPC/events:
- Persistence/schema/migrations:
- Security/privacy:
- Provider/runtime effects:
- Platform/install/remove effects:
- Backward compatibility and data preservation:
- Failure and recovery behavior:

## 7. Ordered implementation plan

1. [First logical slice and why it is first.]
2. [Next slice and prerequisite.]
3. [Integration and cleanup inside the approved boundary.]

Identify any material decision that must stop implementation for new approval.

## 8. Verification plan

### Focused checks

- Slice 1:
- Slice 2:

### Full non-live gate

- Exact commands:
- Expected counts/results:
- Scope/diff checks:
- Security/negative cases:

### Manual or live cases

- Owned by this task:
- Exact authorization required:
- Deferred cases and owning task:

Never run a provider, microphone, portal, installer, uninstaller, or
desktop/system-control case merely because it appears in this section.

## 9. Risks, deviations, and recovery

- Main risks:
- Rollback/recovery:
- Minor in-scope flexibility:
- Significant deviation triggers:
- First-failure diagnosis rule:

Do not use blind retry loops. Stop later slices on the first unexplained
failure.

## 10. Decisions

- User decisions required:
- Existing decision records:
- Proposed new/superseding decision record:

## Phase A outcome

- Files changed: none
- Exact predecessor implementation commit recorded:
- Origin reachability and zero ahead/behind recorded:
- Clean-tree and commit-scope conclusions recorded:
- Temporary predecessor tracker lag requiring Phase B backfill:
- Literal approval phrase:
  <code>APPROVED: IMPLEMENT TASK-[NNNN] AS PLANNED.</code>
- Required final Phase A outcome line:

[Insert exactly one outcome allowed by the active task contract.]

---

## Phase B execution record

Complete only after the exact Phase A approval.

### Starting revalidation

- Approval matched:
- Root/branch/HEAD/status:
- Predecessor closure evidence still satisfies all seven preflight conditions:
- Predecessor tracker closure backfill included in the approved change set:
- Approved boundary unchanged:

### Slice results

| Slice | Files changed | Focused verification | Result |
| --- | --- | --- | --- |
| 1 | [Paths] | [Exact command] | PASS / FAIL / SKIPPED |

### Integrated verification

- Exact command:
- Result/count:
- Skipped or unavailable checks:
- Manual/live evidence:

### Effects

- Behavior implemented:
- Behavior preserved:
- Types/API/IPC/schema/migration effects:
- Security/privacy/failure/recovery effects:
- Minor approved-boundary deviations:
- Remaining limitations and owning tasks:

### Ending Git evidence

- <code>git status --short</code>:
- <code>git diff --stat</code>:
- Diff inspected:
- Commit/push performed by Codex: no, unless explicitly requested
- Recommended commit:

### Required Phase B outcome

[Insert exactly one final outcome line allowed by the active task contract.]

## Manual closure

- [ ] User reviewed status and the complete diff.
- [ ] Required verification evidence is present.
- [ ] User committed the approved task or recorded another explicit closure.
- [ ] User pushed when required.
- [ ] Final clean-tree evidence is available for the successor's fresh
      preflight.
- [ ] If a successor exists, its read-only Phase A independently verifies and
      records the exact implementation commit, ancestry, origin reachability,
      zero ahead/behind, clean tree, and scope consistency.
- [ ] During the successor's approved Phase B, any temporary predecessor
      `PENDING USER` lag is backfilled to `COMPLETE` with exact historical
      evidence in the successor's normal implementation commit.

A separate closure-only commit is not normally required. TASK-0020 has no
successor; its final Git and release closure belongs in the final release
evidence and release commit.
