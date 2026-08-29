#!/usr/bin/env bash

# TASK-0019 packaging validation. No install, build, or system-control action.
#
#   * shell syntax + shellcheck (when available) for every packaging script;
#   * desktop-file-validate on the rendered desktop entry;
#   * appstreamcli validate on the AppStream metainfo;
#   * makepkg --printsrcinfo parse of the PKGBUILD;
#   * a check that install-kde.sh no longer restarts plasmashell.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail=0
strict="${VERIFY_STRICT:-0}"
report() {
  if eval "$2"; then printf 'ok   %s\n' "$1"; else printf 'FAIL %s\n' "$1"; fail=1; fi
}
need() {
  if command -v "$1" >/dev/null 2>&1; then return 0; fi
  if [[ "$strict" == "1" ]]; then
    printf 'FAIL %s is required in strict mode\n' "$1"; fail=1
  else
    printf 'skip %s unavailable\n' "$1"
  fi
  return 1
}

scripts=(
  install-kde.sh
  uninstall-kde.sh
  scripts/verify-fast.sh
  scripts/verify-full.sh
  scripts/check-packaging.sh
  scripts/check-licenses.sh
  scripts/staged-install-test.sh
  voice-runtime/setup.sh
  voice-runtime/setup-high-accuracy.sh
  packaging/ai-agent-control-center.install
)

echo "== shell syntax =="
report "bash -n on all packaging scripts" "bash -n ${scripts[*]}"

echo
echo "== shellcheck =="
if need shellcheck; then
  # SC2016: intentional single-quoted node -e / awk program text.
  # SC2317: functions referenced only indirectly (report/check helpers).
  report "shellcheck packaging scripts" \
    "shellcheck -x -e SC2016,SC2317 install-kde.sh uninstall-kde.sh scripts/verify-fast.sh scripts/verify-full.sh scripts/check-packaging.sh scripts/check-licenses.sh scripts/staged-install-test.sh voice-runtime/setup.sh voice-runtime/setup-high-accuracy.sh"
  # packaging/*.install is sourced by pacman with helper functions predefined.
  report "shellcheck pacman install hook" \
    "shellcheck -s bash -e SC2148,SC2317 packaging/ai-agent-control-center.install"
fi

echo
echo "== desktop entry =="
render_dir="$(mktemp -d)"
trap 'rm -rf "$render_dir"' EXIT
rendered="$render_dir/ai-agent-control-center.desktop"
sed -e 's|@EXEC@|/usr/bin/ai-agent-control-center|g' \
    -e 's|@ICON@|ai-agent-control-center|g' \
    packaging/ai-agent-control-center.desktop.in > "$rendered"
if need desktop-file-validate; then
  report "desktop-file-validate rendered entry" "desktop-file-validate '$rendered'"
fi
report "desktop entry has exactly one main category" \
  "[ \"\$(grep -c '^Categories=Development;\$' packaging/ai-agent-control-center.desktop.in)\" = 1 ]"

echo
echo "== AppStream metainfo =="
if need appstreamcli; then
  report "appstreamcli validate metainfo" \
    "appstreamcli validate --no-net packaging/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml"
fi
report "metainfo declares the proprietary project license" \
  "grep -q 'LicenseRef-proprietary' packaging/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml"

echo
echo "== PKGBUILD =="
if command -v pacman >/dev/null 2>&1 || command -v makepkg >/dev/null 2>&1; then
  if need makepkg; then
    report "makepkg --printsrcinfo parses the PKGBUILD" \
      "( cd packaging && makepkg --printsrcinfo >/dev/null )"
  fi
else
  printf 'skip makepkg checks (not an Arch environment)\n'
fi
report "PKGBUILD declares the proprietary license" \
  "grep -q \"license=('LicenseRef-proprietary')\" packaging/PKGBUILD"
report "PKGBUILD installs the license file" \
  "grep -q 'usr/share/licenses' packaging/PKGBUILD"

echo
echo "== plasmashell restart removed =="
report "install-kde.sh does not restart plasmashell" \
  "! grep -Eq 'restart[[:space:]]+plasma-plasmashell|plasmashell --replace' install-kde.sh"

echo
if (( fail )); then echo "packaging validation: FAILED"; exit 1; fi
echo "packaging validation: passed"
