# Version 1.0.0 handoff

## What you have

A local, single-user desktop control centre for bounded AI agents on Arch Linux
with KDE Plasma on Wayland. It is proprietary (`LicenseRef-proprietary`), needs
no paid API key, and executes work through your installed Codex CLI or a local
Ollama.

## Installing and removing

```
bash ./install-kde.sh                       # user-local install or upgrade, no root
bash ./uninstall-kde.sh                     # remove, KEEPING the database and voice models
bash ./uninstall-kde.sh --purge             # remove everything this app owns
```

For the system package:

```
cd packaging && makepkg -f                  # build
sudo pacman -U ai-agent-control-center-1.0.0-1-x86_64.pkg.tar.zst
sudo pacman -R ai-agent-control-center      # leaves per-user data untouched
```

`pacman -R` deliberately cannot remove per-user data. To delete it, run
`ai-agent-control-center --purge --confirm PURGE` **before** removing the
package. `ai-agent-control-center --print-data-paths` lists every owned location
and whether keep-data removal retains it.

## What guards your machine

Any high-risk action — starting the microphone, taking desktop input control,
resetting state, importing a backup — stops and raises a **trusted KDE dialog
outside the application window**. The window cannot grant on its own: the backend
only proceeds when that dialog returns success. Dismissing it, or answering No,
leaves the request pending and grants nothing. This was verified live, in all
three outcomes, during acceptance.

Approvals are one-use and expire. Codex runs inside a Bubblewrap sandbox scoped
to the workspace you select. The persistent KDE screen-cast permission is owned
by KDE, not by this app: revoke it in **System Settings → Applications**.

## Keeping your data

Settings → Export and import writes a portable v4 backup to a location you
choose, and tells you the exact path. It deliberately excludes provider
credentials, portal grants, delivery evidence, and run history. Import validates
and previews before it applies, and reports whether the backup would raise any
protected security setting.

## Verifying a change

```
npm run verify:fast                          # tests, typecheck, rustfmt, Rust suite
VERIFY_STRICT=1 npm run verify:full          # the full release gate
```

`verify:full` needs `cargo-deny` on PATH for the Rust advisory check to be
conclusive. It lives in `~/.cargo/bin`, which is not on the interactive PATH by
default — prepend it, or the gate reports the advisory status as indeterminate.

## Before the next change

Read [AGENTS.md](../../AGENTS.md) for the development contract and
[planning/TASK_STATUS.md](../TASK_STATUS.md) for execution state. The recorded 1.0
limitations are in [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md); the two
outstanding acceptance cases are listed there with the exact commands to re-run
them.

## Immediately outstanding

- Re-run the Codex bounded-run case after **2026-09-07**, when the ChatGPT usage
  limit resets. It is the only mandatory case not observed.
- No tag, GitHub release, or publish was created. Releasing externally is a
  separate, deliberate act.
