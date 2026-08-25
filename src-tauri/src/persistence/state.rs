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

    pub(crate) fn preferred_terminal(&self) -> Result<Option<TerminalConfig>, AppError> {
        self.profiles
            .lock()
            .map(|store| store.data().preferred_terminal.clone())
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))
    }

    pub(crate) fn repository_network_trust(
        &self,
        repository_path: &str,
    ) -> Result<Option<RepositoryNetworkTrust>, AppError> {
        self.profiles
            .lock()
            .map(|store| store.data().repository_network_trust_for(repository_path))
            .map_err(|_| AppError::new("state_poisoned", "应用账号状态不可用，请重启应用。"))
    }

    /// Copies only the Endpoint and binding metadata required to enrich one
    /// repository's remotes. Context-menu reads must not clone accounts,
    /// recent repositories, or pending-operation state along with it.
    pub(crate) fn remote_endpoint_metadata(
        &self,
        repository_path: &str,
    ) -> Result<(Vec<CodeHostEndpoint>, Vec<RemoteBinding>), AppError> {
        self.profiles
            .lock()
            .map(|store| {
                let store = store.data();
                let endpoints = store.all_endpoints();
                let bindings = store
                    .remote_bindings
                    .iter()
                    .filter(|binding| binding.repository_path == repository_path)
                    .cloned()
                    .collect();
                (endpoints, bindings)
            })
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
