use crate::app_error::AppError;
use crate::credential_broker::delete_personal_access_token;
use crate::domain::{
    AccountProfile, AuthenticationMethod, CodeHostEndpoint, ExternalEditorConfig,
    KeychainReference, RemoteBinding, RepositoryRecord,
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
    #[serde(default)]
    pub recent_repositories: Vec<RepositoryRecord>,
}

fn default_schema_version() -> u32 {
    4
}

impl ProfileStore {
    pub fn all_endpoints(&self) -> Vec<CodeHostEndpoint> {
        let mut endpoints = CodeHostEndpoint::builtin();
        endpoints.extend(self.custom_endpoints.clone());
        endpoints
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<CodeHostEndpoint> {
        self.all_endpoints()
            .into_iter()
            .find(|endpoint| endpoint.id == endpoint_id)
    }

    pub fn upsert_binding(&mut self, binding: RemoteBinding) {
        if let Some(existing) = self.remote_bindings.iter_mut().find(|existing| {
            existing.repository_path == binding.repository_path
                && existing.remote_name == binding.remote_name
        }) {
            *existing = binding;
        } else {
            self.remote_bindings.push(binding);
        }
    }

    pub fn binding_for(&self, repository_path: &str, remote_name: &str) -> Option<RemoteBinding> {
        self.remote_bindings
            .iter()
            .find(|binding| {
                binding.repository_path == repository_path && binding.remote_name == remote_name
            })
            .cloned()
    }

    pub fn remove_account(&mut self, account_id: &str) -> Option<AccountProfile> {
        let index = self
            .accounts
            .iter()
            .position(|account| account.id == account_id)?;
        let removed = self.accounts.remove(index);
        self.remote_bindings.retain(|binding| {
            !matches!(
                &binding.account_selection,
                crate::domain::AccountSelection::Profile(profile_id) if profile_id == account_id
            )
        });
        self.pending_publishes
            .retain(|pending| pending.account_profile_id != account_id);
        self.pending_clones.retain(|pending| {
            !pending.binding.as_ref().is_some_and(|binding| {
                matches!(
                    &binding.account_selection,
                    crate::domain::AccountSelection::Profile(profile_id) if profile_id == account_id
                )
            })
        });
        Some(removed)
    }

    pub fn queue_secret_deletion(&mut self, keychain_ref: KeychainReference) {
        if !self
            .pending_secret_deletions
            .iter()
            .any(|existing| existing == &keychain_ref)
        {
            self.pending_secret_deletions.push(keychain_ref);
        }
    }

    pub fn clear_pending_secret_deletion(&mut self, keychain_ref: &KeychainReference) {
        self.pending_secret_deletions
            .retain(|existing| existing != keychain_ref);
    }

    pub fn queue_pending_publish(&mut self, pending: PendingPublish) {
        self.pending_publishes
            .retain(|existing| existing.repository_path != pending.repository_path);
        self.pending_publishes.push(pending);
    }

    pub fn pending_publish_for(&self, repository_path: &str) -> Option<PendingPublish> {
        self.pending_publishes
            .iter()
            .find(|pending| pending.repository_path == repository_path)
            .cloned()
    }

    pub fn clear_pending_publish(&mut self, repository_path: &str) {
        self.pending_publishes
            .retain(|pending| pending.repository_path != repository_path);
    }

    pub fn queue_pending_clone(&mut self, pending: PendingClone) {
        self.pending_clones
            .retain(|existing| existing.destination_path != pending.destination_path);
        self.pending_clones.push(pending);
    }

    pub fn clear_pending_clone(&mut self, destination_path: &str) {
        self.pending_clones
            .retain(|pending| pending.destination_path != destination_path);
    }

    /// Remove every durable record scoped to a repository path. This is used
    /// after a linked worktree is deleted so a later, unrelated repository
    /// created at the same path cannot inherit its endpoint/account binding.
    pub fn forget_repository_path(&mut self, repository_path: &str) {
        self.recent_repositories
            .retain(|repository| repository.path != repository_path);
        self.remote_bindings
            .retain(|binding| binding.repository_path != repository_path);
        self.pending_publishes
            .retain(|pending| pending.repository_path != repository_path);
        self.pending_clones
            .retain(|pending| pending.destination_path != repository_path);
    }

    /// Register a repository discovered by Git while retaining local-only
    /// metadata already attached to it. Discovery is deliberately not treated
    /// as usage: only an explicit open updates `last_opened_at`.
    pub fn upsert_repository(&mut self, mut repository: RepositoryRecord) -> RepositoryRecord {
        if let Some(existing) = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
        {
            repository.alias = existing.alias.clone();
            repository.group = existing.group.clone();
            repository.host = existing.host.clone();
            repository.namespace = existing.namespace.clone();
            repository.last_opened_at = existing.last_opened_at;
            *existing = repository.clone();
            return repository;
        }

        self.recent_repositories.insert(0, repository.clone());
        self.recent_repositories.truncate(MAX_REPOSITORIES);
        repository
    }

    /// Merge the persisted local-only fields into a freshly read Git record.
    pub fn repository_with_metadata(&self, mut repository: RepositoryRecord) -> RepositoryRecord {
        if let Some(existing) = self
            .recent_repositories
            .iter()
            .find(|existing| existing.path == repository.path)
        {
            repository.alias = existing.alias.clone();
            repository.group = existing.group.clone();
            repository.host = existing.host.clone();
            repository.namespace = existing.namespace.clone();
            repository.last_opened_at = existing.last_opened_at;
        }
        repository
    }

    /// Mark an explicitly opened repository as recently used and refresh its
    /// safe remote owner metadata. A missing recognizable remote intentionally
    /// clears a stale owner group rather than showing incorrect old data.
    pub fn touch_repository(
        &mut self,
        repository: RepositoryRecord,
        host: Option<String>,
        namespace: Option<String>,
        opened_at: u64,
    ) -> RepositoryRecord {
        let repository = self.upsert_repository(repository);
        let index = self
            .recent_repositories
            .iter()
            .position(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        let mut updated = self.recent_repositories.remove(index);
        updated.host = host;
        updated.namespace = namespace;
        updated.last_opened_at = Some(opened_at);
        // The collection is bounded from its tail, so a repository that was
        // explicitly opened must be promoted before subsequent discoveries
        // can evict it.
        self.recent_repositories.insert(0, updated.clone());
        updated
    }

    pub fn set_repository_group(
        &mut self,
        repository: RepositoryRecord,
        group: Option<String>,
    ) -> RepositoryRecord {
        let repository = self.upsert_repository(repository);
        let existing = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        existing.group = group;
        existing.clone()
    }

    pub fn set_repository_alias(
        &mut self,
        repository: RepositoryRecord,
        alias: Option<String>,
    ) -> RepositoryRecord {
        let repository = self.upsert_repository(repository);
        let existing = self
            .recent_repositories
            .iter_mut()
            .find(|existing| existing.path == repository.path)
            .expect("repository was just inserted or updated");
        existing.alias = alias;
        existing.clone()
    }
}

pub struct PersistentProfileStore {
    path: PathBuf,
    data: ProfileStore,
}

/// An update may already have replaced the durable store before a final
/// directory sync fails. Callers must keep the new in-memory value in that
/// case; rolling it back would make memory disagree with `profiles.json`.
enum SaveError {
    BeforeReplace(AppError),
    AfterReplace(AppError),
}

/// Distinguishes a mutation that definitely did not reach `profiles.json`
/// from one that crossed the atomic-replace boundary but could not confirm the
/// final directory sync. External compensations (keychain deletion and Git
/// rollback) must only run for the former.
pub(crate) enum MutationFailure {
    Operation(AppError),
    BeforeReplace(AppError),
    AfterReplace(AppError),
}

impl MutationFailure {
    pub(crate) fn committed(&self) -> bool {
        matches!(self, Self::AfterReplace(_))
    }

    pub(crate) fn into_error(self) -> AppError {
        match self {
            Self::Operation(error) | Self::BeforeReplace(error) | Self::AfterReplace(error) => {
                error
            }
        }
    }
}

impl PersistentProfileStore {
    pub fn load(app_data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(app_data_dir)
            .map_err(|error| AppError::io("创建应用数据目录", error))?;
        let path = app_data_dir.join(STORE_FILE_NAME);
        let data = if path.exists() {
            let content = fs::read(&path).map_err(|error| AppError::io("读取账号配置", error))?;
            let mut data: ProfileStore = serde_json::from_slice(&content).map_err(|_| {
                AppError::new(
                    "profile_store_invalid",
                    "账号配置文件无法解析；为保护现有数据，未自动覆盖它。",
                )
            })?;
            if data.schema_version > default_schema_version() {
                return Err(AppError::new(
                    "profile_store_version_unsupported",
                    "账号配置由更高版本的应用创建，无法安全写入。",
                ));
            }
            data.schema_version = default_schema_version();
            data
        } else {
            empty_profile_store()
        };

        Ok(Self { path, data })
    }

    /// Preserve a malformed store under a unique sibling name, then start
    /// with an empty in-memory store. This lets the application launch while
    /// keeping the original bytes available for recovery or diagnosis.
    pub fn recover_invalid(app_data_dir: &Path) -> Result<Self, AppError> {
        fs::create_dir_all(app_data_dir)
            .map_err(|error| AppError::io("创建应用数据目录", error))?;
        let path = app_data_dir.join(STORE_FILE_NAME);
        if path.exists() {
            let backup = app_data_dir.join(format!("profiles.invalid.{}.json", Uuid::new_v4()));
            fs::rename(&path, backup).map_err(|error| AppError::io("备份损坏的账号配置", error))?;
        }
        Ok(Self {
            path,
            data: empty_profile_store(),
        })
    }

    pub fn data(&self) -> &ProfileStore {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut ProfileStore {
        &mut self.data
    }

    fn save(&self) -> Result<(), SaveError> {
        let serialized = serde_json::to_vec_pretty(&self.data).map_err(|_| {
            SaveError::BeforeReplace(AppError::new(
                "profile_store_serialize_failed",
                "无法保存账号配置",
            ))
        })?;
        let temporary_path = self
            .path
            .with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> io::Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                // Account metadata is not a secret store, but it includes
                // local repository paths and keychain identifiers. Create
                // the replacement with owner-only permissions so a normal
                // umask cannot make the atomically renamed file world-readable.
                options.mode(0o600);
            }
            let mut temporary = options.open(&temporary_path)?;
            temporary.write_all(&serialized)?;
            temporary.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary_path);
            return Err(SaveError::BeforeReplace(AppError::io(
                "写入账号配置临时文件",
                error,
            )));
        }

        if let Err(error) = replace_file(&temporary_path, &self.path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(SaveError::BeforeReplace(AppError::io(
                "原子更新账号配置",
                error,
            )));
        }
        if let Err(_error) = sync_parent_directory(&self.path) {
            return Err(SaveError::AfterReplace(AppError::with_safe_details(
                "profile_store_sync_failed",
                "账号配置已更新，但无法确认其已同步到磁盘。",
                "请避免立即断电，并在恢复后检查账号配置。",
            )));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, destination_path: &Path) -> io::Result<()> {
    fs::rename(temporary_path, destination_path)
}

#[cfg(windows)]
fn replace_file(temporary_path: &Path, destination_path: &Path) -> io::Result<()> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let temporary: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let destination: Vec<u16> = destination_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    // SAFETY: both strings are NUL-terminated UTF-16 paths that remain alive
    // throughout the call. MoveFileExW replaces the existing destination and
    // requests that Windows flush the metadata before returning.
    let replaced = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "profile store has no parent directory",
        )
    })?;
    match fs::File::open(parent)?.sync_all() {
        Ok(()) => Ok(()),
        // Darwin commonly rejects fsync on directory descriptors. The file
        // itself was already synced before its atomic replacement, so this
        // portability limitation must not be surfaced as a failed save.
        #[cfg(target_os = "macos")]
        Err(error) if error.raw_os_error() == Some(libc::EINVAL) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    // Windows uses MOVEFILE_WRITE_THROUGH in `replace_file`; opening a
    // directory through std::fs::File is not portable there.
    Ok(())
}

