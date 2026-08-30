//! TASK-0027 — composed install / upgrade / remove / purge / Arch package
//! acceptance (S8).
//!
//! [`crate::lifecycle_removal`] is already unit-tested in isolation
//! (`task_0019_*`). This module is the composed machine-lifecycle matrix plus
//! the regression home for defects the composition exposes. It covers the two
//! things those unit tests cannot see:
//!
//! * the **sequence** — install, idempotent upgrade, keep-data removal,
//!   reinstall over retained data, purge, and a second idempotent purge run as
//!   one continuous story over a single scratch root, so a step cannot pass in
//!   isolation while corrupting the step after it; and
//! * **cross-artifact parity** — the shipped `install-kde.sh`,
//!   `packaging/PKGBUILD`, `packaging/ai-agent-control-center.install`,
//!   `packaging/…desktop.in`, and the version manifests must all describe the
//!   same layout, the same retained set, and the same removal commands the
//!   binary actually accepts. Drift between them is invisible to any single
//!   subsystem test and is exactly what ships a broken package.
//!
//! Everything here is deterministic, runs as an ordinary user under a scratch
//! `$HOME`, and never installs, removes, or touches real machine state. Live
//! `install-kde.sh`, `uninstall-kde.sh --purge`, `makepkg`, `pacman -U`, and
//! `pacman -R` evidence on the real Arch / KDE session belongs to the TASK-0027
//! live slices and is recorded in `planning/TASK_STATUS.md`.

use crate::lifecycle_removal::{
    DataCategory, RemovalAction, RemovalPaths, RemovalScope, APPLICATION_NAMESPACE,
    BUNDLE_IDENTIFIER, DATABASE_FILE_NAME, PURGE_CONFIRMATION_TOKEN,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

/// Repository root (the parent of `src-tauri`), resolved from the crate
/// manifest so the shipped packaging files are read from the checkout under
/// test rather than from an installed copy.
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

/// A throwaway `$HOME` that models a real user's XDG layout.
struct ScratchHome {
    path: PathBuf,
}

impl ScratchHome {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("aacc-task-0027-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).expect("scenario home should be created");
        Self { path }
    }

    fn data_home(&self) -> PathBuf {
        self.path.join(".local").join("share")
    }

    fn config_home(&self) -> PathBuf {
        self.path.join(".config")
    }

    fn cache_home(&self) -> PathBuf {
        self.path.join(".cache")
    }

    fn runtime_dir(&self) -> PathBuf {
        self.path.join("run")
    }

    /// The user-local install payload `install-kde.sh` lays down.
    fn install_dir(&self) -> PathBuf {
        self.path
            .join(".local")
            .join("lib")
            .join(APPLICATION_NAMESPACE)
    }

    fn removal_paths(&self) -> RemovalPaths {
        RemovalPaths::from_values(
            Some(OsString::from(&self.path)),
            Some(OsString::from(self.data_home())),
            Some(OsString::from(self.config_home())),
            Some(OsString::from(self.cache_home())),
            Some(OsString::from(self.runtime_dir())),
        )
        .expect("scratch home resolves an absolute removal model")
    }

    /// Model the desktop-integration files both install paths create, plus the
    /// payload. Removal of these is the scripts' job, not the binary's, so they
    /// must survive every binary-driven removal mode.
    fn simulate_install(&self) {
        write_file(&self.install_dir().join(APPLICATION_NAMESPACE), "binary");
        write_file(
            &self.install_dir().join("voice-runtime").join("listener.py"),
            "listener",
        );
        write_file(
            &self
                .path
                .join(".local")
                .join("share")
                .join("applications")
                .join(format!("{APPLICATION_NAMESPACE}.desktop")),
            "[Desktop Entry]",
        );
        write_file(
            &self
                .path
                .join(".local")
                .join("bin")
                .join(APPLICATION_NAMESPACE),
            "launcher",
        );
    }

    /// Seed one sentinel file in every owned location, mirroring what a real
    /// session leaves behind after first run.
    fn seed_user_data(&self) {
        let data = self.data_home();
        let config = self.config_home();
        let cache = self.cache_home();

        write_file(
            &data.join(BUNDLE_IDENTIFIER).join(DATABASE_FILE_NAME),
            "sqlite-database",
        );
        write_file(
            &data
                .join(BUNDLE_IDENTIFIER)
                .join(format!("{DATABASE_FILE_NAME}-wal")),
            "write-ahead-log",
        );
        write_file(
            &data.join(BUNDLE_IDENTIFIER).join("logs").join("app.log"),
            "log",
        );
        write_file(
            &config.join(BUNDLE_IDENTIFIER).join("settings.bin"),
            "config",
        );
        write_file(&cache.join(BUNDLE_IDENTIFIER).join("http.bin"), "cache");
        write_file(
            &data
                .join(APPLICATION_NAMESPACE)
                .join("voice-runtime")
                .join("base")
                .join("models")
                .join("model.bin"),
            "voice-model",
        );
        write_file(
            &config
                .join(APPLICATION_NAMESPACE)
                .join("voice-runtime")
                .join("desktop-control-restore-token"),
            "portal-restore-token",
        );
        write_file(
            &cache
                .join(APPLICATION_NAMESPACE)
                .join("voice-runtime")
                .join("vosk.zip"),
            "download",
        );
        write_file(
            &self
                .runtime_dir()
                .join(APPLICATION_NAMESPACE)
                .join("kwin")
                .join("script.js"),
            "kwin",
        );
    }
}

impl Drop for ScratchHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("seeded file always has a parent"))
        .expect("parent directory should be created");
    fs::write(path, contents).expect("sentinel file should be written");
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
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

