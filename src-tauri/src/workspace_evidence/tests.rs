use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ai-agent-control-center-task-0012-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn git(workspace: &TempWorkspace, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(workspace.path())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn initialize_git(workspace: &TempWorkspace) {
    git(workspace, &["init", "-q"]);
    git(workspace, &["config", "user.name", "Task 0012"]);
    git(
        workspace,
        &["config", "user.email", "task-0012@example.invalid"],
    );
}

fn changes_by_path(
    evidence: &WorkspaceChangeEvidenceV1,
) -> BTreeMap<&str, &WorkspaceChangeEntryV1> {
    evidence
        .changes
        .iter()
        .map(|change| (change.path.as_str(), change))
        .collect()
}

#[test]
fn task_0012_non_git_snapshot_covers_add_modify_delete_rename_and_binary() {
    let workspace = TempWorkspace::new("non-git");
    workspace.write("modified.txt", "before\n");
    workspace.write("deleted.txt", "delete me\n");
    workspace.write("rename-old.txt", "same rename bytes\n");
    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());

    workspace.write("modified.txt", "after\n");
    fs::remove_file(workspace.path().join("deleted.txt")).unwrap();
    fs::rename(
        workspace.path().join("rename-old.txt"),
        workspace.path().join("rename-new.txt"),
    )
    .unwrap();
    workspace.write("added.txt", "new\n");
    workspace.write("binary.bin", [0_u8, 1, 2, 3]);

    let evidence = baseline.finish();
    let changes = changes_by_path(&evidence);
    assert_eq!(evidence.mode, WorkspaceEvidenceMode::Filesystem);
    assert_eq!(evidence.status, WorkspaceEvidenceStatus::Complete);
    assert_eq!(
        changes["modified.txt"].change_kind,
        WorkspaceChangeKind::Modified
    );
    assert_eq!(
        changes["deleted.txt"].change_kind,
        WorkspaceChangeKind::Deleted
    );
    assert_eq!(changes["added.txt"].change_kind, WorkspaceChangeKind::Added);
    assert_eq!(
        changes["binary.bin"].change_kind,
        WorkspaceChangeKind::Added
    );
    assert!(changes["binary.bin"].binary);
    assert_eq!(
        changes["rename-new.txt"].change_kind,
        WorkspaceChangeKind::Renamed
    );
    assert_eq!(
        changes["rename-new.txt"].previous_path.as_deref(),
        Some("rename-old.txt")
    );
    assert_eq!(
        evidence.reviewability,
        WorkspaceEvidenceReviewability::HumanReviewRequired
    );
}

#[test]
fn task_0012_git_porcelain_covers_staged_unstaged_untracked_deleted_renamed_and_binary() {
    let workspace = TempWorkspace::new("git-cases");
    initialize_git(&workspace);
    for name in [
        "staged.txt",
        "unstaged.txt",
        "both.txt",
        "deleted.txt",
        "rename-old.txt",
        "forced-binary.txt",
    ] {
        workspace.write(name, format!("baseline {name}\n"));
    }
    workspace.write(".gitattributes", "forced-binary.txt -diff\n");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-qm", "baseline"]);
    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());

    workspace.write("staged.txt", "staged after\n");
    git(&workspace, &["add", "staged.txt"]);
    workspace.write("unstaged.txt", "unstaged after\n");
    workspace.write("both.txt", "both staged\n");
    git(&workspace, &["add", "both.txt"]);
    workspace.write("both.txt", "both working tree\n");
    fs::remove_file(workspace.path().join("deleted.txt")).unwrap();
    git(&workspace, &["add", "-u", "deleted.txt"]);
    git(&workspace, &["mv", "rename-old.txt", "rename-new.txt"]);
    workspace.write("untracked.txt", "untracked\n");
    workspace.write("binary.bin", [0_u8, 4, 5, 6]);
    workspace.write("forced-binary.txt", "text marked binary after\n");

    let evidence = baseline.finish();
    let changes = changes_by_path(&evidence);
    assert_eq!(evidence.mode, WorkspaceEvidenceMode::Git);
    assert_eq!(
        changes["staged.txt"]
            .git_after
            .as_ref()
            .unwrap()
            .index_status
            .as_deref(),
        Some("M")
    );
    assert_eq!(
        changes["unstaged.txt"]
            .git_after
            .as_ref()
            .unwrap()
            .worktree_status
            .as_deref(),
        Some("M")
    );
    let both = changes["both.txt"].git_after.as_ref().unwrap();
    assert_eq!(both.index_status.as_deref(), Some("M"));
    assert_eq!(both.worktree_status.as_deref(), Some("M"));
    assert!(
        changes["untracked.txt"]
            .git_after
            .as_ref()
            .unwrap()
            .untracked
    );
    assert_eq!(
        changes["deleted.txt"].change_kind,
        WorkspaceChangeKind::Deleted
    );
    assert_eq!(
        changes["rename-new.txt"].change_kind,
        WorkspaceChangeKind::Renamed
    );
    assert!(changes["binary.bin"].binary);
    assert!(changes["forced-binary.txt"].binary);
    assert!(changes["forced-binary.txt"].human_review_required);
    assert!(evidence.summary.staged >= 4);
    assert!(evidence.summary.unstaged >= 2);
    assert!(evidence.summary.untracked >= 2);
}

