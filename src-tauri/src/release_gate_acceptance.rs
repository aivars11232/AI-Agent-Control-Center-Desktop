//! TASK-0029 — composed release-candidate gate acceptance (S10).
//!
//! The individual subsystems are covered by their own acceptance modules
//! (`data_lifecycle_acceptance`, `orchestration_acceptance`,
//! `provider_review_acceptance`, `voice_kde_acceptance`,
//! `install_package_acceptance`). This module owns the layer above them: the
//! **release gate itself** — the shipped verification scripts and the CI
//! workflow that decide whether a candidate may ship.
//!
//! That layer had no regression coverage, and it broke: from TASK-0027 until
//! this task, `scripts/check-packaging.sh` required the Arch-only `namcap`
//! outside the Arch-environment guard, so `VERIFY_STRICT=1` on the Ubuntu
//! runner turned "this machine is not Arch" into `FAIL namcap is required in
//! strict mode`. CI was red on `main` for three consecutive pushes
//! (runs 33330544485, 33422628238, 33549730695) and the `licenses`, `secrets`
//! and `packaging` jobs never executed at all — the security gates were dark
//! while the tree looked "merely" red.
//!
//! The scenarios below therefore assert the properties a green candidate
//! depends on:
//!
//! * an Arch-only tool must **skip**, never strict-fail, off Arch;
//! * every mandatory gate is still wired into one ordered CI chain;
//! * CI still starts no provider, microphone, portal, installer, or publish
//!   step;
//! * the local full gate and CI demand the same security tooling; and
//! * the release version is one value across manifests *and* lockfiles.
//!
//! Everything here is deterministic, reads the shipped files from the checkout
//! under test, and touches no real machine state. The one scenario that
//! executes a script runs it read-only under a scratch `PATH` and a throwaway
//! `HOME`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Repository root (the parent of `src-tauri`), resolved from the crate
/// manifest so the shipped gate files are read from the checkout under test.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri always has a parent directory")
        .to_path_buf()
}

fn read_shipped(relative: &str) -> String {
    let path = repository_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} should be readable: {error}", path.display()))
}

/// Strip `#` comment lines so a contract assertion reads the executable text
/// rather than the prose that documents it.
fn executable_lines(script: &str) -> String {
    script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The version every shipped manifest and lockfile must agree on.
fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Tools that exist only on an Arch system. A release gate may require one of
/// these **only** behind an Arch-environment guard; requiring one
/// unconditionally is what took CI down after TASK-0027.
const ARCH_ONLY_TOOLS: [&str; 2] = ["makepkg", "namcap"];

/// The mandatory CI jobs, in the exact order they must run. Each one must
/// `needs:` the job before it, so the workflow stays a single ordered chain
/// rather than a fan-out that can green up while a later gate is skipped.
const MANDATORY_CI_CHAIN: [&str; 6] = [
    "frontend",
    "rust",
    "scripts",
    "licenses",
    "secrets",
    "packaging",
];

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

/// A throwaway directory that is removed when the scenario ends.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(label: &str) -> Self {
        let unique = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "aacc-task-0029-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory should be created");
        Self { path }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Build a `PATH` directory that mirrors the current one **except** for the
/// named tools, so a scenario can observe what the gate does on a machine that
/// simply does not have them. Nothing is installed, moved, or removed; the
/// scratch directory holds symlinks only.
fn path_without(scratch: &Path, hidden: &[&str]) -> String {
    let bin = scratch.join("bin");
    fs::create_dir_all(&bin).expect("scratch bin should be created");
    let hidden: BTreeSet<&str> = hidden.iter().copied().collect();

    let mut linked: BTreeSet<String> = BTreeSet::new();
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if hidden.contains(name.as_str()) || !linked.insert(name.clone()) {
                continue;
            }
            // First directory on PATH wins, exactly like real resolution.
            let _ = std::os::unix::fs::symlink(entry.path(), bin.join(&name));
        }
    }

    bin.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Scenario 1 — an Arch-only gate skips off Arch instead of failing the build
// ---------------------------------------------------------------------------

