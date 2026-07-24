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
        // Hold this before inspecting account metadata so a concurrent account
        // removal cannot delete the durable publish intent after `gh` starts.
        let _account_operation_lease =
            state.acquire_account_operation_lease(&account_profile_id)?;
        if reconcile_pending_publish(&state, &repository, &endpoint_id, &account_profile_id)? {
            return snapshot_for(&state, &repository);
        }
        let git = GitCore::new();
        let repository_record = git.repository_record(&repository)?;
        // A previous `gh repo create --push` can fail before creating either
        // the GitHub repository or local `origin`. `reconcile_pending_publish`
        // above already handled every locally provable completion; when there
        // is still no origin, this explicit user retry safely replaces the
        // stale intent. GitHub enforces repository-name uniqueness, so an
        // ambiguous remote-side creation cannot become a duplicate publish.
        if let Some(pending) = state
            .snapshot()?
            .pending_publish_for(&repository_record.path)
        {
            state.mutate(|store| {
                if store.pending_publish_for(&repository_record.path).as_ref() != Some(&pending) {
                    return Err(AppError::new(
                        "github_publish_reconciliation_required",
                        "待完成的 GitHub 发布状态已变化，请刷新后重试。",
                    ));
                }
                store.clear_pending_publish(&repository_record.path);
                Ok(())
            })?;
        }
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
            store.queue_pending_publish(pending.clone());
            Ok(())
        })?;

        // `gh repo create --push` can create the remote through GitHub's API
        // before a later local-origin or push step fails. The absence of a
        // local origin therefore proves nothing about the remote side effect;
        // retain the durable intent for every post-invocation failure so a
        // retry can reconcile instead of attempting a duplicate publish.
        GitHubAdapter.publish_repository(
            &endpoint,
            &repository,
            &name,
            &account.login,
            is_private,
        )?;
        let origin_exists = git
            .remotes(&repository)?
            .iter()
            .any(|remote| remote.name == "origin");
        if !origin_exists {
            return Err(AppError::new(
                "github_publish_reconciliation_required",
                "GitHub 已返回成功，但本地未找到 origin remote；发布状态已保留，请检查仓库配置后重试。",
            ));
        }
        if !reconcile_pending_publish(&state, &repository, &endpoint_id, &account_profile_id)? {
            return Err(AppError::new(
                "github_publish_reconciliation_required",
                "GitHub 已返回成功，但发布状态尚未完成确认；请检查仓库配置后重试。",
            ));
        }
        snapshot_for(&state, &repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}
