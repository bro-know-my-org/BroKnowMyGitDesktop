use super::support::*;
use super::*;

#[tauri::command]
pub fn get_remote_binding(
    repository_path: String,
    remote_name: String,
    state: State<'_, AppState>,
) -> Result<Option<RemoteBinding>, AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy();
    Ok(state.snapshot()?.binding_for(&canonical, &remote_name))
}

#[tauri::command]
pub fn save_remote_binding(
    binding: RemoteBinding,
    state: State<'_, AppState>,
) -> Result<RemoteBinding, AppError> {
    let repository = repository_root(&binding.repository_path)?;
    let canonical_path = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical_path)?;
    let git = GitCore::new();
    if !git
        .remotes(&repository)?
        .iter()
        .any(|remote| remote.name == binding.remote_name)
    {
        return Err(AppError::new(
            "remote_not_found",
            "仓库中不存在要绑定的 remote。",
        ));
    }
    let fetch_urls = git.remote_urls_for_operation(&repository, &binding.remote_name, false)?;
    let push_urls = git.remote_urls_for_operation(&repository, &binding.remote_name, true)?;

    let mut binding = binding;
    binding.repository_path = canonical_path;
    state.mutate(|store| {
        let endpoint = store
            .endpoint(&binding.endpoint_id)
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?;
        if fetch_urls
            .iter()
            .chain(push_urls.iter())
            .any(|url| validate_codehost_binding_url(url, &endpoint).is_err())
        {
            return Err(AppError::new(
                "endpoint_association_required",
                "该 remote 地址与所选代码托管 Endpoint 不匹配。",
            ));
        }
        if let AccountSelection::Profile(account_profile_id) = &binding.account_selection {
            let account = store
                .accounts
                .iter()
                .find(|account| account.id == *account_profile_id)
                .ok_or_else(|| AppError::new("account_not_found", "找不到要绑定的账号。"))?;
            if account.endpoint_id != endpoint.id {
                return Err(AppError::new(
                    "binding_endpoint_mismatch",
                    "账号不属于该 remote 绑定的 Endpoint。",
                ));
            }
            if matches!(
                account.auth_method,
                AuthenticationMethod::SshAgent
                    |
                AuthenticationMethod::GhCli
                    | AuthenticationMethod::GlabCli
                    | AuthenticationMethod::SystemGitCredentialHelper
            ) {
                return Err(AppError::new(
                    "authentication_method_mismatch",
                    "此认证方式无法安全固定到指定账号；Git 网络操作请绑定 PAT，SSH agent 请由 Git/SSH 配置选择。",
                ));
            }
        }
        store.upsert_binding(binding.clone());
        Ok(binding)
    })
}

#[tauri::command]
pub fn get_codehost_capabilities(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<CodeHostCapabilities, AppError> {
    Ok(CodeHostCapabilities::for_endpoint(&endpoint_from_state(
        &state,
        &endpoint_id,
    )?))
}

#[tauri::command]
pub fn list_endpoints(state: State<'_, AppState>) -> Result<Vec<CodeHostEndpoint>, AppError> {
    Ok(state.snapshot()?.all_endpoints())
}

#[tauri::command]
pub fn save_custom_endpoint(
    mut endpoint: CodeHostEndpoint,
    state: State<'_, AppState>,
) -> Result<CodeHostEndpoint, AppError> {
    endpoint.id = endpoint.id.trim().to_ascii_lowercase();
    endpoint.display_name = endpoint.display_name.trim().to_owned();
    endpoint.web_url = endpoint.web_url.trim().to_owned();
    endpoint.api_url = endpoint.api_url.trim().to_owned();
    endpoint.git_host = normalize_git_authority(&endpoint.git_host)
        .ok_or_else(|| AppError::new("invalid_endpoint", "Endpoint 标识或 Git host 无效。"))?;
    endpoint.is_builtin = false;
    validate_custom_endpoint(&endpoint)?;
    // Account status probes derive CLI compatibility from Endpoint fields
    // such as `api_url`. Change those fields and advance the generation as
    // one operation so a probe using the old configuration cannot commit.
    state.mutate_and_invalidate_account_status_refreshes(|store| {
        if CodeHostEndpoint::builtin()
            .iter()
            .any(|builtin| builtin.id == endpoint.id)
        {
            return Err(AppError::new(
                "builtin_endpoint_immutable",
                "内建 Endpoint 不能被覆盖。",
            ));
        }
        if endpoint_git_host_conflicts(store, &endpoint) {
            return Err(AppError::new(
                "endpoint_git_host_conflict",
                "Git host 已由另一个 Endpoint 使用；请使用唯一的主机或端口。",
            ));
        }
        if endpoint_routing_change_is_referenced(store, &endpoint) {
            return Err(AppError::new(
                "endpoint_in_use_immutable",
                "已有账号或 remote 绑定正在使用该 Endpoint；请先迁移或删除这些关联后再修改平台类型或 Git host。",
            ));
        }

        if let Some(existing) = store
            .custom_endpoints
            .iter_mut()
            .find(|existing| existing.id == endpoint.id)
        {
            *existing = endpoint.clone();
        } else {
            store.custom_endpoints.push(endpoint.clone());
        }
        Ok(endpoint)
    })
}
