#!/usr/bin/env bash

set -euo pipefail

app_id="ai-agent-control-center"
install_dir="$HOME/.local/lib/$app_id"
desktop_file="$HOME/.local/share/applications/$app_id.desktop"
icon_file="$HOME/.local/share/icons/hicolor/512x512/apps/$app_id.png"

if [[ -x "$install_dir/$app_id" ]]; then
  "$install_dir/$app_id" --remove-credentials || true
fi

rm -f "$desktop_file"
rm -f "$icon_file"
rm -rf "$install_dir"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications"
fi

echo "AI Agent Control Center has been removed from this user account."
