#!/usr/bin/env bash

set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
app_id="ai-agent-control-center"
install_dir="$HOME/.local/lib/$app_id"
desktop_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/512x512/apps"

for command_name in npm cargo rustc; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name"
    echo "Install the Arch Linux Tauri prerequisites, then run this installer again."
    exit 1
  fi
done

if ! command -v codex >/dev/null 2>&1 && [[ ! -x "$HOME/.local/bin/codex" ]]; then
  echo "Warning: Codex CLI was not found. The app will install, but autonomous agents"
  echo "will remain offline until Codex is installed and signed in with ChatGPT."
fi

cd "$project_dir"
npm ci
npm run tauri -- build --no-bundle

# Older builds allowed multiple tray instances. Stop them before replacing the binary.
pkill -f -x "$install_dir/$app_id" 2>/dev/null || true
pkill -f "$install_dir/voice-runtime/listener.py" 2>/dev/null || true

install -Dm755 \
  "$project_dir/src-tauri/target/release/$app_id" \
  "$install_dir/$app_id"

install -d "$install_dir/voice-runtime"
install -Dm755 \
  "$project_dir/voice-runtime/setup.sh" \
  "$install_dir/voice-runtime/setup.sh"
install -Dm755 \
  "$project_dir/voice-runtime/setup-high-accuracy.sh" \
  "$install_dir/voice-runtime/setup-high-accuracy.sh"
install -Dm644 \
  "$project_dir/voice-runtime/listener.py" \
  "$install_dir/voice-runtime/listener.py"

voice_config="$HOME/.local/share/$app_id/voice-runtime/listener-config.json"
if [[ -f "$voice_config" ]]; then
  sed -i 's/"wakePhrase":"lucy activate, on"/"wakePhrase":"lucy"/' "$voice_config"
fi

# The Codex runtime uses ChatGPT login and does not need the legacy API key.
"$install_dir/$app_id" --remove-credentials || true

install -Dm644 \
  "$project_dir/src-tauri/icons/icon.png" \
  "$icon_dir/$app_id.png"

install -d "$desktop_dir"
sed \
  -e "s|@EXEC@|$install_dir/$app_id|g" \
  -e "s|@ICON@|$icon_dir/$app_id.png|g" \
  "$project_dir/packaging/$app_id.desktop.in" \
  > "$desktop_dir/$app_id.desktop"

chmod 644 "$desktop_dir/$app_id.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$desktop_dir"
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
fi

if systemctl --user --quiet is-active plasma-plasmashell.service 2>/dev/null; then
  systemctl --user restart plasma-plasmashell.service || true
fi

echo "AI Agent Control Center is installed. Open it from KDE's application menu."
echo "In Settings, select a project workspace before running an autonomous agent."
