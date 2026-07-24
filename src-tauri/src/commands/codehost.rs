use super::support::*;
use super::*;

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
