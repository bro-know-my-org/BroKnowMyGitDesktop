use crate::app_error::AppError;
use crate::codehost::{
    AdapterContext, CodeHostAdapter, CodeHostRepositoryInfo, CreatePullRequestInput,
    PullRequestSummary,
};
use crate::command_models::{GitOperationState, NetworkOperationResult, RepositorySnapshot};
use crate::credential_broker::{
    default_pat_keychain_reference, delete_personal_access_token, gh_auth_status, glab_auth_status,
    import_gh_current_account_new, import_glab_current_account_new,
    personal_access_token_reference, personal_access_token_status, save_personal_access_token,
    validate_remote_url_for_credentials, CredentialBroker, CredentialEnvironment,
    CredentialResolutionRequest,
};
use crate::domain::{
    endpoint_for_host, endpoint_matches_authority, git_authority_has_explicit_port,
    github_endpoint_has_gh_compatible_api, hostname_from_git_authority, normalize_git_authority,
    parse_remote_url, AccountProfile, AccountSelection, AuthenticationMethod, AuthenticationStatus,
    CodeHostCapabilities, CodeHostEndpoint, CodeHostKind, CommitSummary, ExternalEditorConfig,
    GitOperationKind, LfsStatus, RemoteBinding, RemoteProfile, RepositoryRecord, ResetMode,
    StashEntry, SubmoduleInfo, WorktreeInfo,
};
use crate::git_core::{
    normalize_external_editor_config, validate_git_remote_url, CloneOptions, DiffOptions,
    FetchOptions, GitCore, GitEnvironment, PullOptions, PushOptions, RemoteUrlKind,
};
use crate::github_adapter::GitHubAdapter;
use crate::persistence::{AppState, PendingClone, PendingPublish, ProfileStore};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;
use uuid::Uuid;

const DISCOVERY_MAX_DEPTH: usize = 4;
const SSH_AGENT_STATUS_TIMEOUT: Duration = Duration::from_secs(2);
const BRANCH_SWITCH_STASH_MESSAGE: &str = "切换分支前自动暂存";

#[tauri::command]
pub fn list_repositories(
    refresh_owners: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<RepositoryRecord>, AppError> {
    let git = GitCore::new();
    let store = state.snapshot()?;
    let mut repositories = Vec::new();
    for repository in &store.recent_repositories {
        if let Ok(record) = git.repository_record(Path::new(&repository.path)) {
            let mut record = store.repository_with_metadata(record);
            if refresh_owners.unwrap_or(false) {
                // This only reads local Git configuration. It makes owner
                // grouping useful immediately for existing repositories
                // without treating a list refresh as a repository open or
                // changing recent ordering. Keep it opt-in because a large
                // repository catalog should not slow the normal app startup.
                if let Ok(remotes) = git.remotes(Path::new(&record.path)) {
                    let (host, namespace) = repository_owner_from_remotes(&remotes);
                    record.host = host;
                    record.namespace = namespace;
                }
            }
            if !repositories
                .iter()
                .any(|existing: &RepositoryRecord| existing.path == record.path)
            {
                repositories.push(record);
            }
        }
    }
    repositories.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    Ok(repositories)
}

#[tauri::command]
pub fn discover_repositories(
    root_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<RepositoryRecord>, AppError> {
    let repositories =
        GitCore::new().discover_repositories(Path::new(root_path.trim()), DISCOVERY_MAX_DEPTH)?;
    remember_repositories(&state, &repositories)?;
    Ok(repositories)
}

#[tauri::command]
pub fn init_repository(
    destination_path: String,
    initial_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let destination = required_path(&destination_path, "新仓库目录")?;
    let initial_branch = initial_branch.and_then(non_empty_trimmed);
    let destination_key = clone_destination_key(&destination);
    let _clone_lease = state.acquire_clone_lease(&destination_key)?;
    let repository = GitCore::new().init_repository(&destination, initial_branch.as_deref())?;
    remember_repository(&state, repository.clone())?;
    Ok(repository)
}

#[tauri::command]
pub async fn clone_repository(
    url: String,
    destination_path: String,
    endpoint_id: Option<String>,
    account_profile_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = required_path(&destination_path, "克隆目标目录")?;
        // Validate before recording a recovery intent or attempting account
        // resolution. `GitCore` validates again immediately before invoking
        // Git, but the unauthenticated clone path must retain this boundary.
        validate_git_remote_url(&url)?;
        let destination_key = clone_destination_key(&destination);
        let _clone_lease = state.acquire_clone_lease(&destination_key)?;
        let (environment, selected_binding) = clone_environment(
            &state,
            &url,
            &destination,
            endpoint_id.as_deref(),
            account_profile_id.as_deref(),
        )?;
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
            &CloneOptions::default(),
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
        let _ = state.mutate_with_outcome(|store| {
            store.upsert_repository(repository.clone());
            if let Some(mut binding) = selected_binding {
                binding.repository_path = repository.path.clone();
                store.upsert_binding(binding);
            }
            store.clear_pending_clone(&pending_clone.destination_path);
            Ok(())
        });
        // The clone itself succeeded. The pre-recorded intent is either still
        // durable (pre-replace failure) or has been cleared alongside the
        // metadata (post-replace failure), so return the usable repository and
        // let the next snapshot reconcile any remaining intent.
        Ok(repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 克隆后台任务未能完成。"))?
}

#[tauri::command]
pub fn get_repository_snapshot(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    // Clone reconciliation can restore a durable remote binding, so it must
    // not race a concurrent remote edit that could otherwise leave that stale
    // binding behind after the URL has changed.
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
}

#[tauri::command]
pub fn set_repository_group(
    repository_path: String,
    group: Option<String>,
    state: State<'_, AppState>,
) -> Result<RepositoryRecord, AppError> {
    let repository = repository_root(&repository_path)?;
    let repository = GitCore::new().repository_record(&repository)?;
    let group = normalize_repository_group(group)?;
    state.mutate(|store| Ok(store.set_repository_group(repository, group)))
}

#[tauri::command]
pub fn get_file_diff(
    repository_path: String,
    path: String,
    staged: bool,
) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    let paths = (!path.trim().is_empty()).then_some(path);
    GitCore::new().diff(
        &repository,
        &DiffOptions {
            staged,
            paths: paths.into_iter().collect(),
            context_lines: Some(3),
        },
    )
}

#[tauri::command]
pub fn get_file_blame(repository_path: String, path: String) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().blame(&repository, &path)
}

#[tauri::command]
pub fn list_reflog(repository_path: String, limit: Option<usize>) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().reflog(&repository, limit.unwrap_or(100))
}

#[tauri::command]
pub fn preview_clean_untracked(repository_path: String) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().preview_clean_untracked(&repository)
}

#[tauri::command]
pub fn clean_untracked(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().clean_untracked(&repository)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn get_external_editor(
    state: State<'_, AppState>,
) -> Result<Option<ExternalEditorConfig>, AppError> {
    Ok(state.snapshot()?.external_editor)
}

/// Saves an app-local external editor command. This does not read or modify
/// Git's editor or mergetool configuration, which may be scoped to an
/// untrusted repository.
#[tauri::command]
pub fn save_external_editor(
    editor: Option<ExternalEditorConfig>,
    state: State<'_, AppState>,
) -> Result<Option<ExternalEditorConfig>, AppError> {
    let editor = editor.map(normalize_external_editor_config).transpose()?;
    state.mutate(|store| {
        store.external_editor = editor.clone();
        Ok(editor)
    })
}

/// Starts the configured external editor with every currently unresolved
/// conflict file. The editor is not awaited; the user resolves files outside
/// the app, returns here, refreshes, stages, and then continues the operation.
#[tauri::command]
pub fn open_conflicted_files_in_external_editor(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let editor = state.snapshot()?.external_editor.ok_or_else(|| {
        AppError::new(
            "external_editor_not_configured",
            "请先配置用于解决冲突的外部编辑器。",
        )
    })?;
    GitCore::new().open_conflicted_files_in_external_editor(&repository, &editor)
}

#[tauri::command]
pub fn list_commits(
    repository_path: String,
    limit: Option<usize>,
) -> Result<Vec<CommitSummary>, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().commits(&repository, limit.unwrap_or(100))
}

#[tauri::command]
pub fn get_commit_diff(repository_path: String, revision: String) -> Result<String, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().commit_diff(&repository, &revision)
}

