#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

# VERIFY_STRICT=1 (set by CI) turns "advisory tooling unavailable" into a hard
# failure. Locally, missing optional tooling is reported but does not fail.
strict="${VERIFY_STRICT:-0}"

run() {
  printf "+"
  printf " %q" "$@"
  printf "\n"
  "$@"
}

run npm run verify:fast
run npm run build
run cargo clippy --manifest-path src-tauri/Cargo.toml --locked --offline --all-targets -- -D warnings
run bash -n \
  scripts/verify-fast.sh \
  scripts/verify-full.sh \
  scripts/check-packaging.sh \
  scripts/check-licenses.sh \
  scripts/staged-install-test.sh \
  scripts/pacman-transaction-test.sh \
  install-kde.sh \
  uninstall-kde.sh \
  voice-runtime/setup.sh \
  voice-runtime/setup-high-accuracy.sh
run python3 -B -c 'import ast; from pathlib import Path; path = Path("voice-runtime/listener.py"); ast.parse(path.read_text(encoding="utf-8"), filename=str(path)); print(f"valid Python syntax: {path}")'
run node --input-type=module -e 'import { readFileSync } from "node:fs"; for (const file of process.argv.slice(1)) { JSON.parse(readFileSync(file, "utf8")); console.log(`valid JSON: ${file}`); }' \
  package.json \
  package-lock.json \
  src-tauri/capabilities/default.json \
  src-tauri/tauri.conf.json
run npm ls --all
run cargo tree --manifest-path src-tauri/Cargo.toml --locked --offline
run npm audit --omit=dev --audit-level=moderate
run npm audit --audit-level=moderate

# --- TASK-0019 packaging, licensing, and removal gates ---------------
run env VERIFY_STRICT="$strict" bash scripts/check-licenses.sh
run env VERIFY_STRICT="$strict" bash scripts/check-packaging.sh
run bash scripts/staged-install-test.sh
# A real libalpm transaction for the built Arch package, as namespaced root in
# an unprivileged user namespace. Skips on a non-Arch image (CI) or without a
# built package; the skip is deliberate and not strict-gated.
run bash scripts/pacman-transaction-test.sh

if command -v shellcheck >/dev/null 2>&1; then
  run shellcheck -x -e SC2016,SC2317 \
    scripts/verify-fast.sh scripts/verify-full.sh scripts/check-packaging.sh \
    scripts/check-licenses.sh scripts/staged-install-test.sh \
    scripts/pacman-transaction-test.sh \
    install-kde.sh uninstall-kde.sh \
    voice-runtime/setup.sh voice-runtime/setup-high-accuracy.sh
elif [[ "$strict" == "1" ]]; then
  printf "FAIL: shellcheck is required in strict mode.\n"
  exit 1
else
  printf "SKIP: shellcheck is unavailable.\n"
fi

rust_advisory_status="INDETERMINATE"
if command -v cargo-audit >/dev/null 2>&1; then
  run cargo audit --file src-tauri/Cargo.lock
  rust_advisory_status="PASSED"
elif command -v cargo-deny >/dev/null 2>&1; then
  run cargo deny --manifest-path src-tauri/Cargo.toml check advisories
  rust_advisory_status="PASSED"
elif [[ "$strict" == "1" ]]; then
  printf "FAIL: cargo-audit or cargo-deny is required in strict mode.\n"
  exit 1
else
  printf "SKIP: cargo-audit/cargo-deny are unavailable; Rust advisory status is INDETERMINATE.\n"
fi

printf "Full non-live verification completed. Rust advisory status: %s.\n" "$rust_advisory_status"
