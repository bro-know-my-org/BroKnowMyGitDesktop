use super::support::*;
use super::*;

struct TemporaryPublishSource(PathBuf);

impl TemporaryPublishSource {
    fn clone_from(git: &GitCore, repository: &Path) -> Result<Self, AppError> {
        let root =
            std::env::temp_dir().join(format!("broknowmygitdesktop-publish-{}", Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&root)
            .map_err(|error| AppError::io("创建隔离发布目录", error))?;
        let source = root.join("source");
        let result = (|| {
            let _configuration_lock = git.lock_network_configuration(repository)?;
            git.ensure_network_configuration_is_safe(repository)?;
            let mut environment = GitEnvironment::new();
            environment.isolate_network_credentials_and_disable_hooks();
            let repository_path = repository.to_str().ok_or_else(|| {
                AppError::new(
                    "unsupported_repository_path_encoding",
                    "仓库路径无法用于隔离发布。",
                )
            })?;
            git.clone_repository_with_environment(
                repository_path,
                &source,
                &CloneOptions::default(),
                &environment,
            )?;
            // gh must create its own remote inside this disposable clone; the
            // source repository path must never remain a candidate push URL.
            git.remove_remote(&source, "origin")?;
            Ok(Self(source))
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&root);
        }
        result
    }
}

impl Drop for TemporaryPublishSource {
    fn drop(&mut self) {
        if let Some(root) = self.0.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[tauri::command]
pub async fn list_github_publish_owners(
    endpoint_id: String,
    account_profile_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<GitHubPublishOwner>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _account_operation_lease =
            state.acquire_account_operation_lease(&account_profile_id)?;
        let store = state.snapshot()?;
        let endpoint = store
            .endpoint(&endpoint_id)
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?;
        if endpoint.kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "只有 GitHub Endpoint 支持查询发布所有者。",
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
                "查询 GitHub 发布所有者需要与 Endpoint 匹配的 gh CLI 账号。",
            ));
        }
        GitHubAdapter.list_publish_owners_until(
            &endpoint,
            &account.login,
            GitHubAdapter::operation_deadline(),
        )
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

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
pub async fn fork_github_repository(
    repository_path: String,
    remote_name: String,
    endpoint_id: String,
    account_profile_id: String,
    fork_remote_name: String,
    state: State<'_, AppState>,
) -> Result<GitHubForkSummary, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let canonical = repository.to_string_lossy().into_owned();
        let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
        let _account_operation_lease =
            state.acquire_account_operation_lease(&account_profile_id)?;
        let git = GitCore::new();
        let remote = git
            .remotes(&repository)?
            .into_iter()
            .find(|remote| remote.name == remote_name)
            .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
        let fetch_urls = git.remote_urls_for_operation(&repository, &remote_name, false)?;
        let push_urls = git.remote_urls_for_operation(&repository, &remote_name, true)?;
        let store = state.snapshot()?;
        let endpoint = store
            .endpoint(&endpoint_id)
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?;
        if endpoint.kind != CodeHostKind::GitHub
            || !codehost_adapter_urls_match_endpoint(&fetch_urls, &push_urls, &endpoint)
        {
            return Err(AppError::new(
                "github_adapter_auth_unsupported",
                "GitHub 平台信息需要与当前 gh 登录匹配的账号绑定。",
            ));
        }
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == account_profile_id)
            .cloned()
            .ok_or_else(|| AppError::new("account_not_found", "找不到用于 Fork 的账号。"))?;
        if account.endpoint_id != endpoint.id || account.auth_method != AuthenticationMethod::GhCli
        {
            return Err(AppError::new(
                "github_fork_auth_unsupported",
                "Fork GitHub 仓库需要与 Endpoint 匹配的 gh CLI 账号。",
            ));
        }
        let fork_remote_name = fork_remote_name.trim();
        validate_ref_name(fork_remote_name, "远程名称")
            .map_err(|_| AppError::new("invalid_remote_name", "远程名称无效。"))?;
        if fork_remote_name == remote.name {
            return Err(AppError::new(
                "remote_already_exists",
                "Fork remote 名称不能和源 remote 相同。",
            ));
        }
        let mut context_remote = remote.clone();
        context_remote.endpoint_id = Some(endpoint.id.clone());
        context_remote.code_host_kind = endpoint.kind.clone();
        let context = AdapterContext::new(endpoint.clone(), context_remote, account.clone())?;
        let adapter = GitHubAdapter;
        let mut fork = adapter.fork_repository(&context, fork_remote_name)?;
        let fork_url = if matches!(remote.parsed.transport, crate::domain::RemoteTransport::Ssh) {
            fork.ssh_url.clone()
        } else {
            fork.clone_url.clone()
        };
        validate_codehost_binding_url(&fork_url, &endpoint)?;
        let existing_remote = git
            .remotes(&repository)?
            .into_iter()
            .find(|candidate| candidate.name == fork_remote_name);
        match existing_remote {
            Some(existing) => {
                let existing_url = git
                    .remote_urls_for_operation(&repository, &existing.name, false)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        AppError::new("remote_not_found", "找不到已有 Fork remote 的地址。")
                    })?;
                let existing_push_url = git
                    .remote_urls_for_operation(&repository, &existing.name, true)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        AppError::new("remote_not_found", "找不到已有 Fork remote 的推送地址。")
                    })?;
                if existing_url != fork_url || existing_push_url != fork_url {
                    return Err(AppError::new(
                        "remote_already_exists",
                        "仓库中已存在同名 remote，请换一个 Fork remote 名称。",
                    ));
                }
            }
            None => git.add_remote(&repository, fork_remote_name, &fork_url)?,
        }
        fork.remote_name = fork_remote_name.to_owned();
        fork.source_remote_name = remote.name;
        Ok(fork)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "GitHub 后台任务未能完成。"))?
}

