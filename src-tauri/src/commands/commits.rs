use super::support::*;
use super::*;
use crate::domain::{CommitChangedFile, CommitFileDiff};

const DEFAULT_COMMIT_FILE_DIFF_BYTES: usize = 512 * 1024;
const MIN_COMMIT_FILE_DIFF_BYTES: usize = 64 * 1024;
const MAX_COMMIT_FILE_DIFF_BYTES: usize = 4 * 1024 * 1024;

#[tauri::command]
pub async fn list_commits(
    repository_path: String,
    limit: Option<usize>,
) -> Result<Vec<CommitSummary>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().commits(&repository, limit.unwrap_or(100))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "提交历史后台任务未能完成。"))?
}

#[tauri::command]
pub async fn get_commit_diff(
    repository_path: String,
    revision: String,
) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().commit_diff(&repository, &revision)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "提交差异后台任务未能完成。"))?
}

#[tauri::command]
pub async fn list_commit_changed_files(
    repository_path: String,
    revision: String,
) -> Result<Vec<CommitChangedFile>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().commit_changed_files(&repository, &revision)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "提交文件后台任务未能完成。"))?
}

#[tauri::command]
pub async fn get_commit_file_diff(
    repository_path: String,
    revision: String,
    path: String,
    previous_path: Option<String>,
    maximum_bytes: Option<usize>,
) -> Result<CommitFileDiff, AppError> {
    let maximum_bytes = maximum_bytes.unwrap_or(DEFAULT_COMMIT_FILE_DIFF_BYTES);
    if !(MIN_COMMIT_FILE_DIFF_BYTES..=MAX_COMMIT_FILE_DIFF_BYTES).contains(&maximum_bytes) {
        return Err(AppError::new(
            "invalid_diff_limit",
            "提交文件差异读取范围无效。",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().commit_file_diff_preview(
            &repository,
            &revision,
            &path,
            previous_path.as_deref(),
            maximum_bytes,
        )
    })
    .await
    .map_err(|_| AppError::new("command_failed", "提交文件差异后台任务未能完成。"))?
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
    path: String,
    patch: String,
    reverse: bool,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().apply_patch(&repository, &path, &patch, reverse)?;
    snapshot_for(&state, &repository)
}

#[tauri::command]
pub fn preview_discard_changes(
    repository_path: String,
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<DiscardChangesPreview, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let git = GitCore::new();
    let fingerprint = git.capture_discard_changes_fingerprint(&repository, &paths)?;
    // Refuse to issue a token if the selected state changed while the preview
    // was being assembled.
    git.validate_discard_changes_fingerprint(&repository, &fingerprint)?;
    let path_count = fingerprint.path_count();
    let untracked_count = fingerprint.untracked_count();
    #[cfg(not(target_os = "linux"))]
    if path_count > untracked_count {
        return Err(AppError::new(
            "discard_protected_unsupported",
            "当前平台暂不支持安全丢弃已跟踪改动。",
        ));
    }
    let confirmation =
        state.issue_discard_changes_confirmation(&repository.to_string_lossy(), fingerprint)?;
    Ok(DiscardChangesPreview {
        confirmation,
        path_count,
        untracked_count,
    })
}

#[tauri::command]
pub fn discard_changes(
    repository_path: String,
    confirmation: String,
    state: State<'_, AppState>,
) -> Result<RepositorySnapshot, AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    let fingerprint =
        state.take_discard_changes_confirmation(&repository.to_string_lossy(), &confirmation)?;
    GitCore::new().discard_paths_with_fingerprint(&repository, &fingerprint)?;
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
