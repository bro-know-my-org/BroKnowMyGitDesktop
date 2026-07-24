use super::credential_support::*;
use super::*;

pub(super) fn required_path(value: &str, _label: &str) -> Result<PathBuf, AppError> {
    if value.trim().is_empty() {
        return Err(AppError::new("path_required", "路径不能为空。"));
    }
    Ok(PathBuf::from(value))
}

/// A clone target does not exist before Git starts, so canonicalize its parent
/// and append the final path component. This makes concurrent aliases such as
/// `./project` and an absolute path share one in-process lease without
/// requiring the destination itself to exist.
pub(super) fn clone_destination_key(destination: &Path) -> String {
    let absolute = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current_directory| current_directory.join(destination))
            .unwrap_or_else(|_| destination.to_path_buf())
    };
    let Some(file_name) = absolute.file_name() else {
        return absolute.to_string_lossy().into_owned();
    };
    let Some(parent) = absolute.parent() else {
        return absolute.to_string_lossy().into_owned();
    };
    parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(file_name)
        .to_string_lossy()
        .into_owned()
}

pub(super) fn repository_root(path: &str) -> Result<PathBuf, AppError> {
    let path = required_path(path, "仓库路径")?;
    GitCore::new().repository_root(&path)
}

/// Keeps the stash and branch checkout in one repository-operation lease.
/// The frontend only requests this after an explicit user choice, but status
/// is checked again here so a stale UI snapshot never turns an empty working
/// tree into a failed automatic stash.
pub(super) fn stash_local_changes_before_branch_switch(
    git: &GitCore,
    repository: &Path,
    stash_local_changes: bool,
) -> Result<bool, AppError> {
    if stash_local_changes && !git.status(repository)?.entries.is_empty() {
        git.stash_push(repository, Some(BRANCH_SWITCH_STASH_MESSAGE), true)?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn branch_switch_error_after_stash(
    error: AppError,
    stashed_local_changes: bool,
) -> AppError {
    if stashed_local_changes {
        AppError::with_safe_details(
            "branch_switch_changes_stashed",
            "分支未能切换；本地改动已保存到暂存栈。",
            "请从“历史与暂存栈”恢复改动，再检查分支或工作树状态后重试。",
        )
    } else {
        error
    }
}

pub(super) fn snapshot_for(
    state: &AppState,
    repository: &Path,
) -> Result<RepositorySnapshot, AppError> {
    let git = GitCore::new();
    let git_snapshot = git.snapshot(repository)?;
    let operation = git.operation_kind(repository)?;
    let has_conflicts = git_snapshot.status.entries.iter().any(is_conflicted_status);
    let store = state.snapshot()?;
    let repository_path = git_snapshot.repository.path.clone();
    let mut remotes = git_snapshot.remotes;
    enrich_remote_endpoints(&mut remotes, &store, &repository_path);
    let mut repository_record = store.repository_with_metadata(git_snapshot.repository);
    let (host, namespace) = repository_owner_from_remotes(&remotes);
    repository_record.host = host;
    repository_record.namespace = namespace;
    let bindings = store
        .remote_bindings
        .iter()
        .filter(|binding| binding.repository_path == repository_path)
        .cloned()
        .collect();
    Ok(RepositorySnapshot {
        repository: repository_record,
        has_head: git.has_head(repository)?,
        status: git_snapshot.status,
        branches: git_snapshot.branches,
        remote_branches: git_snapshot.remote_branches,
        remotes,
        tags: git_snapshot.tags,
        bindings,
        operation: operation.map(|kind| GitOperationState {
            kind,
            has_conflicts,
        }),
    })
}

/// Finish clone metadata that was intentionally recorded before Git created
/// its destination. A pending binding is reapplied only after both current
/// remote URLs still validate against its Endpoint, preventing a stale intent
/// from selecting an account for an unrelated repository later created there.
pub(super) fn reconcile_pending_clone(state: &AppState, repository: &Path) -> Result<(), AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let pending = state
        .snapshot()?
        .pending_clones
        .into_iter()
        .find(|pending| {
            Path::new(&pending.destination_path)
                .canonicalize()
                .ok()
                .is_some_and(|path| path == Path::new(&repository_record.path))
        });
    let Some(pending) = pending else {
        return Ok(());
    };

    let binding = match pending.binding.clone() {
        // Old intents predate durable repository identity. They can still
        // register the cloned repository, but must never recreate an account
        // binding for whatever later occupies the destination path.
        Some(binding)
            if !pending.expected_remote_authority.is_empty()
                && !pending.expected_repository_slug.is_empty() =>
        {
            let store = state.snapshot()?;
            match store.endpoint(&binding.endpoint_id) {
                Some(endpoint) if binding_selected_account_is_current(&store, &binding) => {
                    // A transient store or Git-config read failure must keep
                    // the durable intent intact for a later reconciliation.
                    let fetch_url =
                        git.remote_url_for_operation(repository, &binding.remote_name, false)?;
                    let push_url =
                        git.remote_url_for_operation(repository, &binding.remote_name, true)?;
                    (validate_codehost_binding_url(&fetch_url, &endpoint).is_ok()
                        && validate_codehost_binding_url(&push_url, &endpoint).is_ok()
                        && pending_clone_remote_matches(
                            &fetch_url,
                            &pending.expected_remote_authority,
                            &pending.expected_repository_slug,
                        )
                        && pending_clone_remote_matches(
                            &push_url,
                            &pending.expected_remote_authority,
                            &pending.expected_repository_slug,
                        ))
                    .then_some(binding)
                }
                _ => None,
            }
        }
        _ => None,
    };

    state.mutate(|store| {
        store.upsert_repository(repository_record.clone());
        if let Some(mut binding) = binding.clone() {
            binding.repository_path = repository_record.path.clone();
            store.upsert_binding(binding);
        }
        store.clear_pending_clone(&pending.destination_path);
        Ok(())
    })
}

/// Complete a `gh repo create --push` whose external side effect succeeded
/// before the profile store could persist the account binding. The intent is
/// durable and contains no secret, so a retry can safely reconcile the local
/// `origin` instead of asking the user to create the repository again.
pub(super) fn reconcile_pending_publish(
    state: &AppState,
    repository: &Path,
) -> Result<bool, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let store = state.snapshot()?;
    let Some(pending) = store.pending_publish_for(&repository_record.path) else {
        return Ok(false);
    };
    let reconciliation_error = || {
        AppError::new(
            "github_publish_reconciliation_failed",
            "已检测到待完成的 GitHub 发布，但 origin 与所选账号或 Endpoint 不匹配。",
        )
    };
    // Pending publishes written by older versions lack a durable repository
    // identity. Fail closed and discard them rather than assigning an account
    // to whichever repository now happens to occupy that local path.
    if pending.expected_head.is_empty() || pending.expected_repository_slug.is_empty() {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    }
    let current_head = git
        .head_commit_id(repository)
        .map_err(|_| reconciliation_error())?;
    if current_head != pending.expected_head {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    }
    let origin_exists = git
        .remotes(repository)?
        .into_iter()
        .any(|remote| remote.name == pending.remote_name);
    if !origin_exists {
        return Ok(false);
    }
    let Some(endpoint) = store.endpoint(&pending.endpoint_id) else {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    };
    let Some(account) = store
        .accounts
        .iter()
        .find(|account| account.id == pending.account_profile_id)
        .cloned()
    else {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    };
    let fetch_url = git
        .remote_url_for_operation(repository, &pending.remote_name, false)
        .map_err(|_| reconciliation_error())?;
    let push_url = git
        .remote_url_for_operation(repository, &pending.remote_name, true)
        .map_err(|_| reconciliation_error())?;
    if account.endpoint_id != endpoint.id
        || validate_codehost_binding_url(&fetch_url, &endpoint).is_err()
        || validate_codehost_binding_url(&push_url, &endpoint).is_err()
        || !pending_publish_remote_matches(&fetch_url, &pending.expected_repository_slug)
        || !pending_publish_remote_matches(&push_url, &pending.expected_repository_slug)
    {
        return Err(reconciliation_error());
    }

    state.mutate(|store| {
        store.upsert_repository(repository_record.clone());
        store.upsert_binding(RemoteBinding {
            repository_path: repository_record.path.clone(),
            remote_name: pending.remote_name.clone(),
            endpoint_id: endpoint.id.clone(),
            account_selection: AccountSelection::Profile(account.id.clone()),
        });
        store.clear_pending_publish(&repository_record.path);
        Ok(())
    })?;
    Ok(true)
}