#[tauri::command]
pub async fn publish_github_repository(
    repository_path: String,
    endpoint_id: String,
    account_profile_id: String,
    owner_login: String,
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
        let git = GitCore::new();
        let repository_record = git.repository_record(&repository)?;
        // `reconcile_pending_publish` returns false for a matching durable
        // intent only when local origin is still absent. Keep that intent so
        // the remote create/push can be retried idempotently.
        let existing_pending = state
            .snapshot()?
            .pending_publish_for(&repository_record.path);
        let existing_remotes = git.remotes(&repository)?;
        if !git.has_head(&repository)? {
            return Err(AppError::new(
                "nothing_to_publish",
                "请先创建至少一个本地提交，再发布到 GitHub。",
            ));
        }
        if git.branch_history_contains_lfs_pointers(&repository)? {
            return Err(AppError::new(
                "github_publish_lfs_unsupported",
                "当前安全发布流程暂不支持包含 Git LFS 指针的仓库。",
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
        let deadline = GitHubAdapter::operation_deadline();
        let validated_owner = GitHubAdapter.validate_publish_owner_until(
            &endpoint,
            &account.login,
            &owner_login,
            deadline,
        )?;
        let expected_repository_slug =
            github_publish_expected_slug(validated_owner.login(), &name)?;
        let expected_head = git.head_commit_id(&repository)?;
        let branch = git.current_branch_name(&repository)?;
        if let Some(existing) = &existing_pending {
            if !pending_publish_matches_requested_identity(
                existing,
                &endpoint_id,
                &account_profile_id,
                &expected_repository_slug,
            ) || existing.expected_head != expected_head
                || existing.remote_repository_id.is_none()
            {
                return Err(AppError::new(
                    "github_publish_reconciliation_required",
                    "待完成的 GitHub 发布缺少可验证的远端身份或已变化，请手动检查后重试。",
                ));
            }
        } else if !existing_remotes.is_empty() {
            return Err(AppError::new(
                "repository_already_has_remote",
                "当前仓库已有 remote，不能使用发布流程覆盖它。",
            ));
        }
        let pending = PendingPublish {
            repository_path: repository_record.path.clone(),
            endpoint_id: endpoint.id.clone(),
            account_profile_id: account.id.clone(),
            remote_name: "origin".into(),
            expected_head,
            expected_repository_slug: expected_repository_slug.clone(),
            remote_repository_id: existing_pending
                .as_ref()
                .and_then(|pending| pending.remote_repository_id.clone()),
        };
        if existing_pending.is_none() {
            git.ensure_network_configuration_is_safe(&repository)?;
            state.mutate(|store| {
                store.queue_pending_publish(pending.clone());
                Ok(())
            })?;
        }

        // gh receives a disposable clone whose config was created while the
        // source config lock was held. Its Git child therefore cannot consume
        // repository-controlled configuration changed after preflight.
        let publish_target = GitHubAdapter.prepare_publish_repository(
            &endpoint,
            &name,
            &validated_owner,
            is_private,
            pending.remote_repository_id.as_deref(),
            deadline,
        )?;
        let mut created_pending = pending.clone();
        created_pending.remote_repository_id = Some(publish_target.repository_id().to_owned());
        if created_pending != pending {
            let persist_result = state.mutate_with_outcome(|store| {
                if store.pending_publish_for(&repository_record.path).as_ref() != Some(&pending) {
                    return Err(AppError::new(
                        "github_publish_reconciliation_required",
                        "待完成的 GitHub 发布状态已变化，请刷新后重试。",
                    ));
                }
                store.queue_pending_publish(created_pending.clone());
                Ok(())
            });
            if let Err(failure) = persist_result {
                if failure.committed() {
                    return Err(failure.into_error());
                }
                let error = failure.into_error();
                let rollback_failed = publish_target.was_created()
                    && GitHubAdapter
                        .rollback_created_publish_repository(
                            &endpoint,
                            &validated_owner,
                            &publish_target,
                            deadline,
                        )
                        .is_err();
                let cleanup_failed = !rollback_failed
                    && state
                        .mutate(|store| {
                            if store.pending_publish_for(&repository_record.path).as_ref()
                                == Some(&pending)
                            {
                                store.clear_pending_publish(&repository_record.path);
                            }
                            Ok(())
                        })
                        .is_err();
                if rollback_failed || cleanup_failed {
                    return Err(AppError::with_safe_details(
                        "github_publish_reconciliation_required",
                        "GitHub 仓库已创建，但本地状态保存和自动回滚均失败。",
                        "请在 GitHub 检查并删除刚创建的同名仓库，再重试发布。",
                    ));
                }
                return Err(error);
            }
        }
        if existing_remotes.iter().any(|remote| remote.name == "origin")
            && reconcile_pending_publish(
                &state,
                &repository,
                &endpoint_id,
                &account_profile_id,
                &expected_repository_slug,
            )?
        {
            return snapshot_for(&state, &repository);
        }
        git.ensure_network_configuration_is_safe(&repository)?;
        let publish_source = TemporaryPublishSource::clone_from(&git, &repository)?;
        if git.head_commit_id(&publish_source.0)? != pending.expected_head
            || git.current_branch_name(&repository)? != branch
            || git.head_commit_id(&repository)? != pending.expected_head
        {
            return Err(AppError::new(
                "github_publish_reconciliation_required",
                "发布期间本地分支或提交已变化，请检查后重试。",
            ));
        }
        GitHubAdapter.push_publish_repository(
            &endpoint,
            &publish_source.0,
            validated_owner,
            &publish_target,
            &branch,
            deadline,
        )?;
        let published_remote_url = publish_target.remote_url().to_owned();
        validate_git_remote_url(&published_remote_url)?;
        git.add_remote(&repository, "origin", &published_remote_url)?;
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
        git.ensure_network_configuration_is_safe(&repository)?;
        if !reconcile_pending_publish(
            &state,
            &repository,
            &endpoint_id,
            &account_profile_id,
            &expected_repository_slug,
        )? {
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
