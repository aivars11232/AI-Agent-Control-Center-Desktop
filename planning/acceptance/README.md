# Version 1.0 acceptance evidence

This directory holds the final acceptance record produced by **TASK-0030** on
2026-09-02. It is the closure of the TASK-0020 release gate, whose sequential
live-acceptance intent was completed through the TASK-0021 – TASK-0030
continuation adopted by [Decision 0003](../decisions/0003-v3-continuation-roadmap.md).

| File | Contents |
| --- | --- |
| [FINAL_ACCEPTANCE_MATRIX.md](FINAL_ACCEPTANCE_MATRIX.md) | Every mandatory criterion with PASS / FAIL / BLOCKED / NOT EXECUTED and its evidence |
| [RELEASE_EVIDENCE.md](RELEASE_EVIDENCE.md) | The exact tested commit, artifact hashes, and gate output |
| [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md) | What 1.0 does not do, and what was not proven |
| [HANDOFF.md](HANDOFF.md) | What the owner needs to know to operate and continue the project |

## How to read this

Every row states how it was established. A row marked PASS was **observed**, not
inferred from code inspection. Where a case could not be executed, it says so and
why; nothing here is waived silently.

Two cases are recorded as exceptions rather than passes, and the release decision
was taken with both visible:

- the Codex bounded-run completion case, blocked by an external ChatGPT usage
  limit that resets 2026-09-07; and
- a spoken microphone command driven through the full GUI voice pipeline, which
  requires a human voice.

Neither indicates a product defect. The Codex properties the release gate
actually turns on — containment, cancellation, and typed failure — were all
observed live.