/// Shared Tauri state. Commands mutate metadata through `mutate`, which persists only after
/// the whole mutation succeeds and restores the in-memory snapshot on a write failure.
#[derive(Clone)]
pub struct AppState {
    profiles: Arc<Mutex<PersistentProfileStore>>,
    active_publishes: Arc<Mutex<HashSet<String>>>,
    active_clones: Arc<Mutex<HashSet<String>>>,
    active_repository_operations: Arc<Mutex<HashSet<String>>>,
    active_account_operations: Arc<Mutex<HashSet<String>>>,
    active_cli_account_imports: Arc<Mutex<HashSet<String>>>,
    account_status_refresh_generation: Arc<Mutex<u64>>,
    clean_untracked_confirmations: Arc<Mutex<HashMap<String, CleanUntrackedConfirmation>>>,
    discard_changes_confirmations: Arc<Mutex<HashMap<String, DiscardChangesConfirmation>>>,
    // An advisory lock held for this AppState's lifetime. It serializes
    // independent desktop processes using the same profile directory.
    _profile_store_lock: Arc<fs::File>,
}

struct CleanUntrackedConfirmation {
    token: String,
    fingerprints: Vec<CleanUntrackedPathFingerprint>,
    issued_at: Instant,
}

struct DiscardChangesConfirmation {
    token: String,
    fingerprint: DiscardChangesFingerprint,
    issued_at: Instant,
}

