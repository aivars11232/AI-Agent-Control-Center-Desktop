#!/usr/bin/env bash

# User-local install / upgrade for AI Agent Control Center on Arch Linux + KDE.
#
# Layout (all under $HOME, no root required):
#   ~/.local/lib/ai-agent-control-center/        payload: binary + voice-runtime
#   ~/.local/bin/ai-agent-control-center         launcher symlink (CLI + Exec=)
#   ~/.local/share/applications/…desktop         desktop entry
#   ~/.local/share/icons/hicolor/512x512/apps/   application icon
#   ~/.local/share/metainfo/…metainfo.xml        AppStream metadata
#   ~/.local/share/licenses/ai-agent-control-center/  LICENSE + notices
#
# Upgrades preserve the previous binary and roll back if the new build fails a
# post-install check. Removal is handled by uninstall-kde.sh (keep-data mode)
# or `uninstall-kde.sh --purge` (full data purge).

set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
app_id="ai-agent-control-center"

lib_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}"
install_dir="$HOME/.local/lib/$app_id"
bin_dir="$HOME/.local/bin"
launcher="$bin_dir/$app_id"
desktop_dir="$lib_dir/applications"
icon_dir="$lib_dir/icons/hicolor/512x512/apps"
metainfo_dir="$lib_dir/metainfo"
license_dir="$lib_dir/licenses/$app_id"
installed_bin="$install_dir/$app_id"
previous_bin="$install_dir/$app_id.previous"

log() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }

# --- prerequisites ---------------------------------------------------------
missing=0
for command_name in npm cargo rustc pkg-config; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    warn "missing required command: $command_name"
    missing=1
  fi
done
if (( missing )); then
  echo "Install the Arch Tauri prerequisites (base-devel, rust, nodejs, npm," >&2
  echo "webkit2gtk-4.1, libappindicator-gtk3, librsvg), then re-run this script." >&2
  exit 1
fi

if [[ -f "$project_dir/.nvmrc" ]] && command -v node >/dev/null 2>&1; then
  want_node="$(tr -d ' \tv' < "$project_dir/.nvmrc")"
  have_node="$(node --version | tr -d 'v')"
  want_major="${want_node%%.*}"
  have_major="${have_node%%.*}"
  if [[ "$have_major" -lt "$want_major" ]]; then
    warn "Node $have_node is older than the pinned $want_node; the build may fail."
  fi
fi

if ! command -v codex >/dev/null 2>&1 && [[ ! -x "$HOME/.local/bin/codex" ]]; then
  warn "Codex CLI not found. The app installs, but autonomous agents stay offline"
  warn "until Codex is installed and signed in with ChatGPT."
fi

# --- stop any running instance before building --------------------------
# A running pre-upgrade instance opens the database during the multi-minute
# build. If its migration DDL is older than the current tree it can leave an
# uninitialised database at the current schema version that the new binary
# cannot re-migrate, so stop it up front rather than after the build. A stale
# instance that still races the build is recovered on the next launch: the new
# binary rebuilds an uninitialised database whose schema is out of date.
if [[ -x "$installed_bin" ]]; then
  log "stopping the running instance and voice listener"
  "$installed_bin" --stop-runtime || true
fi

# --- build ----------------------------------------------------------------
cd "$project_dir"
log "installing JavaScript dependencies (no lifecycle scripts)"
npm ci --ignore-scripts
log "building the desktop binary (no bundle)"
npm run tauri -- build --no-bundle

built_bin="$project_dir/src-tauri/target/release/$app_id"
if [[ ! -x "$built_bin" ]]; then
  echo "Build did not produce $built_bin" >&2
  exit 1
fi

# --- install / upgrade the payload ------------------------------------
install -d "$install_dir" "$bin_dir" "$desktop_dir" "$icon_dir" \
  "$metainfo_dir" "$license_dir" "$install_dir/voice-runtime"

upgrading=0
if [[ -e "$installed_bin" ]]; then
  upgrading=1
  cp -f "$installed_bin" "$previous_bin"
fi

install -Dm755 "$built_bin" "$installed_bin"
install -Dm755 "$project_dir/voice-runtime/setup.sh" "$install_dir/voice-runtime/setup.sh"
install -Dm755 "$project_dir/voice-runtime/setup-high-accuracy.sh" \
  "$install_dir/voice-runtime/setup-high-accuracy.sh"
install -Dm644 "$project_dir/voice-runtime/listener.py" \
  "$install_dir/voice-runtime/listener.py"

ln -sfn "$installed_bin" "$launcher"

# --- post-install verification + rollback ------------------------------
if ! "$installed_bin" --version >/dev/null 2>&1; then
  warn "the newly installed binary failed its post-install check"
  if (( upgrading )) && [[ -e "$previous_bin" ]]; then
    warn "rolling back to the previous binary"
    mv -f "$previous_bin" "$installed_bin"
    ln -sfn "$installed_bin" "$launcher"
    exit 1
  fi
  rm -f "$installed_bin" "$launcher"
  exit 1
fi
rm -f "$previous_bin"

# --- desktop integration ---------------------------------------------
install -Dm644 "$project_dir/src-tauri/icons/icon.png" "$icon_dir/$app_id.png"
install -Dm644 "$project_dir/LICENSE" "$license_dir/LICENSE"
[[ -f "$project_dir/THIRD-PARTY-NOTICES.md" ]] && \
  install -Dm644 "$project_dir/THIRD-PARTY-NOTICES.md" "$license_dir/THIRD-PARTY-NOTICES.md"
install -Dm644 "$project_dir/packaging/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml" \
  "$metainfo_dir/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml"

sed \
  -e "s|@EXEC@|$launcher|g" \
  -e "s|@ICON@|$app_id|g" \
  "$project_dir/packaging/$app_id.desktop.in" \
  > "$desktop_dir/$app_id.desktop"
chmod 644 "$desktop_dir/$app_id.desktop"

# Migrate the legacy wake phrase without touching other configuration.
voice_config="$config_dir/$app_id/voice-runtime/listener-config.json"
if [[ -f "$voice_config" ]]; then
  sed -i 's/"wakePhrase":"lucy activate, on"/"wakePhrase":"lucy"/' "$voice_config"
fi

# The Codex runtime uses ChatGPT login; drop any legacy API key.
"$installed_bin" --remove-credentials || true

# --- non-disruptive cache refresh ----------------------------------
# A new/updated .desktop file and icon are picked up by rebuilding the
# freedesktop and KDE service caches. Restarting plasmashell is NOT needed
# and is deliberately avoided.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$desktop_dir" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$lib_dir/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi

if (( upgrading )); then
  log "AI Agent Control Center upgraded. Existing data was preserved."
else
  log "AI Agent Control Center installed. Open it from the KDE application menu."
fi
echo "    In Settings, select a project workspace before running an autonomous agent."
if ! printf '%s' "$PATH" | tr ':' '\n' | grep -qxF "$bin_dir"; then
  echo "    Add $bin_dir to PATH to run 'ai-agent-control-center --help' from a shell."
fi