#[tauri::command]
pub fn stage_paths(
    repository_path: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stage_paths(&repository, &paths)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn unstage_paths(
    repository_path: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().unstage_paths(&repository, &paths)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn apply_patch(
    repository_path: String,
    patch: String,
    reverse: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().apply_patch(&repository, &patch, reverse)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn discard_changes(
    repository_path: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().discard_paths(&repository, &paths)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn commit_changes(
    repository_path: String,
    message: String,
    amend: Option<bool>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().commit(&repository, &message, amend.unwrap_or(false))?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn create_branch(
    repository_path: String,
    name: String,
    checkout: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    if checkout {
        git.create_and_switch_branch(&repository, &name, None)?;
    } else {
        git.create_branch(&repository, &name, None)?;
    }
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn checkout_branch(
    repository_path: String,
    branch: String,
    stash_local_changes: Option<bool>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    if !git
        .branches(&repository)?
        .iter()
        .any(|candidate| candidate.name == branch)
    {
        return Err(AppError::new("branch_not_found", "找不到该分支。"));
    }
    let stashed_local_changes = stash_local_changes_before_branch_switch(
        &git,
        &repository,
        stash_local_changes.unwrap_or(false),
    )?;
    if let Err(error) = git.switch_branch(&repository, &branch) {
        return Err(branch_switch_error_after_stash(
            error,
            stashed_local_changes,
        ));
    }
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn checkout_remote_branch(
    repository_path: String,
    remote_name: String,
    branch: String,
    stash_local_changes: Option<bool>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    if !git
        .remote_branches(&repository)?
        .iter()
        .any(|candidate| candidate.remote_name == remote_name && candidate.name == branch)
    {
        return Err(AppError::new(
            "remote_branch_not_found",
            "找不到该远程分支；请先获取远程更新后重试。",
        ));
    }
    let stashed_local_changes = stash_local_changes_before_branch_switch(
        &git,
        &repository,
        stash_local_changes.unwrap_or(false),
    )?;
    if let Err(error) = git.switch_remote_branch(&repository, &remote_name, &branch) {
        return Err(branch_switch_error_after_stash(
            error,
            stashed_local_changes,
        ));
    }
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn rename_branch(
    repository_path: String,
    old_name: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().rename_branch(&repository, &old_name, &new_name)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn delete_branch(
    repository_path: String,
    name: String,
    force: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().delete_branch(&repository, &name, force)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn merge_branch(
    repository_path: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().merge_branch(&repository, &branch)
}

#[tauri::command]
pub fn list_worktrees(repository_path: String) -> Result<Vec<WorktreeInfo>, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().worktrees(&repository)
}

#[tauri::command]
pub fn add_worktree(
    repository_path: String,
    worktree_path: String,
    create_branch: Option<String>,
    commitish: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<WorktreeInfo>, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let worktree_path = required_path(&worktree_path, "工作树路径")?;
    let create_branch = create_branch.and_then(non_empty_trimmed);
    let commitish = commitish.and_then(non_empty_trimmed);
    let git = GitCore::new();
    git.add_worktree(
        &repository,
        &worktree_path,
        create_branch.as_deref(),
        commitish.as_deref(),
    )?;
    remember_repository(&state, git.repository_record(&worktree_path)?)?;
    git.worktrees(&repository)
}

#[tauri::command]
pub fn remove_worktree(
    repository_path: String,
    worktree_path: String,
    force: bool,
    state: State<'_, AppState>,
) -> Result<Vec<WorktreeInfo>, AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy().into_owned();
    let worktree_path = required_path(&worktree_path, "工作树路径")?;
    let git = GitCore::new();
    // Resolve the canonical root before Git removes it, so all durable
    // path-scoped metadata uses exactly the same key as normal repository
    // opening and cannot leak into a later repository at this path.
    let removed_repository = git.repository_record(&worktree_path)?;
    let _repository_operation_lease = state.acquire_repository_operation_leases(&[
        canonical.as_str(),
        removed_repository.path.as_str(),
    ])?;
    git.remove_worktree(&repository, &worktree_path, force)?;
    state.mutate(|store| {
        store.forget_repository_path(&removed_repository.path);
        Ok(())
    })?;
    git.worktrees(&repository)
}

#[tauri::command]
pub fn list_submodules(repository_path: String) -> Result<Vec<SubmoduleInfo>, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().submodules(&repository)
}

#[tauri::command]
pub async fn update_submodules(
    repository_path: String,
    initialize: bool,
    recursive: bool,
    state: State<'_, AppState>,
) -> Result<Vec<SubmoduleInfo>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let git = GitCore::new();
        let environment = submodule_update_environment(&state, &repository)?;
        git.update_submodules_with_environment(&repository, initialize, recursive, &environment)?;
        git.submodules(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "子模块后台任务未能完成。"))?
}

#[tauri::command]
pub fn get_lfs_status(repository_path: String) -> Result<LfsStatus, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().lfs_status(&repository)
}

#[tauri::command]
pub fn install_lfs(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<LfsStatus, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    git.install_lfs(&repository)?;
    git.lfs_status(&repository)
}

#[tauri::command]
pub fn track_lfs_pattern(
    repository_path: String,
    pattern: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().track_lfs_pattern(&repository, &pattern)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn start_rebase(
    repository_path: String,
    base: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().start_rebase(&repository, &base)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn cherry_pick_commits(
    repository_path: String,
    revisions: Vec<String>,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().cherry_pick(&repository, &revisions)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn continue_git_operation(
    repository_path: String,
    kind: GitOperationKind,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().continue_operation(&repository, kind)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn abort_git_operation(
    repository_path: String,
    kind: GitOperationKind,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().abort_operation(&repository, kind)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn list_stashes(repository_path: String) -> Result<Vec<StashEntry>, AppError> {
    let repository = repository_root(&repository_path)?;
    GitCore::new().stashes(&repository)
}

#[tauri::command]
pub fn stash_changes(
    repository_path: String,
    message: Option<String>,
    include_untracked: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stash_push(&repository, message.as_deref(), include_untracked)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn pop_stash(
    repository_path: String,
    reference: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stash_pop(&repository, &reference)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn drop_stash(
    repository_path: String,
    reference: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stash_drop(&repository, &reference)
}

#[tauri::command]
pub fn reset_branch(
    repository_path: String,
    target: String,
    mode: ResetMode,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().reset(&repository, &target, mode)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn squash_recent_commits(
    repository_path: String,
    count: usize,
    message: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().squash_recent_commits(&repository, count, &message)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn revert_commit(
    repository_path: String,
    revision: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().revert_commit(&repository, &revision)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub async fn fetch_remote(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<NetworkOperationResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let environment = network_environment(&state, &repository, &remote_name, false)?;
        let git = GitCore::new();
        git.ensure_remote_fetches_all_branches(&repository, &remote_name)?;
        git.fetch_with_environment(
            &repository,
            &FetchOptions {
                remote: remote_name.clone(),
                refspecs: Vec::new(),
                prune: true,
            },
            &environment,
        )?;
        Ok(NetworkOperationResult::completed(format!(
            "已从 {remote_name} 获取全部远程分支。"
        )))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn pull_remote(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<NetworkOperationResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let environment = network_environment(&state, &repository, &remote_name, false)?;
        GitCore::new().pull_with_environment(
            &repository,
            &PullOptions {
                remote: remote_name.clone(),
                refspec: None,
            },
            &environment,
        )?;
        Ok(NetworkOperationResult::completed(format!(
            "已从 {remote_name} 拉取变更。"
        )))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn push_remote(
    repository_path: String,
    remote_name: String,
    force_with_lease: Option<bool>,
    state: State<'_, AppState>,
) -> Result<NetworkOperationResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let environment = network_environment(&state, &repository, &remote_name, true)?;
        GitCore::new().push_with_environment(
            &repository,
            &PushOptions {
                remote: remote_name.clone(),
                refspecs: Vec::new(),
                set_upstream: false,
                force_with_lease: force_with_lease.unwrap_or(false),
                follow_tags: false,
            },
            &environment,
        )?;
        Ok(NetworkOperationResult::completed(format!(
            "已推送到 {remote_name}。"
        )))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 后台任务未能完成。"))?
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountProfile>, AppError> {
    // Account status is persisted after a background refresh. Returning that
    // cache keeps the Accounts route responsive even when a CLI, keychain, or
    // SSH agent is slow to answer.
    Ok(state.snapshot()?.accounts)
}

/// Refresh status checks away from the route-loading command. The blocking
/// checks deliberately run without holding the persistent-store mutex, then
/// the resulting safe status values are merged and atomically persisted.
#[tauri::command]
pub async fn refresh_account_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    let snapshot = state.snapshot()?;
    let endpoints = snapshot.all_endpoints();
    let accounts = snapshot.accounts;
    let refreshed = tauri::async_runtime::spawn_blocking(move || {
        refresh_account_profiles(accounts, &endpoints)
    })
    .await
    .map_err(|_| {
        AppError::new(
            "account_status_refresh_failed",
            "账号认证状态刷新任务未能完成。",
        )
    })?;

    state.mutate(|store| {
        merge_refreshed_account_statuses(store, &refreshed);
        Ok(store.accounts.clone())
    })
}

/// The PAT is one-way input: this command stores it in the system keychain and never returns it.
#[tauri::command]
pub fn save_pat_account(
    endpoint_id: String,
    login: String,
    display_name: Option<String>,
    token: String,
    state: State<'_, AppState>,
) -> Result<AccountProfile, AppError> {
    let login = required_login(&login)?;
    let endpoint = endpoint_from_state(&state, &endpoint_id)?;
    if !CodeHostCapabilities::for_endpoint(&endpoint).pat_keychain {
        return Err(AppError::new(
            "pat_not_supported",
            "该代码托管 Endpoint 尚未启用应用管理的 PAT。",
        ));
    }
    let token = token.trim().to_owned();
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(AppError::new("token_required", "请输入个人访问令牌。"));
    }

    let id = format!("account-{}", Uuid::new_v4());
    let secret_ref = default_pat_keychain_reference(&id);
    // Queue deletion before writing the keychain. If the profile write later
    // fails, startup can discover and remove this generated secret reference
    // instead of leaving an untrackable orphan in the system keychain.
    state.mutate(|store| {
        store.queue_secret_deletion(secret_ref.clone());
        Ok(())
    })?;
    if let Err(error) = save_personal_access_token(&secret_ref, &token) {
        let _ = state.mutate(|store| {
            store.clear_pending_secret_deletion(&secret_ref);
            Ok(())
        });
        return Err(error);
    }

    let account = AccountProfile {
        id,
        endpoint_id: endpoint.id,
        login,
        display_name: display_name.and_then(non_empty_trimmed),
        auth_method: AuthenticationMethod::PersonalAccessToken,
        secret_ref: Some(secret_ref.clone()),
        status: AuthenticationStatus::Valid,
        is_default_candidate: false,
    };

    match state.mutate_with_outcome(|store| {
        store.accounts.push(account.clone());
        store.clear_pending_secret_deletion(&secret_ref);
        Ok(account.clone())
    }) {
        Ok(saved) => Ok(saved),
        Err(error) if error.committed() => Err(error.into_error()),
        Err(error) => {
            // The profile did not reach atomic replacement. Keep the durable
            // deletion queue if keychain cleanup itself fails; it will be
            // retried after startup rather than silently leaking a PAT.
            if delete_personal_access_token(&secret_ref).is_ok() {
                let _ = state.mutate(|store| {
                    store.clear_pending_secret_deletion(&secret_ref);
                    Ok(())
                });
            }
            Err(error.into_error())
        }
    }
}

#[tauri::command]
pub fn save_account_profile(
    endpoint_id: String,
    login: String,
    display_name: Option<String>,
    auth_method: AuthenticationMethod,
    state: State<'_, AppState>,
) -> Result<AccountProfile, AppError> {
    if matches!(auth_method, AuthenticationMethod::PersonalAccessToken) {
        return Err(AppError::new(
            "pat_requires_secure_input",
            "添加 PAT 请使用专用的安全账号添加入口。",
        ));
    }
    let endpoint = endpoint_from_state(&state, &endpoint_id)?;
    validate_method_for_endpoint(&auth_method, &endpoint)?;
    let account = AccountProfile {
        id: format!("account-{}", Uuid::new_v4()),
        endpoint_id: endpoint.id,
        login: required_login(&login)?,
        display_name: display_name.and_then(non_empty_trimmed),
        auth_method,
        secret_ref: None,
        status: AuthenticationStatus::Unknown,
        is_default_candidate: false,
    };
    state.mutate(|store| {
        store.accounts.push(account.clone());
        Ok(account)
    })
}

#[tauri::command]
pub fn remove_account(
    account_profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let account = state
        .snapshot()?
        .accounts
        .into_iter()
        .find(|account| account.id == account_profile_id)
        .ok_or_else(|| AppError::new("account_not_found", "找不到要移除的账号。"))?;

    let secret_ref = personal_access_token_reference(&account).cloned();
    let persistence_warning = match state.mutate_with_outcome(|store| {
        store
            .remove_account(&account_profile_id)
            .ok_or_else(|| AppError::new("account_not_found", "找不到要移除的账号。"))?;
        if let Some(secret_ref) = &secret_ref {
            store.queue_secret_deletion(secret_ref.clone());
        }
        Ok(())
    }) {
        Ok(()) => None,
        // `profiles.json` was atomically replaced, so its deletion queue is
        // durable even though the final directory sync warning must still be
        // reported after the token cleanup completes.
        Err(error) if error.committed() => Some(error.into_error()),
        Err(error) => return Err(error.into_error()),
    };

    if let Some(secret_ref) = &secret_ref {
        delete_personal_access_token(secret_ref)?;
        let clear_warning = match state.mutate_with_outcome(|store| {
            store.clear_pending_secret_deletion(secret_ref);
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            // The queue is still durable from the first mutation. Keeping it
            // is safe: startup treats an already-deleted keychain entry as a
            // successful retry.
            Err(error) => return Err(error.into_error()),
        };
        if let Some(error) = clear_warning {
            return Err(error);
        }
    }
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub async fn import_cli_accounts(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let endpoint = endpoint_from_state(&state, &endpoint_id)?;
        if !CodeHostCapabilities::for_endpoint(&endpoint).account_import {
            return Err(AppError::new(
                "cli_import_not_supported",
                "该 Endpoint 当前配置无法安全使用 CLI 账号导入。",
            ));
        }
        let imported = match endpoint.kind {
            CodeHostKind::GitHub => import_gh_current_account_new(&endpoint)?,
            CodeHostKind::GitLab => import_glab_current_account_new(&endpoint)?,
            _ => {
                return Err(AppError::new(
                    "cli_import_not_supported",
                    "该 Endpoint 不支持 gh 或 glab 账号导入。",
                ));
            }
        };

        state.mutate(|store| {
            let account = if let Some(existing) = store.accounts.iter_mut().find(|account| {
                account.endpoint_id == imported.endpoint_id
                    && account.login.eq_ignore_ascii_case(&imported.login)
                    && account.auth_method == imported.auth_method
            }) {
                existing.status = imported.status.clone();
                existing.clone()
            } else {
                store.accounts.push(imported.clone());
                imported.clone()
            };
            Ok(vec![account])
        })
    })
    .await
    .map_err(|_| AppError::new("command_failed", "CLI 账号导入后台任务未能完成。"))?
}

#[tauri::command]
pub fn set_default_account(
    account_profile_id: String,
    is_default_candidate: bool,
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    state.mutate(|store| {
        let endpoint_id = store
            .accounts
            .iter()
            .find(|account| account.id == account_profile_id)
            .map(|account| account.endpoint_id.clone())
            .ok_or_else(|| AppError::new("account_not_found", "找不到该账号。"))?;
        for account in &mut store.accounts {
            if account.endpoint_id == endpoint_id {
                account.is_default_candidate =
                    account.id == account_profile_id && is_default_candidate;
            }
        }
        Ok(store.accounts.clone())
    })
}

#[tauri::command]
pub fn get_remote_binding(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<Option<RemoteBinding>, AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy();
    Ok(state.snapshot()?.binding_for(&canonical, &remote_name))
}

#[tauri::command]
pub fn save_remote_binding(
    binding: RemoteBinding,
    state: State<'_, AppState>,
) -> Result<RemoteBinding, AppError> {
    let repository = repository_root(&binding.repository_path)?;
    let canonical_path = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical_path)?;
    let git = GitCore::new();
    if !git
        .remotes(&repository)?
        .iter()
        .any(|remote| remote.name == binding.remote_name)
    {
        return Err(AppError::new(
            "remote_not_found",
            "仓库中不存在要绑定的 remote。",
        ));
    }
    let fetch_url = git.remote_url_for_operation(&repository, &binding.remote_name, false)?;
    let push_url = git.remote_url_for_operation(&repository, &binding.remote_name, true)?;

    let mut binding = binding;
    binding.repository_path = canonical_path;
    state.mutate(|store| {
        let endpoint = store
            .endpoint(&binding.endpoint_id)
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?;
        validate_codehost_binding_url(&fetch_url, &endpoint)?;
        validate_codehost_binding_url(&push_url, &endpoint)?;
        if let AccountSelection::Profile(account_profile_id) = &binding.account_selection {
            let account = store
                .accounts
                .iter()
                .find(|account| account.id == *account_profile_id)
                .ok_or_else(|| AppError::new("account_not_found", "找不到要绑定的账号。"))?;
            if account.endpoint_id != endpoint.id {
                return Err(AppError::new(
                    "binding_endpoint_mismatch",
                    "账号不属于该 remote 绑定的 Endpoint。",
                ));
            }
            if matches!(
                account.auth_method,
                AuthenticationMethod::SshAgent
                    |
                AuthenticationMethod::GhCli
                    | AuthenticationMethod::GlabCli
                    | AuthenticationMethod::SystemGitCredentialHelper
            ) {
                return Err(AppError::new(
                    "authentication_method_mismatch",
                    "此认证方式无法安全固定到指定账号；Git 网络操作请绑定 PAT，SSH agent 请由 Git/SSH 配置选择。",
                ));
            }
        }
        store.upsert_binding(binding.clone());
        Ok(binding)
    })
}

#[tauri::command]
pub fn get_codehost_capabilities(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<CodeHostCapabilities, AppError> {
    Ok(CodeHostCapabilities::for_endpoint(&endpoint_from_state(
        &state,
        &endpoint_id,
    )?))
}

#[tauri::command]
pub fn list_endpoints(state: State<'_, AppState>) -> Result<Vec<CodeHostEndpoint>, AppError> {
    Ok(state.snapshot()?.all_endpoints())
}

#[tauri::command]
pub fn save_custom_endpoint(
    mut endpoint: CodeHostEndpoint,
    state: State<'_, AppState>,
) -> Result<CodeHostEndpoint, AppError> {
    endpoint.id = endpoint.id.trim().to_ascii_lowercase();
    endpoint.display_name = endpoint.display_name.trim().to_owned();
    endpoint.web_url = endpoint.web_url.trim().to_owned();
    endpoint.api_url = endpoint.api_url.trim().to_owned();
    endpoint.git_host = normalize_git_authority(&endpoint.git_host)
        .ok_or_else(|| AppError::new("invalid_endpoint", "Endpoint 标识或 Git host 无效。"))?;
    endpoint.is_builtin = false;
    validate_custom_endpoint(&endpoint)?;
    state.mutate(|store| {
        if CodeHostEndpoint::builtin()
            .iter()
            .any(|builtin| builtin.id == endpoint.id)
        {
            return Err(AppError::new(
                "builtin_endpoint_immutable",
                "内建 Endpoint 不能被覆盖。",
            ));
        }
        if endpoint_git_host_conflicts(store, &endpoint) {
            return Err(AppError::new(
                "endpoint_git_host_conflict",
                "Git host 已由另一个 Endpoint 使用；请使用唯一的主机或端口。",
            ));
        }
        if endpoint_routing_change_is_referenced(store, &endpoint) {
            return Err(AppError::new(
                "endpoint_in_use_immutable",
                "已有账号或 remote 绑定正在使用该 Endpoint；请先迁移或删除这些关联后再修改平台类型或 Git host。",
            ));
        }

        if let Some(existing) = store
            .custom_endpoints
            .iter_mut()
            .find(|existing| existing.id == endpoint.id)
        {
            *existing = endpoint.clone();
        } else {
            store.custom_endpoints.push(endpoint.clone());
        }
        Ok(endpoint)
    })
}

#[tauri::command]
pub fn create_tag(
    repository_path: String,
    name: String,
    target: Option<String>,
    message: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().create_tag(&repository, &name, target.as_deref(), message.as_deref())
}

#[tauri::command]
pub fn delete_tag(
    repository_path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().delete_tag(&repository, &name)
}

#[tauri::command]
pub fn add_remote(
    repository_path: String,
    name: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let git = GitCore::new();
    if git
        .remotes(&repository)?
        .iter()
        .any(|remote| remote.name == name)
    {
        return Err(AppError::new(
            "remote_already_exists",
            "仓库中已存在同名 remote。",
        ));
    }
    let previous_binding = state.snapshot()?.binding_for(&canonical, &name);
    // A previous `git remote remove` can have succeeded while its profile
    // cleanup failed. Clear that now-stale selection before a newly added
    // remote reuses the same name, even when the new URL happens to share the
    // old Endpoint.
    let persistence_warning = if previous_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            Err(error) => return Err(error.into_error()),
        }
    } else {
        None
    };
    if let Err(error) = git.add_remote(&repository, &name, &url) {
        // `git remote add` normally fails without a side effect. Restore the
        // old binding only after proving the remote name is still absent; if
        // Git did add it despite an error, leaving the binding cleared is the
        // safe fail-closed result.
        let remote_exists = git
            .remotes(&repository)
            .map(|remotes| remotes.iter().any(|remote| remote.name == name))
            .unwrap_or(true);
        if !remote_exists {
            if let Some(binding) = previous_binding {
                if state
                    .mutate(|store| {
                        store.upsert_binding(binding);
                        Ok(())
                    })
                    .is_err()
                {
                    return Err(AppError::new(
                        "remote_binding_reconciliation_failed",
                        "remote 添加失败，原有账号绑定恢复失败；请检查 remote 后重新关联账号。",
                    ));
                }
            }
        }
        return Err(error);
    }
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub fn set_remote_url(
    repository_path: String,
    name: String,
    url: String,
    kind: RemoteUrlKind,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let git = GitCore::new();
    let store = state.snapshot()?;
    let current_fetch_url = git.remote_url_for_operation(&repository, &name, false)?;
    let current_push_url = git.remote_url_for_operation(&repository, &name, true)?;
    let (effective_fetch_url, effective_push_url) = match kind {
        RemoteUrlKind::Fetch => (url.as_str(), current_push_url.as_str()),
        RemoteUrlKind::Push => (current_fetch_url.as_str(), url.as_str()),
    };
    let binding_is_compatible = remote_binding_matches_urls(
        &store,
        &canonical,
        &name,
        effective_fetch_url,
        effective_push_url,
    );
    git.set_remote_url(&repository, &name, &url, kind)?;
    let persistence_warning = if !binding_is_compatible {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            // The Git URL is now changed while the stale binding remains.
            // It fails closed on endpoint mismatch, so do not roll Git back
            // to an old URL that might have been externally edited.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程地址已更新，但账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    } else {
        None
    };
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub fn rename_remote(
    repository_path: String,
    old_name: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let git = GitCore::new();
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let previous_binding = state.snapshot()?.binding_for(&canonical, &old_name);
    let previous_new_binding = state.snapshot()?.binding_for(&canonical, &new_name);
    // Persist the new name alongside the old one before Git renames it. The
    // dual binding makes both sides of the crash boundary fail safe: before
    // Git changes, the old remote is authorized; afterwards, the new remote
    // is authorized until the obsolete name can be cleaned up.
    let new_binding = previous_binding.as_ref().map(|binding| RemoteBinding {
        repository_path: binding.repository_path.clone(),
        remote_name: new_name.clone(),
        endpoint_id: binding.endpoint_id.clone(),
        account_selection: binding.account_selection.clone(),
    });
    let persistence_warning = if let Some(new_binding) = &new_binding {
        match state.mutate_with_outcome(|store| {
            store.upsert_binding(new_binding.clone());
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            Err(error) => return Err(error.into_error()),
        }
    } else {
        None
    };
    if let Err(error) = git.rename_remote(&repository, &old_name, &new_name) {
        if new_binding.is_some()
            && state
                .mutate(|store| {
                    if let Some(binding) = &previous_new_binding {
                        store.upsert_binding(binding.clone());
                    } else {
                        store.remote_bindings.retain(|binding| {
                            !(binding.repository_path == canonical
                                && binding.remote_name == new_name)
                        });
                    }
                    Ok(())
                })
                .is_err()
        {
            return Err(AppError::new(
                "remote_binding_reconciliation_failed",
                "远程名称未更新，但临时账号绑定恢复失败；请检查 remote 后重新关联账号。",
            ));
        }
        return Err(error);
    }
    let cleanup_warning = if new_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == old_name)
            });
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            // Keeping both bindings is safe: the renamed remote has the
            // copied authorization and the old remote no longer exists.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程名称已更新，但旧账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    } else {
        None
    };
    if let Some(error) = cleanup_warning {
        return Err(error);
    }
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub fn remove_remote(
    repository_path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let git = GitCore::new();
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let previous_binding = state.snapshot()?.binding_for(&canonical, &name);
    git.remove_remote(&repository, &name)?;
    let persistence_warning = if previous_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            // The remote is already gone. A stale binding is harmless until
            // reconciliation because no network operation can address it.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程已移除，但账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    } else {
        None
    };
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub async fn get_codehost_repository_info(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<CodeHostRepositoryInfo, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let context = github_adapter_context(&state, &repository_path, &remote_name)?;
        GitHubAdapter.repository_info(&context)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn list_pull_requests(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<PullRequestSummary>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let context = github_adapter_context(&state, &repository_path, &remote_name)?;
        GitHubAdapter.list_pull_requests(&context)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn create_pull_request(
    repository_path: String,
    remote_name: String,
    input: CreatePullRequestInput,
    state: State<'_, AppState>,
) -> Result<PullRequestSummary, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let context = github_adapter_context(&state, &repository_path, &remote_name)?;
        GitHubAdapter.create_pull_request(&context, &input)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn publish_github_repository(
    repository_path: String,
    endpoint_id: String,
    account_profile_id: String,
    name: String,
    is_private: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let _publish_lease = state.acquire_publish_lease(&repository.to_string_lossy())?;
        if reconcile_pending_publish(&state, &repository)? {
            return snapshot_for(&state, &repository);
        }
        let git = GitCore::new();
        if !git.remotes(&repository)?.is_empty() {
            return Err(AppError::new(
                "repository_already_has_remote",
                "当前仓库已有 remote，不能使用发布流程覆盖它。",
            ));
        }
        if !git.has_head(&repository)? {
            return Err(AppError::new(
                "nothing_to_publish",
                "请先创建至少一个本地提交，再发布到 GitHub。",
            ));
        }

        let store = state.snapshot()?;
        let endpoint = store
            .endpoint(&endpoint_id)
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?;
        if endpoint.kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "只有 GitHub Endpoint 支持此发布流程。",
            ));
        }
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == account_profile_id)
            .cloned()
            .ok_or_else(|| AppError::new("account_not_found", "找不到发布所选的账号。"))?;
        if account.endpoint_id != endpoint.id || account.auth_method != AuthenticationMethod::GhCli
        {
            return Err(AppError::new(
                "github_publish_auth_unsupported",
                "发布 GitHub 仓库需要与 Endpoint 匹配的 gh CLI 账号。",
            ));
        }
        if current_account_status(&account, &store.all_endpoints()) != AuthenticationStatus::Valid {
            return Err(AppError::new(
                "cli_auth_unavailable",
                "当前 gh 登录与所选账号不匹配或已失效。",
            ));
        }

        let repository_record = git.repository_record(&repository)?;
        let expected_head = git.head_commit_id(&repository)?;
        let expected_repository_slug = github_publish_expected_slug(&name, &account.login)?;
        let pending = PendingPublish {
            repository_path: repository_record.path.clone(),
            endpoint_id: endpoint.id.clone(),
            account_profile_id: account.id.clone(),
            remote_name: "origin".into(),
            expected_head,
            expected_repository_slug,
        };
        state.mutate(|store| {
            store.queue_pending_publish(pending);
            Ok(())
        })?;

        // `gh repo create --push` can create the remote and configure origin
        // before a later push step fails. The `?` intentionally leaves the
        // durable intent in place so the next invocation reconciles that
        // completed side effect instead of attempting a second publish.
        GitHubAdapter.publish_repository(&endpoint, &repository, &name, is_private)?;
        let origin_exists = git
            .remotes(&repository)?
            .iter()
            .any(|remote| remote.name == "origin");
        if !origin_exists {
            state.mutate(|store| {
                store.clear_pending_publish(&repository_record.path);
                Ok(())
            })?;
            return Err(AppError::new(
                "github_publish_failed",
                "GitHub 已返回成功，但本地未找到 origin remote；请刷新后检查仓库配置。",
            ));
        }
        reconcile_pending_publish(&state, &repository)?;
        snapshot_for(&state, &repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

fn required_path(value: &str, _label: &str) -> Result<PathBuf, AppError> {
    if value.trim().is_empty() {
        return Err(AppError::new("path_required", "路径不能为空。"));
    }
    Ok(PathBuf::from(value))
}

/// A clone target does not exist before Git starts, so canonicalize its parent
/// and append the final path component. This makes concurrent aliases such as
/// `./project` and an absolute path share one in-process lease without
/// requiring the destination itself to exist.
fn clone_destination_key(destination: &Path) -> String {
    let absolute = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current_directory| current_directory.join(destination))
            .unwrap_or_else(|_| destination.to_path_buf())
    };
    let Some(file_name) = absolute.file_name() else {
        return absolute.to_string_lossy().into_owned();
    };
    let Some(parent) = absolute.parent() else {
        return absolute.to_string_lossy().into_owned();
    };
    parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(file_name)
        .to_string_lossy()
        .into_owned()
}

fn repository_root(path: &str) -> Result<PathBuf, AppError> {
    let path = required_path(path, "仓库路径")?;
    GitCore::new().repository_root(&path)
}

/// Keeps the stash and branch checkout in one repository-operation lease.
/// The frontend only requests this after an explicit user choice, but status
/// is checked again here so a stale UI snapshot never turns an empty working
/// tree into a failed automatic stash.
fn stash_local_changes_before_branch_switch(
    git: &GitCore,
    repository: &Path,
    stash_local_changes: bool,
) -> Result<bool, AppError> {
    if stash_local_changes && !git.status(repository)?.entries.is_empty() {
        git.stash_push(repository, Some(BRANCH_SWITCH_STASH_MESSAGE), true)?;
        return Ok(true);
    }
    Ok(false)
}

fn branch_switch_error_after_stash(error: AppError, stashed_local_changes: bool) -> AppError {
    if stashed_local_changes {
        AppError::with_safe_details(
            "branch_switch_changes_stashed",
            "分支未能切换；本地改动已保存到暂存栈。",
            "请从“历史与暂存栈”恢复改动，再检查分支或工作树状态后重试。",
        )
    } else {
        error
    }
}

fn snapshot_for(state: &AppState, repository: &Path) -> Result<RepositorySnapshot, AppError> {
    let git = GitCore::new();
    let git_snapshot = git.snapshot(repository)?;
    let operation = git.operation_kind(repository)?;
    let has_conflicts = git_snapshot.status.entries.iter().any(is_conflicted_status);
    let store = state.snapshot()?;
    let repository_path = git_snapshot.repository.path.clone();
    let mut remotes = git_snapshot.remotes;
    enrich_remote_endpoints(&mut remotes, &store, &repository_path);
    let mut repository_record = store.repository_with_metadata(git_snapshot.repository);
    let (host, namespace) = repository_owner_from_remotes(&remotes);
    repository_record.host = host;
    repository_record.namespace = namespace;
    let bindings = store
        .remote_bindings
        .iter()
        .filter(|binding| binding.repository_path == repository_path)
        .cloned()
        .collect();
    Ok(RepositorySnapshot {
        repository: repository_record,
        has_head: git.has_head(repository)?,
        status: git_snapshot.status,
        branches: git_snapshot.branches,
        remote_branches: git_snapshot.remote_branches,
        remotes,
        tags: git_snapshot.tags,
        bindings,
        operation: operation.map(|kind| GitOperationState {
            kind,
            has_conflicts,
        }),
    })
}

/// Finish clone metadata that was intentionally recorded before Git created
/// its destination. A pending binding is reapplied only after both current
/// remote URLs still validate against its Endpoint, preventing a stale intent
/// from selecting an account for an unrelated repository later created there.
fn reconcile_pending_clone(state: &AppState, repository: &Path) -> Result<(), AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let pending = state
        .snapshot()?
        .pending_clones
        .into_iter()
        .find(|pending| {
            Path::new(&pending.destination_path)
                .canonicalize()
                .ok()
                .is_some_and(|path| path == Path::new(&repository_record.path))
        });
    let Some(pending) = pending else {
        return Ok(());
    };

    let binding = pending.binding.clone().and_then(|binding| {
        // Old intents predate durable repository identity. They can still
        // register the cloned repository, but must never recreate an account
        // binding for whatever later occupies the destination path.
        if pending.expected_remote_authority.is_empty()
            || pending.expected_repository_slug.is_empty()
        {
            return None;
        }
        let store = state.snapshot().ok()?;
        let endpoint = store.endpoint(&binding.endpoint_id)?;
        if !binding_selected_account_is_current(&store, &binding) {
            return None;
        }
        let fetch_url = git
            .remote_url_for_operation(repository, &binding.remote_name, false)
            .ok()?;
        let push_url = git
            .remote_url_for_operation(repository, &binding.remote_name, true)
            .ok()?;
        (validate_codehost_binding_url(&fetch_url, &endpoint).is_ok()
            && validate_codehost_binding_url(&push_url, &endpoint).is_ok()
            && pending_clone_remote_matches(
                &fetch_url,
                &pending.expected_remote_authority,
                &pending.expected_repository_slug,
            )
            && pending_clone_remote_matches(
                &push_url,
                &pending.expected_remote_authority,
                &pending.expected_repository_slug,
            ))
        .then_some(binding)
    });

    state.mutate(|store| {
        store.upsert_repository(repository_record.clone());
        if let Some(mut binding) = binding.clone() {
            binding.repository_path = repository_record.path.clone();
            store.upsert_binding(binding);
        }
        store.clear_pending_clone(&pending.destination_path);
        Ok(())
    })
}

/// Complete a `gh repo create --push` whose external side effect succeeded
/// before the profile store could persist the account binding. The intent is
/// durable and contains no secret, so a retry can safely reconcile the local
/// `origin` instead of asking the user to create the repository again.
fn reconcile_pending_publish(state: &AppState, repository: &Path) -> Result<bool, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let store = state.snapshot()?;
    let Some(pending) = store.pending_publish_for(&repository_record.path) else {
        return Ok(false);
    };
    let reconciliation_error = || {
        AppError::new(
            "github_publish_reconciliation_failed",
            "已检测到待完成的 GitHub 发布，但 origin 与所选账号或 Endpoint 不匹配。",
        )
    };
    // Pending publishes written by older versions lack a durable repository
    // identity. Fail closed and discard them rather than assigning an account
    // to whichever repository now happens to occupy that local path.
    if pending.expected_head.is_empty() || pending.expected_repository_slug.is_empty() {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    }
    let current_head = git
        .head_commit_id(repository)
        .map_err(|_| reconciliation_error())?;
    if current_head != pending.expected_head {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    }
    let origin_exists = git
        .remotes(repository)?
        .into_iter()
        .any(|remote| remote.name == pending.remote_name);
    if !origin_exists {
        return Ok(false);
    }
    let Some(endpoint) = store.endpoint(&pending.endpoint_id) else {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    };
    let Some(account) = store
        .accounts
        .iter()
        .find(|account| account.id == pending.account_profile_id)
        .cloned()
    else {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    };
    let fetch_url = git
        .remote_url_for_operation(repository, &pending.remote_name, false)
        .map_err(|_| reconciliation_error())?;
    let push_url = git
        .remote_url_for_operation(repository, &pending.remote_name, true)
        .map_err(|_| reconciliation_error())?;
    if account.endpoint_id != endpoint.id
        || validate_codehost_binding_url(&fetch_url, &endpoint).is_err()
        || validate_codehost_binding_url(&push_url, &endpoint).is_err()
        || !pending_publish_remote_matches(&fetch_url, &pending.expected_repository_slug)
        || !pending_publish_remote_matches(&push_url, &pending.expected_repository_slug)
    {
        return Err(reconciliation_error());
    }

    state.mutate(|store| {
        store.upsert_repository(repository_record.clone());
        store.upsert_binding(RemoteBinding {
            repository_path: repository_record.path.clone(),
            remote_name: pending.remote_name.clone(),
            endpoint_id: endpoint.id.clone(),
            account_selection: AccountSelection::Profile(account.id.clone()),
        });
        store.clear_pending_publish(&repository_record.path);
        Ok(())
    })?;
    Ok(true)
}

fn github_publish_expected_slug(name: &str, account_login: &str) -> Result<String, AppError> {
    let name = name.trim();
    let parts: Vec<&str> = name.split('/').collect();
    let (owner, repository) = match parts.as_slice() {
        [repository] => (account_login.trim(), *repository),
        [owner, repository] => (*owner, *repository),
        _ => {
            return Err(AppError::new(
                "invalid_repository_name",
                "GitHub 仓库名称无效。",
            ))
        }
    };
    if !is_github_publish_slug_segment(owner) || !is_github_publish_slug_segment(repository) {
        return Err(AppError::new(
            "invalid_repository_name",
            "GitHub 仓库名称无效。",
        ));
    }
    Ok(format!("{owner}/{repository}"))
}

fn is_github_publish_slug_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.starts_with('-')
        && !matches!(value, "." | "..")
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn pending_publish_remote_matches(raw_remote_url: &str, expected_slug: &str) -> bool {
    parse_remote_url(raw_remote_url)
        .repository_slug()
        .as_deref()
        .is_some_and(|slug| slug.eq_ignore_ascii_case(expected_slug))
}

fn clone_expected_remote_identity(raw_remote_url: &str) -> Result<(String, String), AppError> {
    let parsed = parse_remote_url(raw_remote_url);
    if !matches!(
        parsed.transport,
        crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
    ) {
        return Err(AppError::new(
            "clone_repository_identity_unavailable",
            "无法从 Clone 地址识别仓库路径；请选择有效的 HTTPS 或 SSH Git 地址。",
        ));
    }
    let authority = parsed.authority().filter(|authority| !authority.is_empty());
    let repository_slug = parsed.repository_slug().filter(|slug| !slug.is_empty());
    match (authority, repository_slug) {
        (Some(authority), Some(repository_slug)) => Ok((authority, repository_slug)),
        _ => Err(AppError::new(
            "clone_repository_identity_unavailable",
            "无法从 Clone 地址识别仓库路径；请选择有效的 HTTPS 或 SSH Git 地址。",
        )),
    }
}

fn pending_clone_remote_matches(
    raw_remote_url: &str,
    expected_authority: &str,
    expected_slug: &str,
) -> bool {
    let parsed = parse_remote_url(raw_remote_url);
    parsed
        .authority()
        .as_deref()
        .is_some_and(|authority| authority.eq_ignore_ascii_case(expected_authority))
        && parsed.repository_slug().as_deref() == Some(expected_slug)
}

fn is_conflicted_status(entry: &crate::domain::GitStatusEntry) -> bool {
    matches!(
        (entry.index_status.as_str(), entry.worktree_status.as_str()),
        ("D", "D") | ("A", "U") | ("U", "D") | ("U", "A") | ("D", "U") | ("A", "A") | ("U", "U")
    )
}

fn enrich_remote_endpoints(
    remotes: &mut [RemoteProfile],
    store: &ProfileStore,
    repository_path: &str,
) {
    let endpoints = store.all_endpoints();
    for remote in remotes {
        if remote.endpoint_id.is_some() {
            continue;
        }
        let endpoint = store
            .binding_for(repository_path, &remote.name)
            .and_then(|binding| store.endpoint(&binding.endpoint_id))
            .or_else(|| {
                remote
                    .parsed
                    .authority()
                    .as_deref()
                    .and_then(|authority| endpoint_for_host(authority, &endpoints))
                    .cloned()
            });
        if let Some(endpoint) = endpoint {
            remote.endpoint_id = Some(endpoint.id);
            remote.code_host_kind = endpoint.kind;
        }
    }
}

/// Select an identity for sidebar ownership grouping. `origin` wins whenever
/// it has both a host and an owner path; otherwise any recognizable remote is
/// used. That mirrors the remote users usually consider canonical without
/// making a malformed origin hide useful metadata from another remote.
fn repository_owner_from_remotes(remotes: &[RemoteProfile]) -> (Option<String>, Option<String>) {
    remotes
        .iter()
        .find(|remote| {
            remote.name == "origin"
                && remote.parsed.host.is_some()
                && remote.parsed.namespace.is_some()
        })
        .or_else(|| {
            remotes
                .iter()
                .find(|remote| remote.parsed.host.is_some() && remote.parsed.namespace.is_some())
        })
        .map(|remote| (remote.parsed.host.clone(), remote.parsed.namespace.clone()))
        .unwrap_or((None, None))
}

fn remember_repositories(
    state: &AppState,
    repositories: &[RepositoryRecord],
) -> Result<(), AppError> {
    let repositories = repositories.to_vec();
    state.mutate(|store| {
        for repository in repositories {
            store.upsert_repository(repository);
        }
        Ok(())
    })
}

fn remember_repository(state: &AppState, repository: RepositoryRecord) -> Result<(), AppError> {
    state.mutate(|store| {
        store.upsert_repository(repository);
        Ok(())
    })
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn normalize_repository_group(group: Option<String>) -> Result<Option<String>, AppError> {
    let Some(group) = group else {
        return Ok(None);
    };
    let group = group.trim();
    if group.is_empty() {
        return Ok(None);
    }
    if group.chars().count() > 80 || group.chars().any(char::is_control) {
        return Err(AppError::new(
            "repository_group_invalid",
            "分组名称最多 80 个字符，且不能包含控制字符。",
        ));
    }
    Ok(Some(group.to_owned()))
}

fn endpoint_from_state(state: &AppState, endpoint_id: &str) -> Result<CodeHostEndpoint, AppError> {
    state
        .snapshot()?
        .endpoint(endpoint_id)
        .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn required_login(value: &str) -> Result<String, AppError> {
    let login = value.trim();
    if login.is_empty() || login.len() > 256 || login.chars().any(char::is_whitespace) {
        return Err(AppError::new(
            "invalid_account_login",
            "账号登录名不能为空且不能包含空格。",
        ));
    }
    Ok(login.to_owned())
}

fn validate_method_for_endpoint(
    method: &AuthenticationMethod,
    endpoint: &CodeHostEndpoint,
) -> Result<(), AppError> {
    match (method, &endpoint.kind) {
        (AuthenticationMethod::GhCli, CodeHostKind::GitHub)
            if !git_authority_has_explicit_port(&endpoint.git_host)
                && github_endpoint_has_gh_compatible_api(endpoint) =>
        {
            Ok(())
        }
        (AuthenticationMethod::GlabCli, CodeHostKind::GitLab)
            if !git_authority_has_explicit_port(&endpoint.git_host) =>
        {
            Ok(())
        }
        (AuthenticationMethod::SshAgent, _)
        | (AuthenticationMethod::SystemGitCredentialHelper, _) => Ok(()),
        _ => Err(AppError::new(
            "authentication_method_mismatch",
            "该认证方式不适用于所选 Endpoint。",
        )),
    }
}

fn current_account_status(
    account: &AccountProfile,
    endpoints: &[CodeHostEndpoint],
) -> AuthenticationStatus {
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.id == account.endpoint_id);
    match (&account.auth_method, endpoint) {
        (AuthenticationMethod::PersonalAccessToken, _) => personal_access_token_reference(account)
            .map(personal_access_token_status)
            .unwrap_or(AuthenticationStatus::NeedsReauthentication),
        (AuthenticationMethod::SshAgent, _) => ssh_agent_status(),
        (AuthenticationMethod::SystemGitCredentialHelper, _) => system_git_helper_status(),
        (AuthenticationMethod::GhCli, Some(endpoint)) => {
            if git_authority_has_explicit_port(&endpoint.git_host)
                || !github_endpoint_has_gh_compatible_api(endpoint)
            {
                return AuthenticationStatus::Unavailable;
            }
            let Some(hostname) = hostname_from_git_authority(&endpoint.git_host) else {
                return AuthenticationStatus::Unavailable;
            };
            let status = gh_auth_status(&hostname);
            if status.is_usable()
                && status
                    .login
                    .as_deref()
                    .is_some_and(|login| login.eq_ignore_ascii_case(&account.login))
            {
                AuthenticationStatus::Valid
            } else {
                AuthenticationStatus::NeedsReauthentication
            }
        }
        (AuthenticationMethod::GlabCli, Some(endpoint)) => {
            if git_authority_has_explicit_port(&endpoint.git_host) {
                return AuthenticationStatus::Unavailable;
            }
            let Some(hostname) = hostname_from_git_authority(&endpoint.git_host) else {
                return AuthenticationStatus::Unavailable;
            };
            let status = glab_auth_status(&hostname);
            if status.is_usable()
                && status
                    .login
                    .as_deref()
                    .is_some_and(|login| login.eq_ignore_ascii_case(&account.login))
            {
                AuthenticationStatus::Valid
            } else {
                AuthenticationStatus::NeedsReauthentication
            }
        }
        _ => AuthenticationStatus::Unavailable,
    }
}

fn refresh_account_profiles(
    accounts: Vec<AccountProfile>,
    endpoints: &[CodeHostEndpoint],
) -> Vec<AccountProfile> {
    accounts
        .into_iter()
        .map(|mut account| {
            account.status = current_account_status(&account, endpoints);
            account
        })
        .collect()
}

fn merge_refreshed_account_statuses(store: &mut ProfileStore, refreshed: &[AccountProfile]) {
    for account in refreshed {
        // Profiles are immutable after creation. If a user removed one while
        // the background worker ran, silently leave it removed.
        if let Some(existing) = store
            .accounts
            .iter_mut()
            .find(|existing| existing.id == account.id)
        {
            existing.status = account.status.clone();
        }
    }
}

fn system_git_helper_status() -> AuthenticationStatus {
    // Credential helpers can be configured at repository or URL scope, or be
    // supplied by a platform Git integration.  Looking only at global config
    // would incorrectly disable valid profiles.  This status intentionally
    // means "Git may use the user's existing helper"; Git remains the final
    // authority and the application never asks it to reveal a credential.
    AuthenticationStatus::Available
}

fn ssh_agent_status() -> AuthenticationStatus {
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        return AuthenticationStatus::Unavailable;
    }
    // `ssh-add -l` only asks the already configured agent for public key
    // fingerprints; it neither creates keys nor changes the agent state.
    let mut command = Command::new("ssh-add");
    command
        .arg("-l")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => match wait_for_child_status(&mut child, SSH_AGENT_STATUS_TIMEOUT) {
            Some(status) if status.success() => AuthenticationStatus::Available,
            Some(_) | None => AuthenticationStatus::Unavailable,
        },
        // The socket still gives Git a chance to use a platform-specific
        // agent when ssh-add itself is unavailable.
        Err(_) => AuthenticationStatus::Available,
    }
}

/// Wait for a small status probe without allowing a broken agent or an
/// executable replaced on PATH to block the caller indefinitely. The child is
/// reaped on timeout so repeated account refreshes cannot accumulate zombies.
fn wait_for_child_status(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Resolve an explicitly selected clone identity before a repository exists.
/// Leaving both optional inputs empty deliberately retains normal system Git
/// behavior, which is the correct path for public repositories and users who
/// rely on their existing credential helper.
fn clone_environment(
    state: &AppState,
    remote_url: &str,
    destination: &Path,
    endpoint_id: Option<&str>,
    account_profile_id: Option<&str>,
) -> Result<(Option<GitEnvironment>, Option<RemoteBinding>), AppError> {
    let endpoint_id = endpoint_id.map(str::trim).filter(|value| !value.is_empty());
    let account_profile_id = account_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if endpoint_id.is_none() && account_profile_id.is_none() {
        return Ok((None, None));
    }

    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    for account in &mut store.accounts {
        account.status = current_account_status(account, &endpoints);
    }

    let endpoint = match (endpoint_id, account_profile_id) {
        (Some(endpoint_id), _) => endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
            .cloned()
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?,
        (None, Some(account_id)) => {
            let account = store
                .accounts
                .iter()
                .find(|account| account.id == account_id)
                .ok_or_else(|| AppError::new("account_not_found", "找不到要用于 Clone 的账号。"))?;
            endpoints
                .iter()
                .find(|endpoint| endpoint.id == account.endpoint_id)
                .cloned()
                .ok_or_else(|| {
                    AppError::new("endpoint_not_found", "找不到该账号所属的 Endpoint。")
                })?
        }
        (None, None) => unreachable!("empty clone selection is returned above"),
    };

    let selection = account_profile_id
        .map(|account_id| AccountSelection::Profile(account_id.to_owned()))
        .unwrap_or(AccountSelection::Auto);
    let temporary_binding = RemoteBinding {
        repository_path: destination.to_string_lossy().into_owned(),
        remote_name: "origin".into(),
        endpoint_id: endpoint.id.clone(),
        account_selection: selection,
    };
    let resolution = CredentialBroker.resolve(CredentialResolutionRequest {
        repository_path: &temporary_binding.repository_path,
        remote_name: &temporary_binding.remote_name,
        remote_url,
        endpoint: Some(&endpoint),
        endpoints: &endpoints,
        bindings: std::slice::from_ref(&temporary_binding),
        accounts: &store.accounts,
    })?;
    let account = store
        .accounts
        .iter()
        .find(|account| account.id == resolution.account_profile_id)
        .ok_or_else(|| AppError::new("account_not_found", "找不到已解析的账号。"))?;
    let environment = credential_environment_for_resolution(&resolution, account)?;

    Ok((Some(environment), Some(temporary_binding)))
}

fn network_environment(
    state: &AppState,
    repository: &Path,
    remote_name: &str,
    push: bool,
) -> Result<GitEnvironment, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let raw_remote_url = git.remote_url_for_operation(repository, remote_name, push)?;
    let parsed_remote = parse_remote_url(&raw_remote_url);
    // Every remote transport can honor repository-local upload-pack,
    // receive-pack, include, or SSH configuration. Check it before the
    // unauthenticated transport fast path as well.
    git.ensure_network_configuration_is_safe(repository)?;
    // `git://` and local-path remotes carry no account identity.  Preserve
    // normal system Git behavior instead of requiring an artificial Endpoint
    // or credential profile for them.
    if matches!(
        &parsed_remote.transport,
        crate::domain::RemoteTransport::Git | crate::domain::RemoteTransport::Local
    ) {
        validate_unauthenticated_remote_url(&raw_remote_url)?;
        return Ok(GitEnvironment::default());
    }
    // Reject cleartext, malformed, and credential-bearing remote URLs before
    // either a selected profile or the unscoped system helper sees them.
    validate_remote_url_for_credentials(&raw_remote_url, &parsed_remote.transport)?;
    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    for account in &mut store.accounts {
        account.status = current_account_status(account, &endpoints);
    }
    let binding = store.binding_for(&repository_record.path, remote_name);
    let endpoint = binding
        .as_ref()
        .and_then(|binding| store.endpoint(&binding.endpoint_id))
        .or_else(|| {
            parsed_remote
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints))
                .cloned()
        });

    // System credential helpers select credentials by URL and cannot prove
    // which AccountProfile supplied them. Keep them as a normal, unbound Git
    // fallback only when no explicit PAT/SSH profile is available. An
    // explicit Profile binding must always resolve or fail closed.
    let explicit_profile_binding = matches!(
        binding.as_ref().map(|binding| &binding.account_selection),
        Some(AccountSelection::Profile(_))
    );
    if !explicit_profile_binding
        && matches!(
            &parsed_remote.transport,
            crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
        )
    {
        match endpoint.as_ref() {
            Some(endpoint)
                if CredentialBroker::has_explicit_account_candidate(
                    &store.accounts,
                    endpoint,
                    &parsed_remote.transport,
                ) => {}
            Some(endpoint) => {
                if binding.is_some() {
                    validate_codehost_binding_url(&raw_remote_url, endpoint)?;
                }
                return Ok(GitEnvironment::default());
            }
            None if binding.is_none() => return Ok(GitEnvironment::default()),
            None => {}
        }
    }

    let resolution = CredentialBroker.resolve(CredentialResolutionRequest {
        repository_path: &repository_record.path,
        remote_name,
        remote_url: &raw_remote_url,
        endpoint: endpoint.as_ref(),
        endpoints: &endpoints,
        bindings: &store.remote_bindings,
        accounts: &store.accounts,
    })?;
    let account = store
        .accounts
        .iter()
        .find(|account| account.id == resolution.account_profile_id)
        .ok_or_else(|| AppError::new("account_not_found", "找不到已解析的账号。"))?;
    credential_environment_for_resolution(&resolution, account)
}

/// A submodule update is one Git process, so it can carry one selected
/// askpass strategy. Prefer the parent repository's origin (then its first
/// remote); askpass refuses any prompt whose HTTPS host does not match that
/// selected endpoint, preventing a parent credential from crossing hosts.
fn submodule_update_environment(
    state: &AppState,
    repository: &Path,
) -> Result<GitEnvironment, AppError> {
    let remotes = GitCore::new().remotes(repository)?;
    let remote = remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first());
    let Some(remote) = remote else {
        return Ok(GitEnvironment::default());
    };
    network_environment(state, repository, &remote.name, false)
}

fn credential_environment_for_resolution(
    resolution: &crate::domain::CredentialResolution,
    account: &AccountProfile,
) -> Result<GitEnvironment, AppError> {
    let credential_environment = CredentialEnvironment::from_resolution(resolution, account)?;
    let askpass_path = credential_environment
        .askpass_config()
        .is_some()
        .then(std::env::current_exe)
        .transpose()
        .map_err(|error| AppError::io("定位应用 askpass helper", error))?;
    let pairs = credential_environment.environment_pairs(askpass_path.as_deref())?;
    let mut environment = GitEnvironment::new();
    for (name, value) in pairs {
        environment.set(name, value);
    }
    // Every resolved strategy is selected explicitly. Do not let a
    // repository-provided credential helper or HTTP header run alongside it.
    // A PAT askpass capability is a bearer authorization in Git's inherited
    // environment, so hook-capable commands must also disable hooks before
    // the process starts. SSH has no such secret in the environment.
    if matches!(
        &resolution.strategy,
        crate::domain::CredentialStrategy::KeychainPatAskPass { .. }
    ) {
        environment.isolate_network_credentials_and_disable_hooks();
    } else {
        environment.isolate_network_credentials();
    }
    Ok(environment)
}

/// `git://` and local remotes do not use an account profile, but a malformed
/// pre-existing `git://` URL must not become a loophole for credential-like
/// userinfo or token-bearing query data.  SSH user names remain handled by
/// CredentialBroker because they are part of normal SSH addressing.
fn validate_unauthenticated_remote_url(raw_remote_url: &str) -> Result<(), AppError> {
    let Ok(url) = url::Url::parse(raw_remote_url) else {
        return Ok(());
    };
    if matches!(url.scheme(), "git" | "file")
        && (!url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some())
    {
        return Err(AppError::new(
            "unsafe_remote_url",
            "无凭据 Git remote 地址不能包含认证信息或令牌。",
        ));
    }
    Ok(())
}

fn remote_binding_matches_urls(
    store: &ProfileStore,
    repository_path: &str,
    remote_name: &str,
    fetch_url: &str,
    push_url: &str,
) -> bool {
    let Some(binding) = store.binding_for(repository_path, remote_name) else {
        return true;
    };
    let Some(endpoint) = store.endpoint(&binding.endpoint_id) else {
        return false;
    };
    validate_codehost_binding_url(fetch_url, &endpoint).is_ok()
        && validate_codehost_binding_url(push_url, &endpoint).is_ok()
}

/// Pending clone metadata is intentionally replayed after a failed profile
/// write. Never let it recreate a binding whose explicit account was removed
/// while the clone was still in flight.
fn binding_selected_account_is_current(store: &ProfileStore, binding: &RemoteBinding) -> bool {
    match &binding.account_selection {
        AccountSelection::Auto => true,
        AccountSelection::Profile(account_profile_id) => store.accounts.iter().any(|account| {
            account.id == *account_profile_id && account.endpoint_id == binding.endpoint_id
        }),
    }
}

/// A binding is an authorization decision, so only persist it when every
/// operation URL for that remote belongs to the selected endpoint. This keeps
/// unsupported ports, credential-bearing URLs, and cross-host push URLs from
/// becoming durable UI state that later cannot be used safely.
fn validate_codehost_binding_url(
    raw_remote_url: &str,
    endpoint: &CodeHostEndpoint,
) -> Result<(), AppError> {
    let parsed = parse_remote_url(raw_remote_url);
    if !matches!(
        &parsed.transport,
        crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
    ) || !parsed
        .authority()
        .as_deref()
        .is_some_and(|authority| endpoint_matches_authority(endpoint, authority))
    {
        return Err(AppError::new(
            "endpoint_association_required",
            "该 remote 地址与所选代码托管 Endpoint 不匹配。",
        ));
    }

    if let Ok(url) = url::Url::parse(raw_remote_url) {
        let http = matches!(url.scheme(), "http" | "https");
        if url.password().is_some()
            || (http && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::new(
                "unsafe_remote_url",
                "remote 地址包含不受支持的认证信息或令牌。",
            ));
        }
    }
    Ok(())
}

fn github_adapter_context(
    state: &AppState,
    repository_path: &str,
    remote_name: &str,
) -> Result<AdapterContext, AppError> {
    let repository = repository_root(repository_path)?;
    let git = GitCore::new();
    let repository_record = git.repository_record(&repository)?;
    let mut remote = git
        .remotes(&repository)?
        .into_iter()
        .find(|remote| remote.name == remote_name)
        .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
    let raw_remote_url = git.remote_url_for_operation(&repository, remote_name, false)?;
    let push_remote_url = git.remote_url_for_operation(&repository, remote_name, true)?;
    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    for account in &mut store.accounts {
        account.status = current_account_status(account, &endpoints);
    }
    let binding = store.binding_for(&repository_record.path, remote_name);
    let endpoint = binding
        .as_ref()
        .and_then(|binding| store.endpoint(&binding.endpoint_id))
        .or_else(|| {
            parse_remote_url(&raw_remote_url)
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints))
                .cloned()
        })
        .ok_or_else(|| {
            AppError::new(
                "endpoint_association_required",
                "请先为该 remote 选择代码托管 Endpoint。",
            )
        })?;
    if endpoint.kind != CodeHostKind::GitHub
        || validate_codehost_binding_url(&raw_remote_url, &endpoint).is_err()
        || validate_codehost_binding_url(&push_remote_url, &endpoint).is_err()
    {
        return Err(AppError::new(
            "github_adapter_auth_unsupported",
            "GitHub 平台信息需要与当前 gh 登录匹配的账号绑定。",
        ));
    }
    let account = github_cli_account_for_binding(&store, &endpoint, binding.as_ref())?;
    remote.endpoint_id = Some(endpoint.id.clone());
    remote.code_host_kind = endpoint.kind.clone();
    AdapterContext::new(endpoint, remote, account)
}

/// Platform API authentication is deliberately separate from Git credential
/// resolution. gh has one active account and can safely serve its own API only
/// after that active identity matches this selected profile; it is never
/// repurposed as an unpinned Git HTTPS token source.
fn github_cli_account_for_binding(
    store: &ProfileStore,
    endpoint: &CodeHostEndpoint,
    binding: Option<&RemoteBinding>,
) -> Result<AccountProfile, AppError> {
    let candidates: Vec<&AccountProfile> = store
        .accounts
        .iter()
        .filter(|account| {
            account.endpoint_id == endpoint.id
                && account.auth_method == AuthenticationMethod::GhCli
                && account.status == AuthenticationStatus::Valid
        })
        .collect();
    // Current remote bindings authorize Git credentials. New bindings reject
    // CLI profiles, so a PAT/SSH binding must not suppress the independently
    // selected current gh identity used by GitHub's platform API. Legacy CLI
    // bindings remain readable during migration and still take precedence.
    let legacy_cli_binding = binding
        .and_then(|binding| match &binding.account_selection {
            AccountSelection::Profile(account_profile_id) => Some(account_profile_id),
            AccountSelection::Auto => None,
        })
        .and_then(|account_profile_id| {
            candidates
                .iter()
                .copied()
                .find(|account| account.id == *account_profile_id)
        });
    let selected = if legacy_cli_binding.is_some() {
        legacy_cli_binding
    } else if candidates.len() == 1 {
        candidates.first().copied()
    } else {
        let defaults: Vec<&AccountProfile> = candidates
            .into_iter()
            .filter(|account| account.is_default_candidate)
            .collect();
        (defaults.len() == 1).then(|| defaults[0])
    };
    selected.cloned().ok_or_else(|| {
        AppError::new(
            "github_adapter_auth_unsupported",
            "GitHub 平台信息需要与当前 gh 登录匹配的账号绑定。",
        )
    })
}

fn validate_custom_endpoint(endpoint: &CodeHostEndpoint) -> Result<(), AppError> {
    if endpoint.id.trim().is_empty()
        || endpoint.display_name.trim().is_empty()
        || normalize_git_authority(&endpoint.git_host).is_none()
    {
        return Err(AppError::new(
            "invalid_endpoint",
            "Endpoint 标识或 Git host 无效。",
        ));
    }

    if matches!(endpoint.kind, CodeHostKind::Unknown) {
        return Err(AppError::new(
            "invalid_endpoint",
            "请为自定义 Git host 选择具体平台类型或“通用 Git 服务”。",
        ));
    }

    let requires_platform_urls = !matches!(endpoint.kind, CodeHostKind::GenericGit);
    for value in [&endpoint.web_url, &endpoint.api_url] {
        if value.is_empty() {
            if requires_platform_urls {
                return Err(AppError::new(
                    "invalid_endpoint",
                    "该平台 Endpoint 需要网页地址和 API 地址。",
                ));
            }
            continue;
        }
        let url = url::Url::parse(value)
            .map_err(|_| AppError::new("invalid_endpoint", "Endpoint URL 无效。"))?;
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            return Err(AppError::new(
                "invalid_endpoint",
                "Endpoint URL 必须是 HTTP(S) 地址。",
            ));
        }
    }
    if endpoint.kind == CodeHostKind::GitHub
        && !git_authority_has_explicit_port(&endpoint.git_host)
        && !github_endpoint_has_gh_compatible_api(endpoint)
    {
        return Err(AppError::new(
            "invalid_endpoint",
            "GitHub Endpoint 的 API 地址必须与 gh 的 host 路由一致。",
        ));
    }
    Ok(())
}

