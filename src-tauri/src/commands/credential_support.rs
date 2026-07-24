use super::account_support::*;
use super::repository_support::*;
use super::*;
use crate::domain::CredentialResolution;

/// Credential state and the immutable remote URLs selected before a network
/// command starts. Git receives these URLs positionally rather than looking
/// up the mutable remote name again after an account strategy is resolved.
pub(super) struct NetworkOperationTarget {
    pub(super) environment: GitEnvironment,
    pub(super) urls: Vec<String>,
}

/// A selected Clone identity. Resolving it does not access a PAT; the caller
/// must hold the returned account's operation lease before materializing the
/// Git environment that can invoke the askpass helper.
pub(super) struct CloneCredentialTarget {
    pub(super) binding: RemoteBinding,
    account_profile_id: String,
    resolution: CredentialResolution,
    account: AccountProfile,
}

impl CloneCredentialTarget {
    pub(super) fn account_profile_id(&self) -> &str {
        &self.account_profile_id
    }

    pub(super) fn environment(&self) -> Result<GitEnvironment, AppError> {
        credential_environment_for_resolution(&self.resolution, &self.account)
    }
}

/// Resolve an explicitly selected clone identity before a repository exists.
/// Leaving both optional inputs empty deliberately retains normal system Git
/// behavior, which is the correct path for public repositories and users who
/// rely on their existing credential helper.
pub(super) fn resolve_clone_credential_target(
    state: &AppState,
    remote_url: &str,
    destination: &Path,
    endpoint_id: Option<&str>,
    account_profile_id: Option<&str>,
    pinned_account_profile_id: Option<&str>,
) -> Result<Option<CloneCredentialTarget>, AppError> {
    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    refresh_account_statuses_in_place(&mut store.accounts, &endpoints);
    resolve_clone_credential_target_from_store(
        &store,
        remote_url,
        destination,
        endpoint_id,
        account_profile_id,
        pinned_account_profile_id,
    )
}

pub(super) fn resolve_clone_credential_target_from_store(
    store: &ProfileStore,
    remote_url: &str,
    destination: &Path,
    endpoint_id: Option<&str>,
    account_profile_id: Option<&str>,
    pinned_account_profile_id: Option<&str>,
) -> Result<Option<CloneCredentialTarget>, AppError> {
    let endpoint_id = endpoint_id.map(str::trim).filter(|value| !value.is_empty());
    let account_profile_id = account_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if endpoint_id.is_none() && account_profile_id.is_none() {
        return Ok(None);
    }

    let endpoints = store.all_endpoints();

    let endpoint = match (endpoint_id, account_profile_id) {
        (Some(endpoint_id), _) => endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
            .cloned()
            .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))?,
        (None, Some(account_id)) => {
            let account = store
                .accounts
                .iter()
                .find(|account| account.id == account_id)
                .ok_or_else(|| AppError::new("account_not_found", "找不到要用于 Clone 的账号。"))?;
            endpoints
                .iter()
                .find(|endpoint| endpoint.id == account.endpoint_id)
                .cloned()
                .ok_or_else(|| {
                    AppError::new("endpoint_not_found", "找不到该账号所属的 Endpoint。")
                })?
        }
        (None, None) => unreachable!("empty clone selection is returned above"),
    };

    let selection = account_profile_id
        .map(|account_id| AccountSelection::Profile(account_id.to_owned()))
        .unwrap_or(AccountSelection::Auto);
    let temporary_binding = RemoteBinding {
        repository_path: destination.to_string_lossy().into_owned(),
        remote_name: "origin".into(),
        endpoint_id: endpoint.id.clone(),
        account_selection: selection,
    };
    // The persisted binding preserves the caller's Auto/Profile choice. A
    // second resolution after taking the account-operation lease temporarily
    // pins the initially selected account, so a concurrent metadata change
    // cannot silently switch the Clone to a different identity.
    let resolution_binding = pinned_account_profile_id
        .map(|account_id| RemoteBinding {
            account_selection: AccountSelection::Profile(account_id.to_owned()),
            ..temporary_binding.clone()
        })
        .unwrap_or_else(|| temporary_binding.clone());
    let resolution = CredentialBroker.resolve(CredentialResolutionRequest {
        repository_path: &temporary_binding.repository_path,
        remote_name: &temporary_binding.remote_name,
        remote_url,
        endpoint: Some(&endpoint),
        endpoints: &endpoints,
        bindings: std::slice::from_ref(&resolution_binding),
        accounts: &store.accounts,
    })?;
    let account = store
        .accounts
        .iter()
        .find(|account| account.id == resolution.account_profile_id)
        .cloned()
        .ok_or_else(|| AppError::new("account_not_found", "找不到已解析的账号。"))?;

    Ok(Some(CloneCredentialTarget {
        binding: temporary_binding,
        account_profile_id: resolution.account_profile_id.clone(),
        resolution,
        account,
    }))
}

