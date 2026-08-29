# Third-Party Notices

AI Agent Control Center is proprietary software. See [LICENSE](LICENSE) for its
terms. This file records the third-party components that the application links,
bundles, or downloads at install time, and their licenses. Nothing in
[LICENSE](LICENSE) restricts rights independently granted to you under the
licenses listed here.

This inventory is maintained together with the automated license gate
(`scripts/check-licenses.sh`, `deny.toml`, and the `licenses` job in
`.github/workflows/ci.yml`). That gate **fails the release build** if any
included component carries a license incompatible with proprietary
distribution (for example GPL, AGPL, or a standalone LGPL that is statically
linked). It was last evaluated on 2026-08-29 against the checked-in
`src-tauri/Cargo.lock` and `package-lock.json` with **no incompatible license
found**.

## 1. Rust crates (linked into the desktop binary)

Resolved from `src-tauri/Cargo.lock` (546 crates). Every crate is available to
this project under a permissive license. License families present:

| License family | Representative crates |
| --- | --- |
| `MIT` / `MIT OR Apache-2.0` / `Apache-2.0` | `tauri`, `wry`, `tao`, `serde`, `serde_json`, `tokio`, `reqwest`, `hyper`, `rusqlite`, `libsqlite3-sys`, `ashpd`, `zbus`, `keyring`, `sha2`, `libc`, `rustix`, `glib`, `gtk`, `webkit2gtk`, `javascriptcore-rs` |
| `Unlicense OR MIT` | `jiff`, `jiff-core`, `jiff-static`, `jiff-tzdb`, `memchr`, `byteorder`, `aho-corasick` |
| `Unicode-3.0` | `icu_*`, `tinystr`, `zerovec`, `yoke`, `writeable` |
| `Zlib` / `BSL-1.0` / `BSD-2-Clause` / `BSD-3-Clause` / `ISC` / `0BSD` (as `OR` alternatives or standalone) | `adler2`, `ryu`, `subtle`, `brotli`, `num_enum`, `libloading`, `alloc-no-stdlib` |
| `MPL-2.0` (file-level copyleft, distribution-compatible) | `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors` |

MPL-2.0 is file-level copyleft: the covered source files remain under MPL-2.0
and their source is available from each crate's upstream repository. The
application does not modify those files.

`r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`) is taken under `MIT` /
`Apache-2.0`; it is a UEFI-target transitive dependency and is not linked into
the Linux build.

A full machine-readable list is produced by:

```bash
cargo metadata --format-version 1 --offline \
  --manifest-path src-tauri/Cargo.toml
```

## 2. npm packages

Runtime dependencies bundled into `dist/` by Vite:

| Package | License |
| --- | --- |
| `react`, `react-dom`, `scheduler` | MIT |
| `@tauri-apps/api`, `@tauri-apps/plugin-opener` | MIT OR Apache-2.0 |

Development-only dependencies (Vite, Vitest, TypeScript, Testing Library,
`axe-core`, `jsdom`, `@tauri-apps/cli`, …) are not distributed with the
application. All are MIT, ISC, Apache-2.0, BSD-2/3-Clause, BlueOak-1.0.0,
`MIT-0`, or `MPL-2.0`.

## 3. System libraries (dynamically linked, provided by the operating system)

The Arch `PKGBUILD` and the `install-kde.sh` user-local path link these
libraries dynamically from the host system and do **not** redistribute them:

| Library | License | Notes |
| --- | --- | --- |
| WebKitGTK (`webkit2gtk-4.1`) | LGPL-2.1-or-later, BSD | Dynamic system library. |
| GTK 3, GLib, GDK, Pango, Cairo, ATK | LGPL-2.1-or-later | Dynamic system libraries. |
| SQLite | Public domain (blessing) | Via `libsqlite3` / `libsqlite3-sys`. |
| `xdg-desktop-portal`, `libappindicator`/`ayatana` | LGPL-2.1-or-later / MIT | Dynamic system components. |

If a future distribution artifact **bundles** these LGPL libraries (for example
the scaffolded Tauri `appimage` bundle target), the LGPL-2.1 §6 obligation
applies: the bundled libraries must remain separately replaceable shared
objects and this project must provide the corresponding library source or a
written offer for it. That obligation is why the supported 0.5.x distribution
paths are the Arch package and the user-local script, both of which use system
libraries. Bundled-artifact acceptance is owned by TASK-0020.

## 4. Offline voice runtime (downloaded by the user-run installer)

`voice-runtime/` ships only this project's own `setup.sh`,
`setup-high-accuracy.sh`, and `listener.py`. When the user installs offline
voice, the setup scripts download the following pinned, hash-verified
components onto the user's machine; they are not redistributed inside the
application package:

| Component | License |
| --- | --- |
| Vosk API (`vosk==0.3.45`) | Apache-2.0 |
| `vosk-model-small-en-us-0.15` | Apache-2.0 |
| whisper.cpp (pinned commit) — optional high accuracy | MIT |
| `ggml` Whisper model — optional high accuracy | MIT |
| `pip`, `setuptools`, `wheel` | MIT |
| `packaging` | Apache-2.0 OR BSD-2-Clause |

## 5. Bundled fonts and assets

The application uses the platform system font stack (`font-src 'self'`; no web
fonts are shipped). Application icons and artwork in `src-tauri/icons/`,
`public/`, and `AI-Agents.png` are original works owned by Aivars Rocens and
are covered by [LICENSE](LICENSE), not by any third-party license.
