use super::support::*;
use super::*;

fn repository_authorization_path(repository: &Path) -> Result<String, AppError> {
    Ok(GitCore::new().repository_record(repository)?.path)
}

fn repository_network_security_status(
    state: &AppState,
    repository: &Path,
) -> Result<RepositoryNetworkSecurityStatus, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let assessment = git.network_configuration_assessment(repository)?;
    let authorization = state.repository_network_trust(&repository_record.path)?;
    Ok(repository_network_security_status_from(
        repository_record.path,
        assessment,
        authorization,
    ))
}

fn repository_network_security_status_from(
    repository_path: String,
    assessment: NetworkConfigurationAssessment,
    authorization: Option<RepositoryNetworkTrust>,
) -> RepositoryNetworkSecurityStatus {
    let trusted = assessment.authorization_allowed
        && authorization.as_ref().is_some_and(|authorization| {
            assessment.fingerprint.as_deref() == Some(&authorization.fingerprint)
        });
    RepositoryNetworkSecurityStatus {
        repository_path,
        unsafe_keys: assessment.unsafe_keys,
        current_fingerprint: assessment.fingerprint,
        authorization_allowed: assessment.authorization_allowed,
        trusted,
        authorized_keys: authorization
            .as_ref()
            .map(|authorization| authorization.approved_keys.clone())
            .unwrap_or_default(),
        authorized_fingerprint: authorization
            .as_ref()
            .map(|authorization| authorization.fingerprint.clone()),
        authorization_stale: authorization.is_some() && !trusted,
    }
}

#[tauri::command]
pub async fn get_repository_network_security(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryNetworkSecurityStatus, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let canonical = repository_authorization_path(&repository)?;
        let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
        let locked_repository = repository_root(&repository_path)?;
        let locked_canonical = repository_authorization_path(&locked_repository)?;
        if locked_canonical != canonical {
            return Err(AppError::new(
                "repository_identity_changed",
                "仓库身份已变化，请刷新后重试。",
            ));
        }
        repository_network_security_status(&state, &locked_repository)
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库安全状态后台任务未能完成。"))?
}

#[tauri::command]
pub async fn approve_repository_network_configuration(
    repository_path: String,
    expected_fingerprint: String,
    state: State<'_, AppState>,
) -> Result<RepositoryNetworkSecurityStatus, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let canonical = repository_authorization_path(&repository)?;
        let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
        let locked_repository = repository_root(&repository_path)?;
        let locked_canonical = repository_authorization_path(&locked_repository)?;
        if locked_canonical != canonical {
            return Err(AppError::new(
                "repository_identity_changed",
                "仓库身份已变化，请刷新后重试。",
            ));
        }
        let git = GitCore::new();
        let assessment = git.network_configuration_assessment(&locked_repository)?;
        if !assessment.authorization_allowed {
            return Err(AppError::with_safe_details(
                "unsafe_git_config_unapprovable",
                "该仓库的本地 Git 配置无法安全授权。",
                "仓库使用了 include.path 或 includeIf.*.path。外部包含文件可在检查后变化，因此网络操作始终拒绝这类仓库级配置。",
            ));
        }
        let fingerprint = assessment.fingerprint.clone().ok_or_else(|| {
            AppError::new(
                "repository_network_trust_not_required",
                "当前仓库没有需要授权的危险 Git 配置。",
            )
        })?;
        if fingerprint != expected_fingerprint {
            return Err(AppError::new(
                "unsafe_git_config_changed",
                "危险 Git 配置已变化，请重新确认。",
            ));
        }
        let authorization = RepositoryNetworkTrust {
            repository_path: canonical.clone(),
            fingerprint,
            approved_keys: assessment.unsafe_keys.clone(),
        };
        state.mutate(|store| {
            store.upsert_repository_network_trust(authorization.clone());
            Ok(())
        })?;
        Ok(repository_network_security_status_from(
            canonical,
            assessment,
            Some(authorization),
        ))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "仓库安全授权后台任务未能完成。"))?
}

#[tauri::command]
pub async fn revoke_repository_network_trust(
    repository_path: String,
    expected_fingerprint: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let canonical = repository_authorization_path(&repository)?;
        let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
        let locked_repository = repository_root(&repository_path)?;
        let locked_canonical = repository_authorization_path(&locked_repository)?;
        if locked_canonical != canonical {
            return Err(AppError::new(
                "repository_identity_changed",
                "仓库身份已变化，请刷新后重试。",
            ));
        }
        let authorization = state.repository_network_trust(&canonical)?;
        if authorization
            .as_ref()
            .map(|trust| trust.fingerprint.as_str())
            != Some(expected_fingerprint.as_str())
        {
            return Err(AppError::new(
                "repository_network_trust_changed",
                "仓库网络授权已变化，请刷新后重新确认。",
            ));
        }
        state.mutate(|store| {
            store.revoke_repository_network_trust(&canonical);
            Ok(())
        })
    })
    .await
    .map_err(|_| AppError::new("command_failed", "撤销仓库安全授权的后台任务未能完成。"))?
}
