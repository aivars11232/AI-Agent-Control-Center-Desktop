#!/usr/bin/env bash

# TASK-0019 third-party license gate.
#
# AI Agent Control Center is proprietary (LICENSE). Every bundled or linked
# third-party component must be distributable under a permissive license. This
# script fails the release build if a Rust crate or npm package carries a
# license that is incompatible with proprietary distribution (GPL/AGPL, a
# standalone LGPL, SSPL, and similar), or an unknown license.
#
# It reads the checked-in lockfiles only; it installs nothing.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Permissive SPDX identifiers accepted for proprietary distribution. For an
# "A OR B" expression any accepted operand passes; for "A AND B" all must pass.
export AACC_ALLOWED_LICENSES="MIT,MIT-0,Apache-2.0,Apache-2.0 WITH LLVM-exception,BSD-2-Clause,BSD-3-Clause,BSD-3-Clause-Clear,ISC,Zlib,0BSD,BSL-1.0,Unlicense,Unicode-3.0,Unicode-DFS-2016,CC0-1.0,CC-BY-3.0,CC-BY-4.0,MPL-2.0,WTFPL,BlueOak-1.0.0,NCSA,OpenSSL,PSF-2.0,LicenseRef-proprietary"
# Crates/packages whose own project is this proprietary application.
export AACC_OWN_PACKAGES="ai-agent-control-center"

evaluate='
const allowed = new Set(
  process.env.AACC_ALLOWED_LICENSES.split(",").map((value) => value.trim())
);
const own = new Set(process.env.AACC_OWN_PACKAGES.split(",").map((v) => v.trim()));
// Normalise legacy "A/B" spellings to SPDX "A OR B" (SPDX identifiers never
// contain a slash, so this is safe).
function normalise(expr) {
  return String(expr || "").replace(/\s*\/\s*/g, " OR ");
}
function acceptable(expr) {
  const e = normalise(String(expr || "")).trim();
  if (!e) return false;
  // OR: any operand acceptable. Split on top-level OR only (no nested parens here).
  if (/\bOR\b/i.test(e)) return e.split(/\bOR\b/i).some((part) => acceptable(part));
  // AND: every operand acceptable.
  if (/\bAND\b/i.test(e)) return e.split(/\bAND\b/i).every((part) => acceptable(part));
  const token = e.replace(/^[()\s]+|[()\s]+$/g, "");
  return allowed.has(token);
}
module.exports = { allowed, own, acceptable };
'
evaluator="$(mktemp)"
trap 'rm -f "$evaluator"' EXIT
printf '%s' "$evaluate" > "$evaluator"

fail=0

echo "== Rust crates (src-tauri/Cargo.lock) =="
cargo metadata --format-version 1 --locked --manifest-path src-tauri/Cargo.toml \
  | node -e '
const { acceptable, own } = require(process.argv[1]);
const meta = JSON.parse(require("fs").readFileSync(0, "utf8"));
const bad = [];
for (const pkg of meta.packages) {
  if (own.has(pkg.name)) continue;
  const license = pkg.license || (pkg.license_file ? "LicenseRef-proprietary" : "");
  if (!acceptable(license)) bad.push(`${pkg.name} ${pkg.version} -> ${license || "UNKNOWN"}`);
}
if (bad.length) {
  console.log("DISALLOWED:");
  for (const line of bad) console.log("  " + line);
  process.exit(1);
}
console.log(`ok   ${meta.packages.length - 1} crates, all permissive`);
' "$evaluator" || fail=1

echo
echo "== npm packages (package-lock.json) =="
node -e '
const { acceptable } = require(process.argv[1]);
const fs = require("fs");
const path = require("path");
const bad = [];
function scan(dir) {
  let names;
  try { names = fs.readdirSync(dir); } catch { return; }
  for (const name of names) {
    if (name.startsWith(".")) continue;
    const full = path.join(dir, name);
    if (name.startsWith("@")) { scan(full); continue; }
    const pj = path.join(full, "package.json");
    if (fs.existsSync(pj)) {
      try {
        const j = JSON.parse(fs.readFileSync(pj, "utf8"));
        let lic = j.license;
        if (!lic && Array.isArray(j.licenses)) lic = j.licenses.map((l) => l.type || l).join(" OR ");
        if (typeof lic === "object" && lic) lic = lic.type;
        if (!acceptable(lic)) bad.push(`${j.name || name}@${j.version || "?"} -> ${lic || "UNKNOWN"}`);
      } catch {}
    }
    const nested = path.join(full, "node_modules");
    if (fs.existsSync(nested)) scan(nested);
  }
}
scan("node_modules");
if (bad.length) {
  console.log("DISALLOWED:");
  for (const line of bad) console.log("  " + line);
  process.exit(1);
}
console.log("ok   all installed npm packages permissive");
' "$evaluator" || fail=1

if command -v cargo-deny >/dev/null 2>&1 && [[ -f deny.toml ]]; then
  echo
  echo "== cargo-deny =="
  cargo deny --manifest-path src-tauri/Cargo.toml check licenses bans advisories sources || fail=1
elif [[ "${VERIFY_STRICT:-0}" == "1" ]]; then
  echo
  echo "FAIL cargo-deny is required in strict mode"
  fail=1
fi

echo
if (( fail )); then echo "license gate: FAILED"; exit 1; fi
echo "license gate: passed"
