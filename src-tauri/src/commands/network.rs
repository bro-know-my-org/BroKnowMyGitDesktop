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
        let git = git_for_network_operation(&state, &repository)?;
        let refspecs = git.fetch_refspecs_for_remote(&repository, &remote_name)?;
        let target = network_environment(&state, &git, &repository, &remote_name, false)?;
        let remote_url = target
            .urls
            .first()
            .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
        git.fetch_with_environment_at_url(
            &repository,
            &FetchOptions {
                remote: remote_name.clone(),
                refspecs,
                prune: true,
            },
            remote_url,
            &target.environment,
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
        let git = git_for_network_operation(&state, &repository)?;
        let branch = git.current_branch_name(&repository)?;
        let refspec = git.pull_refspec_for_remote(&repository, &remote_name)?;
        let target = network_environment(&state, &git, &repository, &remote_name, false)?;
        let remote_url = target
            .urls
            .first()
            .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
        if git.current_branch_name(&repository)? != branch {
            return Err(AppError::new(
                "repository_changed",
                "当前分支已变化，请重新执行拉取。",
            ));
        }
        git.pull_with_environment_at_url(
            &repository,
            &PullOptions {
                remote: remote_name.clone(),
                refspec: Some(refspec),
            },
            remote_url,
            &target.environment,
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
    include_tags: Option<bool>,
    state: State<'_, AppState>,
) -> Result<NetworkOperationResult, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let _repository_operation_lease =
            state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
        let git = git_for_network_operation(&state, &repository)?;
        let target = network_environment(&state, &git, &repository, &remote_name, true)?;
        let (refspecs, set_upstream) = git.push_plan_for_remote(&repository, &remote_name)?;
        git.push_with_environment_at_urls(
            &repository,
            &PushOptions {
                remote: remote_name.clone(),
                refspecs,
                set_upstream,
                force_with_lease: force_with_lease.unwrap_or(false),
                follow_tags: false,
                include_tags: include_tags.unwrap_or(false),
            },
            &target.urls,
            &target.environment,
        )?;
        Ok(NetworkOperationResult::completed(format!(
            "已推送到 {remote_name}。"
        )))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "Git 后台任务未能完成。"))?
}
