#!/usr/bin/env bash

# TASK-0027 real pacman transaction test for the built Arch package.
#
# `pacman -U` and `pacman -R` require root, which this repository's
# verification routes do not have. This test still exercises a real libalpm
# transaction by running pacman as namespaced root inside an unprivileged user
# namespace, against a minimal Arch root built from the local package cache.
#
# What that makes real:
#   * a genuine `pacman -U` transaction: conflict, integrity, file-conflict and
#     disk-space checks, extraction, and local database registration;
#   * genuine chroot execution of the packaged `.INSTALL` scriptlet, so the
#     post_install text is observed rather than inferred from the hook source;
#   * a genuine `pacman -R` transaction and its file removal.
#
# What it deliberately does NOT do: touch the host's pacman database, install
# anything on the running system, require sudo, or start the GUI. The packaged
# binary is smoke-tested from the installed tree against the host's libraries
# under a throwaway $HOME, exactly as the staged install test does.
#
# It does NOT replace a transaction against the live system database: the host's
# installed package set, its own hooks, and its file-conflict surface are not
# represented here. That case needs root and is recorded separately.
#
# Skips cleanly (exit 0) when the environment cannot support it: no pacman, no
# built package, no unprivileged user namespaces, or no subordinate uid range.
# CI runs on a non-Arch image and therefore skips; the skip is deliberate and
# is not upgraded to a failure under VERIFY_STRICT.
#
# Set AACC_PACMAN_PACKAGE to test a specific package file.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

app_id="ai-agent-control-center"
cache_dir="${AACC_PACMAN_CACHE:-/var/cache/pacman/pkg}"

skip() { printf 'skip pacman transaction test: %s\n' "$*"; exit 0; }

fail=0
check() {
  if eval "$2"; then
    printf 'ok   %s\n' "$1"
  else
    printf 'FAIL %s\n' "$1"
    fail=1
  fi
}

# --- environment preflight ----------------------------------------------
command -v pacman  >/dev/null 2>&1 || skip "pacman is unavailable (not an Arch environment)"
command -v unshare >/dev/null 2>&1 || skip "unshare is unavailable"

