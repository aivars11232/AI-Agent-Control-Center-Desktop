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

run npm test
run npm run typecheck
run python3 -B -W error::ResourceWarning -m unittest discover -s tests/voice_runtime -p 'test_*.py'
run cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
run cargo test --manifest-path src-tauri/Cargo.toml --locked --offline

printf "Fast non-live verification passed.\n"
