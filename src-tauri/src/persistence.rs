use crate::app_error::AppError;
use crate::credential_broker::delete_personal_access_token;
use crate::domain::{
    AccountProfile, AuthenticationMethod, CodeHostEndpoint, ExternalEditorConfig,
    KeychainReference, RemoteBinding, RepositoryRecord, TerminalConfig,
};
use crate::git_core::{CleanUntrackedPathFingerprint, DiscardChangesFingerprint, GitCore};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const STORE_FILE_NAME: &str = "profiles.json";
const MAX_REPOSITORIES: usize = 200;
const CLEAN_UNTRACKED_CONFIRMATION_TTL: Duration = Duration::from_secs(60);
const MAX_CLEAN_UNTRACKED_CONFIRMATIONS: usize = 256;
const DISCARD_CHANGES_CONFIRMATION_TTL: Duration = Duration::from_secs(60);
const MAX_DISCARD_CHANGES_CONFIRMATIONS: usize = 256;

/// Durable intent recorded before `gh repo create --push`. It contains only
/// local metadata needed to reconcile an origin that was created successfully
/// if persisting the final account binding fails afterward.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingPublish {
    pub repository_path: String,
    pub endpoint_id: String,
    pub account_profile_id: String,
    pub remote_name: String,
    /// `HEAD` at the time the publish side effect was requested. A stale
    /// intent must never bind an unrelated repository recreated at the same
    /// local path.
    #[serde(default)]
    pub expected_head: String,
    /// Expected GitHub `owner/repository` path derived before `gh` runs.
    /// Kept separate from a transport URL because gh may choose HTTPS or SSH.
    #[serde(default)]
    pub expected_repository_slug: String,
}

/// Durable clone metadata intent written before Git creates the destination.
/// It contains no secret and lets a later repository open reconcile a selected
/// account binding if the post-clone profile write was interrupted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingClone {
    pub destination_path: String,
    pub binding: Option<RemoteBinding>,
    /// Expected Git authority from the selected clone URL. This preserves an
    /// explicit port and is checked independently from mutable Endpoint
    /// metadata before a pending account binding can be restored.
    #[serde(default)]
    pub expected_remote_authority: String,
    /// Expected `owner/repository` path parsed from the selected clone URL.
    /// A pending account binding may be restored only when both effective
    /// remote URLs still identify this exact repository.
    #[serde(default)]
    pub expected_repository_slug: String,
}

/// Persisted application metadata. It intentionally has no field capable of holding a PAT.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStore {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub custom_endpoints: Vec<CodeHostEndpoint>,
    #[serde(default)]
    pub accounts: Vec<AccountProfile>,
    /// References queued after durable profile removal when the operating
    /// system keychain could not be updated immediately. These contain no
    /// secret and are retried on subsequent launches.
    #[serde(default)]
    pub pending_secret_deletions: Vec<KeychainReference>,
    #[serde(default)]
    pub pending_publishes: Vec<PendingPublish>,
    #[serde(default)]
    pub pending_clones: Vec<PendingClone>,
    #[serde(default)]
    pub remote_bindings: Vec<RemoteBinding>,
    /// The editor is app-local rather than repository configuration. It is
    /// launched with direct process arguments only after an explicit user
    /// action from the conflict workflow.
    #[serde(default)]
    pub external_editor: Option<ExternalEditorConfig>,
    /// Optional app-local override used only by the explicit "preferred
    /// terminal" action. The system-default action never reads this field.
    #[serde(default)]
    pub preferred_terminal: Option<TerminalConfig>,
    #[serde(default)]
    pub recent_repositories: Vec<RepositoryRecord>,
}

fn default_schema_version() -> u32 {
    5
}

// Preserve the existing private state boundary while separating storage,
// leases/confirmations and model mutations into navigable source files.
include!("persistence/profile_store.rs");
include!("persistence/storage.rs");
include!("persistence/state.rs");
include!("persistence/tests.rs");
