#!/usr/bin/env bash

# TASK-0019 staged install / upgrade / removal / purge test.
#
# Runs entirely inside a throwaway $HOME and $XDG_RUNTIME_DIR. It never touches
# the real desktop, never restarts plasmashell, and never runs a provider,
# microphone, portal, or system-control action.
#
# It exercises:
#   * the binary removal subcommands (--print-data-paths, --stop-runtime,
#     --uninstall keep-data, --purge with and without confirmation);
#   * uninstall-kde.sh in keep-data and --purge modes against a simulated
#     user-local install layout.
#
# The Tauri binary comes from $AACC_TEST_BINARY when set; otherwise it is built
# offline once as a debug binary.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

app_id="ai-agent-control-center"
fail=0
note() { printf '  - %s\n' "$*"; }
check() {
  if eval "$2"; then
    printf 'ok   %s\n' "$1"
  else
    printf 'FAIL %s\n' "$1"
    fail=1
  fi
}

# --- locate a binary -----------------------------------------------------
binary="${AACC_TEST_BINARY:-}"
if [[ -z "$binary" ]]; then
  echo "building the binary offline (set AACC_TEST_BINARY to skip)"
  ( cd src-tauri && cargo build --locked --offline --bin "$app_id" )
  binary="$repo_root/src-tauri/target/debug/$app_id"
fi
[[ -x "$binary" ]] || { echo "no usable binary at $binary" >&2; exit 1; }
echo "using binary: $binary"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# All owned locations, seeded with a sentinel file each.
seed_data() {
  local home="$1"
  local data="$home/.local/share" cfg="$home/.config" cache="$home/.cache"
  mkdir -p \
    "$data/com.aivarsrocens.aiagentcontrolcenter/logs" \
    "$cfg/com.aivarsrocens.aiagentcontrolcenter" \
    "$cache/com.aivarsrocens.aiagentcontrolcenter" \
    "$data/$app_id/voice-runtime/base/models" \
    "$cfg/$app_id/voice-runtime" \
    "$cache/$app_id/voice-runtime" \
    "$home/run/$app_id/kwin"
  echo sqlite   > "$data/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3"
  echo wal      > "$data/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3-wal"
  echo webview  > "$data/com.aivarsrocens.aiagentcontrolcenter/localstorage.bin"
  echo log      > "$data/com.aivarsrocens.aiagentcontrolcenter/logs/app.log"
  echo cfg      > "$cfg/com.aivarsrocens.aiagentcontrolcenter/settings.bin"
  echo cache    > "$cache/com.aivarsrocens.aiagentcontrolcenter/http.bin"
  echo model    > "$data/$app_id/voice-runtime/base/models/model.bin"
  echo listener > "$cfg/$app_id/voice-runtime/listener-config.json"
  echo token    > "$cfg/$app_id/voice-runtime/desktop-control-restore-token"
  echo dl       > "$cache/$app_id/voice-runtime/vosk.zip"
  echo kwin     > "$home/run/$app_id/kwin/script.js"
}

present() { [[ -e "$1" ]]; }
absent()  { [[ ! -e "$1" ]]; }

run_bin() { HOME="$1" XDG_RUNTIME_DIR="$1/run" "$binary" "${@:2}"; }

echo
echo "== binary subcommands =="

h="$work/bin-help"; mkdir -p "$h"
check "--version prints the pinned version" \
  "run_bin '$h' --version | grep -qx '$app_id 0.5.1'"
check "--help lists the removal subcommands" \
  "run_bin '$h' --help | grep -q -- '--purge --confirm PURGE'"

h="$work/bin-paths"; mkdir -p "$h/run"; seed_data "$h"
check "--print-data-paths reports seeded locations as present" \
  "run_bin '$h' --print-data-paths | grep -q 'present'"
check "--stop-runtime succeeds with no owned processes" \
  "run_bin '$h' --stop-runtime | grep -q 'no owned processes'"

h="$work/bin-keep"; mkdir -p "$h/run"; seed_data "$h"
run_bin "$h" --uninstall
check "keep-data retains the database" \
  "present '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3'"
check "keep-data retains the database WAL" \
  "present '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3-wal'"