#[test]
fn task_0012_git_head_change_is_explicit_and_requires_human_review() {
    let workspace = TempWorkspace::new("git-head");
    initialize_git(&workspace);
    workspace.write("tracked.txt", "baseline\n");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-qm", "baseline"]);
    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());

    git(
        &workspace,
        &["commit", "--allow-empty", "-qm", "history change"],
    );
    let evidence = baseline.finish();

    assert_ne!(evidence.baseline_git_head, evidence.final_git_head);
    assert!(evidence
        .issues
        .iter()
        .any(|entry| entry.code == "GIT_HEAD_CHANGED"));
    assert_eq!(evidence.status, WorkspaceEvidenceStatus::Partial);
    assert!(!evidence.is_complete_for_agent_approval());
}

#[test]
fn task_0012_limits_are_explicit_and_block_agent_approval() {
    let workspace = TempWorkspace::new("limits");
    workspace.write("one.txt", "one");
    workspace.write("two.txt", "two");
    workspace.write("three.txt", "three");
    let limits = EvidenceLimits {
        snapshot_entries: 2,
        snapshot_hash_bytes: 2,
        ..EvidenceLimits::default()
    };
    let evidence = WorkspaceEvidenceBaseline::begin_with_limits(workspace.path(), limits).finish();
    assert_eq!(evidence.status, WorkspaceEvidenceStatus::Partial);
    assert!(evidence.before_snapshot_truncated || evidence.after_snapshot_truncated);
    assert!(!evidence.is_complete_for_agent_approval());
    assert!(evidence
        .issues
        .iter()
        .any(|entry| entry.code.contains("LIMIT")));
}

#[cfg(target_family = "unix")]
#[test]
fn task_0012_symlinks_are_recorded_without_following_outside_content() {
    use std::os::unix::fs::symlink;

    let workspace = TempWorkspace::new("symlink");
    let outside = TempWorkspace::new("outside");
    outside.write("private.txt", "OUTSIDE-CONTENT-MUST-NOT-APPEAR");
    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());
    symlink(
        outside.path().join("private.txt"),
        workspace.path().join("link.txt"),
    )
    .unwrap();
    let evidence = baseline.finish();
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert!(!serialized.contains("OUTSIDE-CONTENT-MUST-NOT-APPEAR"));
    assert_eq!(
        changes_by_path(&evidence)["link.txt"]
            .after
            .as_ref()
            .unwrap()
            .kind,
        WorkspaceFileKind::BlockedSymlink
    );
    assert!(evidence
        .issues
        .iter()
        .any(|entry| entry.code == "SYMLINK_NOT_FOLLOWED"));
}

#[test]
fn task_0012_sensitive_paths_and_secret_lines_never_persist_raw_content() {
    let workspace = TempWorkspace::new("redaction");
    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());
    workspace.write(".env", "API_TOKEN=super-secret-value-123456789\n");
    workspace.write(
        "ordinary.txt",
        "authorization: Bearer abcdefghijklmnopqrstuvwxyz123456\n",
    );
    let evidence = baseline.finish();
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert!(!serialized.contains("super-secret-value-123456789"));
    assert!(!serialized.contains("abcdefghijklmnopqrstuvwxyz123456"));
    let env_change = &changes_by_path(&evidence)[".env"];
    assert!(env_change.content_redacted);
    assert!(env_change.after.as_ref().unwrap().sha256.is_none());
    let ordinary_change = &changes_by_path(&evidence)["ordinary.txt"];
    assert!(ordinary_change.content_redacted);
    assert!(ordinary_change.after.as_ref().unwrap().sha256.is_none());
    assert_eq!(
        evidence.reviewability,
        WorkspaceEvidenceReviewability::HumanReviewRequired
    );
}

#[cfg(target_family = "unix")]
#[test]
fn task_0012_git_external_diff_textconv_and_filter_helpers_are_not_executed() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = TempWorkspace::new("git-helpers");
    initialize_git(&workspace);
    workspace.write("tracked.txt", "baseline\n");
    workspace.write(".gitattributes", "*.txt diff=evil filter=evil\n");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-qm", "baseline"]);

    let marker = workspace.path().join("HELPER_EXECUTED");
    let helper = workspace.path().join("helper.sh");
    fs::write(
        &helper,
        format!("#!/bin/sh\nprintf executed > '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&helper).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&helper, permissions).unwrap();
    let helper_value = helper.to_string_lossy().to_string();
    git(&workspace, &["config", "diff.evil.command", &helper_value]);
    git(&workspace, &["config", "diff.evil.textconv", &helper_value]);
    git(&workspace, &["config", "filter.evil.clean", &helper_value]);
    git(&workspace, &["config", "filter.evil.smudge", &helper_value]);
    git(&workspace, &["config", "filter.evil.required", "true"]);

    let baseline = WorkspaceEvidenceBaseline::begin(workspace.path());
    workspace.write("tracked.txt", "changed\n");
    let evidence = baseline.finish();
    assert!(!marker.exists());
    assert!(changes_by_path(&evidence).contains_key("tracked.txt"));
}

#[test]
fn task_0012_porcelain_v2_parser_preserves_spaces_and_rename_sources() {
    let bytes = b"1 M. N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb space name.txt\0? untracked name.txt\x002 R. N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb R100 renamed new.txt\0renamed old.txt\0";
    let parsed = parse_porcelain_v2(bytes).unwrap();
    assert_eq!(parsed["space name.txt"].index_status, Some('M'));
    assert!(parsed["untracked name.txt"].untracked);
    assert_eq!(
        parsed["renamed new.txt"].previous_path.as_deref(),
        Some("renamed old.txt")
    );
}