pub(super) fn network_environment(
    state: &AppState,
    repository: &Path,
    remote_name: &str,
    push: bool,
) -> Result<NetworkOperationTarget, AppError> {
    let git = GitCore::new();
    let repository_record = git.repository_record(repository)?;
    let raw_remote_urls = git.remote_urls_for_operation(repository, remote_name, push)?;
    let raw_remote_url = raw_remote_urls
        .first()
        .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
    let parsed_remotes = raw_remote_urls
        .iter()
        .map(|url| parse_remote_url(url))
        .collect::<Vec<_>>();
    let parsed_remote = parsed_remotes
        .first()
        .expect("a parsed remote exists for every non-empty URL list");
    // Every remote transport can honor repository-local upload-pack,
    // receive-pack, include, or SSH configuration. Check it before the
    // unauthenticated transport fast path as well.
    git.ensure_network_configuration_is_safe(repository)?;
    // `git://` and local-path remotes carry no account identity.  Preserve
    // normal system Git behavior instead of requiring an artificial Endpoint
    // or credential profile for them.
    if parsed_remotes.iter().all(|parsed| {
        matches!(
            &parsed.transport,
            crate::domain::RemoteTransport::Git | crate::domain::RemoteTransport::Local
        )
    }) {
        for remote_url in &raw_remote_urls {
            validate_unauthenticated_remote_url(remote_url)?;
        }
        return Ok(NetworkOperationTarget {
            environment: git.unbound_network_environment()?,
            urls: raw_remote_urls,
        });
    }
    // Reject cleartext, malformed, and credential-bearing remote URLs before
    // either a selected profile or the unscoped system helper sees them.
    for (remote_url, parsed_remote) in raw_remote_urls.iter().zip(&parsed_remotes) {
        validate_remote_url_for_credentials(remote_url, &parsed_remote.transport)?;
    }
    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    refresh_account_statuses_in_place(&mut store.accounts, &endpoints);
    let binding = store.binding_for(&repository_record.path, remote_name);
    let endpoint = binding
        .as_ref()
        .and_then(|binding| store.endpoint(&binding.endpoint_id))
        .or_else(|| {
            parsed_remote
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints))
                .cloned()
        });

    // System credential helpers select credentials by URL and cannot prove
    // which AccountProfile supplied them. Keep them as a normal, unbound Git
    // fallback only when no explicit PAT/SSH profile is available. An
    // explicit Profile binding must always resolve or fail closed.
    let explicit_profile_binding = matches!(
        binding.as_ref().map(|binding| &binding.account_selection),
        Some(AccountSelection::Profile(_))
    );
    if !explicit_profile_binding
        && matches!(
            &parsed_remote.transport,
            crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
        )
    {
        match endpoint.as_ref() {
            Some(endpoint)
                if CredentialBroker::has_explicit_account_candidate(
                    &store.accounts,
                    endpoint,
                    &parsed_remote.transport,
                ) => {}
            Some(endpoint) => {
                if binding.is_some() {
                    for remote_url in &raw_remote_urls {
                        validate_codehost_binding_url(remote_url, endpoint)?;
                    }
                }
                return Ok(NetworkOperationTarget {
                    environment: git.unbound_network_environment()?,
                    urls: raw_remote_urls,
                });
            }
            None if binding.is_none() => {
                return Ok(NetworkOperationTarget {
                    environment: git.unbound_network_environment()?,
                    urls: raw_remote_urls,
                });
            }
            None => {}
        }
    }

    if let Some(endpoint) = endpoint.as_ref() {
        for remote_url in &raw_remote_urls {
            validate_codehost_binding_url(remote_url, endpoint)?;
        }
    }

    let resolution = CredentialBroker.resolve(CredentialResolutionRequest {
        repository_path: &repository_record.path,
        remote_name,
        remote_url: raw_remote_url,
        endpoint: endpoint.as_ref(),
        endpoints: &endpoints,
        bindings: &store.remote_bindings,
        accounts: &store.accounts,
    })?;
    let account = store
        .accounts
        .iter()
        .find(|account| account.id == resolution.account_profile_id)
        .ok_or_else(|| AppError::new("account_not_found", "找不到已解析的账号。"))?;
    Ok(NetworkOperationTarget {
        environment: credential_environment_for_resolution(&resolution, account)?,
        urls: raw_remote_urls,
    })
}

