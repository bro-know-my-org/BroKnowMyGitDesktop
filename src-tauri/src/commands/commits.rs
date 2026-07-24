use super::support::*;
use super::*;

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
