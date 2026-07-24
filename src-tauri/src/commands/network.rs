use super::support::*;
use super::*;

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
