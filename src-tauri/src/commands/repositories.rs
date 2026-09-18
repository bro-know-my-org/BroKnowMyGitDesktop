use super::support::*;
use super::*;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;

const MAX_REPOSITORY_LIST_WORKERS: usize = 4;
static REPOSITORY_LIST_PROBE_POOL: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn repository_list_probe_pool() -> Arc<Semaphore> {
    Arc::clone(
        REPOSITORY_LIST_PROBE_POOL
            .get_or_init(|| Arc::new(Semaphore::new(MAX_REPOSITORY_LIST_WORKERS))),
    )
}

fn repository_records(
    store: &ProfileStore,
    refresh_owners: bool,
) -> Result<Vec<RepositoryRecord>, AppError> {
    if store.recent_repositories.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = store
        .recent_repositories
        .len()
        .min(MAX_REPOSITORY_LIST_WORKERS);
    let chunk_size = store.recent_repositories.len().div_ceil(worker_count);
    let worker_results = std::thread::scope(|scope| {
        let workers = store
            .recent_repositories
            .chunks(chunk_size)
            .map(|repositories| {
                scope.spawn(move || {
                    let git = GitCore::new();
                    repositories
                        .iter()
                        .filter_map(|repository| {
                            let mut record = git
                                .repository_record(Path::new(&repository.path))
                                .ok()
                                .map(|record| store.repository_with_metadata(record))?;
                            if refresh_owners {
                                // This only reads local Git configuration. It makes owner
                                // grouping useful immediately for existing repositories
                                // without treating a list refresh as a repository open or
                                // changing recent ordering.
                                if let Ok(remotes) = git.remotes(Path::new(&record.path)) {
                                    let (host, namespace) = repository_owner_from_remotes(&remotes);
                                    record.host = host;
                                    record.namespace = namespace;
                                }
                            }
                            Some(record)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| {
                worker.join().map_err(|_| {
                    AppError::new("repository_list_failed", "仓库列表后台任务未能完成。")
                })
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let mut repositories = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();
    for records in worker_results {
        for record in records {
            if seen_paths.insert(record.path.clone()) {
                repositories.push(record);
            }
        }
    }
    repositories.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    Ok(repositories)
}

#[tauri::command]
pub async fn list_repositories(
    refresh_owners: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<RepositoryRecord>, AppError> {
    let state = state.inner().clone();
    // Admission waits on the async runtime, before a blocking worker exists.
    // Taking all four slots atomically keeps the catalog's internal parallel
    // probes globally bounded without deadlocking overlapping requests.
    let probe_permits = repository_list_probe_pool()
        .acquire_many_owned(MAX_REPOSITORY_LIST_WORKERS as u32)
        .await
        .map_err(|_| AppError::new("repository_list_failed", "仓库列表任务调度器不可用。"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _probe_permits = probe_permits;
        let store = state.snapshot()?;
        // Validating saved repositories requires one bounded Git process per
        // path. Run a small fixed number in parallel so a normal catalog does
        // not turn process startup latency into a fully serial startup cost.
        repository_records(&store, refresh_owners.unwrap_or(false))
    })
    .await
    .map_err(|_| AppError::new("repository_list_failed", "仓库列表后台任务未能完成。"))?
}

#[tauri::command]
pub async fn discover_repositories(
    root_path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryDiscoveryResult, AppError> {
    let state = state.inner().clone();
    let root_path = root_path.trim().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let repositories =
            GitCore::new().discover_repositories(Path::new(&root_path), DISCOVERY_MAX_DEPTH)?;
        let import_limit =
            MAX_REPOSITORIES.saturating_sub(state.snapshot()?.recent_repositories.len());
        Ok(RepositoryDiscoveryResult {
            repositories,
            import_limit,
        })
    })
    .await
    .map_err(|_| AppError::new("repository_discovery_failed", "仓库发现任务未能完成。"))?
}

#[derive(Debug)]
pub(super) struct RepositoryPathValidationResult {
    pub repositories: Vec<RepositoryRecord>,
    pub rejected_count: usize,
}

pub(super) fn repository_records_for_paths(
    repository_paths: Vec<String>,
) -> Result<RepositoryPathValidationResult, AppError> {
    if repository_paths.len() > MAX_REPOSITORIES {
        return Err(AppError::new(
            "repository_import_limit_exceeded",
            "一次添加的仓库数量过多，请分批重试。",
        ));
    }
    let git = GitCore::new();
    let mut seen_inputs = std::collections::HashSet::new();
    let mut seen_paths = std::collections::HashSet::new();
    let mut repositories = Vec::new();
    let mut rejected_count = 0;
    for repository_path in repository_paths {
        let repository_path = repository_path.trim();
        if !seen_inputs.insert(repository_path.to_owned()) {
            rejected_count += 1;
            continue;
        }
        let Ok(path) = required_path(repository_path, "仓库路径") else {
            rejected_count += 1;
            continue;
        };
        let repository = match git.repository_record(&path) {
            Ok(repository) => repository,
            Err(error) if error.code() == "not_a_git_repository" => {
                rejected_count += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if seen_paths.insert(repository.path.clone()) {
            repositories.push(repository);
        } else {
            rejected_count += 1;
        }
    }
    Ok(RepositoryPathValidationResult {
        repositories,
        rejected_count,
    })
}

#[tauri::command]
pub async fn add_discovered_repositories(
    repository_paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryImportResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_discovered_repositories(repository_paths, &state)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "添加仓库后台任务未能完成。"))?
}

pub(super) fn import_discovered_repositories(
    repository_paths: Vec<String>,
    state: &AppState,
) -> Result<RepositoryImportResult, AppError> {
    let initial_selection = repository_records_for_paths(repository_paths)?;
    let selected_paths = initial_selection
        .repositories
        .iter()
        .map(|repository| repository.path.clone())
        .collect::<Vec<_>>();
    let git = GitCore::new();
    let mut lease_paths = Vec::new();
    for repository in &initial_selection.repositories {
        lease_paths.push(repository.path.clone());
        lease_paths.extend(
            git.worktrees(Path::new(&repository.path))?
                .into_iter()
                .map(|worktree| worktree.path),
        );
    }
    let lease_path_refs = lease_paths.iter().map(String::as_str).collect::<Vec<_>>();
    let _repository_operation_lease = if lease_path_refs.is_empty() {
        None
    } else {
        Some(state.acquire_repository_operation_leases(&lease_path_refs)?)
    };
    let selection = repository_records_for_paths(selected_paths)?;
    let selected_count = selection.repositories.len();
    let (repositories, remaining_capacity, persistence_warning) =
        remember_repositories(state, selection.repositories)?;
    Ok(RepositoryImportResult {
        rejected_count: initial_selection.rejected_count
            + selection.rejected_count
            + selected_count
            - repositories.len(),
        repositories,
        remaining_capacity,
        persistence_warning,
    })
}

#[tauri::command]
pub async fn init_repository(
    destination_path: String,
    initial_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = required_path(&destination_path, "新仓库目录")?;
        let initial_branch = initial_branch.and_then(non_empty_trimmed);
        // `git init .` is a valid existing-directory workflow. The clone
        // lease helpers intentionally require a normal final path component,
        // so derive their canonical keys from the current directory while
        // preserving `.` for GitCore's own init semantics.
        let lease_destination = if destination == Path::new(".") {
            std::fs::canonicalize(&destination)
                .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录。"))?
        } else {
            destination.clone()
        };
        let destination_key = clone_destination_key(&lease_destination)?;
        let clone_parent_key = clone_parent_lease_key(&lease_destination)?;
        let _clone_lease = state.acquire_clone_lease(&clone_parent_key)?;
        // Once Git publishes the worktree, commands addressed to the new path
        // must not race the final recent-repository metadata write below.
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&destination_key)?;
        let repository = GitCore::new().init_repository(&destination, initial_branch.as_deref())?;
        remember_repository(&state, repository.clone())?;
        Ok(repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "初始化仓库后台任务未能完成。"))?
}

#[tauri::command]
pub async fn clone_repository(
    url: String,
    destination_path: String,
    endpoint_id: Option<String>,
    account_profile_id: Option<String>,
    options: Option<CloneOptions>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = required_path(&destination_path, "克隆目标目录")?;
        // Validate before recording a recovery intent or attempting account
        // resolution. `GitCore` validates again immediately before invoking
        // Git, but the unauthenticated clone path must retain this boundary.
        let url = validate_git_remote_url(&url)?;
        let destination_key = clone_destination_key(&destination)?;
        let clone_parent_key = clone_parent_lease_key(&destination)?;
        let _clone_lease = state.acquire_clone_lease(&clone_parent_key)?;
        // Once clone publishes its destination, remote commands addressed to
        // that repository must not race the final metadata/binding write.
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&destination_key)?;
        // Select an account first, acquire its lease, then resolve again with
        // that account pinned. This prevents account deletion from racing the
        // askpass capability and Git process while retaining an Auto binding
        // for future operations when the caller did not select a profile.
        let selected_account_profile_id = resolve_clone_credential_target(
            &state,
            &url,
            &destination,
            endpoint_id.as_deref(),
            account_profile_id.as_deref(),
            None,
        )?
        .map(|target| target.account_profile_id().to_owned());
        let _account_operation_lease = selected_account_profile_id
            .as_deref()
            .map(|account_profile_id| state.acquire_account_operation_lease(account_profile_id))
            .transpose()?;
        let clone_target = match selected_account_profile_id.as_deref() {
            Some(selected_account_profile_id) => resolve_clone_credential_target(
                &state,
                &url,
                &destination,
                endpoint_id.as_deref(),
                account_profile_id.as_deref(),
                Some(selected_account_profile_id),
            )?,
            None => None,
        };
        let (environment, selected_binding) = match clone_target {
            Some(target) => (Some(target.environment()?), Some(target.binding)),
            None => (None, None),
        };
        let (expected_remote_authority, expected_repository_slug) = selected_binding
            .as_ref()
            .map(|_| clone_expected_remote_identity(&url))
            .transpose()?
            .unwrap_or_default();
        // Persist a non-secret recovery intent before Git creates the destination.
        // A post-clone profile write can fail independently from a successful
        // clone, and retrying Clone would otherwise fail on the non-empty folder.
        let pending_clone = PendingClone {
            destination_path: destination_key,
            binding: selected_binding.clone(),
            expected_remote_authority,
            expected_repository_slug,
        };
        state.mutate(|store| {
            store.queue_pending_clone(pending_clone.clone());
            Ok(())
        })?;
        let repository = match GitCore::new().clone_repository_with_environment(
            &url,
            &destination,
            &options.unwrap_or_default(),
            environment.as_ref().unwrap_or(&GitEnvironment::default()),
        ) {
            Ok(repository) => repository,
            Err(error) => {
                // A stale intent has no credential value and is validated again
                // before reconciliation, but remove it promptly when possible.
                let _ = state.mutate(|store| {
                    store.clear_pending_clone(&pending_clone.destination_path);
                    Ok(())
                });
                return Err(error);
            }
        };
        match state.mutate_with_outcome(|store| {
            store.upsert_repository(repository.clone());
            if let Some(mut binding) = selected_binding {
                binding.repository_path = repository.path.clone();
                store.upsert_binding(binding);
            }
            store.clear_pending_clone(&pending_clone.destination_path);
            Ok(())
        }) {
            Ok(()) => Ok(repository),
            Err(error) if error.committed() => Ok(repository),
            Err(_) => {
                // A pre-replace failure leaves the pending intent durable.
                // Reconcile immediately so a successful clone is either
                // visible in Recents now or reports a recoverable metadata
                // failure instead of silently disappearing until a later
                // snapshot happens to open this path.
                reconcile_pending_clone(&state, &destination)?;
                Ok(repository)
            }
        }
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 克隆后台任务未能完成。"))?
}

#[tauri::command]
pub async fn get_repository_snapshot(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        // Clone reconciliation can restore a durable remote binding, so it
        // must not race a concurrent remote edit that could otherwise leave
        // that stale binding behind after the URL has changed.
        {
            let _repository_operation_lease =
                state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
            reconcile_pending_clone(&state, &repository)?;
        }
        let mut snapshot = snapshot_for(&state, &repository)?;
        let (host, namespace) = repository_owner_from_remotes(&snapshot.remotes);
        snapshot.repository = state.mutate(|store| {
            Ok(store.touch_repository(
                snapshot.repository.clone(),
                host,
                namespace,
                current_time_millis(),
            ))
        })?;
        Ok(snapshot)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库快照后台任务未能完成。"))?
}

/// Refreshes an already-open repository without recording another recent-open
/// timestamp. Automatic refreshes can run often, so they must remain a read-only
/// operation over application metadata.
#[tauri::command]
pub async fn refresh_repository_snapshot(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        snapshot_for(&state, &repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库刷新后台任务未能完成。"))?
}

#[tauri::command]
pub async fn set_repository_group(
    repository_path: String,
    group: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let repository = GitCore::new().repository_record(&repository)?;
        let group = normalize_repository_group(group)?;
        state.mutate(|store| Ok(store.set_repository_group(repository, group)))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库分组后台任务未能完成。"))?
}

#[tauri::command]
pub async fn set_repository_alias(
    repository_path: String,
    alias: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let repository = GitCore::new().repository_record(&repository)?;
        let alias = normalize_repository_alias(alias)?;
        state.mutate(|store| Ok(store.set_repository_alias(repository, alias)))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库别名后台任务未能完成。"))?
}
