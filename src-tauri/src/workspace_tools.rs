use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, path::Path};

const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 200;
const DEFAULT_READ_BYTES: usize = 32 * 1024;
const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_HASHABLE_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_NEW_FILE_BYTES: usize = 512 * 1024;
const MAX_PATCH_EDITS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceToolErrorKind {
    InvalidInput,
    NotFound,
    Conflict,
    Limit,
    Unsupported,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceToolError {
    pub(crate) kind: WorkspaceToolErrorKind,
    pub(crate) message: String,
}

impl WorkspaceToolError {
    fn new(kind: WorkspaceToolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::InvalidInput, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::NotFound, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::Conflict, message)
    }

    fn limit(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::Limit, message)
    }

    fn unsupported(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::Unsupported, message)
    }

    fn io(message: impl Into<String>) -> Self {
        Self::new(WorkspaceToolErrorKind::Io, message)
    }
}

impl fmt::Display for WorkspaceToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkspaceToolError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceListEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceListResult {
    pub(crate) path: String,
    pub(crate) entries: Vec<WorkspaceListEntry>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceReadResult {
    pub(crate) path: String,
    pub(crate) content: String,
    pub(crate) offset: u64,
    pub(crate) bytes_read: u64,
    pub(crate) total_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) next_offset: Option<u64>,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceMutationResult {
    pub(crate) path: String,
    pub(crate) sha256: Option<String>,
    pub(crate) bytes_written: Option<u64>,
    pub(crate) created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspacePatchEdit {
    pub(crate) start_byte: u64,
    pub(crate) end_byte: u64,
    pub(crate) replacement: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    #[serde(default = "default_root_path")]
    path: String,
    cursor: Option<String>,
    limit: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArguments {
    path: String,
    offset: Option<u64>,
    max_bytes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateFileArguments {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchArguments {
    path: String,
    expected_sha256: String,
    edits: Vec<WorkspacePatchEdit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDirectoryArguments {
    path: String,
}

fn default_root_path() -> String {
    ".".to_string()
}

pub(crate) struct WorkspaceTools {
    #[cfg(target_os = "linux")]
    platform: linux::LinuxWorkspace,
    #[cfg(not(target_os = "linux"))]
    _root: std::path::PathBuf,
}

impl WorkspaceTools {
    pub(crate) fn open(root: &Path) -> Result<Self, WorkspaceToolError> {
        #[cfg(target_os = "linux")]
        {
            linux::LinuxWorkspace::open(root).map(|platform| Self { platform })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let root = std::fs::canonicalize(root).map_err(|error| {
                WorkspaceToolError::io(format!("Could not open the workspace root: {error}"))
            })?;
            if !root.is_dir() {
                return Err(WorkspaceToolError::invalid(
                    "The selected workspace root is not a directory.",
                ));
            }
            Ok(Self { _root: root })
        }
    }

    pub(crate) fn list(
        &self,
        path: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<WorkspaceListResult, WorkspaceToolError> {
        validate_limit(limit, 1, MAX_LIST_LIMIT, "listing")?;
        #[cfg(target_os = "linux")]
        {
            self.platform.list(path, cursor, limit)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, cursor);
            Err(platform_unsupported())
        }
    }

    pub(crate) fn read(
        &self,
        path: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<WorkspaceReadResult, WorkspaceToolError> {
        validate_limit(max_bytes, 1, MAX_READ_BYTES, "read")?;
        #[cfg(target_os = "linux")]
        {
            self.platform.read(path, offset, max_bytes)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, offset);
            Err(platform_unsupported())
        }
    }

    pub(crate) fn create_file(
        &self,
        path: &str,
        content: &str,
    ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
        validate_text_payload(content, MAX_NEW_FILE_BYTES, "new file")?;
        #[cfg(target_os = "linux")]
        {
            self.platform.create_file(path, content.as_bytes())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(platform_unsupported())
        }
    }

    pub(crate) fn apply_patch(
        &self,
        path: &str,
        expected_sha256: &str,
        edits: &[WorkspacePatchEdit],
    ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
        validate_sha256(expected_sha256)?;
        validate_patch_edits(edits)?;
        #[cfg(target_os = "linux")]
        {
            self.platform.apply_patch(path, expected_sha256, edits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(platform_unsupported())
        }
    }

    pub(crate) fn create_directory(
        &self,
        path: &str,
    ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
        #[cfg(target_os = "linux")]
        {
            self.platform.create_directory(path)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(platform_unsupported())
        }
    }

    pub(crate) fn execute(
        &self,
        file_access: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        let result = match name {
            "list_workspace_files" => {
                let arguments: ListArguments = parse_arguments(arguments)?;
                let limit = optional_usize(arguments.limit, DEFAULT_LIST_LIMIT, "limit")?;
                self.list(&arguments.path, arguments.cursor.as_deref(), limit)
                    .and_then(to_json)
            }
            "read_workspace_file" => {
                let arguments: ReadArguments = parse_arguments(arguments)?;
                let max_bytes =
                    optional_usize(arguments.max_bytes, DEFAULT_READ_BYTES, "maxBytes")?;
                self.read(&arguments.path, arguments.offset.unwrap_or(0), max_bytes)
                    .and_then(to_json)
            }
            "create_workspace_file" => {
                require_write_access(file_access)?;
                let arguments: CreateFileArguments = parse_arguments(arguments)?;
                self.create_file(&arguments.path, &arguments.content)
                    .and_then(to_json)
            }
            "apply_workspace_patch" => {
                require_write_access(file_access)?;
                let arguments: PatchArguments = parse_arguments(arguments)?;
                self.apply_patch(
                    &arguments.path,
                    &arguments.expected_sha256,
                    &arguments.edits,
                )
                .and_then(to_json)
            }
            "create_workspace_directory" => {
                require_write_access(file_access)?;
                let arguments: CreateDirectoryArguments = parse_arguments(arguments)?;
                self.create_directory(&arguments.path).and_then(to_json)
            }
            _ => {
                return Err(format!(
                    "`{name}` is not an available local workspace tool."
                ))
            }
        };
        result.map_err(|error| format!("{}: {}", error_code(error.kind), error.message))
    }
}

pub(crate) fn ollama_workspace_tools(file_access: &str) -> Vec<Value> {
    let mut tools = vec![
        json!({
          "type": "function",
          "function": {
            "name": "list_workspace_files",
            "description": "List one directory inside the selected workspace in stable name order. Continue with nextCursor when truncated is true. Symlinks are reported but never followed.",
            "parameters": {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "path": { "type": "string", "description": "Relative directory path; use . for the workspace root." },
                "cursor": { "type": "string", "description": "The exact nextCursor from the preceding page for this directory." },
                "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIST_LIMIT }
              }
            }
          }
        }),
        json!({
          "type": "function",
          "function": {
            "name": "read_workspace_file",
            "description": "Read a bounded UTF-8 byte range and return the SHA-256 of the complete file. Continue with nextOffset when truncated is true. Byte offsets must be UTF-8 boundaries.",
            "parameters": {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "path": { "type": "string", "description": "Relative file path." },
                "offset": { "type": "integer", "minimum": 0 },
                "maxBytes": { "type": "integer", "minimum": 1, "maximum": MAX_READ_BYTES }
              },
              "required": ["path"]
            }
          }
        }),
    ];
    if matches!(file_access, "write" | "full") {
        tools.extend([
            json!({
              "type": "function",
              "function": {
                "name": "create_workspace_file",
                "description": "Atomically create a new UTF-8 file. This never replaces an existing path.",
                "parameters": {
                  "type": "object",
                  "additionalProperties": false,
                  "properties": {
                    "path": { "type": "string", "description": "Relative new file path whose parent already exists." },
                    "content": { "type": "string", "description": "Initial complete content for the new file." }
                  },
                  "required": ["path", "content"]
                }
              }
            }),
            json!({
              "type": "function",
              "function": {
                "name": "apply_workspace_patch",
                "description": "Atomically apply non-overlapping UTF-8 byte-range edits only when the complete current file still matches expectedSha256. Read the relevant ranges first; a stale hash is a conflict and never overwrites the file.",
                "parameters": {
                  "type": "object",
                  "additionalProperties": false,
                  "properties": {
                    "path": { "type": "string", "description": "Relative existing file path." },
                    "expectedSha256": { "type": "string", "description": "Complete-file SHA-256 returned by read_workspace_file." },
                    "edits": {
                      "type": "array",
                      "minItems": 1,
                      "maxItems": MAX_PATCH_EDITS,
                      "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                          "startByte": { "type": "integer", "minimum": 0 },
                          "endByte": { "type": "integer", "minimum": 0 },
                          "replacement": { "type": "string" }
                        },
                        "required": ["startByte", "endByte", "replacement"]
                      }
                    }
                  },
                  "required": ["path", "expectedSha256", "edits"]
                }
              }
            }),
            json!({
              "type": "function",
              "function": {
                "name": "create_workspace_directory",
                "description": "Create one new directory inside the workspace. The parent must already exist and an existing target is a conflict.",
                "parameters": {
                  "type": "object",
                  "additionalProperties": false,
                  "properties": {
                    "path": { "type": "string", "description": "Relative new directory path." }
                  },
                  "required": ["path"]
                }
              }
            }),
        ]);
    }
    tools
}

fn require_write_access(file_access: &str) -> Result<(), String> {
    matches!(file_access, "write" | "full")
        .then_some(())
        .ok_or_else(|| "This agent does not have workspace-write access for this run.".to_string())
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(arguments: &Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone())
        .map_err(|error| format!("Invalid workspace tool arguments: {error}"))
}

fn optional_usize(value: Option<u64>, default: usize, name: &str) -> Result<usize, String> {
    value
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| format!("The `{name}` workspace tool argument is too large."))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn to_json<T: Serialize>(value: T) -> Result<String, WorkspaceToolError> {
    serde_json::to_string(&value).map_err(|error| {
        WorkspaceToolError::io(format!(
            "Could not encode the workspace tool result: {error}"
        ))
    })
}

fn error_code(kind: WorkspaceToolErrorKind) -> &'static str {
    match kind {
        WorkspaceToolErrorKind::InvalidInput => "INVALID_INPUT",
        WorkspaceToolErrorKind::NotFound => "NOT_FOUND",
        WorkspaceToolErrorKind::Conflict => "CONFLICT",
        WorkspaceToolErrorKind::Limit => "LIMIT_EXCEEDED",
        WorkspaceToolErrorKind::Unsupported => "UNSUPPORTED",
        WorkspaceToolErrorKind::Io => "IO_ERROR",
    }
}

fn validate_limit(
    value: usize,
    minimum: usize,
    maximum: usize,
    label: &str,
) -> Result<(), WorkspaceToolError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(WorkspaceToolError::invalid(format!(
            "The workspace {label} limit must be between {minimum} and {maximum}."
        )));
    }
    Ok(())
}

