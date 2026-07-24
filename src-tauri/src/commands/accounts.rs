use super::support::*;
use super::*;

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountProfile>, AppError> {
    // Account status is persisted after a background refresh. Returning that
    // cache keeps the Accounts route responsive even when a CLI, keychain, or
    // SSH agent is slow to answer.
    Ok(state.snapshot()?.accounts)
}

/// Refresh status checks away from the route-loading command. The blocking
/// checks deliberately run without holding the persistent-store mutex, then
/// the resulting safe status values are merged and atomically persisted.
#[tauri::command]
pub async fn refresh_account_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    let refresh_generation = state.begin_account_status_refresh()?;
    let snapshot = state.snapshot()?;
    let endpoints = snapshot.all_endpoints();
    let accounts = snapshot.accounts;
    let refreshed = tauri::async_runtime::spawn_blocking(move || {
        refresh_account_profiles(accounts, &endpoints)
    })
    .await
    .map_err(|_| {
        AppError::new(
            "account_status_refresh_failed",
            "账号认证状态刷新任务未能完成。",
        )
    })?;

    match state.commit_account_status_refresh(refresh_generation, |store| {
        merge_refreshed_account_statuses(store, &refreshed);
        Ok(store.accounts.clone())
    })? {
        Some(accounts) => Ok(accounts),
        // A newer refresh or account import completed while this blocking
        // probe ran. Return current metadata rather than restoring stale
        // statuses from the earlier snapshot.
        None => Ok(state.snapshot()?.accounts),
    }
}

/// The PAT is one-way input: this command stores it in the system keychain and never returns it.
#[tauri::command]
pub fn save_pat_account(
    endpoint_id: String,
    login: String,
    display_name: Option<String>,
    token: String,
    state: State<'_, AppState>,
) -> Result<AccountProfile, AppError> {
    let login = required_login(&login)?;
    let endpoint = endpoint_from_state(&state, &endpoint_id)?;
    if !CodeHostCapabilities::for_endpoint(&endpoint).pat_keychain {
        return Err(AppError::new(
            "pat_not_supported",
            "该代码托管 Endpoint 尚未启用应用管理的 PAT。",
        ));
    }
    let token = token.trim().to_owned();
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(AppError::new("token_required", "请输入个人访问令牌。"));
    }

    let endpoint_id = endpoint.id;
    let id = format!("account-{}", Uuid::new_v4());
    let account_id = id.clone();
    let secret_ref = default_pat_keychain_reference(&id);
    // Queue deletion before writing the keychain. If the profile write later
    // fails, startup can discover and remove this generated secret reference
    // instead of leaving an untrackable orphan in the system keychain.
    state.mutate(|store| {
        store.queue_secret_deletion(secret_ref.clone());
        Ok(())
    })?;
    if let Err(error) = save_personal_access_token(&secret_ref, &token) {
        let _ = state.mutate(|store| {
            store.clear_pending_secret_deletion(&secret_ref);
            Ok(())
        });
        return Err(error);
    }

    match state.mutate_with_outcome(|store| {
        let endpoint = endpoint_from_store(store, &endpoint_id)?;
        if !CodeHostCapabilities::for_endpoint(&endpoint).pat_keychain {
            return Err(AppError::new(
                "pat_not_supported",
                "该代码托管 Endpoint 尚未启用应用管理的 PAT。",
            ));
        }
        let account = AccountProfile {
            id: account_id,
            endpoint_id: endpoint.id,
            login,
            display_name: display_name.and_then(non_empty_trimmed),
            auth_method: AuthenticationMethod::PersonalAccessToken,
            secret_ref: Some(secret_ref.clone()),
            status: AuthenticationStatus::Valid,
            is_default_candidate: false,
        };
        if store
            .accounts
            .iter()
            .any(|existing| account_profile_identity_matches(existing, &account))
        {
            return Err(AppError::new(
                "account_already_exists",
                "该 Endpoint 已存在相同登录名和认证方式的账号。",
            ));
        }
        store.accounts.push(account.clone());
        store.clear_pending_secret_deletion(&secret_ref);
        Ok(account.clone())
    }) {
        Ok(saved) => Ok(saved),
        Err(error) if error.committed() => state
            .snapshot()?
            .accounts
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| error.into_error()),
        Err(error) => {
            // The profile did not reach atomic replacement. Keep the durable
            // deletion queue if keychain cleanup itself fails; it will be
            // retried after startup rather than silently leaking a PAT.
            if delete_personal_access_token(&secret_ref).is_ok() {
                let _ = state.mutate(|store| {
                    store.clear_pending_secret_deletion(&secret_ref);
                    Ok(())
                });
            }
            Err(error.into_error())
        }
    }
}