/// The exact TASK-0027 CI regression, reproduced as a test.
///
/// `scripts/check-packaging.sh` is executed with `VERIFY_STRICT=1` and a `PATH`
/// from which `pacman`, `makepkg` and `namcap` are absent — the Ubuntu runner's
/// situation. The namcap section must report a skip and must not report the
/// strict-mode failure that took the `scripts` job down.
///
/// The overall exit status is deliberately **not** asserted: the surrounding
/// non-Arch validators (`shellcheck`, `desktop-file-validate`, `appstreamcli`)
/// are strict-required and may legitimately be absent from whatever machine
/// runs this test, which would fail the script for an unrelated reason. The
/// assertion is scoped to the property this scenario owns.
#[test]
fn s10_arch_only_packaging_gates_skip_off_arch_under_strict_verification() {
    let scratch = ScratchDir::new("nonarch");
    let path = path_without(&scratch.path, &["pacman", "makepkg", "namcap"]);
    let home = scratch.path.join("home");
    fs::create_dir_all(&home).expect("scratch home should be created");

    let output = Command::new("bash")
        .arg("scripts/check-packaging.sh")
        .current_dir(repository_root())
        .env_clear()
        .env("PATH", &path)
        .env("HOME", &home)
        .env("VERIFY_STRICT", "1")
        .output()
        .expect("the packaging gate should be executable");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for tool in ARCH_ONLY_TOOLS {
        assert!(
            !combined.contains(&format!("{tool} is required in strict mode")),
            "the packaging gate strict-failed on the Arch-only tool `{tool}` \
             from a non-Arch environment, which is the TASK-0027 CI regression:\n{combined}"
        );
    }
    assert!(
        combined.contains("skip namcap checks (not an Arch environment)"),
        "the namcap section must announce its off-Arch skip:\n{combined}"
    );
    assert!(
        combined.contains("skip makepkg checks (not an Arch environment)"),
        "the makepkg section must announce its off-Arch skip:\n{combined}"
    );
    // The checks that do not need Arch must still have run, so the skip cannot
    // be mistaken for the whole gate quietly opting out.
    assert!(
        combined.contains("ok   PKGBUILD declares the proprietary license")
            && combined.contains("ok   package.json version matches src-tauri/Cargo.toml"),
        "the non-Arch portion of the packaging gate must still execute:\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 — no Arch-only requirement escapes the guard
// ---------------------------------------------------------------------------

/// Scenario 1 proves the current behaviour on the machine running the tests.
/// This one proves the *structure*, so the regression cannot come back through
/// a new section that happens not to be exercised by the environment above.
#[test]
fn s10_every_arch_only_tool_requirement_is_wrapped_in_the_arch_guard() {
    let gate = read_shipped("scripts/check-packaging.sh");
    let executable = executable_lines(&gate);

    assert!(
        executable.contains("arch_environment() {"),
        "the packaging gate must define one shared Arch-environment predicate"
    );

    for tool in ARCH_ONLY_TOOLS {
        let requirement = format!("need {tool}");
        let occurrences: Vec<&str> = executable
            .lines()
            .filter(|line| line.contains(&requirement))
            .collect();
        assert!(
            !occurrences.is_empty(),
            "the packaging gate no longer requires `{tool}` at all; \
             delete it from ARCH_ONLY_TOOLS or restore the check"
        );
        for line in occurrences {
            let trimmed = line.trim();
            let guarded = trimmed.starts_with("elif need") || trimmed.starts_with("if need");
            assert!(
                guarded,
                "`{requirement}` must appear as the condition of a guarded \
                 branch, found: {trimmed}"
            );
        }
    }

    // Both Arch-only sections must be reached through the shared predicate.
    assert!(
        executable.contains("if arch_environment; then"),
        "the makepkg section must be reached through `arch_environment`"
    );
    assert!(
        executable.contains("if ! arch_environment; then"),
        "the namcap section must be reached through `arch_environment`"
    );
    // `need` still has to be strict for tools that are not Arch-specific,
    // otherwise this fix would have disarmed the whole gate.
    assert!(
        executable.contains("FAIL %s is required in strict mode"),
        "`need` must still fail the build for a missing non-Arch tool"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 — every mandatory gate is one ordered CI chain
// ---------------------------------------------------------------------------

#[test]
fn s10_ci_runs_every_mandatory_release_gate_in_one_ordered_chain() {
    let workflow = read_shipped(".github/workflows/ci.yml");

    for job in MANDATORY_CI_CHAIN {
        assert!(
            workflow.contains(&format!("\n  {job}:\n")),
            "the CI workflow no longer defines the mandatory `{job}` job"
        );
    }
    // Each job after the first depends on exactly the previous one, so a gate
    // cannot be skipped while the workflow still reports success.
    for pair in MANDATORY_CI_CHAIN.windows(2) {
        let (previous, current) = (pair[0], pair[1]);
        let block = workflow
            .split(&format!("\n  {current}:\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("`{current}` job body should be present"));
        let declaration = block
            .lines()
            .find(|line| line.trim_start().starts_with("needs:"))
            .unwrap_or_else(|| panic!("`{current}` must declare a `needs:` dependency"));
        assert!(
            declaration.contains(previous),
            "`{current}` must run after `{previous}`, found: {declaration}"
        );
    }

    assert!(
        workflow.contains("VERIFY_STRICT: \"1\""),
        "CI must run the gates in strict mode"
    );
    // A verification workflow must not also be a release channel.
    for forbidden in [
        "softprops/action-gh-release",
        "actions/upload-artifact",
        "gh release create",
        "npm publish",
        "cargo publish",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "the CI workflow must not publish or release ({forbidden})"
        );
    }
    assert!(
        workflow.contains("permissions:\n  contents: read"),
        "the CI workflow must keep read-only repository permissions"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — CI starts no live provider, microphone, portal, or installer
// ---------------------------------------------------------------------------

#[test]
fn s10_ci_never_starts_a_live_provider_microphone_portal_or_real_install() {
    let workflow = executable_lines(&read_shipped(".github/workflows/ci.yml"));

    for forbidden in [
        "ollama",
        "codex exec",
        "arecord",
        "pipewire",
        "xdg-desktop-portal",
        "org.freedesktop.portal",
        "install-kde.sh",
        "uninstall-kde.sh",
        "pacman -U",
        "pacman -R",
        "systemctl",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "the CI workflow must not perform the live action `{forbidden}`; \
             live acceptance belongs to the sequential live gate, not CI"
        );
    }

    // The staged lifecycle test is allowed, precisely because it runs against a
    // scratch HOME rather than the machine.
    assert!(
        workflow.contains("scripts/staged-install-test.sh"),
        "CI must still run the staged (non-destructive) lifecycle test"
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 — the local full gate and CI demand the same security tooling
// ---------------------------------------------------------------------------

/// A local `VERIFY_STRICT=1` run is the pre-flight for CI. If the two disagree
/// about what is mandatory, a candidate can pass locally and still be unproven.
#[test]
fn s10_local_strict_verification_and_ci_require_the_same_security_tooling() {
    let verify_full = executable_lines(&read_shipped("scripts/verify-full.sh"));
    let licenses = executable_lines(&read_shipped("scripts/check-licenses.sh"));
    let workflow = executable_lines(&read_shipped(".github/workflows/ci.yml"));

    // Rust advisories: mandatory in strict mode locally, mandatory in CI.
    assert!(
        verify_full.contains("cargo-audit or cargo-deny is required in strict mode"),
        "the local full gate must require Rust advisory tooling in strict mode"
    );
    assert!(
        licenses.contains("cargo-deny is required in strict mode"),
        "the license gate must require cargo-deny in strict mode"
    );
    assert!(
        workflow.contains("cargo deny --manifest-path src-tauri/Cargo.toml check"),
        "CI must run the full cargo-deny check (advisories, licenses, bans, sources)"
    );

    // Shell linting: mandatory in strict mode locally, installed in CI.
    assert!(
        verify_full.contains("shellcheck is required in strict mode"),
        "the local full gate must require shellcheck in strict mode"
    );

    // Secret scanning is CI-only tooling, so it must at least be pinned there.
    let gitleaks = workflow
        .lines()
        .find(|line| line.trim_start().starts_with("version=8."))
        .expect("CI must pin an exact gitleaks version");
    assert!(
        workflow.contains("gitleaks detect --source . --redact"),
        "CI must run a redacted secret scan"
    );
    assert!(
        gitleaks.trim().starts_with("version=8."),
        "the gitleaks pin must stay exact, found: {gitleaks}"
    );

    // Both npm audit surfaces, in both places. Until TASK-0029 only the local
    // full gate audited npm dependencies, so a published high-severity
    // advisory in a build dependency could reach a release candidate with CI
    // fully green.
    for gate in [&verify_full, &workflow] {
        assert!(
            gate.contains("npm audit --omit=dev --audit-level=moderate")
                && gate.contains("npm audit --audit-level=moderate"),
            "every gate must audit production and development npm dependencies"
        );
    }
    assert!(
        verify_full.contains("clippy") && verify_full.contains("-D warnings"),
        "the local full gate must deny Clippy warnings"
    );
    assert!(
        workflow.contains("-- -D warnings"),
        "CI must deny Clippy warnings"
    );
}

// ---------------------------------------------------------------------------
// Scenario 6 — CI actions run on a supported runtime
// ---------------------------------------------------------------------------

/// GitHub forced every `node20` action onto `node24` and annotated each job
/// with a deprecation warning. The release gate requires no unexplained
/// warnings, so the pinned majors must be ones that declare a supported
/// runtime themselves. `actions/checkout` and `actions/setup-node` moved to
/// `node24` in their v5 majors.
#[test]
fn s10_ci_actions_are_pinned_to_a_supported_runtime_major() {
    let workflow = read_shipped(".github/workflows/ci.yml");

    for action in ["actions/checkout", "actions/setup-node"] {
        let uses: Vec<&str> = workflow
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with(&format!("- uses: {action}@")))
            .collect();
        assert!(
            !uses.is_empty(),
            "the CI workflow no longer uses `{action}`"
        );
        for line in uses {
            let major = line
                .rsplit('@')
                .next()
                .and_then(|tag| tag.trim_start_matches('v').parse::<u32>().ok())
                .unwrap_or_else(|| panic!("`{action}` must be pinned to a major tag: {line}"));
            assert!(
                major >= 5,
                "`{action}` majors below v5 declare the deprecated node20 \
                 runtime and annotate every CI job with a warning: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 7 — one release version across manifests and lockfiles
// ---------------------------------------------------------------------------

/// `install_package_acceptance::s8_release_version_is_consistent_across_every_shipped_manifest`
/// covers the shipped manifests. A release candidate additionally has to have
/// lockfiles that agree, otherwise `npm ci` and `cargo build --locked` resolve
/// a different version than the one the package metadata promises.
#[test]
fn s10_release_version_is_consistent_across_manifests_and_lockfiles() {
    let version = crate_version();

    let package_lock = read_shipped("package-lock.json");
    assert!(
        package_lock.contains(&format!("\n  \"version\": \"{version}\",\n")),
        "package-lock.json root version is not {version}; run `npm install` \
         after a version bump"
    );
    assert!(
        package_lock.contains(&format!(
            "\"name\": \"ai-agent-control-center\",\n      \"version\": \"{version}\""
        )),
        "package-lock.json root package entry is not {version}"
    );

    let cargo_lock = read_shipped("src-tauri/Cargo.lock");
    assert!(
        cargo_lock.contains(&format!(
            "name = \"ai-agent-control-center\"\nversion = \"{version}\""
        )),
        "src-tauri/Cargo.lock does not record this crate at {version}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 8 — the pinned Node toolchain satisfies every declared engine range
// ---------------------------------------------------------------------------

/// A `major.minor.patch` version, comparable.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit());
    Some((major, minor, patch.parse().ok()?))
}

/// Every `^A.B.C` / `>=A.B.C` floor an engine range declares for `major`.
///
/// Only clauses on the pinned major line matter: a range like
/// `^22.22.2 || >=24.15.0` constrains a v22 toolchain through its `^22` clause
/// alone. Both `^` and `>=` lower-bound the same way for this purpose.
fn engine_floors_for_major(range: &str, major: u32) -> Vec<(u32, u32, u32)> {
    range
        .split("||")
        .filter_map(|clause| {
            let clause = clause.trim();
            let bare = clause
                .trim_start_matches(['^', '>', '=', '~', 'v'])
                .trim_start();
            parse_version(bare).filter(|floor| floor.0 == major)
        })
        .collect()
}

/// The TASK-0029 CI regression that was hiding in plain sight: `.nvmrc` pinned
/// Node 22.12.0 while five installed development dependencies declared a higher
/// floor on the v22 line (`jsdom@30.0.1` wants `^22.22.2`). npm reports that as
/// an `EBADENGINE` **warning**, so every CI run scrolled five of them past a
/// green check. The release gate does not accept unexplained warnings.
#[test]
fn s10_pinned_node_toolchain_satisfies_every_declared_engine_range() {
    let nvmrc = read_shipped(".nvmrc");
    let pin = parse_version(nvmrc.trim())
        .unwrap_or_else(|| panic!(".nvmrc must pin an exact major.minor.patch: {nvmrc:?}"));

    // The application's own declared range must admit the pin.
    let package_json: serde_json::Value =
        serde_json::from_str(&read_shipped("package.json")).expect("package.json should parse");
    let declared = package_json["engines"]["node"]
        .as_str()
        .expect("package.json must declare an engines.node range");
    let declared_floors = engine_floors_for_major(declared, pin.0);
    assert!(
        declared_floors.iter().any(|floor| pin >= *floor),
        "the .nvmrc pin {pin:?} does not satisfy the declared engines.node \
         range {declared:?}"
    );

    // And so must every installed dependency's range, or `npm ci` warns.
    let lock: serde_json::Value = serde_json::from_str(&read_shipped("package-lock.json"))
        .expect("package-lock.json should parse");
    let packages = lock["packages"]
        .as_object()
        .expect("package-lock.json must have a packages map");

    let mut violations = Vec::new();
    for (name, entry) in packages {
        let Some(range) = entry["engines"]["node"].as_str() else {
            continue;
        };
        let floors = engine_floors_for_major(range, pin.0);
        // A dependency that says nothing about the pinned major line is
        // satisfied through one of its other clauses; npm resolves those.
        if !floors.is_empty() && !floors.iter().any(|floor| pin >= *floor) {
            let label = if name.is_empty() { "<root>" } else { name };
            violations.push(format!("{label} requires {range}"));
        }
    }
    assert!(
        violations.is_empty(),
        "the pinned Node toolchain {}.{}.{} is below the engine floor declared \
         by {} dependenc(ies), which makes `npm ci` emit EBADENGINE warnings:\n  {}",
        pin.0,
        pin.1,
        pin.2,
        violations.len(),
        violations.join("\n  ")
    );

    // npm, not this test, is the semver authority at install time. CI must let
    // it fail the build rather than warn.
    let workflow = read_shipped(".github/workflows/ci.yml");
    for line in workflow
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- run: npm ci"))
    {
        assert!(
            line.contains("--engine-strict"),
            "every CI `npm ci` must run with --engine-strict so an engine \
             mismatch fails instead of warning: {line}"
        );
    }
    assert!(
        workflow.contains("node-version-file: .nvmrc"),
        "CI must take its Node version from the pinned .nvmrc"
    );
}

// ---------------------------------------------------------------------------
// Scenario 9 — one license policy, expressed in two gates
// ---------------------------------------------------------------------------

/// Extract the quoted identifiers from a `name = [ "a", "b" ]` style block.
fn quoted_list_after(text: &str, marker: &str) -> BTreeSet<String> {
    let Some(rest) = text.split_once(marker).map(|(_, rest)| rest) else {
        return BTreeSet::new();
    };
    let body = rest.split_once(']').map(|(body, _)| body).unwrap_or(rest);
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// `deny.toml` gates Rust crates and `scripts/check-licenses.sh` gates both
/// ecosystems, so they encode one policy in two places. Drift between them is
/// how a copyleft dependency slips past the half that was not updated.
#[test]
fn s10_rust_and_npm_license_gates_encode_one_consistent_policy() {
    let deny = read_shipped("deny.toml");
    let rust_allowed = quoted_list_after(&deny, "allow = [");
    assert!(
        rust_allowed.len() > 5,
        "failed to parse the cargo-deny license allowlist"
    );

    let script = read_shipped("scripts/check-licenses.sh");
    let combined_line = script
        .lines()
        .find(|line| line.contains("AACC_ALLOWED_LICENSES="))
        .expect("the license gate must declare its allowlist");
    let combined: BTreeSet<String> = combined_line
        .split_once('=')
        .expect("allowlist assignment")
        .1
        .trim()
        .trim_matches('"')
        .split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();

    let missing: Vec<&String> = rust_allowed.difference(&combined).collect();
    assert!(
        missing.is_empty(),
        "these licenses are allowed for Rust crates but not by the shared \
         license gate, so the two gates disagree: {missing:?}"
    );

    // Neither gate may admit a license that is incompatible with proprietary
    // distribution, whatever else drifts.
    for identifier in rust_allowed.iter().chain(combined.iter()) {
        let upper = identifier.to_ascii_uppercase();
        for forbidden in ["GPL", "AGPL", "SSPL", "CDDL", "EUPL"] {
            // "LGPL" and "GPL" both contain "GPL"; MPL and BlueOak do not.
            assert!(
                !upper.contains(forbidden),
                "`{identifier}` is copyleft/reciprocal and must not be allowed \
                 by a proprietary-distribution license gate"
            );
        }
    }

    // cargo-deny must keep checking all four surfaces, not just advisories.
    assert!(
        deny.contains("[advisories]")
            && deny.contains("[licenses]")
            && deny.contains("[bans]")
            && deny.contains("[sources]"),
        "deny.toml must configure advisories, licenses, bans, and sources"
    );
    assert!(
        deny.contains("yanked = \"deny\""),
        "a yanked crate must fail the release gate"
    );
    assert!(
        deny.contains("unknown-registry = \"deny\"") && deny.contains("unknown-git = \"deny\""),
        "an unknown crate source must fail the release gate"
    );
}