/// In-process lease for a repository publish workflow. GitHub publishing has
/// an external side effect plus a durable reconciliation intent, so two
/// concurrent attempts for the same repository must not interleave.
pub struct PublishLease {
    active_publishes: Arc<Mutex<HashSet<String>>>,
    repository_path: String,
}

/// In-process lease for a clone destination. The durable intent is keyed by
/// destination path, so concurrent clone attempts must not overwrite or clear
/// one another's recovery record.
pub struct CloneLease {
    active_clones: Arc<Mutex<HashSet<String>>>,
    destination_path: String,
}

/// In-process lease for one repository's mutating Git work, remote/binding
/// changes, and platform operations that resolve a remote identity. This
/// prevents concurrent IPC calls from interleaving index/worktree changes or
/// from constructing a platform request against a remote that is changing.
pub struct RepositoryOperationLease {
    active_repository_operations: Arc<Mutex<HashSet<String>>>,
    repository_paths: Vec<String>,
}

/// In-process lease for one or more accounts supplying a long-running
/// external operation. It prevents account removal from deleting recovery
/// metadata or revoking an identity while that operation is still executing.
pub struct AccountOperationLease {
    active_account_operations: Arc<Mutex<HashSet<String>>>,
    account_profile_ids: Vec<String>,
}

/// Serializes a CLI import with creation or deletion of every CLI account for
/// the same Endpoint. Account IDs are not known until an external CLI returns,
/// so an endpoint-and-authentication-method key is required in addition to
/// the ordinary per-account lease.
pub struct CliAccountImportLease {
    active_cli_account_imports: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for PublishLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_publishes.lock() {
            active.remove(&self.repository_path);
        }
    }
}

impl Drop for CloneLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_clones.lock() {
            active.remove(&self.destination_path);
        }
    }
}

impl Drop for RepositoryOperationLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_repository_operations.lock() {
            for repository_path in &self.repository_paths {
                active.remove(repository_path);
            }
        }
    }
}

impl Drop for AccountOperationLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_account_operations.lock() {
            for account_profile_id in &self.account_profile_ids {
                active.remove(account_profile_id);
            }
        }
    }
}

impl Drop for CliAccountImportLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_cli_account_imports.lock() {
            active.remove(&self.key);
        }
    }
}

impl AppState {
    pub fn load(app_data_dir: &Path) -> Result<Self, AppError> {
        let profile_store_lock = acquire_profile_store_lock(app_data_dir)?;
        let profiles = match PersistentProfileStore::load(app_data_dir) {
            Ok(profiles) => profiles,
            Err(error) if error.code() == "profile_store_invalid" => {
                PersistentProfileStore::recover_invalid(app_data_dir)?
            }
            Err(error) => return Err(error),
        };
        let state = Self {
            profiles: Arc::new(Mutex::new(profiles)),
            active_publishes: Arc::new(Mutex::new(HashSet::new())),
            active_clones: Arc::new(Mutex::new(HashSet::new())),
            active_repository_operations: Arc::new(Mutex::new(HashSet::new())),
            active_account_operations: Arc::new(Mutex::new(HashSet::new())),
            active_cli_account_imports: Arc::new(Mutex::new(HashSet::new())),
            account_status_refresh_generation: Arc::new(Mutex::new(0)),
            clean_untracked_confirmations: Arc::new(Mutex::new(HashMap::new())),
            discard_changes_confirmations: Arc::new(Mutex::new(HashMap::new())),
            _profile_store_lock: Arc::new(profile_store_lock),
        };
        state.retry_pending_secret_deletions_in_background();
        Ok(state)
    }

