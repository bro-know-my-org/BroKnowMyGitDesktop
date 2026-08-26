use super::support::repository_root;
use super::*;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRepositoryGitDirectoriesResult {
    opened_count: usize,
    failed_count: usize,
}

#[tauri::command]
pub async fn list_repository_locks(
    repository_path: String,
) -> Result<RepositoryLocksResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        GitCore::new().repository_locks(&repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 锁文件检查任务未能完成。"))?
}

#[tauri::command]
pub async fn open_repository_lock_directory(
    repository_path: String,
    lock_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let lock_directory = {
            let _repository_operation_lease =
                state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
            GitCore::new().repository_lock_directory(&repository, &lock_id)?
        };
        tauri_plugin_opener::open_path(lock_directory, None::<String>)
            .map_err(|_| AppError::new("command_failed", "无法打开 Git 元数据目录。"))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "文件管理器后台任务未能完成。"))?
}

#[tauri::command]
pub async fn open_repository_git_directories(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<OpenRepositoryGitDirectoriesResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let directories = {
            let _repository_operation_lease =
                state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
            GitCore::new().repository_metadata_directories(&repository)?
        };
        let mut opened_count = 0;
        let mut failed_count = 0;
        for directory in directories {
            if tauri_plugin_opener::open_path(directory, None::<String>).is_ok() {
                opened_count += 1;
            } else {
                failed_count += 1;
            }
        }
        if opened_count > 0 {
            Ok(OpenRepositoryGitDirectoriesResult {
                opened_count,
                failed_count,
            })
        } else {
            Err(AppError::new("command_failed", "无法打开 Git 元数据目录。"))
        }
    })
    .await
    .map_err(|_| AppError::new("command_failed", "文件管理器后台任务未能完成。"))?
}
