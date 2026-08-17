use super::credential_support::*;
use super::*;
use std::path::Component;

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
pub(super) fn clone_destination_key(destination: &Path) -> Result<String, AppError> {
    let (parent, file_name) = clone_destination_parent_and_name(destination)?;
    Ok(parent.join(file_name).to_string_lossy().into_owned())
}

/// Serializes clone and initialization requests sharing one destination
/// parent. This deliberately uses the canonical parent rather than a final
/// name: a not-yet-created `Project` and `project` can identify the same
/// directory on a case-insensitive filesystem, where no canonical final path
/// exists yet to collapse the aliases.
pub(super) fn clone_parent_lease_key(destination: &Path) -> Result<String, AppError> {
    Ok(clone_destination_parent_and_name(destination)?
        .0
        .to_string_lossy()
        .into_owned())
}

fn clone_destination_parent_and_name(
    destination: &Path,
) -> Result<(PathBuf, &std::ffi::OsStr), AppError> {
    let file_name = match destination.components().next_back() {
        Some(Component::Normal(file_name)) => file_name,
        _ => {
            return Err(AppError::new(
                "invalid_clone_destination",
                "克隆目标必须是父目录下的一个文件夹。",
            ))
        }
    };
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let absolute_parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current_directory| current_directory.join(parent))
            .map_err(|_| AppError::new("invalid_clone_destination", "无法解析克隆目标的父目录。"))?
    };
    // Resolve the original parent before any lexical fallback. `link/..` is
    // interpreted relative to the link target by the filesystem, so reducing
    // it first would create a lease key for a different destination.
    let parent = absolute_parent
        .canonicalize()
        .map_err(|_| AppError::new("invalid_clone_destination", "无法访问克隆目标的父目录。"))?;
    Ok((parent, file_name))
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

/// A branch switch may already have completed when assembling the refreshed
/// snapshot fails. Preserve the same recovery guidance as a failed switch so
/// a successful pre-switch stash is never mistaken for lost local work.
pub(super) fn branch_switch_snapshot_error_after_stash(
    error: AppError,
    stashed_local_changes: bool,
) -> AppError {
    if stashed_local_changes {
        AppError::with_safe_details(
            "branch_switch_changes_stashed",
            "分支可能已切换，但刷新仓库状态失败；本地改动已保存到暂存栈。",
            "请刷新仓库状态，并从“历史与暂存栈”确认或恢复改动。",
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

    let (binding, validated_endpoint) = match pending.binding.clone() {
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
                    let fetch_urls =
                        git.remote_urls_for_operation(repository, &binding.remote_name, false)?;
                    let push_urls =
                        git.remote_urls_for_operation(repository, &binding.remote_name, true)?;
                    let matches_expected_repository = pending_clone_operation_urls_match(
                        &fetch_urls,
                        &push_urls,
                        &endpoint,
                        &pending.expected_remote_authority,
                        &pending.expected_repository_slug,
                    );
                    if matches_expected_repository {
                        (Some(binding), Some(endpoint.clone()))
                    } else {
                        (None, None)
                    }
                }
                _ => (None, None),
            }
        }
        _ => (None, None),
    };

    state.mutate(|store| {
        let pending_is_current = store
            .pending_clones
            .iter()
            .any(|current| current == &pending);
        let binding_is_current = match (&binding, &validated_endpoint) {
            (Some(binding), Some(validated_endpoint)) => {
                store
                    .endpoint(&binding.endpoint_id)
                    .is_some_and(|current| current == *validated_endpoint)
                    && binding_selected_account_is_current(store, binding)
            }
            (None, None) => true,
            _ => false,
        };
        if !pending_is_current || !binding_is_current {
            return Err(AppError::new(
                "clone_reconciliation_failed",
                "待完成的 Clone 元数据已变更；请重新打开仓库后重试。",
            ));
        }
        store.upsert_repository(repository_record.clone());
        if let Some(mut binding) = binding.clone() {
            binding.repository_path = repository_record.path.clone();
            store.upsert_binding(binding);
        }
        store.clear_pending_clone(&pending.destination_path);
        Ok(())
    })
}

/// A recovered Clone binding is an authorization decision, so every effective
/// fetch and push URL must still refer to the original endpoint and repository.
/// In particular, an added `pushurl` mirror must not regain the selected
/// account merely because the first URL remains valid.
pub(super) fn pending_clone_operation_urls_match(
    fetch_urls: &[String],
    push_urls: &[String],
    endpoint: &CodeHostEndpoint,
    expected_authority: &str,
    expected_repository_slug: &str,
) -> bool {
    !fetch_urls.is_empty()
        && !push_urls.is_empty()
        && fetch_urls.iter().chain(push_urls.iter()).all(|url| {
            validate_codehost_binding_url(url, endpoint).is_ok()
                && pending_clone_remote_matches(url, expected_authority, expected_repository_slug)
        })
}

