# Decision 0002: Proprietary License and Commercial Business Model

- **Status:** Accepted
- **Date:** 2026-08-29
- **Scope:** Project-wide (licensing, packaging, distribution, release metadata)
- **Established by:** Owner decision supplied during TASK-0019, incorporated in
  the approved TASK-0019 Phase A plan
- **Relation to [Decision 0001](0001-fixed-project-decisions.md):** additive.
  0001 is not superseded. In particular, 0001 product decision 6 ("do not
  require a paid API key or paid external service") is unchanged: the AI
  execution paths remain the installed Codex CLI (ChatGPT login) and local
  Ollama, with no mandatory paid API key. This decision concerns the license of
  the application itself, not its runtime requirements.

## Context

Through TASK-0018 the repository had no `LICENSE` file and empty
`license`/`repository` fields, which the audit flagged as a distribution
blocker. TASK-0019 owns release metadata and needed an explicit owner choice
before authoring `LICENSE` and the manifest fields.

## Decision

AI Agent Control Center is:

- commercial, subscription-based software;
- proprietary and **not** open source;
- source-visible only where the repository is public; visibility grants no
  license;
- distributed to authorised users under separate written commercial terms.

Exact licensing:

- SPDX identifier: `LicenseRef-proprietary`
- License title: **AI Agent Control Center Proprietary License**
- Copyright holder: **Aivars Rocens**; statement:
  `Copyright (c) 2026 Aivars Rocens. All rights reserved.`
- Root [`LICENSE`](../../LICENSE) holds the full proprietary text verbatim.

Metadata representation:

| File | Value |
| --- | --- |
| `package.json` | `"private": true`, `"license": "SEE LICENSE IN LICENSE"` |
| `src-tauri/Cargo.toml` | `license-file = "../LICENSE"` (not the SPDX `license` field), `repository`, `publish = false` |
| `packaging/PKGBUILD` | `license=('LicenseRef-proprietary')`; installs `LICENSE` under `/usr/share/licenses/${pkgname}/` |
| `packaging/*.metainfo.xml` | `<metadata_license>CC0-1.0</metadata_license>` (metadata file only) plus `<project_license>LicenseRef-proprietary=<LICENSE URL></project_license>` |

Third-party components keep their own licenses. TASK-0019 verified that every
distributable Rust crate and npm package is available under a permissive license
(no GPL/AGPL/standalone-LGPL/SSPL); the inventory and obligations are recorded in
[`THIRD-PARTY-NOTICES.md`](../../THIRD-PARTY-NOTICES.md). The CI license gate
(`scripts/check-licenses.sh`, `deny.toml`) fails the build if a future
dependency introduces a license incompatible with proprietary distribution.

An open-source license must not be applied to the application. If a validator
requires a mechanically different but equivalent representation of the same
proprietary license and URL, that exact requirement is reported rather than
substituting an open-source license.

## Consequences

- `LICENSE`, `THIRD-PARTY-NOTICES.md`, and the manifest/desktop/AppStream
  metadata are authored in TASK-0019.
- No publication or public release claim is made before TASK-0020.
- Distribution channels, pricing, and the commercial agreement text are outside
  the current roadmap and are owned by the project owner.

## Supersession

The owner may change the license or business model with a new numbered decision
record and a coordinated update of every affected authority, per
[Decision 0001](0001-fixed-project-decisions.md) supersession rules.
