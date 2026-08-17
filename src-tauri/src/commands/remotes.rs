use super::support::*;
use super::*;
use percent_encoding::{percent_decode_str, percent_encode, AsciiSet, CONTROLS};
use url::Url;

const URL_PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'\\')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

#[tauri::command]
pub async fn get_repository_remote_web_url(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repository = repository_root(&repository_path)?;
        let canonical = repository.to_string_lossy().into_owned();
        let mut remotes = GitCore::new().remotes(&repository)?;
        let (endpoints, bindings) = state.remote_endpoint_metadata(&canonical)?;
        enrich_remote_endpoints_from_metadata(&mut remotes, &endpoints, &bindings, &canonical);
        Ok(repository_web_url_from_remotes(&remotes, &endpoints))
    })
    .await
    .map_err(|_| AppError::new("command_failed", "远程仓库读取任务未能完成。"))?
}

pub(super) fn repository_web_url_from_remotes(
    remotes: &[RemoteProfile],
    endpoints: &[CodeHostEndpoint],
) -> Option<String> {
    let origin = remotes.iter().find(|remote| remote.name == "origin");
    origin
        .into_iter()
        .chain(remotes.iter().filter(|remote| remote.name != "origin"))
        .find_map(|remote| remote_web_url(remote, endpoints))
}

fn remote_web_url(remote: &RemoteProfile, endpoints: &[CodeHostEndpoint]) -> Option<String> {
    let endpoint = endpoints
        .iter()
        .find(|endpoint| Some(endpoint.id.as_str()) == remote.endpoint_id.as_deref())?;
    let authority = remote.parsed.authority()?;
    if !endpoint_matches_authority(endpoint, &authority) {
        return None;
    }
    if !matches!(
        endpoint.kind,
        CodeHostKind::GitHub
            | CodeHostKind::GitLab
            | CodeHostKind::Gitee
            | CodeHostKind::Gitea
            | CodeHostKind::Forgejo
    ) {
        return None;
    }
    let namespace = remote.parsed.namespace.as_deref()?;
    let repository = remote.parsed.repository.as_deref()?;
    if repository.is_empty() || namespace.split('/').any(str::is_empty) {
        return None;
    }
    let url = Url::parse(&endpoint.web_url).ok()?;
    if url.cannot_be_a_base() || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let encoded_path = namespace
        .split('/')
        .chain(std::iter::once(repository))
        .map(|segment| {
            let decoded = percent_decode_str(segment).collect::<Vec<_>>();
            if matches!(decoded.as_slice(), b"." | b"..") {
                return None;
            }
            Some(percent_encode(&decoded, URL_PATH_SEGMENT_ENCODE_SET).to_string())
        })
        .collect::<Option<Vec<_>>>()?
        .join("/");
    (!encoded_path.is_empty())
        .then(|| format!("{}/{}", url.as_str().trim_end_matches('/'), encoded_path))
}

#[tauri::command]
pub fn create_tag(
    repository_path: String,
    name: String,
    target: Option<String>,
    message: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().create_tag(&repository, &name, target.as_deref(), message.as_deref())
}

#[tauri::command]
pub fn delete_tag(
    repository_path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let _repository_operation_lease =
        state.acquire_repository_operation_lease(&repository.to_string_lossy())?;
    GitCore::new().delete_tag(&repository, &name)
}