/// Complete a `gh repo create --push` whose external side effect succeeded
/// before the profile store could persist the account binding. The intent is
/// durable and contains no secret, so a retry can safely reconcile the local
/// `origin` instead of asking the user to create the repository again.
pub(super) fn reconcile_pending_publish(
    state: &AppState,
    repository: &Path,
    requested_endpoint_id: &str,
    requested_account_profile_id: &str,
    requested_repository_slug: &str,
) -> Result<bool, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let store = state.snapshot()?;
    let Some(pending) = store.pending_publish_for(&repository_record.path) else {
        return Ok(false);
    };
    // A retry is an explicit request to publish using the caller's selected
    // identity. Never let it silently finalize an earlier pending publish
    // that belongs to a different account or Endpoint.
    if !pending_publish_matches_requested_identity(
        &pending,
        requested_endpoint_id,
        requested_account_profile_id,
        requested_repository_slug,
    ) {
        return Err(AppError::new(
            "github_publish_reconciliation_required",
            "检测到待完成的 GitHub 发布，但所选账号、Endpoint 或仓库所有者与该发布不匹配；请使用原发布目标重试。",
        ));
    }
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
    let fetch_urls = git
        .remote_urls_for_operation(repository, &pending.remote_name, false)
        .map_err(|_| reconciliation_error())?;
    let push_urls = git
        .remote_urls_for_operation(repository, &pending.remote_name, true)
        .map_err(|_| reconciliation_error())?;
    if account.endpoint_id != endpoint.id
        || fetch_urls.iter().chain(push_urls.iter()).any(|url| {
            validate_codehost_binding_url(url, &endpoint).is_err()
                || !pending_publish_remote_matches(url, &pending.expected_repository_slug)
        })
    {
        state.mutate(|store| {
            store.clear_pending_publish(&repository_record.path);
            Ok(())
        })?;
        return Err(reconciliation_error());
    }

    let branch = git
        .current_branch_name(repository)
        .map_err(|_| reconciliation_error())?;
    let environment = git
        .unbound_network_environment()
        .map_err(|_| reconciliation_error())?;
    let remote_head = git
        .remote_branch_tip(repository, &pending.remote_name, &branch, &environment)
        .map_err(|_| reconciliation_error())?;
    if remote_head.as_deref() != Some(pending.expected_head.as_str()) {
        return Err(AppError::new(
            "github_publish_reconciliation_required",
            "已找到 origin，但尚未确认当前提交已推送到远程分支；请检查远程状态后重试。",
        ));
    }

    state.mutate(|store| {
        let account_is_current = store
            .accounts
            .iter()
            .any(|current| current.id == account.id && current.endpoint_id == endpoint.id);
        let pending_is_current =
            store.pending_publish_for(&repository_record.path).as_ref() == Some(&pending);
        if !account_is_current || !pending_is_current {
            return Err(reconciliation_error());
        }
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

pub(super) fn pending_publish_matches_requested_identity(
    pending: &PendingPublish,
    endpoint_id: &str,
    account_profile_id: &str,
    expected_repository_slug: &str,
) -> bool {
    pending.endpoint_id == endpoint_id
        && pending.account_profile_id == account_profile_id
        && pending
            .expected_repository_slug
            .eq_ignore_ascii_case(expected_repository_slug)
}

pub(super) fn github_publish_expected_slug(
    owner: &str,
    repository: &str,
) -> Result<String, AppError> {
    let owner = owner.trim();
    let repository = repository.trim();
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
    enrich_remote_endpoints_from_metadata(
        remotes,
        &endpoints,
        &store.remote_bindings,
        repository_path,
    );
}

pub(super) fn enrich_remote_endpoints_from_metadata(
    remotes: &mut [RemoteProfile],
    endpoints: &[CodeHostEndpoint],
    bindings: &[RemoteBinding],
    repository_path: &str,
) {
    for remote in remotes {
        if remote.endpoint_id.is_some() {
            continue;
        }
        let endpoint = bindings
            .iter()
            .find(|binding| {
                binding.repository_path == repository_path && binding.remote_name == remote.name
            })
            .and_then(|binding| {
                endpoints
                    .iter()
                    .find(|endpoint| endpoint.id == binding.endpoint_id)
            })
            .or_else(|| {
                remote
                    .parsed
                    .authority()
                    .as_deref()
                    .and_then(|authority| endpoint_for_host(authority, endpoints))
            });
        if let Some(endpoint) = endpoint {
            remote.endpoint_id = Some(endpoint.id.clone());
            remote.code_host_kind = endpoint.kind.clone();
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
    normalize_repository_label(
        group,
        "repository_group_invalid",
        "分组名称最多 80 个字符，且不能包含控制字符。",
    )
}

pub(super) fn normalize_repository_alias(
    alias: Option<String>,
) -> Result<Option<String>, AppError> {
    normalize_repository_label(
        alias,
        "repository_alias_invalid",
        "仓库别名最多 80 个字符，且不能包含控制字符。",
    )
}

fn normalize_repository_label(
    value: Option<String>,
    error_code: &'static str,
    error_message: &'static str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(AppError::new(error_code, error_message));
    }
    Ok(Some(value.to_owned()))
}