check "keep-data retains downloaded voice models" \
  "present '$h/.local/share/$app_id/voice-runtime/base/models/model.bin'"
check "keep-data removes the debug logs" \
  "absent '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter/logs'"
check "keep-data removes the app config dir" \
  "absent '$h/.config/com.aivarsrocens.aiagentcontrolcenter'"
check "keep-data removes the app cache dir" \
  "absent '$h/.cache/com.aivarsrocens.aiagentcontrolcenter'"
check "keep-data removes the KDE portal restore token" \
  "absent '$h/.config/$app_id/voice-runtime/desktop-control-restore-token'"
check "keep-data removes the voice download cache" \
  "absent '$h/.cache/$app_id/voice-runtime'"
check "keep-data removes the runtime dir" \
  "absent '$h/run/$app_id'"

h="$work/bin-purge"; mkdir -p "$h/run"; seed_data "$h"
check "--purge without confirmation refuses (exit 2) and deletes nothing" \
  "! run_bin '$h' --purge >/dev/null 2>&1 && present '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3'"
run_bin "$h" --purge --confirm PURGE
for leftover in \
  ".local/share/com.aivarsrocens.aiagentcontrolcenter" \
  ".config/com.aivarsrocens.aiagentcontrolcenter" \
  ".cache/com.aivarsrocens.aiagentcontrolcenter" \
  ".local/share/$app_id" \
  ".config/$app_id" \
  ".cache/$app_id" \
  "run/$app_id"; do
  check "purge removes $leftover" "absent '$h/$leftover'"
done
check "a second purge is idempotent" \
  "run_bin '$h' --purge --confirm PURGE | grep -q 'purge complete'"

echo
echo "== uninstall-kde.sh =="

simulate_install() {
  local home="$1"
  local install_dir="$home/.local/lib/$app_id"
  mkdir -p "$install_dir/voice-runtime" "$home/.local/bin" \
    "$home/.local/share/applications" \
    "$home/.local/share/icons/hicolor/512x512/apps" \
    "$home/.local/share/metainfo" \
    "$home/.local/share/licenses/$app_id"
  cp "$binary" "$install_dir/$app_id"
  cp voice-runtime/listener.py "$install_dir/voice-runtime/listener.py"
  ln -sfn "$install_dir/$app_id" "$home/.local/bin/$app_id"
  : > "$home/.local/share/applications/$app_id.desktop"
  : > "$home/.local/share/icons/hicolor/512x512/apps/$app_id.png"
  : > "$home/.local/share/metainfo/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml"
  : > "$home/.local/share/licenses/$app_id/LICENSE"
}

h="$work/script-keep"; mkdir -p "$h/run"; simulate_install "$h"; seed_data "$h"
HOME="$h" XDG_RUNTIME_DIR="$h/run" PATH="$PATH" bash uninstall-kde.sh >/dev/null
check "script keep-data removes the install payload" "absent '$h/.local/lib/$app_id'"
check "script keep-data removes the launcher symlink" "absent '$h/.local/bin/$app_id'"
check "script keep-data removes the desktop entry" \
  "absent '$h/.local/share/applications/$app_id.desktop'"
check "script keep-data removes the metainfo file" \
  "absent '$h/.local/share/metainfo/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml'"
check "script keep-data preserves the database" \
  "present '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3'"
check "script keep-data removes the portal restore token" \
  "absent '$h/.config/$app_id/voice-runtime/desktop-control-restore-token'"

h="$work/script-purge"; mkdir -p "$h/run"; simulate_install "$h"; seed_data "$h"
HOME="$h" XDG_RUNTIME_DIR="$h/run" bash uninstall-kde.sh --purge --yes >/dev/null
check "script purge removes the install payload" "absent '$h/.local/lib/$app_id'"
check "script purge removes the database" \
  "absent '$h/.local/share/com.aivarsrocens.aiagentcontrolcenter'"
check "script purge removes voice data" "absent '$h/.local/share/$app_id'"
check "script purge leaves the throwaway HOME otherwise intact" "present '$h'"

echo
if (( fail )); then
  echo "staged install/removal test: FAILED"
  exit 1
fi
echo "staged install/removal test: passed"
