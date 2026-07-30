use super::support::*;
use super::*;

#[tauri::command]
pub async fn list_worktrees(repository_path: String) -> Result<Vec<WorktreeInfo>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().worktrees(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "工作树后台任务未能完成。"))?
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
    let worktree_path = required_path(&worktree_path, "工作树路径")?;
    let create_branch = create_branch.and_then(non_empty_trimmed);
    let commitish = commitish.and_then(non_empty_trimmed);
    let git = GitCore::new();
    // Linked worktrees share refs and administrative state. Acquiring only
    // the worktree from which this command was launched would let a sibling
    // mutate that shared state concurrently. Include the future destination
    // as well, so a later operation addressed through that path is covered.
    let mut lease_paths = git
        .worktrees(&repository)?
        .into_iter()
        .map(|worktree| worktree.path)
        .collect::<Vec<_>>();
    lease_paths.push(repository.to_string_lossy().into_owned());
    lease_paths.push(clone_destination_key(&worktree_path)?);
    let lease_path_refs = lease_paths.iter().map(String::as_str).collect::<Vec<_>>();
    let _repository_operation_lease =
        state.acquire_repository_operation_leases(&lease_path_refs)?;
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
    // Linked worktrees share Git refs and administrative state. Hold every
    // currently known worktree key so a mutation launched from a sibling
    // cannot race removal through its own otherwise-distinct path lease.
    let mut lease_paths = git
        .worktrees(&repository)?
        .into_iter()
        .map(|worktree| worktree.path)
        .collect::<Vec<_>>();
    lease_paths.push(canonical);
    lease_paths.push(removed_repository.path.clone());
    let lease_path_refs = lease_paths.iter().map(String::as_str).collect::<Vec<_>>();
    let _repository_operation_lease =
        state.acquire_repository_operation_leases(&lease_path_refs)?;
    git.remove_worktree(&repository, &worktree_path, force)?;
    state.mutate(|store| {
        store.forget_repository_path(&removed_repository.path);
        Ok(())
    })?;
    git.worktrees(&repository)
}

#[tauri::command]
pub async fn list_submodules(repository_path: String) -> Result<Vec<SubmoduleInfo>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().submodules(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "子模块读取后台任务未能完成。"))?
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
        let git = GitCore::new();
        let mut lease_paths = git.submodule_operation_repository_paths(&repository, recursive)?;
        // Linked worktrees share the parent repository's config and module
        // administration. Reserve every sibling root as well as the module
        // destinations, so an update started from one worktree cannot race an
        // update initiated from another.
        lease_paths.extend(
            git.worktrees(&repository)?
                .into_iter()
                .map(|worktree| worktree.path),
        );
        lease_paths.sort_unstable();
        lease_paths.dedup();
        let lease_path_refs = lease_paths.iter().map(String::as_str).collect::<Vec<_>>();
        let _repository_operation_lease =
            state.acquire_repository_operation_leases(&lease_path_refs)?;
        git.update_submodules(&repository, initialize, recursive)?;
        git.submodules(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "子模块后台任务未能完成。"))?
}

#[tauri::command]
pub async fn get_lfs_status(repository_path: String) -> Result<LfsStatus, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().lfs_status(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git LFS 状态后台任务未能完成。"))?
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
pub async fn list_stashes(repository_path: String) -> Result<Vec<StashEntry>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().stashes(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "暂存栈后台任务未能完成。"))?
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
    expected_commit_id: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stash_pop(&repository, &reference, &expected_commit_id)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn drop_stash(
    repository_path: String,
    reference: String,
    expected_commit_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().stash_drop(&repository, &reference, &expected_commit_id)
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