fn validate_text_payload(
    content: &str,
    maximum: usize,
    label: &str,
) -> Result<(), WorkspaceToolError> {
    if content.as_bytes().contains(&0) {
        return Err(WorkspaceToolError::invalid(
            "Workspace text cannot contain null bytes.",
        ));
    }
    if content.len() > maximum {
        return Err(WorkspaceToolError::limit(format!(
            "The workspace {label} exceeds the {maximum}-byte bound."
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), WorkspaceToolError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WorkspaceToolError::invalid(
            "expectedSha256 must be exactly 64 hexadecimal characters.",
        ));
    }
    Ok(())
}

fn validate_patch_edits(edits: &[WorkspacePatchEdit]) -> Result<(), WorkspaceToolError> {
    if edits.is_empty() || edits.len() > MAX_PATCH_EDITS {
        return Err(WorkspaceToolError::invalid(format!(
            "A workspace patch must contain between 1 and {MAX_PATCH_EDITS} edits."
        )));
    }
    let mut previous_end = 0_u64;
    let mut replacement_bytes = 0_usize;
    for (index, edit) in edits.iter().enumerate() {
        if edit.start_byte > edit.end_byte {
            return Err(WorkspaceToolError::invalid(format!(
                "Workspace patch edit {index} has startByte after endByte."
            )));
        }
        if index > 0 && edit.start_byte < previous_end {
            return Err(WorkspaceToolError::invalid(
                "Workspace patch edits must be ordered and non-overlapping.",
            ));
        }
        validate_text_payload(&edit.replacement, MAX_NEW_FILE_BYTES, "patch replacement")?;
        replacement_bytes = replacement_bytes.saturating_add(edit.replacement.len());
        if replacement_bytes > MAX_NEW_FILE_BYTES {
            return Err(WorkspaceToolError::limit(format!(
                "Workspace patch replacements exceed the {MAX_NEW_FILE_BYTES}-byte bound."
            )));
        }
        previous_end = edit.end_byte;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn platform_unsupported() -> WorkspaceToolError {
    WorkspaceToolError::unsupported(
        "Descriptor-confined local workspace tools are unavailable on this platform.",
    )
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::{
        fd::OwnedFd,
        fs::{
            fstat, fsync, mkdirat, openat, openat2, renameat_with, statat, unlinkat, AtFlags, Dir,
            FileType, Mode, OFlags, RenameFlags, ResolveFlags, Stat, CWD,
        },
        io::Errno,
    };
    use sha2::{Digest, Sha256};
    use std::{
        collections::BinaryHeap,
        fs::{File, Permissions},
        io::{Read, Write},
        os::unix::fs::PermissionsExt,
        path::{Component, Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    const SECURE_RESOLVE: ResolveFlags = ResolveFlags::BENEATH
        .union(ResolveFlags::NO_MAGICLINKS)
        .union(ResolveFlags::NO_SYMLINKS);
    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);
    const FILE_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::NONBLOCK);
    const CREATE_FLAGS: OFlags = OFlags::WRONLY
        .union(OFlags::CREATE)
        .union(OFlags::EXCL)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW);
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

    pub(super) struct LinuxWorkspace {
        root: OwnedFd,
    }

    #[derive(Debug)]
    struct FileSnapshot {
        bytes: Vec<u8>,
        sha256: String,
        stat: Stat,
    }

    impl LinuxWorkspace {
        pub(super) fn open(root: &Path) -> Result<Self, WorkspaceToolError> {
            let root_fd = match openat2(
                CWD,
                root,
                DIRECTORY_FLAGS,
                Mode::empty(),
                ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
            ) {
                Ok(fd) => fd,
                Err(error) if error == Errno::NOSYS => {
                    openat(CWD, root, DIRECTORY_FLAGS, Mode::empty())
                        .map_err(|error| map_open_error(error, "workspace root"))?
                }
                Err(error) => return Err(map_open_error(error, "workspace root")),
            };
            let stat = fstat(&root_fd).map_err(|error| {
                WorkspaceToolError::io(format!("Could not inspect the workspace root: {error}"))
            })?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
                return Err(WorkspaceToolError::invalid(
                    "The selected workspace root is not a directory.",
                ));
            }
            Ok(Self { root: root_fd })
        }

        pub(super) fn list(
            &self,
            path: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<WorkspaceListResult, WorkspaceToolError> {
            let relative = validated_relative_path(path, true)?;
            let directory = secure_open(&self.root, &relative, DIRECTORY_FLAGS)?;
            let mut reader = Dir::read_from(&directory).map_err(|error| {
                WorkspaceToolError::io(format!("Could not read the workspace directory: {error}"))
            })?;
            let cursor = cursor.unwrap_or_default();
            if cursor.as_bytes().contains(&0) || cursor.contains('/') {
                return Err(WorkspaceToolError::invalid(
                    "The workspace listing cursor is invalid for this directory.",
                ));
            }
            let mut smallest = BinaryHeap::new();
            let page_capacity = limit.saturating_add(1);
            while let Some(entry) = reader.read() {
                let entry = entry.map_err(|error| {
                    WorkspaceToolError::io(format!(
                        "Could not continue reading the workspace directory: {error}"
                    ))
                })?;
                let name = entry.file_name().to_str().map_err(|_| {
                    WorkspaceToolError::unsupported(
                        "The workspace directory contains a non-UTF-8 file name.",
                    )
                })?;
                if matches!(name, "." | ".." | ".git") || name <= cursor {
                    continue;
                }
                let stat =
                    statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
                        WorkspaceToolError::io(format!(
                            "Could not inspect workspace entry `{name}`: {error}"
                        ))
                    })?;
                let file_type = FileType::from_raw_mode(stat.st_mode);
                let (kind, size_bytes) = match file_type {
                    FileType::Directory => ("directory", None),
                    FileType::RegularFile => ("file", u64::try_from(stat.st_size).ok()),
                    FileType::Symlink => ("blocked-symlink", None),
                    _ => ("unsupported", None),
                };
                let display_path = join_display_path(&relative, name);
                smallest.push((name.to_string(), display_path, kind.to_string(), size_bytes));
                if smallest.len() > page_capacity {
                    smallest.pop();
                }
            }
            let mut entries = smallest.into_sorted_vec();
            let truncated = entries.len() > limit;
            if truncated {
                entries.pop();
            }
            let entries = entries
                .into_iter()
                .map(|(name, path, kind, size_bytes)| WorkspaceListEntry {
                    name,
                    path,
                    kind,
                    size_bytes,
                })
                .collect::<Vec<_>>();
            let next_cursor = truncated
                .then(|| entries.last().map(|entry| entry.name.clone()))
                .flatten();
            Ok(WorkspaceListResult {
                path: display_relative_path(&relative),
                entries,
                next_cursor,
                truncated,
            })
        }

        pub(super) fn read(
            &self,
            path: &str,
            offset: u64,
            max_bytes: usize,
        ) -> Result<WorkspaceReadResult, WorkspaceToolError> {
            let relative = validated_relative_path(path, false)?;
            let snapshot = self.read_snapshot(&relative)?;
            let content = std::str::from_utf8(&snapshot.bytes).map_err(|_| {
                WorkspaceToolError::unsupported(
                    "The requested workspace file is not valid UTF-8 text.",
                )
            })?;
            if snapshot.bytes.contains(&0) {
                return Err(WorkspaceToolError::unsupported(
                    "The requested workspace file contains null bytes and is not treated as text.",
                ));
            }
            let offset = usize::try_from(offset).map_err(|_| {
                WorkspaceToolError::invalid("The workspace read offset is too large.")
            })?;
            if offset > content.len() {
                return Err(WorkspaceToolError::invalid(format!(
                    "The workspace read offset exceeds the {}-byte file length.",
                    content.len()
                )));
            }
            if !content.is_char_boundary(offset) {
                return Err(WorkspaceToolError::invalid(
                    "The workspace read offset must be a UTF-8 byte boundary.",
                ));
            }
            let mut end = offset.saturating_add(max_bytes).min(content.len());
            while end > offset && !content.is_char_boundary(end) {
                end -= 1;
            }
            let next_offset = (end < content.len()).then_some(end as u64);
            Ok(WorkspaceReadResult {
                path: display_relative_path(&relative),
                content: content[offset..end].to_string(),
                offset: offset as u64,
                bytes_read: (end - offset) as u64,
                total_bytes: content.len() as u64,
                sha256: snapshot.sha256,
                next_offset,
                truncated: offset > 0 || end < content.len(),
            })
        }

        pub(super) fn create_file(
            &self,
            path: &str,
            content: &[u8],
        ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
            let relative = validated_relative_path(path, false)?;
            let (parent_path, leaf) = parent_and_leaf(&relative)?;
            let parent = secure_open(&self.root, &parent_path, DIRECTORY_FLAGS)?;
            let (temp_name, mut temp_file) = create_temp_file(&parent)?;
            if let Err(error) = write_and_sync(&mut temp_file, content, 0o600) {
                let _ = unlinkat(&parent, &temp_name, AtFlags::empty());
                return Err(error);
            }
            match renameat_with(&parent, &temp_name, &parent, &leaf, RenameFlags::NOREPLACE) {
                Ok(()) => {}
                Err(error) => {
                    let _ = unlinkat(&parent, &temp_name, AtFlags::empty());
                    if error == Errno::EXIST {
                        return Err(WorkspaceToolError::conflict(format!(
                            "Workspace path `{}` already exists; create-only writes never replace it.",
                            display_relative_path(&relative)
                        )));
                    }
                    return Err(WorkspaceToolError::io(format!(
                        "Could not atomically create the workspace file: {error}"
                    )));
                }
            }
            fsync(&parent).map_err(|error| {
                WorkspaceToolError::io(format!(
                    "The workspace file was created, but its directory could not be synchronized: {error}"
                ))
            })?;
            Ok(WorkspaceMutationResult {
                path: display_relative_path(&relative),
                sha256: Some(sha256_hex(content)),
                bytes_written: Some(content.len() as u64),
                created: true,
            })
        }

        pub(super) fn apply_patch(
            &self,
            path: &str,
            expected_sha256: &str,
            edits: &[WorkspacePatchEdit],
        ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
            let relative = validated_relative_path(path, false)?;
            let original = self.read_snapshot(&relative)?;
            if !original.sha256.eq_ignore_ascii_case(expected_sha256) {
                return Err(hash_conflict(&relative, &original.sha256));
            }
            let original_text = std::str::from_utf8(&original.bytes).map_err(|_| {
                WorkspaceToolError::unsupported(
                    "The requested workspace file is not valid UTF-8 text.",
                )
            })?;
            if original.bytes.contains(&0) {
                return Err(WorkspaceToolError::unsupported(
                    "The requested workspace file contains null bytes and cannot be patched as text.",
                ));
            }
            let patched = apply_edits(original_text, edits)?;
            if patched.len() > MAX_HASHABLE_FILE_BYTES {
                return Err(WorkspaceToolError::limit(format!(
                    "The patched workspace file would exceed the {MAX_HASHABLE_FILE_BYTES}-byte bound."
                )));
            }
            let (parent_path, leaf) = parent_and_leaf(&relative)?;
            let parent = secure_open(&self.root, &parent_path, DIRECTORY_FLAGS)?;
            let (temp_name, mut temp_file) = create_temp_file(&parent)?;
            let permissions = original.stat.st_mode & 0o777;
            if let Err(error) = write_and_sync(&mut temp_file, patched.as_bytes(), permissions) {
                let _ = unlinkat(&parent, &temp_name, AtFlags::empty());
                return Err(error);
            }
            let patched_sha256 = sha256_hex(patched.as_bytes());
            if let Err(error) =
                renameat_with(&parent, &temp_name, &parent, &leaf, RenameFlags::EXCHANGE)
            {
                let _ = unlinkat(&parent, &temp_name, AtFlags::empty());
                return Err(match error {
                    Errno::NOENT => WorkspaceToolError::conflict(
                        "The workspace file disappeared before the patch could be committed.",
                    ),
                    _ => WorkspaceToolError::io(format!(
                        "Could not atomically exchange the workspace patch: {error}"
                    )),
                });
            }

            let displaced = read_snapshot_from_parent(&parent, &temp_name);
            let displaced_matches = displaced.as_ref().is_ok_and(|snapshot| {
                same_file_identity(&snapshot.stat, &original.stat)
                    && snapshot.sha256.eq_ignore_ascii_case(expected_sha256)
            });
            if !displaced_matches {
                let rollback =
                    renameat_with(&parent, &temp_name, &parent, &leaf, RenameFlags::EXCHANGE);
                if let Err(error) = rollback {
                    return Err(WorkspaceToolError::io(format!(
                        "A workspace conflict was detected after atomic exchange, and rollback failed; both versions were preserved under `{}` and `{temp_name}`: {error}",
                        display_relative_path(&relative)
                    )));
                }
                let temporary = read_snapshot_from_parent(&parent, &temp_name)?;
                if temporary.sha256 == patched_sha256 {
                    unlinkat(&parent, &temp_name, AtFlags::empty()).map_err(|error| {
                        WorkspaceToolError::io(format!(
                            "The conflicting workspace file was restored, but temporary patch cleanup failed at `{temp_name}`: {error}"
                        ))
                    })?;
                } else {
                    return Err(WorkspaceToolError::io(format!(
                        "The conflicting workspace file was restored, but a concurrently changed temporary patch was preserved at `{temp_name}`."
                    )));
                }
                let actual_hash = displaced
                    .as_ref()
                    .map(|snapshot| snapshot.sha256.as_str())
                    .unwrap_or("unreadable");
                return Err(hash_conflict(&relative, actual_hash));
            }

            unlinkat(&parent, &temp_name, AtFlags::empty()).map_err(|error| {
                WorkspaceToolError::io(format!(
                    "The workspace patch was committed, but displaced-file cleanup failed at `{temp_name}`: {error}"
                ))
            })?;
            fsync(&parent).map_err(|error| {
                WorkspaceToolError::io(format!(
                    "The workspace patch was committed, but its directory could not be synchronized: {error}"
                ))
            })?;
            Ok(WorkspaceMutationResult {
                path: display_relative_path(&relative),
                sha256: Some(patched_sha256),
                bytes_written: Some(patched.len() as u64),
                created: false,
            })
        }

        pub(super) fn create_directory(
            &self,
            path: &str,
        ) -> Result<WorkspaceMutationResult, WorkspaceToolError> {
            let relative = validated_relative_path(path, false)?;
            let (parent_path, leaf) = parent_and_leaf(&relative)?;
            let parent = secure_open(&self.root, &parent_path, DIRECTORY_FLAGS)?;
            match mkdirat(&parent, &leaf, Mode::from_raw_mode(0o700)) {
                Ok(()) => {}
                Err(error) if error == Errno::EXIST => {
                    return Err(WorkspaceToolError::conflict(format!(
                        "Workspace path `{}` already exists.",
                        display_relative_path(&relative)
                    )))
                }
                Err(error) => {
                    return Err(WorkspaceToolError::io(format!(
                        "Could not create the workspace directory: {error}"
                    )))
                }
            }
            fsync(&parent).map_err(|error| {
                WorkspaceToolError::io(format!(
                    "The workspace directory was created, but its parent could not be synchronized: {error}"
                ))
            })?;
            Ok(WorkspaceMutationResult {
                path: display_relative_path(&relative),
                sha256: None,
                bytes_written: None,
                created: true,
            })
        }

        fn read_snapshot(&self, relative: &Path) -> Result<FileSnapshot, WorkspaceToolError> {
            let fd = secure_open(&self.root, relative, FILE_FLAGS)?;
            read_snapshot_fd(fd)
        }
    }

    fn validated_relative_path(
        input: &str,
        allow_root: bool,
    ) -> Result<PathBuf, WorkspaceToolError> {
        let trimmed = input.trim();
        let input = if trimmed.is_empty() { "." } else { trimmed };
        if input.len() > 4_096 || input.contains('\0') {
            return Err(WorkspaceToolError::invalid(
                "The workspace path is empty, contains a null byte, or exceeds 4096 bytes.",
            ));
        }
        let path = Path::new(input);
        if path.is_absolute() {
            return Err(path_boundary_error());
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(component) => {
                    if component == ".git" {
                        return Err(WorkspaceToolError::invalid(
                            "The local Ollama agent cannot access the workspace's .git directory.",
                        ));
                    }
                    normalized.push(component);
                }
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(path_boundary_error())
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            if allow_root {
                return Ok(PathBuf::from("."));
            }
            return Err(WorkspaceToolError::invalid(
                "This workspace tool requires a non-root relative path.",
            ));
        }
        Ok(normalized)
    }

    fn path_boundary_error() -> WorkspaceToolError {
        WorkspaceToolError::invalid(
            "Workspace tool paths must be relative and remain inside the selected workspace.",
        )
    }

    fn secure_open(
        root: &OwnedFd,
        relative: &Path,
        flags: OFlags,
    ) -> Result<OwnedFd, WorkspaceToolError> {
        match openat2(root, relative, flags, Mode::empty(), SECURE_RESOLVE) {
            Ok(fd) => Ok(fd),
            Err(error) if error == Errno::NOSYS => fallback_open(root, relative, flags),
            Err(error) => Err(map_open_error(error, &display_relative_path(relative))),
        }
    }

    fn fallback_open(
        root: &OwnedFd,
        relative: &Path,
        final_flags: OFlags,
    ) -> Result<OwnedFd, WorkspaceToolError> {
        let components = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(component) => Some(component),
                _ => None,
            })
            .collect::<Vec<_>>();
        if components.is_empty() {
            return openat(root, ".", final_flags, Mode::empty())
                .map_err(|error| map_open_error(error, "."));
        }
        let mut current = openat(root, ".", DIRECTORY_FLAGS, Mode::empty())
            .map_err(|error| map_open_error(error, "."))?;
        for (index, component) in components.iter().enumerate() {
            let flags = if index + 1 == components.len() {
                final_flags
            } else {
                DIRECTORY_FLAGS
            };
            current = openat(&current, *component, flags, Mode::empty())
                .map_err(|error| map_open_error(error, &component.to_string_lossy()))?;
        }
        Ok(current)
    }

    fn map_open_error(error: Errno, path: &str) -> WorkspaceToolError {
        match error {
            Errno::NOENT => {
                WorkspaceToolError::not_found(format!("Workspace path `{path}` does not exist."))
            }
            Errno::LOOP | Errno::XDEV => WorkspaceToolError::invalid(format!(
                "Workspace path `{path}` crosses a symbolic-link or mount boundary."
            )),
            Errno::NOTDIR => WorkspaceToolError::invalid(format!(
                "Workspace path `{path}` contains a non-directory component."
            )),
            _ => WorkspaceToolError::io(format!("Could not open workspace path `{path}`: {error}")),
        }
    }

    fn read_snapshot_from_parent(
        parent: &OwnedFd,
        leaf: &str,
    ) -> Result<FileSnapshot, WorkspaceToolError> {
        let fd = secure_open(parent, Path::new(leaf), FILE_FLAGS)?;
        read_snapshot_fd(fd)
    }

    fn read_snapshot_fd(fd: OwnedFd) -> Result<FileSnapshot, WorkspaceToolError> {
        let before = fstat(&fd).map_err(|error| {
            WorkspaceToolError::io(format!("Could not inspect the workspace file: {error}"))
        })?;
        if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile {
            return Err(WorkspaceToolError::invalid(
                "The requested workspace item is not a regular file.",
            ));
        }
        let size = usize::try_from(before.st_size).map_err(|_| {
            WorkspaceToolError::limit("The workspace file size cannot be represented safely.")
        })?;
        if size > MAX_HASHABLE_FILE_BYTES {
            return Err(WorkspaceToolError::limit(format!(
                "The workspace file exceeds the {MAX_HASHABLE_FILE_BYTES}-byte hash-and-patch bound."
            )));
        }
        let mut file = File::from(fd);
        let mut bytes = Vec::with_capacity(size);
        Read::by_ref(&mut file)
            .take((MAX_HASHABLE_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                WorkspaceToolError::io(format!("Could not read the workspace file: {error}"))
            })?;
        if bytes.len() > MAX_HASHABLE_FILE_BYTES {
            return Err(WorkspaceToolError::limit(format!(
                "The workspace file grew beyond the {MAX_HASHABLE_FILE_BYTES}-byte hash-and-patch bound."
            )));
        }
        let after = fstat(&file).map_err(|error| {
            WorkspaceToolError::io(format!(
                "Could not re-inspect the workspace file after reading: {error}"
            ))
        })?;
        if !same_snapshot(&before, &after) || bytes.len() as i64 != after.st_size {
            return Err(WorkspaceToolError::conflict(
                "The workspace file changed while it was being read; read it again before editing.",
            ));
        }
        Ok(FileSnapshot {
            sha256: sha256_hex(&bytes),
            bytes,
            stat: after,
        })
    }

    fn same_snapshot(left: &Stat, right: &Stat) -> bool {
        same_file_identity(left, right)
            && left.st_size == right.st_size
            && left.st_mtime == right.st_mtime
            && left.st_mtime_nsec == right.st_mtime_nsec
            && left.st_ctime == right.st_ctime
            && left.st_ctime_nsec == right.st_ctime_nsec
    }

    fn same_file_identity(left: &Stat, right: &Stat) -> bool {
        left.st_dev == right.st_dev && left.st_ino == right.st_ino
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn hash_conflict(path: &Path, actual: &str) -> WorkspaceToolError {
        WorkspaceToolError::conflict(format!(
            "Workspace file `{}` no longer matches expectedSha256 (current SHA-256: {actual}); read it again and rebuild the patch.",
            display_relative_path(path)
        ))
    }

    fn apply_edits(
        original: &str,
        edits: &[WorkspacePatchEdit],
    ) -> Result<String, WorkspaceToolError> {
        let mut capacity = original.len();
        for edit in edits {
            let start = usize::try_from(edit.start_byte).map_err(|_| {
                WorkspaceToolError::invalid("A workspace patch startByte is too large.")
            })?;
            let end = usize::try_from(edit.end_byte).map_err(|_| {
                WorkspaceToolError::invalid("A workspace patch endByte is too large.")
            })?;
            if end > original.len() {
                return Err(WorkspaceToolError::invalid(format!(
                    "Workspace patch range {start}..{end} exceeds the {}-byte file length.",
                    original.len()
                )));
            }
            if !original.is_char_boundary(start) || !original.is_char_boundary(end) {
                return Err(WorkspaceToolError::invalid(
                    "Workspace patch ranges must begin and end on UTF-8 byte boundaries.",
                ));
            }
            capacity = capacity
                .saturating_sub(end - start)
                .saturating_add(edit.replacement.len());
        }
        if capacity > MAX_HASHABLE_FILE_BYTES {
            return Err(WorkspaceToolError::limit(format!(
                "The patched workspace file would exceed the {MAX_HASHABLE_FILE_BYTES}-byte bound."
            )));
        }
        let mut output = String::with_capacity(capacity);
        let mut cursor = 0_usize;
        for edit in edits {
            let start = edit.start_byte as usize;
            let end = edit.end_byte as usize;
            output.push_str(&original[cursor..start]);
            output.push_str(&edit.replacement);
            cursor = end;
        }
        output.push_str(&original[cursor..]);
        Ok(output)
    }

    fn parent_and_leaf(relative: &Path) -> Result<(PathBuf, String), WorkspaceToolError> {
        let leaf = relative
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| WorkspaceToolError::invalid("The workspace path has no file name."))?
            .to_string();
        let parent = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Ok((parent, leaf))
    }

    fn create_temp_file(parent: &OwnedFd) -> Result<(String, File), WorkspaceToolError> {
        for _ in 0..32 {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let name = format!(".aacc-workspace-{}-{id}.tmp", std::process::id());
            match openat(parent, &name, CREATE_FLAGS, Mode::from_raw_mode(0o600)) {
                Ok(fd) => return Ok((name, File::from(fd))),
                Err(error) if error == Errno::EXIST => continue,
                Err(error) => {
                    return Err(WorkspaceToolError::io(format!(
                        "Could not create an atomic workspace temporary file: {error}"
                    )))
                }
            }
        }
        Err(WorkspaceToolError::conflict(
            "Could not allocate a unique atomic workspace temporary file.",
        ))
    }

    fn write_and_sync(
        file: &mut File,
        content: &[u8],
        permissions: u32,
    ) -> Result<(), WorkspaceToolError> {
        file.set_permissions(Permissions::from_mode(permissions))
            .map_err(|error| {
                WorkspaceToolError::io(format!(
                    "Could not preserve workspace file permissions: {error}"
                ))
            })?;
        file.write_all(content).map_err(|error| {
            WorkspaceToolError::io(format!("Could not write the workspace file: {error}"))
        })?;
        file.sync_all().map_err(|error| {
            WorkspaceToolError::io(format!(
                "Could not synchronize the workspace file before commit: {error}"
            ))
        })
    }

    fn display_relative_path(path: &Path) -> String {
        if path == Path::new(".") {
            ".".to_string()
        } else {
            path.to_string_lossy().into_owned()
        }
    }

    fn join_display_path(parent: &Path, name: &str) -> String {
        if parent == Path::new(".") {
            name.to_string()
        } else {
            parent.join(name).to_string_lossy().into_owned()
        }
    }
}

#[cfg(test)]
mod tests;
