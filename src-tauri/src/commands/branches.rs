use super::support::*;
use super::*;

#[tauri::command]
pub fn create_branch(
    repository_path: String,
    name: String,
    checkout: bool,
    stash_local_changes: Option<bool>,
    state: State<'_, AppState>,
) -> Result<CreateBranchResult, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    validate_ref_name(&name, "分支名")?;
    if git
        .branches(&repository)?
        .iter()
        .any(|branch| branch.name == name)
    {
        return Err(AppError::new("branch_already_exists", "该分支已存在。"));
    }
    let stashed_local_changes = if checkout {
        stash_local_changes_before_branch_switch(
            &git,
            &repository,
            stash_local_changes.unwrap_or(false),
        )?
    } else {
        false
    };
    let result = if checkout {
        git.create_and_switch_branch(&repository, &name, None)
    } else {
        git.create_branch(&repository, &name, None)
    };
    if let Err(error) = result {
        return Err(branch_switch_error_after_stash(
            error,
            stashed_local_changes,
        ));
    }
    let snapshot = snapshot_for(&state, &repository)
        .map_err(|error| branch_switch_snapshot_error_after_stash(error, stashed_local_changes))?;
    Ok(CreateBranchResult {
        snapshot,
        stashed_local_changes,
    })
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
    let branches = git.branches(&repository)?;
    let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
        return Err(AppError::new("branch_not_found", "找不到该分支。"));
    };
    // A direct IPC request can target the already checked-out branch even if
    // the UI normally hides that action. Do not stash changes for a switch
    // that Git would treat as a successful no-op.
    if target.is_current {
        return snapshot_for(&state, &repository);
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
        .map_err(|error| branch_switch_snapshot_error_after_stash(error, stashed_local_changes))
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
    if git.is_current_remote_branch_target(&repository, &remote_name, &branch)? {
        return snapshot_for(&state, &repository);
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
        .map_err(|error| branch_switch_snapshot_error_after_stash(error, stashed_local_changes))
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
