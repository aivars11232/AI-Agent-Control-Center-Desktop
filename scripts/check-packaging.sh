#!/usr/bin/env bash

# TASK-0019 packaging validation. No install, build, or system-control action.
#
#   * shell syntax + shellcheck (when available) for every packaging script;
#   * desktop-file-validate on the rendered desktop entry;
#   * appstreamcli validate on the AppStream metainfo;
#   * makepkg --printsrcinfo parse of the PKGBUILD;
#   * namcap on the PKGBUILD (and on a built package when one is present),
#     allowing only the explicitly justified findings listed below;
#   * release-version parity across every manifest that carries a version;
#   * a check that install-kde.sh no longer restarts plasmashell.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail=0
strict="${VERIFY_STRICT:-0}"
app_warning='ai-agent-control-center W:'
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
  scripts/pacman-transaction-test.sh
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
    "shellcheck -x -e SC2016,SC2317 install-kde.sh uninstall-kde.sh scripts/verify-fast.sh scripts/verify-full.sh scripts/check-packaging.sh scripts/check-licenses.sh scripts/staged-install-test.sh scripts/pacman-transaction-test.sh voice-runtime/setup.sh voice-runtime/setup-high-accuracy.sh"
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
echo "== version parity =="
# Every manifest that carries the release version must agree with the Rust
# crate version the binary reports through `--version`. Drift here ships a
# package whose metadata contradicts its own binary.
json_top_level_version() { sed -n 's/^  "version": "\([^"]*\)".*/\1/p' "$1" | head -1; }
cargo_version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' src-tauri/Cargo.toml | head -1)"
npm_version="$(json_top_level_version package.json)"
tauri_version="$(json_top_level_version src-tauri/tauri.conf.json)"
pkgbuild_version="$(sed -n 's/^pkgver=\(.*\)$/\1/p' packaging/PKGBUILD | head -1)"
metainfo_version="$(sed -n 's/.*<release version="\([^"]*\)".*/\1/p' \
  packaging/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml | head -1)"
printf 'Cargo.toml=%s package.json=%s tauri.conf.json=%s PKGBUILD=%s metainfo=%s\n' \
  "${cargo_version:-<none>}" "${npm_version:-<none>}" "${tauri_version:-<none>}" \
  "${pkgbuild_version:-<none>}" "${metainfo_version:-<none>}"
report "a version was extracted from every manifest" \
  "[ -n \"\$cargo_version\" ] && [ -n \"\$npm_version\" ] && [ -n \"\$tauri_version\" ] \
   && [ -n \"\$pkgbuild_version\" ] && [ -n \"\$metainfo_version\" ]"
report "package.json version matches src-tauri/Cargo.toml" \
  "[ \"\$npm_version\" = \"\$cargo_version\" ]"
report "tauri.conf.json version matches src-tauri/Cargo.toml" \
  "[ \"\$tauri_version\" = \"\$cargo_version\" ]"
report "PKGBUILD pkgver matches src-tauri/Cargo.toml" \
  "[ \"\$pkgbuild_version\" = \"\$cargo_version\" ]"
report "metainfo newest release matches src-tauri/Cargo.toml" \
  "[ \"\$metainfo_version\" = \"\$cargo_version\" ]"

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
echo "== namcap =="
# namcap exits 0 even when it prints findings, so the gate parses its output and
# fails on any line that is not an explicitly justified finding. Findings are
# matched as extended regexes because namcap emits the library / module lists
# inside a finding in a non-deterministic order.
#
# Justified for the PKGBUILD:
#
#   E: File referenced in $startdir
#       This is a local, private, non-AUR PKGBUILD that deliberately builds the
#       checked-out repository shipping it (`_srcroot="$startdir/.."`). There is
#       no downloadable source tuple to reference instead, and the proprietary
#       license forbids AUR publication, so the finding is by design.
namcap_allowed_pkgbuild=(
  '^PKGBUILD \(ai-agent-control-center\) E: File referenced in \$startdir$'
)
# Justified for the built package:
#
#   Referenced python module 'vosk.*' is an uninstalled dependency
#       The offline voice runtime installs Vosk into a per-user virtual
#       environment through voice-runtime/setup.sh. It is optional and never a
#       system package, which is why `python` is an optdepend, not a depend.
#   Dependency python detected but optional
#       Same reason: the listener is only reachable through the optional
#       offline-voice runtime.
#   Unused shared library '/usr/lib64/ld-linux-x86-64.so.2'
#       The ELF interpreter itself; namcap reports it for every binary.
#   Dependency <name> detected and implicitly satisfied
#       Transitive through a declared dependency. Arch packaging guidelines say
#       not to declare these again. The allowed names are pinned, so a NEW
#       implicitly-satisfied dependency fails the gate and needs a decision.
#   Dependency included, but may not be needed ('libappindicator-gtk3')
#       The Tauri tray dlopen's libayatana-appindicator3.so.1 /
#       libappindicator3.so.1 at runtime, so namcap's ELF scan cannot see the
#       reference. It is required and stays declared.
namcap_allowed_package=(
  "^$app_warning Referenced python module 'vosk\.[A-Za-z]+' is an uninstalled dependency "
  "^$app_warning Dependency python detected but optional "
  "^$app_warning Unused shared library '/usr/lib64/ld-linux-x86-64\.so\.2' "
  "^$app_warning Dependency (cairo|libsoup3|dbus|libgcc|hicolor-icon-theme|gdk-pixbuf2|glib2|glibc|bash) detected and implicitly satisfied "
  "^$app_warning Dependency included, but may not be needed \('libappindicator-gtk3'\)$"
)
namcap_gate() {
  local target="$1" kind="$2" output allowed=() pattern
  case "$kind" in
    pkgbuild) allowed=("${namcap_allowed_pkgbuild[@]}") ;;
    package) allowed=("${namcap_allowed_package[@]}") ;;
  esac
  output="$(namcap "$target" 2>&1)"
  for pattern in "${allowed[@]}"; do
    output="$(printf '%s\n' "$output" | grep -vE "$pattern" || true)"
  done
  output="$(printf '%s' "$output" | sed '/^$/d')"
  if [[ -n "$output" ]]; then
    printf 'unjustified namcap findings for %s:\n%s\n' "$target" "$output"
    return 1
  fi
  return 0
}
if need namcap; then
  report "namcap on the PKGBUILD reports only justified findings" \
    "namcap_gate packaging/PKGBUILD pkgbuild"
  built_package=""
  for candidate in packaging/*.pkg.tar.*; do
    [[ -e "$candidate" && "$candidate" != *.sig ]] || continue
    if [[ -z "$built_package" || "$candidate" -nt "$built_package" ]]; then
      built_package="$candidate"
    fi
  done
  if [[ -n "$built_package" ]]; then
    report "namcap on $built_package reports only justified findings" \
      "namcap_gate '$built_package' package"
  else
    printf 'skip namcap package check (no built package in packaging/)\n'
  fi
fi

echo
echo "== plasmashell restart removed =="
report "install-kde.sh does not restart plasmashell" \
  "! grep -Eq 'restart[[:space:]]+plasma-plasmashell|plasmashell --replace' install-kde.sh"

echo
if (( fail )); then echo "packaging validation: FAILED"; exit 1; fi
echo "packaging validation: passed"
