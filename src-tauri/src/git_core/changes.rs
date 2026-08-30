use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::runtime::git_path_from_output;
use super::validation::*;
use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_EXTERNAL_EDITOR_REAPERS: usize = 4;
static ACTIVE_EXTERNAL_EDITOR_REAPERS: AtomicUsize = AtomicUsize::new(0);
const MAX_REPOSITORY_EDITOR_REAPERS: usize = 128;
static ACTIVE_REPOSITORY_EDITOR_REAPERS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct WorktreeExcludesFile {
    pub(crate) path: PathBuf,
    pub(crate) watch_externally: bool,
    pub(crate) load_rules: bool,
}

/// A filesystem identity captured with an untracked-file cleanup preview.
/// It stays in memory only; it is neither exposed through IPC nor persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanUntrackedPathFingerprint {
    pub(crate) path: String,
    identity: CleanUntrackedPathIdentity,
}

/// Exact repository and filesystem state authorized by a discard preview.
/// This value remains backend-only and is consumed through a short-lived
/// token rather than being trusted from IPC input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscardChangesFingerprint {
    paths: Vec<String>,
    head_commit: Option<String>,
    head_reference: Option<String>,
    entries: Vec<GitStatusEntry>,
    index_entries: Vec<u8>,
    worktree_paths: Vec<DiscardWorktreePathFingerprint>,
    removal_fingerprints: Vec<CleanUntrackedPathFingerprint>,
    untracked_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscardWorktreePathFingerprint {
    Missing(String),
    Present(CleanUntrackedPathFingerprint),
}

impl DiscardChangesFingerprint {
    pub(crate) fn path_count(&self) -> usize {
        self.paths.len()
    }

    pub(crate) fn untracked_count(&self) -> usize {
        self.untracked_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanUntrackedPathIdentity {
    kind: CleanUntrackedPathKind,
    length: u64,
    modified_at: Option<std::time::SystemTime>,
    created_at: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at: (i64, i64),
    #[cfg(windows)]
    file_attributes: u32,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    last_write_time: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanUntrackedPathKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// An ancestor directory captured immediately before cleanup starts. It is
/// used only on Linux to remove an empty directory after its reviewed leaves
/// have been deleted, without deleting a directory another process created
/// at the same pathname meanwhile.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct CleanUntrackedDirectoryFingerprint {
    path: String,
    device: u64,
    inode: u64,
}

// Keep the security-sensitive filesystem helpers in this module's privacy
// boundary while making each workflow independently navigable.
include!("changes/clean.rs");
include!("changes/discard_transaction.rs");
include!("changes/operations.rs");
