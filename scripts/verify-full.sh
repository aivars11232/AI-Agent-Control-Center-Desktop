#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

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

rust_advisory_status="INDETERMINATE"
if command -v cargo-audit >/dev/null 2>&1; then
  run cargo audit --file src-tauri/Cargo.lock
  rust_advisory_status="PASSED"
else
  printf "SKIP: cargo-audit is unavailable; Rust advisory status is INDETERMINATE.\n"
fi

printf "Full non-live verification completed. Rust advisory status: %s.\n" "$rust_advisory_status"
