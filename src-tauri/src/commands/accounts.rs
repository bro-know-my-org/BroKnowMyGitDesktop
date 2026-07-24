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

    state.mutate(|store| {
        merge_refreshed_account_statuses(store, &refreshed);
        Ok(store.accounts.clone())
    })
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

    let id = format!("account-{}", Uuid::new_v4());
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

    let account = AccountProfile {
        id,
        endpoint_id: endpoint.id,
        login,
        display_name: display_name.and_then(non_empty_trimmed),
        auth_method: AuthenticationMethod::PersonalAccessToken,
        secret_ref: Some(secret_ref.clone()),
        status: AuthenticationStatus::Valid,
        is_default_candidate: false,
    };

    match state.mutate_with_outcome(|store| {
        store.accounts.push(account.clone());
        store.clear_pending_secret_deletion(&secret_ref);
        Ok(account.clone())
    }) {
        Ok(saved) => Ok(saved),
        Err(error) if error.committed() => Err(error.into_error()),
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
    let account = AccountProfile {
        id: format!("account-{}", Uuid::new_v4()),
        endpoint_id: endpoint.id,
        login: required_login(&login)?,
        display_name: display_name.and_then(non_empty_trimmed),
        auth_method,
        secret_ref: None,
        status: AuthenticationStatus::Unknown,
        is_default_candidate: false,
    };
    state.mutate(|store| {
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

    let secret_ref = personal_access_token_reference(&account).cloned();
    let persistence_warning = match state.mutate_with_outcome(|store| {
        store
            .remove_account(&account_profile_id)
            .ok_or_else(|| AppError::new("account_not_found", "找不到要移除的账号。"))?;
        if let Some(secret_ref) = &secret_ref {
            store.queue_secret_deletion(secret_ref.clone());
        }
        Ok(())
    }) {
        Ok(()) => None,
        // `profiles.json` was atomically replaced, so its deletion queue is
        // durable even though the final directory sync warning must still be
        // reported after the token cleanup completes.
        Err(error) if error.committed() => Some(error.into_error()),
        Err(error) => return Err(error.into_error()),
    };

    if let Some(secret_ref) = &secret_ref {
        delete_personal_access_token(secret_ref)?;
        let clear_warning = match state.mutate_with_outcome(|store| {
            store.clear_pending_secret_deletion(secret_ref);
            Ok(())
        }) {
            Ok(()) => None,
            Err(error) if error.committed() => Some(error.into_error()),
            // The queue is still durable from the first mutation. Keeping it
            // is safe: startup treats an already-deleted keychain entry as a
            // successful retry.
            Err(error) => return Err(error.into_error()),
        };
        if let Some(error) = clear_warning {
            return Err(error);
        }
    }
    persistence_warning.map_or(Ok(()), Err)
}

#[tauri::command]
pub async fn import_cli_accounts(
    endpoint_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<AccountProfile>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let endpoint = endpoint_from_state(&state, &endpoint_id)?;
        if !CodeHostCapabilities::for_endpoint(&endpoint).account_import {
            return Err(AppError::new(
                "cli_import_not_supported",
                "该 Endpoint 当前配置无法安全使用 CLI 账号导入。",
            ));
        }
        let imported = match endpoint.kind {
            CodeHostKind::GitHub => import_gh_current_account_new(&endpoint)?,
            CodeHostKind::GitLab => import_glab_current_account_new(&endpoint)?,
            _ => {
                return Err(AppError::new(
                    "cli_import_not_supported",
                    "该 Endpoint 不支持 gh 或 glab 账号导入。",
                ));
            }
        };

        state.mutate(|store| {
            let account = if let Some(existing) = store.accounts.iter_mut().find(|account| {
                account.endpoint_id == imported.endpoint_id
                    && account.login.eq_ignore_ascii_case(&imported.login)
                    && account.auth_method == imported.auth_method
            }) {
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