package="${AACC_PACMAN_PACKAGE:-}"
if [[ -z "$package" ]]; then
  for candidate in packaging/*.pkg.tar.*; do
    [[ -e "$candidate" && "$candidate" != *.sig ]] || continue
    if [[ -z "$package" || "$candidate" -nt "$package" ]]; then
      package="$candidate"
    fi
  done
fi
[[ -n "$package" && -e "$package" ]] || skip "no built package in packaging/ (run: cd packaging && makepkg -f)"
package="$(cd -- "$(dirname -- "$package")" && pwd)/$(basename -- "$package")"

# `--map-auto` needs a subordinate uid/gid range: pacman chowns its download
# directory to a non-root uid, which a single-uid mapping cannot represent.
grep -q "^$(id -un):" /etc/subuid 2>/dev/null || skip "no subordinate uid range for $(id -un) in /etc/subuid"
grep -q "^$(id -un):" /etc/subgid 2>/dev/null || skip "no subordinate gid range for $(id -un) in /etc/subgid"
unshare --user --map-root-user --map-auto -- true 2>/dev/null \
  || skip "unprivileged user namespaces are unavailable"

# The minimal root needs a working /bin/sh and `cat` for the scriptlet chroot:
# the install hook is a `cat` heredoc and bash has no `cat` builtin.
base_packages=()
for name in filesystem glibc bash coreutils ncurses readline libxcrypt gcc-libs \
            iana-etc tzdata linux-api-headers; do
  newest=""
  for candidate in "$cache_dir/$name"-[0-9]*.pkg.tar.*; do
    [[ -e "$candidate" && "$candidate" != *.sig ]] || continue
    if [[ -z "$newest" || "$candidate" -nt "$newest" ]]; then
      newest="$candidate"
    fi
  done
  [[ -n "$newest" ]] || skip "$name is not in $cache_dir; cannot build a minimal root offline"
  base_packages+=("$newest")
done

echo "package: $package"
echo "cache:   $cache_dir"

work="$(mktemp -d)"
trap 'chmod -R u+w "$work" 2>/dev/null || true; rm -rf "$work"' EXIT
root="$work/root"
mkdir -p "$root/var/lib/pacman" "$root/var/cache/pacman/pkg" "$root/var/log"

# `-dd` skips dependency resolution: the minimal root deliberately contains no
# runtime dependencies, and the declared dependency set is separately asserted
# against the PKGBUILD below and gated by namcap in scripts/check-packaging.sh.
pac() {
  unshare --user --map-root-user --map-auto -- \
    pacman "$@" --root "$root" --dbpath "$root/var/lib/pacman" \
    --cachedir "$root/var/cache/pacman/pkg" --logfile "$root/var/log/pacman.log" \
    2>&1 | grep -vE "^warning: database file for '.*' does not exist" || true
}

# The no-PlasmaShell-restart guarantee: nothing here may disturb the session.
plasma_before="$(pgrep -x plasmashell | head -1 || true)"

echo
echo "== minimal root =="
base_log="$(pac -U --noconfirm -dd "${base_packages[@]}")"
base_log_has() { printf '%s\n' "$base_log" | grep -q "$1"; }
check "the minimal root installs from the local cache" \
  "base_log_has 'Processing package changes'"
check "the minimal root provides /bin/sh for chroot scriptlets" "[ -e '$root/bin/sh' ]"
check "the minimal root provides cat for the install hook" "[ -x '$root/usr/bin/cat' ]"
if (( fail )); then
  printf '%s\n' "$base_log"
  echo "pacman transaction test: FAILED (minimal root)"
  exit 1
fi

echo
echo "== pacman -U =="
install_log="$(pac -U --noconfirm -dd "$package")"
install_log_has()   { printf '%s\n' "$install_log" | grep -q "$1"; }
install_log_hasf()  { printf '%s\n' "$install_log" | grep -qF -- "$1"; }
install_log_clean() { ! printf '%s\n' "$install_log" | grep -q '^error:'; }
check "pacman -U commits the transaction" "install_log_has 'installing $app_id'"
check "pacman -U reports no error" "install_log_clean"

# The scriptlet runs chrooted into the new root, so this output is produced by
# the packaged .INSTALL, not read from the source tree.
check "the post_install hook runs and states the install path" \
  "install_log_has 'installed at /usr/bin/$app_id'"
check "the post_install hook states pacman -R cannot remove per-user data" \
  "install_log_has 'cannot remove per-user data'"
check "the post_install hook names the keep-data removal command" \
  "install_log_hasf '$app_id --uninstall'"
check "the post_install hook names the purge command" \
  "install_log_hasf '--purge --confirm PURGE'"
check "the post_install hook states the KDE grant is revoked in System Settings" \
  "install_log_has 'KDE System Settings'"

# Every line of the shipped hook's message must actually reach the user.
hook_missing=0
while IFS= read -r line; do
  [[ -n "$line" ]] || continue
  install_log_hasf "$line" || {
    printf '  hook line not emitted: %s\n' "$line"
    hook_missing=1
  }
done < <(sed -n "/<<'MSG'/,/^MSG$/p" "packaging/$app_id.install" | sed '1d;$d')
check "every line of the shipped install hook is emitted verbatim" \
  "[ '$hook_missing' = 0 ]"

echo
echo "== installed package state =="
expected_version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' src-tauri/Cargo.toml | head -1)"
pkgrel="$(sed -n 's/^pkgrel=\(.*\)$/\1/p' packaging/PKGBUILD | head -1)"
query_version="$(pac -Q "$app_id" | tr -d '\r')"
registered_version_is() { [[ "$query_version" == "$1" ]]; }
check "the local database registers $app_id $expected_version-$pkgrel" \
  "registered_version_is '$app_id $expected_version-$pkgrel'"

# `-Ql` prints paths prefixed with --root; strip it back to package paths.
installed_files="$(pac -Ql "$app_id" | sed -e "s|^$app_id ||" -e "s|^$root||" \
  | grep -v '/$' | sort)"
expected_files="$(printf '%s\n' \
  "/usr/bin/$app_id" \
  "/usr/lib/$app_id/$app_id" \
  "/usr/lib/$app_id/voice-runtime/listener.py" \
  "/usr/lib/$app_id/voice-runtime/setup-high-accuracy.sh" \
  "/usr/lib/$app_id/voice-runtime/setup.sh" \
  "/usr/share/applications/$app_id.desktop" \
  "/usr/share/icons/hicolor/512x512/apps/$app_id.png" \
  "/usr/share/licenses/$app_id/LICENSE" \
  "/usr/share/licenses/$app_id/THIRD-PARTY-NOTICES.md" \
  "/usr/share/metainfo/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml" | sort)"
installed_files_match() {
  if [[ "$installed_files" == "$expected_files" ]]; then return 0; fi
  diff <(printf '%s\n' "$expected_files") <(printf '%s\n' "$installed_files") \
    | sed 's/^/  /' || true
  return 1
}
check "the installed file list is exactly the ten expected payload files" \
  "installed_files_match"
check "/usr/bin/$app_id is a symlink onto the payload" \
  "[ \"\$(readlink '$root/usr/bin/$app_id')\" = '/usr/lib/$app_id/$app_id' ]"

# The registered metadata must match the PKGBUILD that produced it.
package_info="$(pac -Qi "$app_id")"
declared_depends="$(sed -n "s/^depends=(\(.*\))$/\1/p" packaging/PKGBUILD \
  | tr -d "'" | tr ' ' '\n' | sort | sed '/^$/d')"
registered_depends="$(printf '%s\n' "$package_info" \
  | sed -n 's/^Depends On *: *//p' | tr ' ' '\n' | sort | sed '/^$/d')"