    pub fn snapshot(&self) -> Result<ProfileStore, AppError> {
        self.profiles
            .lock()
            .map(|store| store.data().clone())
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))
    }

    /// Starts a status refresh generation. Results from an older generation
    /// must not overwrite a newer refresh or a concurrent account import.
    pub(crate) fn begin_account_status_refresh(&self) -> Result<u64, AppError> {
        let mut generation = self
            .account_status_refresh_generation
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        *generation = generation.wrapping_add(1);
        Ok(*generation)
    }

    /// Changes account or Endpoint metadata while advancing the refresh
    /// generation under the same lock. A refresh cannot snapshot the old
    /// metadata after the generation check but before this mutation commits.
    pub(crate) fn mutate_and_invalidate_account_status_refreshes<T>(
        &self,
        operation: impl FnOnce(&mut ProfileStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut generation = self
            .account_status_refresh_generation
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        match self.mutate_with_outcome(operation) {
            Ok(value) => {
                *generation = generation.wrapping_add(1);
                Ok(value)
            }
            Err(error) if error.committed() => {
                // The durable store already changed despite a later sync
                // warning, so an older refresh must still be invalidated.
                *generation = generation.wrapping_add(1);
                Err(error.into_error())
            }
            Err(error) => Err(error.into_error()),
        }
    }

    /// Persists a refresh result only while it is still the newest requested
    /// generation. Holding the generation lock through the profile mutation
    /// makes the check and write one ordered operation relative to a later
    /// refresh or account import.
    pub(crate) fn commit_account_status_refresh<T>(
        &self,
        generation: u64,
        operation: impl FnOnce(&mut ProfileStore) -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        let current = self
            .account_status_refresh_generation
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if *current != generation {
            return Ok(None);
        }
        self.mutate(operation).map(Some)
    }

    /// Records the exact untracked file set shown to the user. This state is
    /// deliberately in-memory and short lived: a restart invalidates a
    /// destructive confirmation rather than recreating it.
    pub(crate) fn issue_clean_untracked_confirmation(
        &self,
        repository_path: &str,
        fingerprints: Vec<CleanUntrackedPathFingerprint>,
    ) -> Result<String, AppError> {
        let token = Uuid::new_v4().to_string();
        let mut confirmations = self
            .clean_untracked_confirmations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        confirmations.retain(|_, confirmation| {
            confirmation.issued_at.elapsed() <= CLEAN_UNTRACKED_CONFIRMATION_TTL
        });
        if !confirmations.contains_key(repository_path)
            && confirmations.len() >= MAX_CLEAN_UNTRACKED_CONFIRMATIONS
        {
            if let Some(oldest_path) = confirmations
                .iter()
                .min_by_key(|(_, confirmation)| confirmation.issued_at)
                .map(|(path, _)| path.clone())
            {
                confirmations.remove(&oldest_path);
            }
        }
        confirmations.insert(
            repository_path.to_owned(),
            CleanUntrackedConfirmation {
                token: token.clone(),
                fingerprints,
                issued_at: Instant::now(),
            },
        );
        Ok(token)
    }

    /// Consumes a preview confirmation. A confirmation cannot be replayed or
    /// used for a different repository path.
    pub(crate) fn take_clean_untracked_confirmation(
        &self,
        repository_path: &str,
        token: &str,
    ) -> Result<Vec<CleanUntrackedPathFingerprint>, AppError> {
        let mut confirmations = self
            .clean_untracked_confirmations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        // Preserve the requested entry long enough to return the more useful
        // expiration error below, while pruning stale confirmations belonging
        // to every other repository.
        confirmations.retain(|path, confirmation| {
            path == repository_path
                || confirmation.issued_at.elapsed() <= CLEAN_UNTRACKED_CONFIRMATION_TTL
        });
        let confirmation = confirmations.get(repository_path).ok_or_else(|| {
            AppError::new(
                "clean_untracked_confirmation_required",
                "请先重新预览未跟踪文件，再执行清理。",
            )
        })?;
        if confirmation.token != token {
            return Err(AppError::new(
                "clean_untracked_confirmation_required",
                "请先重新预览未跟踪文件，再执行清理。",
            ));
        }
        if confirmation.issued_at.elapsed() > CLEAN_UNTRACKED_CONFIRMATION_TTL {
            confirmations.remove(repository_path);
            return Err(AppError::new(
                "clean_untracked_confirmation_expired",
                "未跟踪文件预览已过期，请重新确认后再清理。",
            ));
        }
        let fingerprints = confirmation.fingerprints.clone();
        confirmations.remove(repository_path);
        Ok(fingerprints)
    }

    /// Records the exact selected Git and filesystem state shown by a discard
    /// confirmation. A later preview for the same repository supersedes the
    /// earlier one instead of allowing multiple destructive grants to coexist.
    pub(crate) fn issue_discard_changes_confirmation(
        &self,
        repository_path: &str,
        fingerprint: DiscardChangesFingerprint,
    ) -> Result<String, AppError> {
        let token = Uuid::new_v4().to_string();
        let mut confirmations = self
            .discard_changes_confirmations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        confirmations.retain(|_, confirmation| {
            confirmation.issued_at.elapsed() <= DISCARD_CHANGES_CONFIRMATION_TTL
        });
        if !confirmations.contains_key(repository_path)
            && confirmations.len() >= MAX_DISCARD_CHANGES_CONFIRMATIONS
        {
            if let Some(oldest_path) = confirmations
                .iter()
                .min_by_key(|(_, confirmation)| confirmation.issued_at)
                .map(|(path, _)| path.clone())
            {
                confirmations.remove(&oldest_path);
            }
        }
        confirmations.insert(
            repository_path.to_owned(),
            DiscardChangesConfirmation {
                token: token.clone(),
                fingerprint,
                issued_at: Instant::now(),
            },
        );
        Ok(token)
    }

    /// Consumes a repository-scoped discard confirmation. Invalid attempts do
    /// not consume a matching grant, but a successful take is always one-shot.
    pub(crate) fn take_discard_changes_confirmation(
        &self,
        repository_path: &str,
        token: &str,
    ) -> Result<DiscardChangesFingerprint, AppError> {
        let mut confirmations = self
            .discard_changes_confirmations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        confirmations.retain(|path, confirmation| {
            path == repository_path
                || confirmation.issued_at.elapsed() <= DISCARD_CHANGES_CONFIRMATION_TTL
        });
        let confirmation = confirmations.get(repository_path).ok_or_else(|| {
            AppError::new(
                "discard_confirmation_required",
                "请先重新确认要丢弃的改动。",
            )
        })?;
        if confirmation.token != token {
            return Err(AppError::new(
                "discard_confirmation_required",
                "请先重新确认要丢弃的改动。",
            ));
        }
        if confirmation.issued_at.elapsed() > DISCARD_CHANGES_CONFIRMATION_TTL {
            confirmations.remove(repository_path);
            return Err(AppError::new(
                "discard_confirmation_expired",
                "丢弃改动确认已过期，请重新确认。",
            ));
        }
        let confirmation = confirmations.remove(repository_path).ok_or_else(|| {
            AppError::new(
                "discard_confirmation_required",
                "请先重新确认要丢弃的改动。",
            )
        })?;
        Ok(confirmation.fingerprint)
    }

    pub fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut ProfileStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.mutate_with_outcome(operation)
            .map_err(MutationFailure::into_error)
    }

    pub(crate) fn mutate_with_outcome<T>(
        &self,
        operation: impl FnOnce(&mut ProfileStore) -> Result<T, AppError>,
    ) -> Result<T, MutationFailure> {
        let mut store = self.profiles.lock().map_err(|_| {
            MutationFailure::Operation(AppError::new(
                "state_poisoned",
                "应用账号状态不可用，请重启应用。",
            ))
        })?;
        let before = store.data().clone();
        let result = operation(store.data_mut());
        match result {
            Ok(value) => match store.save() {
                Ok(()) => Ok(value),
                Err(SaveError::BeforeReplace(error)) => {
                    *store.data_mut() = before;
                    Err(MutationFailure::BeforeReplace(error))
                }
                Err(SaveError::AfterReplace(error)) => Err(MutationFailure::AfterReplace(error)),
            },
            Err(error) => {
                *store.data_mut() = before;
                Err(MutationFailure::Operation(error))
            }
        }
    }

    /// Acquire an exclusive publish lease for one canonical repository path.
    /// This lock is deliberately separate from the persistent-store mutex so
    /// no filesystem or network operation runs while metadata is locked.
    pub fn acquire_publish_lease(&self, repository_path: &str) -> Result<PublishLease, AppError> {
        let repository_path = repository_path.to_owned();
        let mut active = self
            .active_publishes
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if !active.insert(repository_path.clone()) {
            return Err(AppError::new(
                "github_publish_in_progress",
                "该仓库正在发布到 GitHub，请等待当前操作完成。",
            ));
        }
        Ok(PublishLease {
            active_publishes: Arc::clone(&self.active_publishes),
            repository_path,
        })
    }

    /// Acquire an exclusive clone lease for a normalized destination path.
    /// It is intentionally independent from the store mutex, so the Git
    /// operation itself never holds metadata locked.
    pub fn acquire_clone_lease(&self, destination_path: &str) -> Result<CloneLease, AppError> {
        let destination_path = destination_path.to_owned();
        let mut active = self
            .active_clones
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if !active.insert(destination_path.clone()) {
            return Err(AppError::new(
                "clone_in_progress",
                "该目标目录正在执行 Clone，请等待当前操作完成。",
            ));
        }
        Ok(CloneLease {
            active_clones: Arc::clone(&self.active_clones),
            destination_path,
        })
    }

    /// Acquires exclusive use of one persisted account identity. This is
    /// separate from the store mutex so an external CLI process never holds
    /// metadata locked, while profile removal can still fail promptly instead
    /// of interleaving with that process.
    pub fn acquire_account_operation_lease(
        &self,
        account_profile_id: &str,
    ) -> Result<AccountOperationLease, AppError> {
        self.acquire_account_operation_leases(&[account_profile_id])
    }

    /// Atomically acquires exclusive use of several account identities. CLI
    /// imports protect every existing profile that could be reconciled so a
    /// concurrent removal cannot make the import recreate a deleted account.
    pub fn acquire_account_operation_leases(
        &self,
        account_profile_ids: &[&str],
    ) -> Result<AccountOperationLease, AppError> {
        let mut account_profile_ids: Vec<String> = account_profile_ids
            .iter()
            .map(|account_profile_id| (*account_profile_id).to_owned())
            .collect();
        account_profile_ids.sort_unstable();
        account_profile_ids.dedup();
        if account_profile_ids.is_empty()
            || account_profile_ids
                .iter()
                .any(|account_profile_id| account_profile_id.is_empty())
        {
            return Err(AppError::new(
                "account_operation_account_required",
                "账号标识不能为空。",
            ));
        }
        let mut active = self
            .active_account_operations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if account_profile_ids
            .iter()
            .any(|account_profile_id| active.contains(account_profile_id))
        {
            return Err(AppError::new(
                "account_operation_in_progress",
                "该账号正在执行操作，请等待当前操作完成。",
            ));
        }
        active.extend(account_profile_ids.iter().cloned());
        Ok(AccountOperationLease {
            active_account_operations: Arc::clone(&self.active_account_operations),
            account_profile_ids,
        })
    }

    /// Acquires the short-lived mutation lease for a CLI account type. A
    /// caller receives `None` for account types that cannot be imported by a
    /// CLI, so PAT and SSH profile changes retain their existing behavior.
    pub fn acquire_cli_account_import_lease(
        &self,
        endpoint_id: &str,
        auth_method: &AuthenticationMethod,
    ) -> Result<Option<CliAccountImportLease>, AppError> {
        let Some(key) = cli_account_import_key(endpoint_id, auth_method) else {
            return Ok(None);
        };
        let mut active = self
            .active_cli_account_imports
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if !active.insert(key.clone()) {
            return Err(AppError::new(
                "account_import_in_progress",
                "该 Endpoint 正在导入 CLI 账号，请等待当前操作完成。",
            ));
        }
        Ok(Some(CliAccountImportLease {
            active_cli_account_imports: Arc::clone(&self.active_cli_account_imports),
            key,
        }))
    }

    /// Acquire exclusive access to one repository's Git mutations, remote
    /// configuration, credentialed network operations, and platform context
    /// resolution. The persistent-store mutex is never held while this lease
    /// is active.
    pub fn acquire_repository_operation_lease(
        &self,
        repository_path: &str,
    ) -> Result<RepositoryOperationLease, AppError> {
        let git = GitCore::new();
        let repository = std::path::Path::new(repository_path);
        let root = match git.repository_root(repository) {
            Ok(root) => root,
            // Clone and init acquire this lease before a Git worktree exists.
            // Their destination key remains a single-path lease until Git has
            // published the repository and normal operations can resolve all
            // linked worktrees.
            Err(error) if error.code() == "not_a_git_repository" => {
                return self.acquire_repository_operation_leases(&[repository_path]);
            }
            Err(error) => return Err(error),
        };
        let mut repository_paths = git
            .worktrees(&root)?
            .into_iter()
            .map(|worktree| worktree.path)
            .collect::<Vec<_>>();
        repository_paths.push(root.to_string_lossy().into_owned());
        let repository_path_refs = repository_paths
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.acquire_repository_operation_leases(&repository_path_refs)
    }

    /// Atomically acquire exclusive access to multiple canonical worktree
    /// paths. Linked worktrees share refs and may be removed by another
    /// worktree, so their keys must be acquired together rather than one at a
    /// time (which would leave a deadlock window).
    pub fn acquire_repository_operation_leases(
        &self,
        repository_paths: &[&str],
    ) -> Result<RepositoryOperationLease, AppError> {
        let mut repository_paths: Vec<String> = repository_paths
            .iter()
            .map(|repository_path| (*repository_path).to_owned())
            .collect();
        repository_paths.sort_unstable();
        repository_paths.dedup();
        if repository_paths.is_empty() || repository_paths.iter().any(|path| path.is_empty()) {
            return Err(AppError::new(
                "repository_operation_path_required",
                "仓库路径不能为空。",
            ));
        }
        let mut active = self
            .active_repository_operations
            .lock()
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))?;
        if repository_paths.iter().any(|path| active.contains(path)) {
            return Err(AppError::new(
                "repository_operation_in_progress",
                "该仓库正在执行 Git 操作，请稍后重试。",
            ));
        }
        active.extend(repository_paths.iter().cloned());
        Ok(RepositoryOperationLease {
            active_repository_operations: Arc::clone(&self.active_repository_operations),
            repository_paths,
        })
    }

    pub(crate) fn retry_pending_secret_deletions_in_background(&self) {
        let state = self.clone();
        tauri::async_runtime::spawn_blocking(move || state.retry_pending_secret_deletions());
    }

    fn retry_pending_secret_deletions(&self) {
        let Ok(pending) = self.snapshot().map(|store| store.pending_secret_deletions) else {
            return;
        };
        for keychain_ref in pending {
            if delete_personal_access_token(&keychain_ref).is_ok() {
                let _ = self.mutate(|store| {
                    store.clear_pending_secret_deletion(&keychain_ref);
                    Ok(())
                });
            }
        }
    }
}

