# Release evidence — version 1.0.0

## Tested tree

| Fact | Value |
| --- | --- |
| Repository | `/mnt/F/AI Agent OS/ai-agent-control-center-desktop` |
| Branch | `main` |
| Base commit | `f87effb` (`task29`), `HEAD == origin/main`, zero ahead / zero behind, clean tree at TASK-0030 preflight |
| Release commit | the TASK-0030 implementation commit that carries this directory |
| Acceptance date | 2026-09-02 |

TASK-0029 preflight verified all seven successor-closure conditions: tracker Phase A
COMPLETE / approval YES / Phase B COMPLETE / verification PASSED; implementation
commit `f87effb` identified from actual Git history and matching its reported
scope (CI workflow, `.nvmrc`, `deny.toml`, `check-packaging.sh`, and the new S10
module); that commit is the checked-out HEAD and reachable from `origin/main`;
branch and `origin/main` aligned zero/zero; working tree clean; no unexplained
intervening state. Its Git closure is backfilled to COMPLETE by this task.

## Environment

| Component | Version |
| --- | --- |
| OS | Arch Linux, kernel 7.2.2-arch1-1 |
| Desktop | KDE Plasma, KWin 6.7.4, Wayland, one 1920x1080 output at scale 1 |
| Codex CLI | `codex-cli 0.144.5`, signed in with ChatGPT |
| Ollama | 0.32.3, 5 local models |
| Node (local run) | v26.8.1; CI uses the pinned `.nvmrc` 22.23.2 |
| cargo-deny | 0.20.2 (from `~/.cargo/bin`, which is absent from the interactive PATH) |
| gitleaks | 8.30.1 |

## Artifacts

Built by `makepkg -f --noconfirm` from the final 1.0.0 tree:

```
packaging/ai-agent-control-center-1.0.0-1-x86_64.pkg.tar.zst
  sha256 9bded22019e01b59641c4f26e218b97da5328101a45e124bf423e5f1d09bbf8d
  7440431 bytes

src-tauri/target/release/ai-agent-control-center
  sha256 516cbbaff6f4a84dbbc14772efbd273e26e28fe8c3ef0ec7e1a05be566d85825
  30057080 bytes, reports "ai-agent-control-center 1.0.0"
```

The live GUI acceptance ran against the `install-kde.sh` build of the same tree,
sha256 `fa6a9a9612f3e4d18024e4480e09c8a18989220459bfc45ac7de2a4fc58976a9`. The two
binaries differ by design: `makepkg` applies the distribution's own compiler and
linker flags, `install-kde.sh` does not.

The `pacman -U` / `pacman -R` live system-database transaction was executed
against the 0.5.1-1 package built from the identical tree immediately before the
version bump; the bump changed only version metadata, no packaged behaviour.

## Version coupling

All seven coupled locations agree at `1.0.0`, enforced by
`install_package_acceptance::s8_release_version_is_consistent_across_every_shipped_manifest`
and `release_gate_acceptance::s10_release_version_is_consistent_across_manifests_and_lockfiles`,
both of which derive the expected value from `env!("CARGO_PKG_VERSION")`:

`package.json`, `package-lock.json` (root and root package entry),
`src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`,
`packaging/PKGBUILD`, and the AppStream metainfo, whose newest release is now
`version="1.0.0" type="stable"`.

The TASK-0019 tripwire asserting `type="development"` was inverted rather than
deleted: the newest release must now be `stable` and must match the crate
version, so the metadata cannot silently regress to a development build.

## Branch CI

The candidate's only CI run before this task, `33553945326` on `f87effb`, was
**red**: the axe scenario exceeded Vitest's implicit 5000 ms default on the
runner. TASK-0029's final gate had been run locally only, so the release
candidate had never had a green branch gate. That is fixed and covered by a new
S10 scenario. The green run for this release is the one triggered by pushing the
TASK-0030 commit; its result is recorded in `planning/TASK_STATUS.md` rather than
predicted here.

## Not done

No tag, no GitHub release, no publish, and no external distribution step was
performed. Nothing outside this repository and the local machine was changed.