depends_match() { [[ "$declared_depends" == "$registered_depends" ]]; }
info_has() { printf '%s\n' "$package_info" | grep -q "$1"; }
check "the registered dependencies are exactly the PKGBUILD depends" "depends_match"
check "the package declares the proprietary license in the database" \
  "info_has 'LicenseRef-proprietary'"

echo
echo "== packaged binary smoke =="
# Run the pacman-installed payload against the host's libraries (the minimal
# root has none) under a throwaway $HOME, so no real user data is touched.
binary="$root/usr/lib/$app_id/$app_id"
smoke_home="$work/home"
mkdir -p "$smoke_home/run"
run_bin() { HOME="$smoke_home" XDG_RUNTIME_DIR="$smoke_home/run" "$binary" "$@"; }
purge_refuses_with_exit_2() {
  local status=0
  run_bin --purge >/dev/null 2>&1 || status=$?
  [[ "$status" == 2 ]]
}
check "the installed binary is executable" "[ -x '$binary' ]"
check "--version reports the crate version" \
  "run_bin --version | grep -qx '$app_id $expected_version'"
check "--help lists the purge command" \
  "run_bin --help | grep -q -- '--purge --confirm PURGE'"
check "--print-data-paths reports the owned locations on a clean home" \
  "run_bin --print-data-paths | grep -q 'owned location(s) tracked'"
check "--stop-runtime succeeds with no owned processes" \
  "run_bin --stop-runtime | grep -q 'no owned processes'"
check "--purge without confirmation refuses with exit 2" "purge_refuses_with_exit_2"

# Per-user data the package does not own must survive `pacman -R`, which is
# exactly what the post_install hook promises.
sentinel="$smoke_home/.local/share/com.aivarsrocens.aiagentcontrolcenter/application-state.sqlite3"
mkdir -p "$(dirname "$sentinel")"
echo sentinel > "$sentinel"

echo
echo "== pacman -R =="
remove_log="$(pac -R --noconfirm "$app_id")"
remove_log_has()   { printf '%s\n' "$remove_log" | grep -q "$1"; }
remove_log_clean() { ! printf '%s\n' "$remove_log" | grep -q '^error:'; }
deregistered()     { ! pac -Q "$app_id" | grep -q "^$app_id "; }
check "pacman -R commits the transaction" "remove_log_has 'removing $app_id'"
check "pacman -R reports no error" "remove_log_clean"
check "pacman -R deregisters the package" "deregistered"
while IFS= read -r path; do
  check "pacman -R removes $path" "[ ! -e '$root'\"$path\" ]"
done <<< "$expected_files"
check "pacman -R leaves per-user data untouched, as the hook promises" \
  "[ -e '$sentinel' ]"

echo
echo "== session integrity =="
plasma_after="$(pgrep -x plasmashell | head -1 || true)"
plasmashell_unchanged() { [[ "$plasma_before" == "$plasma_after" ]]; }
check "plasmashell was not restarted by the transaction" "plasmashell_unchanged"

echo
if (( fail )); then
  echo "pacman transaction test: FAILED"
  exit 1
fi
echo "pacman transaction test: passed"