pub(super) fn github_publish_expected_slug(
    name: &str,
    account_login: &str,
) -> Result<String, AppError> {
    let name = name.trim();
    let parts: Vec<&str> = name.split('/').collect();
    let (owner, repository) = match parts.as_slice() {
        [repository] => (account_login.trim(), *repository),
        [owner, repository] => (*owner, *repository),
        _ => {
            return Err(AppError::new(
                "invalid_repository_name",
                "GitHub 仓库名称无效。",
            ))
        }
    };
    if !is_github_publish_slug_segment(owner) || !is_github_publish_slug_segment(repository) {
        return Err(AppError::new(
            "invalid_repository_name",
            "GitHub 仓库名称无效。",
        ));
    }
    Ok(format!("{owner}/{repository}"))
}

pub(super) fn is_github_publish_slug_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.starts_with('-')
        && !matches!(value, "." | "..")
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

pub(super) fn pending_publish_remote_matches(raw_remote_url: &str, expected_slug: &str) -> bool {
    parse_remote_url(raw_remote_url)
        .repository_slug()
        .as_deref()
        .is_some_and(|slug| slug.eq_ignore_ascii_case(expected_slug))
}

pub(super) fn clone_expected_remote_identity(
    raw_remote_url: &str,
) -> Result<(String, String), AppError> {
    let parsed = parse_remote_url(raw_remote_url);
    if !matches!(
        parsed.transport,
        crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
    ) {
        return Err(AppError::new(
            "clone_repository_identity_unavailable",
            "无法从 Clone 地址识别仓库路径；请选择有效的 HTTPS 或 SSH Git 地址。",
        ));
    }
    let authority = parsed.authority().filter(|authority| !authority.is_empty());
    let repository_slug = parsed.repository_slug().filter(|slug| !slug.is_empty());
    match (authority, repository_slug) {
        (Some(authority), Some(repository_slug)) => Ok((authority, repository_slug)),
        _ => Err(AppError::new(
            "clone_repository_identity_unavailable",
            "无法从 Clone 地址识别仓库路径；请选择有效的 HTTPS 或 SSH Git 地址。",
        )),
    }
}