#[tauri::command]
pub fn save_account_profile(
    endpoint_id: String,
    login: String,
    display_name: Option<String>,
    auth_method: AuthenticationMethod,
    state: State<'_, AppState>,
) -> Result<AccountProfile, AppError> {
    if matches!(auth_method, AuthenticationMethod::PersonalAccessToken) {
        return Err(AppError::new(
            "pat_requires_secure_input",
            "添加 PAT 请使用专用的安全账号添加入口。",
        ));
    }
    let endpoint = endpoint_from_state(&state, &endpoint_id)?;
    validate_method_for_endpoint(&auth_method, &endpoint)?;
    let _cli_account_import_lease =
        state.acquire_cli_account_import_lease(&endpoint.id, &auth_method)?;
    let login = required_login(&login)?;
    let display_name = display_name.and_then(non_empty_trimmed);
    let account_id = format!("account-{}", Uuid::new_v4());
    state.mutate(|store| {
        let endpoint = endpoint_from_store(store, &endpoint_id)?;
        validate_method_for_endpoint(&auth_method, &endpoint)?;
        let account = AccountProfile {
            id: account_id,
            endpoint_id: endpoint.id,
            login,
            display_name,
            auth_method,
            secret_ref: None,
            status: AuthenticationStatus::Unknown,
            is_default_candidate: false,
        };
        if store
            .accounts
            .iter()
            .any(|existing| account_profile_identity_matches(existing, &account))
        {
            return Err(AppError::new(
                "account_already_exists",
                "该 Endpoint 已存在相同登录名和认证方式的账号。",
            ));
        }
        store.accounts.push(account.clone());
        Ok(account)
    })
}

#[tauri::command]
pub fn remove_account(
    account_profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let account = state
        .snapshot()?
        .accounts
        .into_iter()
        .find(|account| account.id == account_profile_id)
        .ok_or_else(|| AppError::new("account_not_found", "找不到要移除的账号。"))?;
    // Hold the endpoint-wide CLI import lease before deleting a profile. An
    // import learns its account ID only after the external command returns;
    // this prevents it from reviving a CLI profile removed while it ran.
    let _cli_account_import_lease =
        state.acquire_cli_account_import_lease(&account.endpoint_id, &account.auth_method)?;
    let _account_operation_lease = state.acquire_account_operation_lease(&account_profile_id)?;

    let secret_ref = personal_access_token_reference(&account).cloned();
    match state.mutate_with_outcome(|store| {
        store
            .remove_account(&account_profile_id)
            .ok_or_else(|| AppError::new("account_not_found", "找不到要移除的账号。"))?;
        if let Some(secret_ref) = &secret_ref {
            store.queue_secret_deletion(secret_ref.clone());
        }
        Ok(())
    }) {
        Ok(()) => {}
        // `profiles.json` was atomically replaced, so its deletion queue is
        // durable even if the final directory sync later reports a warning.
        Err(error) if error.committed() => {}
        Err(error) => return Err(error.into_error()),
    }

    if let Some(secret_ref) = &secret_ref {
        if delete_personal_access_token(secret_ref).is_ok() {
            // A failure to clear the durable queue is harmless: startup
            // treats an already-deleted keychain item as a successful retry.
            // The account removal itself has completed and must not be
            // reported as failed merely because best-effort cleanup lagged.
            let _ = state.mutate(|store| {
                store.clear_pending_secret_deletion(secret_ref);
                Ok(())
            });
        } else {
            // The durable queue is retained above. Start one bounded retry in
            // this running session as well, so a temporarily locked keychain
            // does not keep a removed account's PAT usable until restart.
            state.retry_pending_secret_deletions_in_background();
        }
    }
    // Once the profile deletion reached atomic replacement, cleanup remains
    // durable. Return success even when the operating-system keychain is
    // temporarily unavailable so the UI never implies the profile remains.
    Ok(())
}

