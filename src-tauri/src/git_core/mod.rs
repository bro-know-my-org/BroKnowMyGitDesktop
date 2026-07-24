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
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const NETWORK_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const NETWORK_GIT_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_NETWORK_GIT_DIAGNOSTIC_BYTES: usize = 256 * 1024;
const MAX_EXTERNAL_EDITOR_ARGUMENTS: usize = 32;
const MAX_EXTERNAL_EDITOR_VALUE_LENGTH: usize = 4096;
const REMOTE_BRANCH_FETCH_REFSPEC_PATTERN: &str = "^[+]?refs/heads/.*";

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

pub use editor::normalize_external_editor_config;
pub use environment::GitEnvironment;
pub use models::{
    CloneOptions, CommitResult, DiffOptions, FetchOptions, PullOptions, PushOptions, RemoteUrlKind,
    RepositorySnapshot,
};
pub(crate) use validation::validate_git_remote_url;

impl Default for GitCore {
    fn default() -> Self {
        Self::new()
    }
}
