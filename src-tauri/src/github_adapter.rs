use crate::app_error::AppError;
use crate::codehost::{
    AdapterContext, CodeHostAdapter, CodeHostRepositoryInfo, CreatePullRequestInput,
    GitHubForkSummary, GitHubPublishOwner, GitHubPublishOwnerKind, PullRequestSummary,
};
use crate::domain::{
    git_authority_has_explicit_port, github_endpoint_has_gh_compatible_api,
    hostname_from_git_authority, CodeHostCapabilities, CodeHostEndpoint, CodeHostKind,
};
#[cfg(windows)]
use crate::process_tree::terminate_process_tree;
use crate::process_tree::{attach_process_tree, ProcessTreeHandle};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const GH_COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const GH_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_GH_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_GH_DIAGNOSTIC_BYTES: usize = 512 * 1024;
const MAX_PULL_REQUEST_PAGES: usize = 10;
const MAX_PUBLISH_OWNER_PAGES: usize = 10;
const MAX_GH_PIPE_OPERATIONS: usize = 4;
const GH_DETACHED_PIPE_RELEASE_GRACE: Duration = Duration::from_secs(10);

static ACTIVE_GH_PIPE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

/// One permit remains held while a gh operation's reader/writer workers drain
/// their pipes. An escaped descendant can retain a pipe after its parent is
/// killed, so abandoned workers eventually release their concurrency slot.
struct GhPipeOperationPermit {
    released: AtomicBool,
}

impl GhPipeOperationPermit {
    fn acquire() -> Result<Arc<Self>, AppError> {
        let mut active = ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire);
        loop {
            if active >= MAX_GH_PIPE_OPERATIONS {
                return Err(AppError::new(
                    "github_api_busy",
                    "GitHub 请求正在清理先前的网络任务，请稍后重试。",
                ));
            }
            match ACTIVE_GH_PIPE_OPERATIONS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Arc::new(Self {
                        released: AtomicBool::new(false),
                    }));
                }
                Err(observed) => active = observed,
            }
        }
    }

    /// Blocking reads and writes cannot be safely cancelled from another Rust
    /// thread. Keep backpressure while the process tree is being terminated,
    /// then free the slot so a stale descriptor cannot make GitHub actions
    /// permanently report `github_api_busy`.
    fn release_after_cleanup_grace(self: &Arc<Self>) {
        let permit = Arc::downgrade(self);
        thread::spawn(move || {
            thread::sleep(GH_DETACHED_PIPE_RELEASE_GRACE);
            if let Some(permit) = permit.upgrade() {
                permit.release();
            }
        });
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            ACTIVE_GH_PIPE_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for GhPipeOperationPermit {
    fn drop(&mut self) {
        self.release();
    }
}

/// GitHub adapter backed by the user's existing gh authentication. It never
/// changes the active gh account or asks gh to persist Git configuration.
pub struct GitHubAdapter;

pub(crate) struct ValidatedGitHubPublishOwner {
    endpoint_id: String,
    login: String,
    identity: PinnedGhIdentity,
}

impl ValidatedGitHubPublishOwner {
    pub(crate) fn login(&self) -> &str {
        &self.login
    }
}

struct PinnedGhIdentity {
    hostname: String,
    token: Zeroizing<String>,
    config_directory: PinnedGhConfigDirectory,
}

struct PinnedGhConfigDirectory {
    path: PathBuf,
}

impl PinnedGhConfigDirectory {
    fn create() -> Result<Self, AppError> {
        let parent = std::env::temp_dir();
        for _ in 0..16 {
            let path = parent.join(format!("broknowmygitdesktop-gh-{}", uuid::Uuid::new_v4()));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AppError::io("创建临时 gh 配置目录", error)),
            }
        }
        Err(AppError::new("io_error", "无法创建临时 gh 配置目录。"))
    }
}

impl Drop for PinnedGhConfigDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// The adapter fragments remain one private module so process execution and
// identity-pinning helpers cannot be bypassed through wider visibility.
include!("github_adapter/adapter.rs");
include!("github_adapter/models_and_process.rs");
include!("github_adapter/pull_requests.rs");
include!("github_adapter/tests.rs");