pub(super) fn credential_environment_for_resolution(
    resolution: &crate::domain::CredentialResolution,
    account: &AccountProfile,
) -> Result<GitEnvironment, AppError> {
    let credential_environment = CredentialEnvironment::from_resolution(resolution, account)?;
    let askpass_path = credential_environment
        .askpass_config()
        .is_some()
        .then(std::env::current_exe)
        .transpose()
        .map_err(|error| AppError::io("定位应用 askpass helper", error))?;
    let pairs = credential_environment.environment_pairs(askpass_path.as_deref())?;
    let mut environment = GitEnvironment::new();
    for (name, value) in pairs {
        environment.set(name, value);
    }
    // Every resolved strategy is selected explicitly. Do not let a
    // repository-provided credential helper or HTTP header run alongside it.
    // A PAT askpass capability is a bearer authorization in Git's inherited
    // environment, so hook-capable commands must also disable hooks before
    // the process starts. SSH has no such secret in the environment.
    if matches!(
        &resolution.strategy,
        crate::domain::CredentialStrategy::KeychainPatAskPass { .. }
    ) {
        environment.isolate_network_credentials_and_disable_hooks();
    } else {
        environment.isolate_network_credentials();
    }
    Ok(environment)
}

/// `git://` and local remotes do not use an account profile, but a malformed
/// pre-existing `git://` URL must not become a loophole for credential-like
/// userinfo or token-bearing query data.  SSH user names remain handled by
/// CredentialBroker because they are part of normal SSH addressing.
pub(super) fn validate_unauthenticated_remote_url(raw_remote_url: &str) -> Result<(), AppError> {
    let Ok(url) = url::Url::parse(raw_remote_url) else {
        return Ok(());
    };
    if matches!(url.scheme(), "git" | "file")
        && (!url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some())
    {
        return Err(AppError::new(
            "unsafe_remote_url",
            "无凭据 Git remote 地址不能包含认证信息或令牌。",
        ));
    }
    Ok(())
}

pub(super) fn remote_binding_matches_all_urls(
    store: &ProfileStore,
    repository_path: &str,
    remote_name: &str,
    fetch_urls: &[String],
    push_urls: &[String],
) -> bool {
    let Some(binding) = store.binding_for(repository_path, remote_name) else {
        return true;
    };
    let Some(endpoint) = store.endpoint(&binding.endpoint_id) else {
        return false;
    };
    !fetch_urls.is_empty()
        && !push_urls.is_empty()
        && fetch_urls
            .iter()
            .all(|url| validate_codehost_binding_url(url, &endpoint).is_ok())
        && push_urls
            .iter()
            .all(|url| validate_codehost_binding_url(url, &endpoint).is_ok())
}

/// Pending clone metadata is intentionally replayed after a failed profile
/// write. Never let it recreate a binding whose explicit account was removed
/// while the clone was still in flight.
pub(super) fn binding_selected_account_is_current(
    store: &ProfileStore,
    binding: &RemoteBinding,
) -> bool {
    match &binding.account_selection {
        AccountSelection::Auto => true,
        AccountSelection::Profile(account_profile_id) => store.accounts.iter().any(|account| {
            account.id == *account_profile_id && account.endpoint_id == binding.endpoint_id
        }),
    }
}

/// A binding is an authorization decision, so only persist it when every
/// operation URL for that remote belongs to the selected endpoint. This keeps
/// unsupported ports, credential-bearing URLs, and cross-host push URLs from
/// becoming durable UI state that later cannot be used safely.
pub(super) fn validate_codehost_binding_url(
    raw_remote_url: &str,
    endpoint: &CodeHostEndpoint,
) -> Result<(), AppError> {
    let parsed = parse_remote_url(raw_remote_url);
    if !matches!(
        &parsed.transport,
        crate::domain::RemoteTransport::Https | crate::domain::RemoteTransport::Ssh
    ) || !parsed
        .authority()
        .as_deref()
        .is_some_and(|authority| endpoint_matches_authority(endpoint, authority))
    {
        return Err(AppError::new(
            "endpoint_association_required",
            "该 remote 地址与所选代码托管 Endpoint 不匹配。",
        ));
    }

    if let Ok(url) = url::Url::parse(raw_remote_url) {
        let http = matches!(url.scheme(), "http" | "https");
        if url.password().is_some()
            || (http && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::new(
                "unsafe_remote_url",
                "remote 地址包含不受支持的认证信息或令牌。",
            ));
        }
    }
    Ok(())
}