/// The version every shipped manifest must agree on.
fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// Scenario 1 — the whole machine lifecycle as one continuous sequence
// ---------------------------------------------------------------------------

#[test]
fn s8_install_upgrade_keep_data_reinstall_purge_sequence_is_data_faithful() {
    let home = ScratchHome::new();
    home.simulate_install();
    home.seed_user_data();
    let paths = home.removal_paths();

    let database = paths.database_file();
    let voice_model = home
        .data_home()
        .join(APPLICATION_NAMESPACE)
        .join("voice-runtime")
        .join("base")
        .join("models")
        .join("model.bin");
    let portal_token = paths.portal_restore_token_file();
    let database_before = fs::read(&database).expect("seeded database is readable");

    // --- upgrade: a reinstall over an existing install must not remove data.
    // install-kde.sh only ever stops the runtime and replaces the payload, so
    // the owned inventory before and after an upgrade is identical.
    let inventory_before_upgrade = paths.inventory();
    home.simulate_install();
    assert_eq!(
        inventory_before_upgrade,
        paths.inventory(),
        "an idempotent upgrade must leave every owned data location untouched"
    );

    // --- keep-data removal.
    let keep = paths.execute(RemovalScope::KeepUserData, false);
    assert!(!keep.had_failure, "keep-data removal reported a failure");
    for outcome in &keep.outcomes {
        let expected_retained = outcome.category.retained_on_keep_user_data();
        assert_eq!(
            matches!(outcome.action, RemovalAction::Retained),
            expected_retained,
            "{} disposition disagrees with the documented retained set",
            outcome.category.key()
        );
    }
    assert!(exists(&database), "keep-data must retain the database");
    assert!(
        exists(&voice_model),
        "keep-data must retain downloaded voice models"
    );
    assert!(
        !exists(&portal_token),
        "keep-data must remove the KDE portal restore token"
    );
    assert_eq!(
        fs::read(&database).expect("retained database is readable"),
        database_before,
        "keep-data must retain the database byte for byte"
    );

    // The scripts, not the binary, own the payload and desktop integration.
    assert!(
        exists(&home.install_dir()),
        "the binary must not remove the install payload; uninstall-kde.sh owns it"
    );

    // --- reinstall over retained data: the preserved database is reused as is.
    home.simulate_install();
    assert_eq!(
        fs::read(&database).expect("database survives the reinstall"),
        database_before,
        "a reinstall must reuse the retained database untouched"
    );

    // --- purge.
    let purge = paths.execute(RemovalScope::Purge, false);
    assert!(!purge.had_failure, "purge reported a failure");
    assert!(purge.fully_removed(), "purge left owned data behind");
    for location in paths.locations() {
        assert!(
            !exists(&location.path),
            "purge left {} behind at {}",
            location.category.key(),
            location.path.display()
        );
    }

    // --- second purge: idempotent, every location already absent, no failure.
    let second = paths.execute(RemovalScope::Purge, false);
    assert!(!second.had_failure, "a second purge must not fail");
    assert!(
        second.fully_removed(),
        "a second purge must still report removal"
    );
    assert!(
        second
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome.action, RemovalAction::Absent)),
        "a second purge must report every owned location as already absent"
    );

    // Purge is bounded to owned locations: the scratch home itself and the
    // unrelated payload directory the scripts own are still there.
    assert!(
        exists(&home.path),
        "purge must stay inside the owned namespaces"
    );
    assert!(
        exists(&home.install_dir()),
        "purge must not remove the payload the removal script owns"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 — the retained set is exactly what every shipped artifact promises
// ---------------------------------------------------------------------------

#[test]
fn s8_keep_data_retained_set_matches_every_documented_promise() {
    let retained: BTreeSet<&str> = [
        DataCategory::ApplicationLogs,
        DataCategory::LocalDataAndWebview,
        DataCategory::ApplicationConfig,
        DataCategory::ApplicationCache,
        DataCategory::VoiceData,
        DataCategory::VoiceConfigAndPortalToken,
        DataCategory::VoiceCache,
        DataCategory::RuntimeState,
    ]
    .into_iter()
    .filter(|category| category.retained_on_keep_user_data())
    .map(|category| category.key())
    .collect();

    assert_eq!(
        retained,
        BTreeSet::from(["LocalDataAndWebview", "VoiceData"]),
        "the keep-data retained set changed; every promise below must change with it"
    );

    // The pacman hook promises the same two things in prose.
    let hook = read_shipped("packaging/ai-agent-control-center.install");
    assert!(
        hook.contains("KEEPING the database and any"),
        "the pacman hook must say keep-data keeps the database"
    );
    assert!(
        hook.contains("downloaded offline-voice models"),
        "the pacman hook must say keep-data keeps downloaded voice models"
    );

    // uninstall-kde.sh promises the same two things.
    let uninstall = read_shipped("uninstall-kde.sh");
    assert!(
        uninstall.contains("SQLite database + downloaded voice models"),
        "uninstall-kde.sh must document the same retained set"
    );
    assert!(
        uninstall.contains("User data was preserved;"),
        "uninstall-kde.sh keep-data mode must tell the user data was preserved"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 — the purge refusal is a dry run that changes nothing
// ---------------------------------------------------------------------------

#[test]
fn s8_unconfirmed_purge_dry_run_changes_nothing_on_disk() {
    let home = ScratchHome::new();
    home.seed_user_data();
    let paths = home.removal_paths();

    let before = paths.inventory();
    let dry = paths.execute(RemovalScope::Purge, true);

    assert!(!dry.had_failure, "a dry run must not fail");
    assert!(
        !dry.fully_removed(),
        "a dry run must never claim data was removed"
    );
    assert!(
        dry.outcomes
            .iter()
            .all(|outcome| matches!(outcome.action, RemovalAction::WouldRemove)),
        "every seeded location should be reported as 'would remove'"
    );
    assert_eq!(
        before,
        paths.inventory(),
        "a dry run changed the filesystem"
    );

    // The CLI refuses an unconfirmed purge and names the exact token that
    // authorises it, so the refusal text and the accepted flag cannot drift.
    let cli = read_shipped("src-tauri/src/lib.rs");
    assert!(
        cli.contains("Refusing to purge without confirmation."),
        "the CLI must refuse an unconfirmed purge"
    );
    assert!(
        cli.contains("--purge --confirm {PURGE_CONFIRMATION_TOKEN}"),
        "the refusal must name the confirmation token from lifecycle_removal"
    );
    assert_eq!(
        PURGE_CONFIRMATION_TOKEN, "PURGE",
        "the removal scripts and the pacman hook hard-code this token"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — both install paths ship the same artifact set
// ---------------------------------------------------------------------------

#[test]
fn s8_user_local_and_arch_package_ship_the_same_artifacts() {
    let install = read_shipped("install-kde.sh");
    let pkgbuild = read_shipped("packaging/PKGBUILD");

    // (needle, description) pairs every shipping path must place.
    let shipped = [
        ("voice-runtime/setup.sh", "the offline-voice setup script"),
        (
            "voice-runtime/setup-high-accuracy.sh",
            "the high-accuracy voice setup script",
        ),
        ("voice-runtime/listener.py", "the offline listener"),
        ("icons/hicolor/512x512/apps", "the 512x512 application icon"),
        (
            "com.aivarsrocens.aiagentcontrolcenter.metainfo.xml",
            "the AppStream metainfo",
        ),
        ("LICENSE", "the project license"),
        ("THIRD-PARTY-NOTICES.md", "the third-party notices"),
        ("desktop.in", "the rendered desktop entry"),
    ];

    for (needle, description) in shipped {
        assert!(
            install.contains(needle),
            "install-kde.sh does not ship {description}"
        );
        assert!(
            pkgbuild.contains(needle),
            "packaging/PKGBUILD does not ship {description}"
        );
    }

    // Both provide the launcher as a symlink onto the payload binary, so the
    // desktop entry's Exec target and the CLI are the same file in both paths.
    assert!(
        install.contains("ln -sfn \"$installed_bin\" \"$launcher\""),
        "install-kde.sh must link the launcher at the installed binary"
    );
    assert!(
        pkgbuild.contains("ln -s \"/usr/lib/$pkgname/$pkgname\" \"$pkgdir/usr/bin/$pkgname\""),
        "the package must link /usr/bin onto the payload binary"
    );

    // The package declares the runtime libraries the binary actually needs.
    // Read the `depends=(...)` line itself rather than the file, so a mention
    // in a comment cannot satisfy the assertion.
    let depends = pkgbuild
        .lines()
        .find(|line| line.starts_with("depends="))
        .expect("packaging/PKGBUILD must declare depends");
    for dependency in ["webkit2gtk-4.1", "gtk3", "sqlite"] {
        assert!(
            depends.contains(dependency),
            "packaging/PKGBUILD must declare the {dependency} runtime dependency, got {depends}"
        );
    }
    // Dlopen'd at runtime by the Tauri tray, so namcap's ELF scan cannot see it
    // and reports it as possibly unneeded. It is required.
    assert!(
        depends.contains("libappindicator-gtk3"),
        "the dlopen'd tray dependency must stay declared, got {depends}"
    );
    // Already guaranteed by gtk3, not linked, and no SVG asset is shipped, so
    // declaring it again is the redundancy namcap flags.
    assert!(
        !depends.contains("librsvg"),
        "librsvg is guaranteed by gtk3 and must not be declared again, got {depends}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 — the desktop entry renders identically apart from its Exec target
// ---------------------------------------------------------------------------

#[test]
fn s8_desktop_entry_differs_only_in_its_exec_target_between_install_paths() {
    let template = read_shipped("packaging/ai-agent-control-center.desktop.in");
    let user_local = template
        .replace("@EXEC@", "/home/user/.local/bin/ai-agent-control-center")
        .replace("@ICON@", APPLICATION_NAMESPACE);
    let packaged = template
        .replace("@EXEC@", "/usr/bin/ai-agent-control-center")
        .replace("@ICON@", APPLICATION_NAMESPACE);

    assert!(
        !user_local.contains('@') && !packaged.contains('@'),
        "every desktop-entry placeholder must be substituted by both install paths"
    );

    let differing: Vec<(&str, &str)> = user_local
        .lines()
        .zip(packaged.lines())
        .filter(|(left, right)| left != right)
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "only the Exec line may differ between the two install paths, got {differing:?}"
    );
    assert!(
        differing[0].0.starts_with("Exec="),
        "the differing line must be Exec, got {:?}",
        differing[0]
    );

    // The tray/window matching key must equal the binary name in both paths or
    // KDE cannot associate the window with the desktop entry.
    assert!(
        packaged.contains(&format!("StartupWMClass={APPLICATION_NAMESPACE}")),
        "the desktop entry must set StartupWMClass to the binary name"
    );
    assert!(
        packaged.contains(&format!("Icon={APPLICATION_NAMESPACE}")),
        "the desktop entry must reference the installed icon name"
    );
}

// ---------------------------------------------------------------------------
// Scenario 6 — one release version across every shipped manifest
// ---------------------------------------------------------------------------

#[test]
fn s8_release_version_is_consistent_across_every_shipped_manifest() {
    let version = crate_version();

    let package_json = read_shipped("package.json");
    assert!(
        package_json.contains(&format!("\"version\": \"{version}\"")),
        "package.json does not declare version {version}"
    );

    let tauri_conf = read_shipped("src-tauri/tauri.conf.json");
    assert!(
        tauri_conf.contains(&format!("\"version\": \"{version}\"")),
        "src-tauri/tauri.conf.json does not declare version {version}"
    );

    let pkgbuild = read_shipped("packaging/PKGBUILD");
    assert!(
        pkgbuild.contains(&format!("\npkgver={version}\n")),
        "packaging/PKGBUILD pkgver is not {version}"
    );

    let metainfo = read_shipped("packaging/com.aivarsrocens.aiagentcontrolcenter.metainfo.xml");
    assert!(
        metainfo.contains(&format!("<release version=\"{version}\"")),
        "the AppStream metainfo has no release entry for {version}"
    );
    assert!(
        metainfo.contains("type=\"development\""),
        "the release must stay marked development until the 1.0 gate passes"
    );
}

// ---------------------------------------------------------------------------
// Scenario 7 — removal scripts delegate data removal to the binary
// ---------------------------------------------------------------------------

#[test]
fn s8_removal_scripts_delegate_owned_data_removal_to_the_binary() {
    let uninstall = executable_lines(&read_shipped("uninstall-kde.sh"));

    assert!(
        uninstall.contains("\"$installed_bin\" --uninstall"),
        "keep-data removal must delegate to the binary"
    );
    assert!(
        uninstall.contains("\"$installed_bin\" --purge --confirm PURGE"),
        "purge must delegate to the binary with the confirmation token"
    );

    // The script may only remove the payload and desktop-integration files it
    // installs. Hard-coding an owned *data* path here is the drift this guards.
    for owned_data in [BUNDLE_IDENTIFIER, DATABASE_FILE_NAME, "voice-runtime/base"] {
        for line in uninstall.lines() {
            let line = line.trim();
            if !line.starts_with("rm ") && !line.starts_with("rm -") {
                continue;
            }
            assert!(
                !line.contains(owned_data),
                "uninstall-kde.sh removes owned data directly ({line}); \
                 delegate it to the binary so the inventory cannot drift"
            );
        }
    }

    // If the binary is missing the script must say what could not be cleaned
    // rather than silently claiming a complete removal.
    assert!(
        uninstall.contains("installed binary not found at"),
        "uninstall-kde.sh must report a missing binary"
    );
    assert!(
        uninstall.contains("could not be cleaned"),
        "uninstall-kde.sh must say per-user data was not cleaned when the binary is missing"
    );

    // The closing message must be gated on the delegated removal having
    // actually run. A second `--purge` (the payload is already gone) warned
    // that nothing could be cleaned and then still printed "all local data have
    // been removed" — a removal claim the run had just contradicted.
    assert!(
        uninstall.contains("data_removal_ran=1"),
        "uninstall-kde.sh must record whether the delegated removal ran"
    );
    let claim = "AI Agent Control Center and all local data have been removed.";
    let claim_line = uninstall
        .lines()
        .position(|line| line.contains(claim))
        .expect("uninstall-kde.sh must have a purge success message");
    let guard_line = uninstall
        .lines()
        .position(|line| line.contains("if (( ! data_removal_ran ))"))
        .expect("uninstall-kde.sh must guard its closing message");
    assert!(
        guard_line < claim_line,
        "the 'all local data have been removed' claim must sit behind the \
         data_removal_ran guard, not before it"
    );
    assert!(
        uninstall.contains("Per-user data was NOT touched"),
        "a run that could not delegate removal must say the data was not touched"
    );
}

// ---------------------------------------------------------------------------
// Scenario 8 — no install or removal path restarts PlasmaShell
// ---------------------------------------------------------------------------

#[test]
fn s8_no_install_or_removal_path_restarts_plasmashell() {
    let scripts = [
        ("install-kde.sh", read_shipped("install-kde.sh")),
        ("uninstall-kde.sh", read_shipped("uninstall-kde.sh")),
        (
            "packaging/ai-agent-control-center.install",
            read_shipped("packaging/ai-agent-control-center.install"),
        ),
    ];

    for (name, script) in &scripts {
        let executable = executable_lines(script);
        for forbidden in [
            "plasmashell --replace",
            "restart plasma-plasmashell",
            "killall plasmashell",
            "pkill plasmashell",
            "systemctl --user restart plasma",
        ] {
            assert!(
                !executable.contains(forbidden),
                "{name} restarts the desktop shell via `{forbidden}`"
            );
        }
    }

    // Cache refreshes must stay non-disruptive and must never fail the run.
    let install = executable_lines(&scripts[0].1);
    assert!(
        install.contains("update-desktop-database") && install.contains("gtk-update-icon-cache"),
        "install-kde.sh must refresh the freedesktop caches"
    );
    assert!(
        install.contains("kbuildsycoca6") || install.contains("kbuildsycoca5"),
        "install-kde.sh must rebuild the KDE service cache"
    );
    // The pacman hook must NOT repeat that work: pacman's own PostTransaction
    // hooks already refresh both caches on the exact paths this package
    // installs, for install, upgrade, and remove alike, and a root-run KDE
    // service-cache rebuild would only touch root's per-user cache.
    //
    // Checked against the raw file rather than `executable_lines`, because
    // namcap's `externalhooks` rule substring-matches the whole `.INSTALL`
    // including comments -- naming these commands even in prose reopens the
    // warning.
    let hook = &scripts[2].1;
    for redundant in [
        concat!("update-desktop", "-database"),
        concat!("gtk-update-", "icon-cache"),
        "kbuildsycoca",
    ] {
        assert!(
            !hook.contains(redundant),
            "the pacman hook names `{redundant}`; pacman's own hooks already handle it \
             and namcap matches the whole file including comments"
        );
    }
    assert!(
        hook.contains("post_install"),
        "the pacman hook must still print the removal contract on install"
    );
}

// ---------------------------------------------------------------------------
// Scenario 9 — the pacman hook tells the truth about pacman's limits
// ---------------------------------------------------------------------------

#[test]
fn s8_pacman_hook_states_what_pacman_cannot_remove_and_names_working_commands() {
    let hook = read_shipped("packaging/ai-agent-control-center.install");
    let cli = read_shipped("src-tauri/src/lib.rs");

    assert!(
        hook.contains("`pacman -R` cannot remove per-user data or stop a running instance."),
        "the hook must state that pacman cannot remove per-user data"
    );
    assert!(
        hook.contains("The persistent KDE screen-cast / remote-desktop permission is revoked in"),
        "the hook must state that the portal grant is revoked in KDE System Settings"
    );

    // Every command the hook tells the user to run must be a flag the CLI
    // actually accepts, spelled the same way.
    for command in [
        "ai-agent-control-center --uninstall",
        "ai-agent-control-center --purge --confirm PURGE",
    ] {
        assert!(hook.contains(command), "the hook must document `{command}`");
    }
    for flag in ["--uninstall", "--purge", "--confirm", "--stop-runtime"] {
        assert!(
            cli.contains(&format!("has(\"{flag}\")")) || cli.contains(&format!("== \"{flag}\"")),
            "the CLI must accept {flag}"
        );
    }

    // The per-user roots the hook names must be the roots the removal model
    // actually derives its locations from.
    for root in ["~/.local/share", "~/.config", "~/.cache"] {
        assert!(
            hook.contains(root),
            "the hook must name the {root} per-user root"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 10 — hostile XDG values cannot escape the owned namespaces
// ---------------------------------------------------------------------------

#[test]
fn s8_hostile_xdg_roots_fail_closed_and_leave_the_tree_intact() {
    let home = ScratchHome::new();
    let bystander = home.path.join("unrelated-user-file");
    write_file(&bystander, "must survive");

    // XDG_DATA_HOME pointing at $HOME itself would make the owned data location
    // `$HOME/<identifier>` — inside a known root, but the safety check still has
    // to refuse to delete anything that is not below a *distinct* root.
    let hostile = RemovalPaths::from_values(
        Some(OsString::from(&home.path)),
        Some(OsString::from(&home.path)),
        Some(OsString::from(&home.path)),
        Some(OsString::from(&home.path)),
        None,
    )
    .expect("hostile values still resolve a model");

    // Seed the location the hostile model would target, then confirm removal is
    // bounded to the namespaced child and never climbs to the root itself.
    write_file(&home.path.join(BUNDLE_IDENTIFIER).join("x"), "owned");
    let report = hostile.execute(RemovalScope::Purge, false);
    assert!(!report.had_failure, "the hostile model reported a failure");
    assert!(
        exists(&home.path),
        "removal must never delete the root it was pointed at"
    );
    assert!(
        exists(&bystander),
        "removal must never touch unrelated files beside the owned namespace"
    );

    // A relative override is ignored in favour of the documented $HOME default,
    // so a hostile relative value cannot redirect removal into the CWD.
    let relative = RemovalPaths::from_values(
        Some(OsString::from(&home.path)),
        Some(OsString::from("relative/data")),
        None,
        None,
        Some(OsString::from("also/relative")),
    )
    .expect("relative overrides fall back to the home defaults");
    assert_eq!(
        relative.database_file(),
        home.path
            .join(".local")
            .join("share")
            .join(BUNDLE_IDENTIFIER)
            .join(DATABASE_FILE_NAME),
        "a relative XDG_DATA_HOME must fall back to the $HOME default"
    );
    assert!(
        relative
            .locations()
            .iter()
            .all(|location| location.category != DataCategory::RuntimeState),
        "a relative XDG_RUNTIME_DIR must not produce a runtime location"
    );
}

// ---------------------------------------------------------------------------
// Scenario 11 — the upgrade keeps a rollback binary and verifies before commit
// ---------------------------------------------------------------------------

#[test]
fn s8_upgrade_keeps_a_verified_rollback_path() {
    let install = executable_lines(&read_shipped("install-kde.sh"));

    assert!(
        install.contains("previous_bin=\"$install_dir/$app_id.previous\""),
        "an upgrade must keep the previous binary for rollback"
    );
    assert!(
        install.contains("cp -f \"$installed_bin\" \"$previous_bin\""),
        "an upgrade must copy the previous binary before overwriting it"
    );
    assert!(
        install.contains("if ! \"$installed_bin\" --version >/dev/null 2>&1; then"),
        "the new binary must be smoke-checked before the install is committed"
    );
    assert!(
        install.contains("mv -f \"$previous_bin\" \"$installed_bin\""),
        "a failed post-install check must roll back to the previous binary"
    );
    assert!(
        install.contains("rm -f \"$previous_bin\""),
        "a successful install must clean up the rollback copy"
    );

    // A running pre-upgrade instance holds the database open across the build,
    // so it is stopped before the build rather than after it (TASK-0022).
    let stop_index = install
        .find("--stop-runtime")
        .expect("install-kde.sh must stop the running instance");
    let build_index = install
        .find("npm run tauri -- build")
        .expect("install-kde.sh must build the binary");
    assert!(
        stop_index < build_index,
        "the running instance must be stopped before the build starts"
    );
}

// ---------------------------------------------------------------------------
// Scenario 12 — the portal grant is reported as a limit, never claimed removed
// ---------------------------------------------------------------------------

#[test]
fn s8_portal_grant_is_reported_as_a_platform_limit_not_claimed_removed() {
    let cli = read_shipped("src-tauri/src/lib.rs");
    assert!(
        cli.contains("the application cannot revoke it."),
        "the CLI must report the portal grant as a limit it cannot revoke"
    );
    assert!(
        cli.contains("PORTAL_PERMISSION_NOTE"),
        "the portal note must be a shared constant"
    );

    // The purge path must print the note and clear the stored provider key, so
    // the user is told exactly what removal did and did not reach.
    assert!(
        cli.contains("clear_stored_credentials_for_removal()"),
        "removal must clear the stored provider key"
    );
    assert!(
        cli.contains("purge complete; no owned data remains within scope"),
        "a successful purge must say so explicitly"
    );

    // The restore token itself *is* owned and is removed in both modes; only
    // the portal grant behind it is out of reach.
    let home = ScratchHome::new();
    home.seed_user_data();
    let paths = home.removal_paths();
    let token = paths.portal_restore_token_file();
    assert!(exists(&token), "the seeded restore token should exist");
    paths.execute(RemovalScope::KeepUserData, false);
    assert!(
        !exists(&token),
        "keep-data removal must delete the KDE portal restore token"
    );
}
