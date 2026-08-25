//! Safe, system-Git based repository operations.
//!
//! `GitCore` deliberately has no knowledge of CodeHost accounts or tokens.  A
//! caller that has resolved a [`crate::domain::CredentialStrategy`] can pass a
//! backend-only [`GitEnvironment`] to the network methods.  The environment is
//! not serializable and is never logged by this module.

use crate::app_error::AppError;
use crate::domain::{
    endpoint_for_host, parse_remote_url, sanitize_remote_url, BranchInfo, CodeHostEndpoint,
    CodeHostKind, CommitSummary, ExternalEditorConfig, GitOperationKind, GitStatus, GitStatusEntry,
    LfsStatus, RemoteBranchInfo, RemoteProfile, RepositoryRecord, ResetMode, StashEntry,
    SubmoduleInfo, SubmoduleStatus, TagInfo, WorktreeInfo,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const NETWORK_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const NETWORK_GIT_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_NETWORK_GIT_DIAGNOSTIC_BYTES: usize = 256 * 1024;
const BOUNDED_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LOCAL_GIT_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXTERNAL_EDITOR_ARGUMENTS: usize = 32;
const MAX_EXTERNAL_EDITOR_VALUE_LENGTH: usize = 4096;

/// Fixed first argument used when the packaged application acts as a harmless
/// Git editor for a sequencer continuation. It is intentionally not user
/// configurable; see `main.rs` for the early-exit handler.
pub const GIT_NOOP_EDITOR_ARGUMENT: &str = "--bkm-git-noop-editor";

#[cfg(windows)]
const DISABLED_GIT_HOOKS_PATH: &str = "NUL";
#[cfg(not(windows))]
const DISABLED_GIT_HOOKS_PATH: &str = "/dev/null";

/// A small wrapper around the system `git` executable.
///
/// Every invocation is built with `Command` arguments.  This type never
/// invokes a shell, which keeps repository paths, commit messages, refspecs,
/// and remote names from being interpreted as shell code.
#[derive(Debug, Clone)]
pub struct GitCore {
    executable: PathBuf,
    trusted_network_configurations: HashMap<PathBuf, String>,
}

mod branches;
mod changes;
mod editor;
mod environment;
mod error;
mod history;
mod models;
mod network;
mod parser;
mod remotes;
mod repository;
mod runtime;
mod stashes;
pub(crate) mod validation;
mod workspace;

#[cfg(test)]
mod tests;

pub(crate) use changes::{CleanUntrackedPathFingerprint, DiscardChangesFingerprint};
pub use editor::normalize_external_editor_config;
pub use environment::GitEnvironment;
pub use models::{
    CloneOptions, CommitResult, DiffOptions, FetchOptions, PullOptions, PushOptions, RemoteUrlKind,
    RepositorySnapshot,
};
pub(crate) use network::PublishedRemoteState;
pub(crate) use remotes::NetworkConfigurationAssessment;
pub(crate) use validation::{validate_git_remote_url, validate_ref_name};

impl Default for GitCore {
    fn default() -> Self {
        Self::new()
    }
}