pub(super) fn github_adapter_context(
    state: &AppState,
    repository_path: &str,
    remote_name: &str,
) -> Result<AdapterContext, AppError> {
    let repository = repository_root(repository_path)?;
    let git = GitCore::new();
    let repository_record = git.repository_record(&repository)?;
    let mut remote = git
        .remotes(&repository)?
        .into_iter()
        .find(|remote| remote.name == remote_name)
        .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote。"))?;
    let fetch_urls = git.remote_urls_for_operation(&repository, remote_name, false)?;
    let push_urls = git.remote_urls_for_operation(&repository, remote_name, true)?;
    let raw_remote_url = fetch_urls
        .first()
        .ok_or_else(|| AppError::new("remote_not_found", "找不到该 remote 的 fetch 地址。"))?;
    let mut store = state.snapshot()?;
    let endpoints = store.all_endpoints();
    refresh_account_statuses_in_place(&mut store.accounts, &endpoints);
    let binding = store.binding_for(&repository_record.path, remote_name);
    let endpoint = binding
        .as_ref()
        .and_then(|binding| store.endpoint(&binding.endpoint_id))
        .or_else(|| {
            parse_remote_url(raw_remote_url)
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints))
                .cloned()
        })
        .ok_or_else(|| {
            AppError::new(
                "endpoint_association_required",
                "请先为该 remote 选择代码托管 Endpoint。",
            )
        })?;
    if endpoint.kind != CodeHostKind::GitHub
        || !codehost_adapter_urls_match_endpoint(&fetch_urls, &push_urls, &endpoint)
    {
        return Err(AppError::new(
            "github_adapter_auth_unsupported",
            "GitHub 平台信息需要与当前 gh 登录匹配的账号绑定。",
        ));
    }
    let account = github_cli_account_for_binding(&store, &endpoint, binding.as_ref())?;
    remote.endpoint_id = Some(endpoint.id.clone());
    remote.code_host_kind = endpoint.kind.clone();
    AdapterContext::new(endpoint, remote, account)
}

/// Platform API calls use one selected account, so every Git operation URL
/// associated with the remote must resolve to that same endpoint. A later
/// `pushurl` mirror cannot be allowed to inherit a GitHub platform identity
/// that was determined from only the first URL.
pub(super) fn codehost_adapter_urls_match_endpoint(
    fetch_urls: &[String],
    push_urls: &[String],
    endpoint: &CodeHostEndpoint,
) -> bool {
    !fetch_urls.is_empty()
        && !push_urls.is_empty()
        && fetch_urls
            .iter()
            .chain(push_urls.iter())
            .all(|url| validate_codehost_binding_url(url, endpoint).is_ok())
}

/// Platform API authentication is deliberately separate from Git credential
/// resolution. gh has one active account and can safely serve its own API only
/// after that active identity matches this selected profile; it is never
/// repurposed as an unpinned Git HTTPS token source.
pub(super) fn github_cli_account_for_binding(
    store: &ProfileStore,
    endpoint: &CodeHostEndpoint,
    binding: Option<&RemoteBinding>,
) -> Result<AccountProfile, AppError> {
    let candidates: Vec<&AccountProfile> = store
        .accounts
        .iter()
        .filter(|account| {
            account.endpoint_id == endpoint.id
                && account.auth_method == AuthenticationMethod::GhCli
                && account.status == AuthenticationStatus::Valid
        })
        .collect();
    // Current remote bindings authorize Git credentials. New bindings reject
    // CLI profiles, so a PAT/SSH binding must not suppress the independently
    // selected current gh identity used by GitHub's platform API. Legacy CLI
    // bindings remain readable during migration and still take precedence.
    let legacy_cli_binding = binding
        .and_then(|binding| match &binding.account_selection {
            AccountSelection::Profile(account_profile_id) => Some(account_profile_id),
            AccountSelection::Auto => None,
        })
        .and_then(|account_profile_id| {
            candidates
                .iter()
                .copied()
                .find(|account| account.id == *account_profile_id)
        });
    let selected = if legacy_cli_binding.is_some() {
        legacy_cli_binding
    } else if candidates.len() == 1 {
        candidates.first().copied()
    } else {
        let defaults: Vec<&AccountProfile> = candidates
            .into_iter()
            .filter(|account| account.is_default_candidate)
            .collect();
        (defaults.len() == 1).then(|| defaults[0])
    };
    selected.cloned().ok_or_else(|| {
        AppError::new(
            "github_adapter_auth_unsupported",
            "GitHub 平台信息需要与当前 gh 登录匹配的账号绑定。",
        )
    })
}
