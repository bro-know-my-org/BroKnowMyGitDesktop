use super::support::*;
use super::*;

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