#[tauri::command]
pub fn add_remote(
    repository_path: String,
    name: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let git = GitCore::new();
    if git
        .remotes(&repository)?
        .iter()
        .any(|remote| remote.name == name)
    {
        return Err(AppError::new(
            "remote_already_exists",
            "仓库中已存在同名 remote。",
        ));
    }
    let previous_binding = state.snapshot()?.binding_for(&canonical, &name);
    // A previous `git remote remove` can have succeeded while its profile
    // cleanup failed. Clear that now-stale selection before a newly added
    // remote reuses the same name, even when the new URL happens to share the
    // old Endpoint.
    if previous_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.committed() => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    if let Err(error) = git.add_remote(&repository, &name, &url) {
        // `git remote add` normally fails without a side effect. Restore the
        // old binding only after proving the remote name is still absent; if
        // Git did add it despite an error, leaving the binding cleared is the
        // safe fail-closed result.
        let remote_exists = git
            .remotes(&repository)
            .map(|remotes| remotes.iter().any(|remote| remote.name == name))
            .unwrap_or(true);
        if !remote_exists {
            if let Some(binding) = previous_binding {
                if state
                    .mutate(|store| {
                        store.upsert_binding(binding);
                        Ok(())
                    })
                    .is_err()
                {
                    return Err(AppError::new(
                        "remote_binding_reconciliation_failed",
                        "remote 添加失败，原有账号绑定恢复失败；请检查 remote 后重新关联账号。",
                    ));
                }
            }
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn set_remote_url(
    repository_path: String,
    name: String,
    url: String,
    kind: RemoteUrlKind,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let git = GitCore::new();
    let store = state.snapshot()?;
    let mut effective_fetch_urls = git.remote_urls_for_operation(&repository, &name, false)?;
    let mut effective_push_urls = git.remote_urls_for_operation(&repository, &name, true)?;
    match kind {
        RemoteUrlKind::Fetch => effective_fetch_urls[0] = url.clone(),
        // The first explicit push URL changes Git from the implicit
        // "push to every fetch URL" rule to an explicit push-url list.
        RemoteUrlKind::Push => effective_push_urls = vec![url.clone()],
    };
    let binding_is_compatible = remote_binding_matches_all_urls(
        &store,
        &canonical,
        &name,
        &effective_fetch_urls,
        &effective_push_urls,
    );
    git.set_remote_url(&repository, &name, &url, kind)?;
    if !binding_is_compatible {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.committed() => {}
            // The Git URL is now changed while the stale binding remains.
            // It fails closed on endpoint mismatch, so do not roll Git back
            // to an old URL that might have been externally edited.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程地址已更新，但账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn rename_remote(
    repository_path: String,
    old_name: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let git = GitCore::new();
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let remotes = git.remotes(&repository)?;
    if !remotes.iter().any(|remote| remote.name == old_name) {
        return Err(AppError::new(
            "remote_not_found",
            "找不到要重命名的 remote。",
        ));
    }
    if remotes.iter().any(|remote| remote.name == new_name) {
        return Err(AppError::new(
            "remote_already_exists",
            "仓库中已存在同名 remote。",
        ));
    }
    let previous_binding = state.snapshot()?.binding_for(&canonical, &old_name);
    let previous_new_binding = state.snapshot()?.binding_for(&canonical, &new_name);
    // Persist the new name alongside the old one before Git renames it. The
    // dual binding makes both sides of the crash boundary fail safe: before
    // Git changes, the old remote is authorized; afterwards, the new remote
    // is authorized until the obsolete name can be cleaned up.
    let new_binding = previous_binding.as_ref().map(|binding| RemoteBinding {
        repository_path: binding.repository_path.clone(),
        remote_name: new_name.clone(),
        endpoint_id: binding.endpoint_id.clone(),
        account_selection: binding.account_selection.clone(),
    });
    if let Some(new_binding) = &new_binding {
        match state.mutate_with_outcome(|store| {
            store.upsert_binding(new_binding.clone());
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.committed() => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    if let Err(error) = git.rename_remote(&repository, &old_name, &new_name) {
        if new_binding.is_some()
            && state
                .mutate(|store| {
                    if let Some(binding) = &previous_new_binding {
                        store.upsert_binding(binding.clone());
                    } else {
                        store.remote_bindings.retain(|binding| {
                            !(binding.repository_path == canonical
                                && binding.remote_name == new_name)
                        });
                    }
                    Ok(())
                })
                .is_err()
        {
            return Err(AppError::new(
                "remote_binding_reconciliation_failed",
                "远程名称未更新，但临时账号绑定恢复失败；请检查 remote 后重新关联账号。",
            ));
        }
        return Err(error);
    }
    if new_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == old_name)
            });
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.committed() => {}
            // Keeping both bindings is safe: the renamed remote has the
            // copied authorization and the old remote no longer exists.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程名称已更新，但旧账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn remove_remote(
    repository_path: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let repository = repository_root(&repository_path)?;
    let git = GitCore::new();
    let canonical = repository.to_string_lossy().into_owned();
    let _repository_operation_lease = state.acquire_repository_operation_lease(&canonical)?;
    let previous_binding = state.snapshot()?.binding_for(&canonical, &name);
    git.remove_remote(&repository, &name)?;
    if previous_binding.is_some() {
        match state.mutate_with_outcome(|store| {
            store.remote_bindings.retain(|binding| {
                !(binding.repository_path == canonical && binding.remote_name == name)
            });
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) if error.committed() => {}
            // The remote is already gone. A stale binding is harmless until
            // reconciliation because no network operation can address it.
            Err(_) => {
                return Err(AppError::new(
                    "remote_binding_reconciliation_failed",
                    "远程已移除，但账号绑定清理未保存；请刷新后重新关联账号。",
                ));
            }
        }
    }
    Ok(())
}