/// Account auth methods and remote bindings are scoped to an Endpoint's
/// platform type and Git host. Editing cosmetic metadata is safe, but changing
/// that routing identity while it is referenced would make existing bindings
/// silently point at a different service.
fn endpoint_routing_change_is_referenced(
    store: &ProfileStore,
    endpoint: &CodeHostEndpoint,
) -> bool {
    let Some(existing) = store
        .custom_endpoints
        .iter()
        .find(|existing| existing.id == endpoint.id)
    else {
        return false;
    };
    let routing_changed =
        existing.kind != endpoint.kind || !endpoint_matches_authority(existing, &endpoint.git_host);
    routing_changed
        && (store
            .accounts
            .iter()
            .any(|account| account.endpoint_id == endpoint.id)
            || store
                .remote_bindings
                .iter()
                .any(|binding| binding.endpoint_id == endpoint.id))
}

/// Automatic remote routing must have exactly one Endpoint for an authority.
/// Built-ins are included because custom metadata may not shadow a public
/// host such as github.com and silently make its account routing ambiguous.
fn endpoint_git_host_conflicts(store: &ProfileStore, endpoint: &CodeHostEndpoint) -> bool {
    store.all_endpoints().iter().any(|existing| {
        existing.id != endpoint.id && endpoint_matches_authority(existing, &endpoint.git_host)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_validation_rejects_whitespace() {
        assert!(required_login("work account").is_err());
        assert_eq!(required_login(" account ").unwrap(), "account");
    }

    #[test]
    fn generic_endpoint_needs_only_a_safe_git_host() {
        let generic = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        assert!(validate_custom_endpoint(&generic).is_ok());

        let mut port_scoped = generic.clone();
        port_scoped.id = "git.example.test-8443".into();
        port_scoped.git_host = "git.example.test:8443".into();
        assert!(validate_custom_endpoint(&port_scoped).is_ok());

        let mut platform = generic;
        platform.kind = CodeHostKind::GitLab;
        assert_eq!(
            validate_custom_endpoint(&platform).unwrap_err().code(),
            "invalid_endpoint"
        );

        let github = CodeHostEndpoint {
            id: "github.example.test".into(),
            kind: CodeHostKind::GitHub,
            display_name: "Example GitHub".into(),
            web_url: "https://github.example.test".into(),
            api_url: "https://github.example.test/api/v3".into(),
            git_host: "github.example.test".into(),
            is_builtin: false,
        };
        assert!(validate_custom_endpoint(&github).is_ok());

        let mut mismatched_github = github;
        mismatched_github.api_url = "https://api.other.test/api/v3".into();
        assert_eq!(
            validate_custom_endpoint(&mismatched_github)
                .unwrap_err()
                .code(),
            "invalid_endpoint"
        );
    }

    #[test]
    fn endpoint_git_authorities_cannot_shadow_builtin_or_custom_routing() {
        let builtin_collision = CodeHostEndpoint {
            id: "private-github".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Private GitHub".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "github.com".into(),
            is_builtin: false,
        };
        assert!(endpoint_git_host_conflicts(
            &ProfileStore::default(),
            &builtin_collision
        ));

        let port_endpoint = CodeHostEndpoint {
            id: "git.example.test-8443".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test:8443".into(),
            is_builtin: false,
        };
        let store = ProfileStore {
            custom_endpoints: vec![port_endpoint.clone()],
            ..ProfileStore::default()
        };
        let mut duplicate = port_endpoint.clone();
        duplicate.id = "another-id".into();
        assert!(endpoint_git_host_conflicts(&store, &duplicate));

        let mut default_port = duplicate;
        default_port.git_host = "git.example.test".into();
        assert!(!endpoint_git_host_conflicts(&store, &default_port));
    }

    #[test]
    fn referenced_endpoint_cannot_change_routing_identity() {
        let endpoint = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        let store = ProfileStore {
            custom_endpoints: vec![endpoint.clone()],
            accounts: vec![AccountProfile {
                id: "account-1".into(),
                endpoint_id: endpoint.id.clone(),
                login: "alice".into(),
                display_name: None,
                auth_method: AuthenticationMethod::SystemGitCredentialHelper,
                secret_ref: None,
                status: AuthenticationStatus::Available,
                is_default_candidate: false,
            }],
            ..ProfileStore::default()
        };

        let mut renamed = endpoint.clone();
        renamed.display_name = "Renamed only".into();
        assert!(!endpoint_routing_change_is_referenced(&store, &renamed));

        let mut moved = endpoint;
        moved.git_host = "git.other.test".into();
        assert!(endpoint_routing_change_is_referenced(&store, &moved));
    }

    #[test]
    fn custom_generic_endpoint_is_recognized_without_a_binding() {
        let endpoint = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        let store = ProfileStore {
            custom_endpoints: vec![endpoint],
            ..ProfileStore::default()
        };
        let mut remotes = vec![RemoteProfile {
            name: "origin".into(),
            fetch_url: "https://git.example.test/team/project.git".into(),
            push_url: "https://git.example.test/team/project.git".into(),
            parsed: parse_remote_url("https://git.example.test/team/project.git"),
            endpoint_id: None,
            code_host_kind: CodeHostKind::Unknown,
        }];

        enrich_remote_endpoints(&mut remotes, &store, "/work/project");

        assert_eq!(remotes[0].endpoint_id.as_deref(), Some("git.example.test"));
        assert_eq!(remotes[0].code_host_kind, CodeHostKind::GenericGit);
    }

    #[test]
    fn repository_owner_grouping_prefers_origin() {
        let remotes = vec![
            RemoteProfile {
                name: "upstream".into(),
                fetch_url: "https://github.com/upstream/project.git".into(),
                push_url: "https://github.com/upstream/project.git".into(),
                parsed: parse_remote_url("https://github.com/upstream/project.git"),
                endpoint_id: Some("github.com".into()),
                code_host_kind: CodeHostKind::GitHub,
            },
            RemoteProfile {
                name: "origin".into(),
                fetch_url: "git@gitlab.com:team/platform/project.git".into(),
                push_url: "git@gitlab.com:team/platform/project.git".into(),
                parsed: parse_remote_url("git@gitlab.com:team/platform/project.git"),
                endpoint_id: Some("gitlab.com".into()),
                code_host_kind: CodeHostKind::GitLab,
            },
        ];

        let (host, namespace) = repository_owner_from_remotes(&remotes);

        assert_eq!(host.as_deref(), Some("gitlab.com"));
        assert_eq!(namespace.as_deref(), Some("team/platform"));
    }

    #[test]
    fn repository_group_validation_trims_and_rejects_control_characters() {
        assert_eq!(
            normalize_repository_group(Some("  工作项目  ".into())).unwrap(),
            Some("工作项目".into())
        );
        assert_eq!(
            normalize_repository_group(Some("   ".into())).unwrap(),
            None
        );
        assert_eq!(
            normalize_repository_group(Some("工作\n项目".into()))
                .expect_err("line breaks are not valid group names")
                .code(),
            "repository_group_invalid"
        );
    }

    #[test]
    fn unauthenticated_remote_url_cannot_hide_userinfo_or_tokens() {
        let error = validate_unauthenticated_remote_url(
            "git://secret-token@git.example.test/team/project.git?access_token=hidden",
        )
        .expect_err("a git protocol URL must not carry credential-like data");
        assert_eq!(error.code(), "unsafe_remote_url");
        assert!(
            validate_unauthenticated_remote_url("git://git.example.test/team/project.git").is_ok()
        );
        assert!(validate_unauthenticated_remote_url("/srv/git/project.git").is_ok());
    }

    #[test]
    fn remote_binding_requires_both_effective_operation_urls_to_match() {
        let repository_path = "/work/project";
        let store = ProfileStore {
            remote_bindings: vec![RemoteBinding {
                repository_path: repository_path.into(),
                remote_name: "origin".into(),
                endpoint_id: "github.com".into(),
                account_selection: AccountSelection::Auto,
            }],
            ..ProfileStore::default()
        };

        assert!(remote_binding_matches_urls(
            &store,
            repository_path,
            "origin",
            "https://github.com/example/project.git",
            "git@github.com:example/project.git",
        ));
        assert!(!remote_binding_matches_urls(
            &store,
            repository_path,
            "origin",
            "https://gitlab.com/example/project.git",
            "https://github.com/example/project.git",
        ));
        assert!(!remote_binding_matches_urls(
            &store,
            repository_path,
            "origin",
            "https://github.com/example/project.git",
            "https://gitlab.com/example/project.git",
        ));
    }

    #[test]
    fn pending_publish_identity_requires_the_expected_github_slug() {
        assert_eq!(
            github_publish_expected_slug("project", "octo").unwrap(),
            "octo/project"
        );
        assert_eq!(
            github_publish_expected_slug("team/project", "octo").unwrap(),
            "team/project"
        );
        assert!(github_publish_expected_slug("../project", "octo").is_err());
        assert!(pending_publish_remote_matches(
            "https://github.com/octo/project.git",
            "octo/project"
        ));
        assert!(pending_publish_remote_matches(
            "git@github.com:octo/project.git",
            "octo/project"
        ));
        assert!(!pending_publish_remote_matches(
            "https://github.com/octo/other.git",
            "octo/project"
        ));
    }

    #[test]
    fn pending_clone_identity_requires_the_expected_repository_on_both_urls() {
        assert_eq!(
            clone_expected_remote_identity("git@git.example.test:team/project.git").unwrap(),
            ("git.example.test".into(), "team/project".into())
        );
        assert!(pending_clone_remote_matches(
            "https://git.example.test/team/project.git",
            "git.example.test",
            "team/project"
        ));
        assert!(pending_clone_remote_matches(
            "git@git.example.test:team/project.git",
            "git.example.test",
            "team/project"
        ));
        assert!(!pending_clone_remote_matches(
            "https://git.example.test/team/other.git",
            "git.example.test",
            "team/project"
        ));
        assert!(!pending_clone_remote_matches(
            "https://git.example.test:8443/team/project.git",
            "git.example.test",
            "team/project"
        ));
        assert!(clone_expected_remote_identity("/srv/git/project.git").is_err());
    }

    #[test]
    fn pending_clone_binding_requires_its_selected_profile_to_still_exist() {
        let binding = RemoteBinding {
            repository_path: "/work/project".into(),
            remote_name: "origin".into(),
            endpoint_id: "github.com".into(),
            account_selection: AccountSelection::Profile("account-1".into()),
        };
        let mut store = ProfileStore::default();

        assert!(!binding_selected_account_is_current(&store, &binding));

        store.accounts.push(AccountProfile {
            id: "account-1".into(),
            endpoint_id: "gitlab.com".into(),
            login: "alice".into(),
            display_name: None,
            auth_method: AuthenticationMethod::GhCli,
            secret_ref: None,
            status: AuthenticationStatus::Valid,
            is_default_candidate: false,
        });
        assert!(!binding_selected_account_is_current(&store, &binding));

        store.accounts[0].endpoint_id = "github.com".into();
        assert!(binding_selected_account_is_current(&store, &binding));
    }

    #[test]
    fn system_git_helper_is_not_limited_to_global_configuration() {
        assert_eq!(system_git_helper_status(), AuthenticationStatus::Available);
    }

    #[cfg(unix)]
    #[test]
    fn stalled_status_probe_is_killed_before_it_can_block_account_refresh() {
        let mut child = Command::new("sh")
            .args(["-c", "exec sleep 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("test shell starts");
        let started = Instant::now();

        assert!(wait_for_child_status(&mut child, Duration::from_millis(20)).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn background_status_merge_does_not_restore_removed_accounts() {
        let cached = AccountProfile {
            id: "kept".into(),
            endpoint_id: "github.com".into(),
            login: "octo".into(),
            display_name: None,
            auth_method: AuthenticationMethod::GhCli,
            secret_ref: None,
            status: AuthenticationStatus::Unknown,
            is_default_candidate: false,
        };
        let mut refreshed = cached.clone();
        refreshed.status = AuthenticationStatus::Valid;
        let removed_while_refreshing = AccountProfile {
            id: "removed".into(),
            ..refreshed.clone()
        };
        let mut store = ProfileStore {
            accounts: vec![cached],
            ..ProfileStore::default()
        };

        merge_refreshed_account_statuses(&mut store, &[refreshed, removed_while_refreshing]);

        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts[0].id, "kept");
        assert_eq!(store.accounts[0].status, AuthenticationStatus::Valid);
    }
}