pub(super) fn pending_clone_remote_matches(
    raw_remote_url: &str,
    expected_authority: &str,
    expected_slug: &str,
) -> bool {
    let parsed = parse_remote_url(raw_remote_url);
    parsed
        .authority()
        .as_deref()
        .is_some_and(|authority| authority.eq_ignore_ascii_case(expected_authority))
        && parsed.repository_slug().as_deref() == Some(expected_slug)
}

pub(super) fn is_conflicted_status(entry: &crate::domain::GitStatusEntry) -> bool {
    matches!(
        (entry.index_status.as_str(), entry.worktree_status.as_str()),
        ("D", "D") | ("A", "U") | ("U", "D") | ("U", "A") | ("D", "U") | ("A", "A") | ("U", "U")
    )
}

pub(super) fn enrich_remote_endpoints(
    remotes: &mut [RemoteProfile],
    store: &ProfileStore,
    repository_path: &str,
) {
    let endpoints = store.all_endpoints();
    for remote in remotes {
        if remote.endpoint_id.is_some() {
            continue;
        }
        let endpoint = store
            .binding_for(repository_path, &remote.name)
            .and_then(|binding| store.endpoint(&binding.endpoint_id))
            .or_else(|| {
                remote
                    .parsed
                    .authority()
                    .as_deref()
                    .and_then(|authority| endpoint_for_host(authority, &endpoints))
                    .cloned()
            });
        if let Some(endpoint) = endpoint {
            remote.endpoint_id = Some(endpoint.id);
            remote.code_host_kind = endpoint.kind;
        }
    }
}

/// Select an identity for sidebar ownership grouping. `origin` wins whenever
/// it has both a host and an owner path; otherwise any recognizable remote is
/// used. That mirrors the remote users usually consider canonical without
/// making a malformed origin hide useful metadata from another remote.
pub(super) fn repository_owner_from_remotes(
    remotes: &[RemoteProfile],
) -> (Option<String>, Option<String>) {
    remotes
        .iter()
        .find(|remote| {
            remote.name == "origin"
                && remote.parsed.host.is_some()
                && remote.parsed.namespace.is_some()
        })
        .or_else(|| {
            remotes
                .iter()
                .find(|remote| remote.parsed.host.is_some() && remote.parsed.namespace.is_some())
        })
        .map(|remote| (remote.parsed.host.clone(), remote.parsed.namespace.clone()))
        .unwrap_or((None, None))
}

pub(super) fn remember_repositories(
    state: &AppState,
    repositories: &[RepositoryRecord],
) -> Result<(), AppError> {
    let repositories = repositories.to_vec();
    state.mutate(|store| {
        for repository in repositories {
            store.upsert_repository(repository);
        }
        Ok(())
    })
}

pub(super) fn remember_repository(
    state: &AppState,
    repository: RepositoryRecord,
) -> Result<(), AppError> {
    state.mutate(|store| {
        store.upsert_repository(repository);
        Ok(())
    })
}

pub(super) fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn normalize_repository_group(
    group: Option<String>,
) -> Result<Option<String>, AppError> {
    let Some(group) = group else {
        return Ok(None);
    };
    let group = group.trim();
    if group.is_empty() {
        return Ok(None);
    }
    if group.chars().count() > 80 || group.chars().any(char::is_control) {
        return Err(AppError::new(
            "repository_group_invalid",
            "分组名称最多 80 个字符，且不能包含控制字符。",
        ));
    }
    Ok(Some(group.to_owned()))
}