#[tauri::command]
pub async fn import_cli_accounts(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let initial_snapshot = state.snapshot()?;
        let endpoint = endpoint_from_store(&initial_snapshot, &endpoint_id)?;
        if !CodeHostCapabilities::for_endpoint(&endpoint).account_import {
            return Err(AppError::new(
                "cli_import_not_supported",
                "该 Endpoint 当前配置无法安全使用 CLI 账号导入。",
            ));
        }
        let cli_auth_method = match endpoint.kind {
            CodeHostKind::GitHub => AuthenticationMethod::GhCli,
            CodeHostKind::GitLab => AuthenticationMethod::GlabCli,
            _ => {
                return Err(AppError::new(
                    "cli_import_not_supported",
                    "该 Endpoint 不支持 gh 或 glab 账号导入。",
                ));
            }
        };
        // This endpoint-wide lease also covers profiles created after the
        // initial snapshot. It is held across the external CLI invocation so
        // account creation/removal cannot race reconciliation below.
        let _cli_account_import_lease =
            state.acquire_cli_account_import_lease(&endpoint.id, &cli_auth_method)?;

        let invocation_endpoint = endpoint.clone();
        let mut imported = match endpoint.kind {
            CodeHostKind::GitHub => import_gh_current_account_new(&endpoint)?,
            CodeHostKind::GitLab => import_glab_current_account_new(&endpoint)?,
            _ => {
                return Err(AppError::new(
                    "cli_import_not_supported",
                    "该 Endpoint 不支持 gh 或 glab 账号导入。",
                ));
            }
        };

        // A CLI import can update an existing account's cached status. Commit
        // it while advancing the refresh generation so a probe that observed
        // the older profile cannot overwrite this result afterward.
        state.mutate_and_invalidate_account_status_refreshes(|store| {
            let endpoint = endpoint_from_store(store, &endpoint_id)?;
            if endpoint.kind != invocation_endpoint.kind
                || endpoint.git_host != invocation_endpoint.git_host
                || endpoint.api_url != invocation_endpoint.api_url
            {
                return Err(AppError::new(
                    "endpoint_changed_during_import",
                    "Endpoint 已在账号导入期间变更，请重试。",
                ));
            }
            if !CodeHostCapabilities::for_endpoint(&endpoint).account_import {
                return Err(AppError::new(
                    "cli_import_not_supported",
                    "该 Endpoint 当前配置无法安全使用 CLI 账号导入。",
                ));
            }
            validate_method_for_endpoint(&imported.auth_method, &endpoint)?;
            imported.endpoint_id = endpoint.id;
            let account = if let Some(existing) = store
                .accounts
                .iter_mut()
                .find(|account| account_profile_identity_matches(account, &imported))
            {
                existing.status = imported.status.clone();
                existing.clone()
            } else {
                store.accounts.push(imported.clone());
                imported.clone()
            };
            Ok(vec![account])
        })
    })
    .await
    .map_err(|_| AppError::new("command_failed", "CLI 账号导入后台任务未能完成。"))?
}

#[tauri::command]
pub fn set_default_account(
    account_profile_id: String,
    is_default_candidate: bool,
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    state.mutate(|store| {
        let endpoint_id = store
            .accounts
            .iter()
            .find(|account| account.id == account_profile_id)
            .map(|account| account.endpoint_id.clone())
            .ok_or_else(|| AppError::new("account_not_found", "找不到该账号。"))?;
        for account in &mut store.accounts {
            if account.endpoint_id == endpoint_id {
                account.is_default_candidate =
                    account.id == account_profile_id && is_default_candidate;
            }
        }
        Ok(store.accounts.clone())
    })
}
