#![cfg(target_os = "linux")]

use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "aacc-task-0008-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary workspace should be created");
        Self { path }
    }

    fn tools(&self) -> WorkspaceTools {
        WorkspaceTools::open(&self.path).expect("workspace tools should open")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("temporary workspace should be removed");
    }
}

#[test]
fn task_0008_listing_is_stable_paginated_and_hides_git_metadata() {
    let workspace = TestWorkspace::new("listing");
    for name in ["delta.txt", "alpha.txt", "charlie.txt", "bravo.txt"] {
        fs::write(workspace.path().join(name), name).expect("fixture file should write");
    }
    fs::create_dir(workspace.path().join("echo")).expect("fixture directory should write");
    fs::create_dir(workspace.path().join(".git")).expect("fixture .git should write");
    let tools = workspace.tools();

    let first = tools.list(".", None, 2).expect("first page should list");
    assert_eq!(
        first
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha.txt", "bravo.txt"]
    );
    assert!(first.truncated);
    assert_eq!(first.next_cursor.as_deref(), Some("bravo.txt"));

    let second = tools
        .list(".", first.next_cursor.as_deref(), 2)
        .expect("second page should list");
    assert_eq!(
        second
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["charlie.txt", "delta.txt"]
    );
    assert!(second.truncated);

    let third = tools
        .list(".", second.next_cursor.as_deref(), 2)
        .expect("third page should list");
    assert_eq!(third.entries.len(), 1);
    assert_eq!(third.entries[0].name, "echo");
    assert_eq!(third.entries[0].kind, "directory");
    assert!(!third.truncated);
    assert_eq!(third.next_cursor, None);
}

#[test]
fn task_0008_large_file_reads_are_ranged_and_patch_only_selected_bytes() {
    let workspace = TestWorkspace::new("large-file");
    let mut original = "a".repeat(96 * 1024);
    original.push_str("TARGET");
    original.push_str(&"z".repeat(96 * 1024));
    fs::write(workspace.path().join("large.txt"), &original).expect("large fixture should write");
    let tools = workspace.tools();

    let first = tools
        .read("large.txt", 0, 8 * 1024)
        .expect("first range should read");
    assert_eq!(first.bytes_read, 8 * 1024);
    assert!(first.truncated);
    assert_eq!(first.next_offset, Some(8 * 1024));
    assert_eq!(first.total_bytes, original.len() as u64);

    let target_start = 96 * 1024;
    let around_target = tools
        .read("large.txt", (target_start - 8) as u64, 32)
        .expect("target range should read");
    assert!(around_target.content.contains("TARGET"));
    assert_eq!(around_target.sha256, first.sha256);

    let mutation = tools
        .apply_patch(
            "large.txt",
            &first.sha256,
            &[WorkspacePatchEdit {
                start_byte: target_start as u64,
                end_byte: (target_start + "TARGET".len()) as u64,
                replacement: "REPAIRED".to_string(),
            }],
        )
        .expect("large file patch should commit");
    let actual =
        fs::read_to_string(workspace.path().join("large.txt")).expect("patched file should read");
    let expected = original.replacen("TARGET", "REPAIRED", 1);
    assert_eq!(actual, expected);
    assert_eq!(mutation.bytes_written, Some(expected.len() as u64));
    assert_ne!(mutation.sha256.as_deref(), Some(first.sha256.as_str()));
}

#[test]
fn task_0008_stale_hash_conflicts_preserve_the_newer_file() {
    let workspace = TestWorkspace::new("conflict");
    let file = workspace.path().join("shared.txt");
    fs::write(&file, "first state\n").expect("fixture should write");
    let tools = workspace.tools();
    let read = tools
        .read("shared.txt", 0, DEFAULT_READ_BYTES)
        .expect("fixture should read");
    fs::write(&file, "newer user state\n").expect("concurrent fixture change should write");

    let error = tools
        .apply_patch(
            "shared.txt",
            &read.sha256,
            &[WorkspacePatchEdit {
                start_byte: 0,
                end_byte: 5,
                replacement: "agent".to_string(),
            }],
        )
        .expect_err("stale patch should conflict");

    assert_eq!(error.kind, WorkspaceToolErrorKind::Conflict);
    assert_eq!(
        fs::read_to_string(&file).expect("newer file should remain"),
        "newer user state\n"
    );
}

