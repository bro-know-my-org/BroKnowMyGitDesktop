//! Credential selection and safe Git authentication plumbing.
//!
//! This module deliberately works with account metadata only.  A personal
//! access token is accepted only by [`save_personal_access_token`], stored in
//! the operating system credential store, and read again only by the askpass
//! helper process.  It is never part of a domain model, a serializable type,
//! a command-line argument, or an environment variable.

use crate::app_error::AppError;
use crate::domain::{
    endpoint_for_host, endpoint_matches_authority, git_authority_has_explicit_port,
    hostname_from_git_authority, normalize_git_authority, parse_remote_url, AccountProfile,
    AccountSelection, AuthenticationMethod, AuthenticationStatus, CodeHostEndpoint, CodeHostKind,
    CredentialResolution, CredentialStrategy, KeychainReference, RemoteBinding, RemoteTransport,
};
#[cfg(windows)]
use crate::process_tree::terminate_process_tree;
use crate::process_tree::{attach_process_tree, ProcessTreeHandle};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

/// Stable service name used for application-managed PAT entries.
pub const PAT_KEYCHAIN_SERVICE: &str = "io.github.broknowmyorg.broknowmygitdesktop.pat";

/// Non-secret sentinel used by `main.rs` to enter the private askpass mode
/// before Tauri starts.
pub const ASKPASS_MODE_ENV: &str = "BKM_GIT_ASKPASS_MODE";
const ASKPASS_CAPABILITY_ENV: &str = "BKM_GIT_ASKPASS_CAPABILITY";
const ASKPASS_MODE_KEYCHAIN: &str = "keychain-pat";
const ASKPASS_CAPABILITY_DIRECTORY: &str = "broknowmygitdesktop-askpass";
const ASKPASS_CAPABILITY_TTL: Duration = Duration::from_secs(180);
const PLATFORM_CLI_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PLATFORM_CLI_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PLATFORM_CLI_STDOUT_BYTES: usize = 256 * 1024;
const MAX_PLATFORM_CLI_STDERR_BYTES: usize = 256 * 1024;
const MAX_PLATFORM_CLI_PIPE_OPERATIONS: usize = 4;
const PLATFORM_CLI_DETACHED_PIPE_RELEASE_GRACE: Duration = Duration::from_secs(10);

static ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

/// A permit is retained by each CLI operation while its output workers drain
/// their pipes. If a descendant escapes process-tree termination and retains
/// a pipe, a short expiry releases the concurrency slot rather than leaving
/// gh/glab account detection permanently unavailable.
struct PlatformCliPipeOperationPermit {
    released: AtomicBool,
}

impl PlatformCliPipeOperationPermit {
    fn acquire() -> Result<Arc<Self>, PlatformCliCommandError> {
        let mut active = ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire);
        loop {
            if active >= MAX_PLATFORM_CLI_PIPE_OPERATIONS {
                return Err(PlatformCliCommandError::Busy);
            }
            match ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.compare_exchange_weak(
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

    /// A terminated child normally closes both pipes immediately. A descendant
    /// may keep one open forever, however, and Rust cannot safely cancel a
    /// blocking `Read` on another thread. Keep the short-term backpressure,
    /// then release the operation slot even if that detached reader survives.
    fn release_after_cleanup_grace(self: &Arc<Self>) {
        let permit = Arc::downgrade(self);
        thread::spawn(move || {
            thread::sleep(PLATFORM_CLI_DETACHED_PIPE_RELEASE_GRACE);
            if let Some(permit) = permit.upgrade() {
                permit.release();
            }
        });
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for PlatformCliPipeOperationPermit {
    fn drop(&mut self) {
        self.release();
    }
}

/// Input to one remote credential resolution.
///
/// This type intentionally does not implement `Debug` or serde traits because
/// `remote_url` can originate outside the application and may contain unsafe
/// userinfo.  The resolver rejects HTTP(S) userinfo before selecting a
/// credential.
pub struct CredentialResolutionRequest<'a> {
    /// Canonical repository path used by persisted `RemoteBinding` records.
    pub repository_path: &'a str,
    pub remote_name: &'a str,
    /// The exact fetch or push URL that Git is about to use.
    pub remote_url: &'a str,
    /// A manually associated endpoint, when one was already selected by UI.
    pub endpoint: Option<&'a CodeHostEndpoint>,
    pub endpoints: &'a [CodeHostEndpoint],
    pub bindings: &'a [RemoteBinding],
    pub accounts: &'a [AccountProfile],
}

/// Stateless broker for remote-to-account resolution.
#[derive(Default)]
pub struct CredentialBroker;

// These source fragments share one module intentionally: resolution,
// keychain, askpass and CLI internals must not become public APIs just to split
// the implementation into reviewable files.
include!("credential_broker/resolution.rs");
include!("credential_broker/environment.rs");
include!("credential_broker/askpass.rs");
include!("credential_broker/cli.rs");
include!("credential_broker/tests.rs");