fn acquire_profile_store_lock(app_data_dir: &Path) -> Result<fs::File, AppError> {
    fs::create_dir_all(app_data_dir).map_err(|error| AppError::io("创建应用数据目录", error))?;
    restrict_profile_store_directory_permissions(app_data_dir)?;
    let lock_path = app_data_dir.join("profiles.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| AppError::io("打开账号配置锁", error))?;
    lock.try_lock_exclusive().map_err(|_| {
        AppError::new(
            "profile_store_in_use",
            "另一个 BroKnowMyGitDesktop 进程正在使用账号配置；请先关闭另一个实例。",
        )
    })?;
    Ok(lock)
}

#[cfg(unix)]
fn restrict_profile_store_directory_permissions(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| AppError::io("保护应用数据目录", error))
}

#[cfg(not(unix))]
fn restrict_profile_store_directory_permissions(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

fn cli_account_import_key(endpoint_id: &str, auth_method: &AuthenticationMethod) -> Option<String> {
    let method = match auth_method {
        AuthenticationMethod::GhCli => "gh",
        AuthenticationMethod::GlabCli => "glab",
        AuthenticationMethod::PersonalAccessToken
        | AuthenticationMethod::SshAgent
        | AuthenticationMethod::SystemGitCredentialHelper => return None,
    };
    Some(format!("{endpoint_id}\0{method}"))
}

fn empty_profile_store() -> ProfileStore {
    ProfileStore {
        schema_version: default_schema_version(),
        ..ProfileStore::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccountSelection, AuthenticationMethod, AuthenticationStatus};

    fn repository(path: &str, name: &str) -> RepositoryRecord {
        RepositoryRecord {
            path: path.into(),
            name: name.into(),
            alias: None,
            group: None,
            host: None,
            namespace: None,
            last_opened_at: None,
        }
    }

    #[test]
    fn serialized_profile_metadata_has_no_pat_field() {
        let store = ProfileStore {
            schema_version: 1,
            accounts: vec![AccountProfile {
                id: "account-1".into(),
                endpoint_id: "gitee.com".into(),
                login: "alice".into(),
                display_name: None,
                auth_method: AuthenticationMethod::PersonalAccessToken,
                secret_ref: None,
                status: AuthenticationStatus::Available,
                is_default_candidate: false,
            }],
            ..ProfileStore::default()
        };
        let json = serde_json::to_string(&store).expect("metadata serializes");
        assert!(!json.contains("token"));
        assert!(!json.contains("password"));
    }

    #[test]
    fn older_repository_records_load_without_grouping_fields() {
        let record: RepositoryRecord =
            serde_json::from_str(r#"{"path":"/work/project","name":"project"}"#)
                .expect("older persisted records remain compatible");

        assert_eq!(record.group, None);
        assert_eq!(record.alias, None);
        assert_eq!(record.host, None);
        assert_eq!(record.namespace, None);
        assert_eq!(record.last_opened_at, None);
    }

    #[test]
    fn invalid_profile_store_is_backed_up_and_recovers_to_empty_state() {
        let directory = std::env::temp_dir().join(format!("bkm-profile-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(STORE_FILE_NAME), b"{ invalid json").unwrap();

        let state = AppState::load(&directory).expect("invalid metadata must not prevent launch");
        assert!(state.snapshot().unwrap().accounts.is_empty());
        assert!(!directory.join(STORE_FILE_NAME).exists());
        let backups = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("profiles.invalid.")
            })
            .count();
        assert_eq!(backups, 1);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clone_lease_prevents_two_operations_from_sharing_one_destination_intent() {
        let directory = std::env::temp_dir().join(format!("bkm-clone-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_clone_lease("/work/clone-target")
            .expect("first clone owns its destination");

        let error = match state.acquire_clone_lease("/work/clone-target") {
            Ok(_) => panic!("a concurrent clone must not overwrite the pending intent"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "clone_in_progress");

        drop(lease);
        assert!(state.acquire_clone_lease("/work/clone-target").is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn repository_operation_lease_serializes_conflicting_git_operations() {
        let directory =
            std::env::temp_dir().join(format!("bkm-repository-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_repository_operation_leases(&["/work/project", "/work/project-linked"])
            .expect("first operation owns both linked worktrees");

        let error = match state.acquire_repository_operation_lease("/work/project") {
            Ok(_) => panic!("conflicting Git operations must not overlap"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "repository_operation_in_progress");
        let linked_error = match state.acquire_repository_operation_lease("/work/project-linked") {
            Ok(_) => panic!("a linked worktree must be covered by the same lease"),
            Err(error) => error,
        };
        assert_eq!(linked_error.code(), "repository_operation_in_progress");

        drop(lease);
        assert!(state
            .acquire_repository_operation_leases(&["/work/project", "/work/project-linked"])
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn account_operation_lease_blocks_concurrent_account_removal() {
        let directory = std::env::temp_dir().join(format!("bkm-account-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_account_operation_lease("account-1")
            .expect("a publish operation owns its selected account");

        let error = match state.acquire_account_operation_lease("account-1") {
            Ok(_) => panic!("account removal must not overlap a publish operation"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");

        drop(lease);
        assert!(state.acquire_account_operation_lease("account-1").is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn account_operation_leases_acquire_all_profiles_or_none() {
        let directory = std::env::temp_dir().join(format!("bkm-account-leases-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let existing = state
            .acquire_account_operation_lease("account-2")
            .expect("another operation owns the second profile");

        let error = match state.acquire_account_operation_leases(&["account-1", "account-2"]) {
            Ok(_) => panic!("a partially conflicting account group must not be acquired"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");
        // The failed group acquisition must not leave the first profile
        // reserved, otherwise a later import could spuriously block removal.
        assert!(state.acquire_account_operation_lease("account-1").is_ok());

        drop(existing);
        let group = state
            .acquire_account_operation_leases(&["account-1", "account-2"])
            .expect("both profiles become available together");
        let error = match state.acquire_account_operation_lease("account-2") {
            Ok(_) => panic!("a group lease protects every account"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_operation_in_progress");

        drop(group);
        assert!(state
            .acquire_account_operation_leases(&["account-1", "account-2"])
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cli_account_import_lease_serializes_matching_endpoint_profiles() {
        let directory =
            std::env::temp_dir().join(format!("bkm-cli-import-lease-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let lease = state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
            .expect("CLI import lease is acquired")
            .expect("GitHub CLI has an import lease");

        let error = match state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
        {
            Ok(_) => panic!("matching CLI import cannot overlap"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "account_import_in_progress");
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GlabCli)
            .is_ok());
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::SshAgent)
            .expect("SSH does not use a CLI import lease")
            .is_none());

        drop(lease);
        assert!(state
            .acquire_cli_account_import_lease("github.com", &AuthenticationMethod::GhCli)
            .is_ok());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_account_status_refresh_is_not_committed() {
        let directory =
            std::env::temp_dir().join(format!("bkm-account-refresh-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let stale_generation = state
            .begin_account_status_refresh()
            .expect("first refresh generation is issued");
        state
            .mutate_and_invalidate_account_status_refreshes(|_| Ok(()))
            .expect("an account or Endpoint update invalidates the old refresh atomically");

        let committed = state
            .commit_account_status_refresh(stale_generation, |_| -> Result<(), AppError> {
                panic!("a stale refresh must not mutate the profile store")
            })
            .expect("stale generation is ignored safely");
        assert!(committed.is_none());

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clean_untracked_confirmation_is_repository_scoped_and_one_shot() {
        let directory =
            std::env::temp_dir().join(format!("bkm-clean-confirmation-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        GitCore::new()
            .init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::create_dir_all(repository.join("generated"))
            .expect("untracked directory is created");
        std::fs::write(repository.join("generated/output.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprints = GitCore::new()
            .capture_clean_untracked_path_fingerprints(
                &repository,
                &["generated/output.txt".into()],
            )
            .expect("preview paths are captured");
        let token = state
            .issue_clean_untracked_confirmation(&repository_path, fingerprints)
            .expect("preview issues a token");

        assert_eq!(
            state
                .take_clean_untracked_confirmation(&repository_path, &token)
                .expect("matching token consumes its preview")
                .into_iter()
                .map(|fingerprint| fingerprint.path)
                .collect::<Vec<_>>(),
            vec!["generated/output.txt"]
        );
        assert_eq!(
            state
                .take_clean_untracked_confirmation(&repository_path, &token)
                .expect_err("a consumed preview cannot be replayed")
                .code(),
            "clean_untracked_confirmation_required"
        );

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_is_repository_scoped_and_one_shot() {
        let directory =
            std::env::temp_dir().join(format!("bkm-discard-confirmation-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let other_repository = directory.join("other-repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        git.init_repository(&other_repository, None)
            .expect("other repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let other_repository_path = other_repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let token = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("discard preview issues a token");

        assert_eq!(
            state
                .take_discard_changes_confirmation(&other_repository_path, &token)
                .expect_err("a token cannot authorize another repository")
                .code(),
            "discard_confirmation_required"
        );
        let fingerprint = state
            .take_discard_changes_confirmation(&repository_path, &token)
            .expect("the matching repository consumes its confirmation");
        assert_eq!(fingerprint.path_count(), 1);
        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &token)
                .expect_err("a consumed confirmation cannot be replayed")
                .code(),
            "discard_confirmation_required"
        );

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_expires_and_is_removed() {
        let directory = std::env::temp_dir().join(format!(
            "bkm-discard-confirmation-expiry-{}",
            Uuid::new_v4()
        ));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let token = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("discard preview issues a token");
        {
            let mut confirmations = state
                .discard_changes_confirmations
                .lock()
                .expect("confirmation state is available");
            confirmations
                .get_mut(&repository_path)
                .expect("discard confirmation exists")
                .issued_at =
                Instant::now() - DISCARD_CHANGES_CONFIRMATION_TTL - Duration::from_secs(1);
        }

        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &token)
                .expect_err("an expired confirmation cannot be consumed")
                .code(),
            "discard_confirmation_expired"
        );
        assert!(!state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available")
            .contains_key(&repository_path));

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn newer_discard_preview_supersedes_the_previous_token() {
        let directory = std::env::temp_dir().join(format!(
            "bkm-discard-confirmation-replace-{}",
            Uuid::new_v4()
        ));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let repository_path = repository.to_string_lossy().into_owned();
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");
        let first = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint.clone())
            .expect("first preview issues a token");
        let second = state
            .issue_discard_changes_confirmation(&repository_path, fingerprint)
            .expect("second preview replaces the token");

        assert_ne!(first, second);
        assert_eq!(
            state
                .take_discard_changes_confirmation(&repository_path, &first)
                .expect_err("the older token is invalidated")
                .code(),
            "discard_confirmation_required"
        );
        state
            .take_discard_changes_confirmation(&repository_path, &second)
            .expect("the newest token remains valid");

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn discard_confirmation_store_evicts_the_oldest_repository() {
        let directory =
            std::env::temp_dir().join(format!("bkm-discard-confirmation-bound-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let repository = directory.join("repository");
        let git = GitCore::new();
        git.init_repository(&repository, None)
            .expect("test repository initializes");
        std::fs::write(repository.join("untracked.txt"), "previewed\n")
            .expect("untracked file is created");
        let fingerprint = git
            .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
            .expect("discard state is captured");

        for index in 0..MAX_DISCARD_CHANGES_CONFIRMATIONS {
            state
                .issue_discard_changes_confirmation(
                    &format!("repository-{index}"),
                    fingerprint.clone(),
                )
                .expect("bounded confirmation is issued");
        }
        state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available")
            .get_mut("repository-0")
            .expect("oldest confirmation exists")
            .issued_at = Instant::now() - Duration::from_secs(1);
        state
            .issue_discard_changes_confirmation(
                &format!("repository-{}", MAX_DISCARD_CHANGES_CONFIRMATIONS),
                fingerprint,
            )
            .expect("a newer repository evicts the oldest confirmation");
        let confirmations = state
            .discard_changes_confirmations
            .lock()
            .expect("confirmation state is available");
        assert_eq!(confirmations.len(), MAX_DISCARD_CHANGES_CONFIRMATIONS);
        assert!(!confirmations.contains_key("repository-0"));
        assert!(confirmations
            .contains_key(&format!("repository-{}", MAX_DISCARD_CHANGES_CONFIRMATIONS)));

        drop(confirmations);
        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn issuing_clean_confirmation_prunes_expired_previews_for_other_repositories() {
        let directory =
            std::env::temp_dir().join(format!("bkm-clean-confirmation-prune-{}", Uuid::new_v4()));
        let state = AppState::load(&directory).expect("profile store loads");
        let stale_repository = directory.join("stale");
        let current_repository = directory.join("current");
        let git = GitCore::new();
        git.init_repository(&stale_repository, None)
            .expect("stale test repository initializes");
        git.init_repository(&current_repository, None)
            .expect("current test repository initializes");
        std::fs::write(stale_repository.join("old.txt"), "old\n")
            .expect("stale untracked file is created");
        std::fs::write(current_repository.join("current.txt"), "current\n")
            .expect("current untracked file is created");
        let stale_path = stale_repository.to_string_lossy().into_owned();
        let current_path = current_repository.to_string_lossy().into_owned();
        let stale_fingerprints = git
            .capture_clean_untracked_path_fingerprints(&stale_repository, &["old.txt".into()])
            .expect("stale preview paths are captured");
        state
            .issue_clean_untracked_confirmation(&stale_path, stale_fingerprints)
            .expect("stale preview is issued");
        {
            let mut confirmations = state
                .clean_untracked_confirmations
                .lock()
                .expect("confirmation state is available");
            confirmations
                .get_mut(&stale_path)
                .expect("stale preview exists")
                .issued_at =
                Instant::now() - CLEAN_UNTRACKED_CONFIRMATION_TTL - Duration::from_secs(1);
        }

        let current_fingerprints = git
            .capture_clean_untracked_path_fingerprints(&current_repository, &["current.txt".into()])
            .expect("current preview paths are captured");
        state
            .issue_clean_untracked_confirmation(&current_path, current_fingerprints)
            .expect("current preview is issued");
        let confirmations = state
            .clean_untracked_confirmations
            .lock()
            .expect("confirmation state is available");
        assert!(!confirmations.contains_key(&stale_path));
        assert!(confirmations.contains_key(&current_path));

        drop(confirmations);
        drop(state);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn newer_profile_store_version_is_preserved_and_blocks_startup() {
        let directory =
            std::env::temp_dir().join(format!("bkm-profile-version-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(STORE_FILE_NAME),
            br#"{"schemaVersion":5,"futureField":{"preserve":true}}"#,
        )
        .unwrap();

        let error = match AppState::load(&directory) {
            Ok(_) => panic!("newer metadata must not be overwritten"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "profile_store_version_unsupported");
        assert!(directory.join(STORE_FILE_NAME).exists());
        assert!(std::fs::read_to_string(directory.join(STORE_FILE_NAME))
            .unwrap()
            .contains("futureField"));
        let backups = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("profiles.invalid.")
            })
            .count();
        assert_eq!(backups, 0);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn saved_profile_store_is_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!("bkm-profile-mode-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        {
            let state = AppState::load(&directory).expect("profile store loads");
            state
                .mutate(|store| {
                    store.upsert_repository(repository("/work/private", "private"));
                    Ok(())
                })
                .expect("profile store saves");
        }

        let permissions = std::fs::metadata(directory.join(STORE_FILE_NAME))
            .expect("saved store exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(permissions, 0o600);

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn repository_upsert_retains_local_display_metadata() {
        let mut store = ProfileStore::default();
        let mut first = repository("/work/project", "project");
        first.alias = Some("项目主仓".into());
        first.group = Some("工作".into());
        first.host = Some("github.com".into());
        first.namespace = Some("example-org".into());
        first.last_opened_at = Some(123);
        store.upsert_repository(first);

        let updated = store.upsert_repository(repository("/work/project", "renamed-project"));

        assert_eq!(updated.name, "renamed-project");
        assert_eq!(updated.alias.as_deref(), Some("项目主仓"));
        assert_eq!(updated.group.as_deref(), Some("工作"));
        assert_eq!(updated.host.as_deref(), Some("github.com"));
        assert_eq!(updated.namespace.as_deref(), Some("example-org"));
        assert_eq!(updated.last_opened_at, Some(123));
    }

    #[test]
    fn touching_repository_updates_usage_without_losing_manual_group() {
        let mut store = ProfileStore::default();
        let mut record = repository("/work/project", "project");
        record.alias = Some("客户端主仓".into());
        record.group = Some("客户端".into());
        store.upsert_repository(record);

        let touched = store.touch_repository(
            repository("/work/project", "project"),
            Some("gitlab.com".into()),
            Some("team/platform".into()),
            456,
        );

        assert_eq!(touched.alias.as_deref(), Some("客户端主仓"));
        assert_eq!(touched.group.as_deref(), Some("客户端"));
        assert_eq!(touched.host.as_deref(), Some("gitlab.com"));
        assert_eq!(touched.namespace.as_deref(), Some("team/platform"));
        assert_eq!(touched.last_opened_at, Some(456));
    }

    #[test]
    fn touching_repository_promotes_it_before_later_discoveries() {
        let mut store = ProfileStore::default();
        store.upsert_repository(repository("/work/older", "older"));
        store.upsert_repository(repository("/work/newer", "newer"));

        store.touch_repository(repository("/work/older", "older"), None, None, 456);

        assert_eq!(store.recent_repositories[0].path, "/work/older");
        assert_eq!(store.recent_repositories[1].path, "/work/newer");
    }

    #[test]
    fn forgetting_a_worktree_path_clears_all_path_scoped_metadata() {
        let path = "/work/linked-worktree";
        let mut store = ProfileStore {
            recent_repositories: vec![repository(path, "linked-worktree")],
            remote_bindings: vec![RemoteBinding {
                repository_path: path.into(),
                remote_name: "origin".into(),
                endpoint_id: "github.com".into(),
                account_selection: AccountSelection::Auto,
            }],
            pending_publishes: vec![PendingPublish {
                repository_path: path.into(),
                endpoint_id: "github.com".into(),
                account_profile_id: "account-1".into(),
                remote_name: "origin".into(),
                expected_head: "0123456789012345678901234567890123456789".into(),
                expected_repository_slug: "octo/project".into(),
            }],
            ..ProfileStore::default()
        };

        store.forget_repository_path(path);

        assert!(store.recent_repositories.is_empty());
        assert!(store.remote_bindings.is_empty());
        assert!(store.pending_publishes.is_empty());
    }

    #[test]
    fn removing_an_account_clears_its_pending_clone_binding() {
        let mut store = ProfileStore {
            accounts: vec![AccountProfile {
                id: "account-1".into(),
                endpoint_id: "github.com".into(),
                login: "octo".into(),
                display_name: None,
                auth_method: AuthenticationMethod::GhCli,
                secret_ref: None,
                status: AuthenticationStatus::Valid,
                is_default_candidate: false,
            }],
            pending_clones: vec![PendingClone {
                destination_path: "/work/pending-clone".into(),
                binding: Some(RemoteBinding {
                    repository_path: "/work/pending-clone".into(),
                    remote_name: "origin".into(),
                    endpoint_id: "github.com".into(),
                    account_selection: AccountSelection::Profile("account-1".into()),
                }),
                expected_remote_authority: "github.com".into(),
                expected_repository_slug: "octo/pending-clone".into(),
            }],
            ..ProfileStore::default()
        };

        assert!(store.remove_account("account-1").is_some());
        assert!(store.pending_clones.is_empty());
    }
}