#[test]
fn task_0008_path_escape_symlink_and_git_boundaries_fail_closed() {
    let workspace = TestWorkspace::new("boundary");
    let outside = workspace
        .path()
        .parent()
        .expect("fixture should have parent")
        .join(format!("aacc-task-0008-outside-{}", std::process::id()));
    fs::write(&outside, "outside secret").expect("outside fixture should write");
    symlink(&outside, workspace.path().join("escape-link")).expect("escape symlink should write");
    fs::create_dir(workspace.path().join(".git")).expect("git fixture should write");
    fs::write(workspace.path().join(".git/config"), "secret")
        .expect("git fixture file should write");
    let tools = workspace.tools();

    for path in [
        "../outside",
        outside.to_string_lossy().as_ref(),
        ".git/config",
    ] {
        let error = tools
            .read(path, 0, DEFAULT_READ_BYTES)
            .expect_err("boundary escape should fail");
        assert_eq!(error.kind, WorkspaceToolErrorKind::InvalidInput);
    }
    let symlink_error = tools
        .read("escape-link", 0, DEFAULT_READ_BYTES)
        .expect_err("symlink should not be followed");
    assert_eq!(symlink_error.kind, WorkspaceToolErrorKind::InvalidInput);
    assert_eq!(
        fs::read_to_string(&outside).expect("outside file should remain readable"),
        "outside secret"
    );

    fs::remove_file(outside).expect("outside fixture should be removed");
}

#[test]
fn task_0008_create_is_atomic_create_only_and_patch_validation_is_non_mutating() {
    let workspace = TestWorkspace::new("create-only");
    let tools = workspace.tools();
    let created = tools
        .create_file("new.txt", "original")
        .expect("new file should be created");
    assert!(created.created);
    let duplicate = tools
        .create_file("new.txt", "replacement")
        .expect_err("duplicate create should conflict");
    assert_eq!(duplicate.kind, WorkspaceToolErrorKind::Conflict);
    assert_eq!(
        fs::read_to_string(workspace.path().join("new.txt")).expect("created file should read"),
        "original"
    );

    let invalid = tools
        .apply_patch(
            "new.txt",
            created
                .sha256
                .as_deref()
                .expect("create should return hash"),
            &[
                WorkspacePatchEdit {
                    start_byte: 0,
                    end_byte: 4,
                    replacement: "one".to_string(),
                },
                WorkspacePatchEdit {
                    start_byte: 3,
                    end_byte: 5,
                    replacement: "two".to_string(),
                },
            ],
        )
        .expect_err("overlapping edits should fail");
    assert_eq!(invalid.kind, WorkspaceToolErrorKind::InvalidInput);
    assert_eq!(
        fs::read_to_string(workspace.path().join("new.txt")).expect("original file should remain"),
        "original"
    );
}

#[test]
fn task_0008_tool_contract_removes_unconditional_replacement_and_enforces_access() {
    let read_tools = ollama_workspace_tools("read");
    let read_names = tool_names(&read_tools);
    assert_eq!(read_names, ["list_workspace_files", "read_workspace_file"]);

    let write_tools = ollama_workspace_tools("write");
    let write_names = tool_names(&write_tools);
    assert!(write_names.contains(&"create_workspace_file"));
    assert!(write_names.contains(&"apply_workspace_patch"));
    assert!(!write_names.contains(&"write_workspace_file"));

    let workspace = TestWorkspace::new("access");
    let error = workspace
        .tools()
        .execute(
            "read",
            "create_workspace_file",
            &json!({ "path": "denied.txt", "content": "no" }),
        )
        .expect_err("read-only access should reject creates");
    assert!(error.contains("does not have workspace-write access"));
    assert!(!workspace.path().join("denied.txt").exists());
}

fn tool_names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect()
}
