#!/usr/bin/env bash

# User-local removal for AI Agent Control Center.
#
#   uninstall-kde.sh              remove the application, keep user data
#                                 (SQLite database + downloaded voice models).
#   uninstall-kde.sh --purge      also irreversibly delete ALL local data:
#                                 database, voice models, config, caches, logs,
#                                 and the KDE portal restore token.
#   uninstall-kde.sh --purge --yes   skip the interactive PURGE confirmation.
#
# Data removal is delegated to the installed binary so paths never drift from
# the code that writes them. In every mode the tray process and the offline
# voice listener are stopped and the stored provider key is cleared.

set -euo pipefail

app_id="ai-agent-control-center"
lib_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
install_dir="$HOME/.local/lib/$app_id"
installed_bin="$install_dir/$app_id"
launcher="$HOME/.local/bin/$app_id"
desktop_file="$lib_dir/applications/$app_id.desktop"
icon_file="$lib_dir/icons/hicolor/512x512/apps/$app_id.png"
metainfo_file="$lib_dir/metainfo/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml"
license_dir="$lib_dir/licenses/$app_id"

purge=0
assume_yes=0
for argument in "$@"; do
  case "$argument" in
    --purge) purge=1 ;;
    --yes|-y) assume_yes=1 ;;
    *) echo "unknown option: $argument" >&2; exit 2 ;;
  esac
done

warn() { printf 'warning: %s\n' "$*" >&2; }

# --- data removal (delegated to the binary) ---------------------------
if [[ -x "$installed_bin" ]]; then
  if (( purge )); then
    if (( ! assume_yes )); then
      echo "This will irreversibly delete every local AI Agent Control Center data"
      echo "store, including the database and downloaded voice models."
      read -r -p "Type PURGE to continue: " reply
      if [[ "$reply" != "PURGE" ]]; then
        echo "Aborted; nothing was removed."
        exit 1
      fi
    fi
    "$installed_bin" --purge --confirm PURGE
  else
    "$installed_bin" --uninstall
  fi
else
  warn "installed binary not found at $installed_bin"
  warn "the tray process, voice listener, and per-user data could not be cleaned"
  warn "automatically. Reinstall, then run this script again for a full removal."
fi

# --- application files ------------------------------------------------
rm -f "$launcher" "$desktop_file" "$icon_file" "$metainfo_file"
rm -rf "$install_dir" "$license_dir"

# --- non-disruptive cache refresh (no plasmashell restart) ------------
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$lib_dir/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$lib_dir/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi

if (( purge )); then
  echo "AI Agent Control Center and all local data have been removed."
else
  echo "AI Agent Control Center has been removed. User data was preserved;"
  echo "run '$0 --purge' to delete it as well."
fi
