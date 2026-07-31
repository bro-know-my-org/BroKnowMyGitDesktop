impl CredentialBroker {
    /// Resolve the one credential strategy that may be used for a Git network
    /// operation.  This never falls through from an invalid explicit binding
    /// to a default account: doing so could authenticate as the wrong user.
    pub fn resolve(
        &self,
        request: CredentialResolutionRequest<'_>,
    ) -> Result<CredentialResolution, AppError> {
        let parsed = parse_remote_url(request.remote_url);
        validate_remote_url(request.remote_url, &parsed.transport)?;

        let binding = matching_binding(&request)?;
        let authority = parsed.authority();
        let endpoint = resolve_endpoint(&request, binding, authority.as_deref())?;
        let account = resolve_account(binding, endpoint, request.accounts, &parsed.transport)?;
        let strategy = strategy_for_account(account, endpoint, &parsed.transport)
            .ok_or_else(account_selection_required)?;

        Ok(CredentialResolution {
            endpoint_id: endpoint.id.clone(),
            account_profile_id: account.id.clone(),
            strategy,
        })
    }

    /// Whether this endpoint has an account whose credentials can be pinned
    /// to a selected profile for the given transport. System Git credential
    /// helpers are deliberately excluded: Git chooses them by URL and they
    /// cannot prove which saved account answered the request.
    pub fn has_explicit_account_candidate(
        accounts: &[AccountProfile],
        endpoint: &CodeHostEndpoint,
        transport: &RemoteTransport,
    ) -> bool {
        accounts
            .iter()
            .any(|account| is_usable_with_transport(account, endpoint, transport))
    }
}

/// Validate an HTTPS/SSH remote before handing it to an unscoped system Git
/// credential helper. This keeps the fallback on the same no-userinfo policy
/// as an explicitly resolved account strategy.
pub fn validate_remote_url_for_credentials(
    raw: &str,
    transport: &RemoteTransport,
) -> Result<(), AppError> {
    validate_remote_url(raw, transport)
}

fn validate_remote_url(raw: &str, transport: &RemoteTransport) -> Result<(), AppError> {
    match transport {
        RemoteTransport::Https | RemoteTransport::Ssh => {}
        RemoteTransport::Http
        | RemoteTransport::Git
        | RemoteTransport::Local
        | RemoteTransport::Other => {
            return Err(AppError::new(
                "unsupported_remote_transport",
                "账号认证只支持 HTTPS 或 SSH Git 地址。",
            ));
        }
    }

    let parsed_url = Url::parse(raw).ok();
    if matches!(transport, RemoteTransport::Https) && parsed_url.is_none() {
        return Err(AppError::new("unsafe_remote_url", "远程 HTTPS 地址无效。"));
    }
    if matches!(transport, RemoteTransport::Ssh) && parsed_url.is_none() && raw.contains(['?', '#'])
    {
        return Err(AppError::new(
            "unsafe_remote_url",
            "远程地址包含不安全的认证信息或查询参数。",
        ));
    }
    // A username in an HTTP remote can itself be a token. Query strings and
    // fragments are also outside normal Git remote addressing and can retain
    // credentials in repository configuration, so never carry them into a
    // command or a displayed error.
    if parsed_url.is_some_and(|url| {
        url.password().is_some()
            || (matches!(transport, RemoteTransport::Https | RemoteTransport::Http)
                && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
    }) {
        return Err(AppError::new(
            "unsafe_remote_url",
            "远程地址包含不安全的认证信息或查询参数。",
        ));
    }

    Ok(())
}

fn matching_binding<'a>(
    request: &CredentialResolutionRequest<'a>,
) -> Result<Option<&'a RemoteBinding>, AppError> {
    let repository_path = normalized_repository_path(request.repository_path);
    let remote_name = request.remote_name.trim();
    let mut matches = request.bindings.iter().filter(|binding| {
        binding.remote_name.trim() == remote_name
            && normalized_repository_path(&binding.repository_path) == repository_path
    });

    let binding = matches.next();
    if matches.next().is_some() {
        // Duplicate bindings make an otherwise explicit selection ambiguous.
        return Err(account_selection_required());
    }

    Ok(binding)
}

fn normalized_repository_path(path: &str) -> String {
    let value = path.trim().replace('\\', "/");
    let absolute = value.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();

    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(previous) if *previous != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }

    let joined = parts.join("/");
    let normalized = if absolute {
        format!("/{joined}")
    } else {
        joined
    };

    #[cfg(windows)]
    {
        normalized.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

fn resolve_endpoint<'a>(
    request: &CredentialResolutionRequest<'a>,
    binding: Option<&'a RemoteBinding>,
    authority: Option<&str>,
) -> Result<&'a CodeHostEndpoint, AppError> {
    let automatic_endpoint =
        authority.and_then(|value| endpoint_for_host(value, request.endpoints));
    let supplied_endpoint = request.endpoint;

    if let (Some(automatic), Some(supplied)) = (automatic_endpoint, supplied_endpoint) {
        if automatic.id != supplied.id {
            return Err(endpoint_association_required());
        }
    }

    let mut endpoint = supplied_endpoint.or(automatic_endpoint);

    if let Some(binding) = binding {
        let binding_endpoint = endpoint_by_id(request, &binding.endpoint_id)
            .ok_or_else(endpoint_association_required)?;

        if let Some(selected) = endpoint {
            if selected.id != binding_endpoint.id {
                return Err(endpoint_association_required());
            }
        }
        endpoint = Some(binding_endpoint);
    }

    let endpoint = endpoint.ok_or_else(endpoint_association_required)?;
    if let Some(authority) = authority {
        if !endpoint_matches_authority(endpoint, authority) {
            return Err(endpoint_association_required());
        }
    }

    Ok(endpoint)
}

fn endpoint_by_id<'a>(
    request: &CredentialResolutionRequest<'a>,
    endpoint_id: &str,
) -> Option<&'a CodeHostEndpoint> {
    request
        .endpoint
        .filter(|endpoint| endpoint.id == endpoint_id)
        .or_else(|| {
            request
                .endpoints
                .iter()
                .find(|endpoint| endpoint.id == endpoint_id)
        })
}

fn resolve_account<'a>(
    binding: Option<&RemoteBinding>,
    endpoint: &CodeHostEndpoint,
    accounts: &'a [AccountProfile],
    transport: &RemoteTransport,
) -> Result<&'a AccountProfile, AppError> {
    if let Some(RemoteBinding {
        account_selection: AccountSelection::Profile(account_id),
        ..
    }) = binding
    {
        let mut matched = accounts.iter().filter(|account| account.id == *account_id);
        let account = matched.next().ok_or_else(account_selection_required)?;
        if matched.next().is_some()
            || account.endpoint_id != endpoint.id
            || !is_usable_with_transport(account, endpoint, transport)
        {
            return Err(account_selection_required());
        }
        return Ok(account);
    }

    let candidates: Vec<&AccountProfile> = accounts
        .iter()
        .filter(|account| is_usable_with_transport(account, endpoint, transport))
        .collect();

    // IDs are persisted identifiers.  Duplicate IDs indicate corrupt
    // metadata, and must not be silently treated as a single identity.
    let mut seen_ids = HashSet::new();
    if candidates
        .iter()
        .any(|account| !seen_ids.insert(&account.id))
    {
        return Err(account_selection_required());
    }

    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }

    let defaults: Vec<&AccountProfile> = candidates
        .into_iter()
        .filter(|account| account.is_default_candidate)
        .collect();
    if defaults.len() == 1 {
        return Ok(defaults[0]);
    }

    Err(account_selection_required())
}

fn is_usable_with_transport(
    account: &AccountProfile,
    endpoint: &CodeHostEndpoint,
    transport: &RemoteTransport,
) -> bool {
    matches!(
        &account.status,
        AuthenticationStatus::Available | AuthenticationStatus::Valid
    ) && strategy_for_account(account, endpoint, transport).is_some()
}

fn strategy_for_account(
    account: &AccountProfile,
    endpoint: &CodeHostEndpoint,
    transport: &RemoteTransport,
) -> Option<CredentialStrategy> {
    if account.id.trim().is_empty() {
        return None;
    }

    match (&account.auth_method, transport) {
        // SSH agent identity selection belongs to OpenSSH host configuration
        // and cannot be pinned by this profile ID. Treat it as an unscoped
        // Git fallback instead of claiming an explicit account binding.
        (AuthenticationMethod::SshAgent, RemoteTransport::Ssh) => None,
        // A system credential helper is scoped to a URL, not an application
        // account profile. It must remain an unscoped fallback instead of
        // becoming a per-profile remote binding.
        (AuthenticationMethod::SystemGitCredentialHelper, RemoteTransport::Https) => None,
        // gh/glab expose one process-global active account and their token
        // commands do not provide a safe, supported per-account selector.
        // Never turn a CLI profile into Git HTTPS credentials: doing so could
        // either fail outright or authenticate a bound remote as the wrong
        // account after another process changes the active CLI identity.
        (AuthenticationMethod::GhCli | AuthenticationMethod::GlabCli, RemoteTransport::Https) => {
            None
        }
        (AuthenticationMethod::PersonalAccessToken, RemoteTransport::Https) => {
            let keychain_ref = personal_access_token_reference(account)?;
            if !is_safe_username(&account.login) {
                return None;
            }
            Some(CredentialStrategy::KeychainPatAskPass {
                account_profile_id: account.id.clone(),
                keychain_ref: keychain_ref.clone(),
                authority: normalize_git_authority(&endpoint.git_host)?,
                username: account.login.clone(),
            })
        }
        _ => None,
    }
}

fn account_selection_required() -> AppError {
    AppError::new(
        "account_selection_required",
        "该远程地址需要选择一个可用且匹配的账号。",
    )
}

fn endpoint_association_required() -> AppError {
    AppError::new(
        "endpoint_association_required",
        "无法安全识别该远程地址的平台，请关联一个匹配的代码托管 Endpoint。",
    )
}

fn is_safe_metadata_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn is_safe_username(value: &str) -> bool {
    is_safe_metadata_value(value) && !value.chars().any(char::is_whitespace)
}

/// Create the stable keychain reference for an application-managed PAT.
pub fn default_pat_keychain_reference(account_profile_id: &str) -> KeychainReference {
    KeychainReference {
        service: PAT_KEYCHAIN_SERVICE.to_owned(),
        account: account_profile_id.to_owned(),
    }
}

/// Return the only keychain reference that may be used for a PAT profile.
/// Profile metadata is locally editable, so merely checking the application
/// service is insufficient: a tampered profile must not point at another
/// profile's token.
pub fn personal_access_token_reference(account: &AccountProfile) -> Option<&KeychainReference> {
    let keychain_ref = account.secret_ref.as_ref()?;
    (matches!(
        account.auth_method,
        AuthenticationMethod::PersonalAccessToken
    ) && is_safe_metadata_value(&account.id)
        && keychain_ref.service == PAT_KEYCHAIN_SERVICE
        && keychain_ref.account == account.id
        && is_safe_metadata_value(&keychain_ref.service)
        && is_safe_metadata_value(&keychain_ref.account))
    .then_some(keychain_ref)
}

/// Store a PAT in the operating system credential store.
///
/// This is intentionally a write-only API.  There is no public function that
/// returns a saved PAT to application code or Tauri IPC.
pub fn save_personal_access_token(
    keychain_ref: &KeychainReference,
    personal_access_token: &str,
) -> Result<(), AppError> {
    if personal_access_token.trim().is_empty()
        || personal_access_token.chars().any(char::is_whitespace)
    {
        return Err(AppError::new(
            "invalid_personal_access_token",
            "个人访问令牌不能为空。",
        ));
    }

    let entry = keychain_entry(keychain_ref)?;
    entry.set_password(personal_access_token).map_err(|_| {
        AppError::new(
            "keychain_write_failed",
            "无法保存个人访问令牌到系统钥匙串。",
        )
    })
}

/// Remove an application-managed PAT from the operating system credential
/// store.  Account metadata should be removed separately by its owner.
pub fn delete_personal_access_token(keychain_ref: &KeychainReference) -> Result<(), AppError> {
    let entry = keychain_entry(keychain_ref)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(AppError::new(
            "keychain_delete_failed",
            "无法从系统钥匙串移除个人访问令牌。",
        )),
    }
}

/// Check keychain entry availability without materializing its secret in the
/// long-lived desktop process. PAT bytes are only read by the short-lived
/// askpass helper when Git requests them.
pub fn personal_access_token_status(keychain_ref: &KeychainReference) -> AuthenticationStatus {
    let Ok(entry) = keychain_entry(keychain_ref) else {
        return AuthenticationStatus::Unavailable;
    };

    match entry.get_attributes() {
        Ok(_) => AuthenticationStatus::Valid,
        Err(keyring::Error::NoEntry) => AuthenticationStatus::NeedsReauthentication,
        Err(_) => AuthenticationStatus::Unavailable,
    }
}

fn keychain_entry(keychain_ref: &KeychainReference) -> Result<Entry, AppError> {
    // Account metadata is editable on the local filesystem. Never let a
    // tampered profile turn this app into a reader/deleter for an unrelated
    // system-keychain service.
    if keychain_ref.service != PAT_KEYCHAIN_SERVICE
        || !is_safe_metadata_value(&keychain_ref.service)
        || !is_safe_metadata_value(&keychain_ref.account)
    {
        return Err(AppError::new(
            "invalid_keychain_reference",
            "账号的系统钥匙串引用无效。",
        ));
    }

    Entry::new(&keychain_ref.service, &keychain_ref.account)
        .map_err(|_| AppError::new("keychain_unavailable", "系统钥匙串不可用。"))
}

fn read_personal_access_token(keychain_ref: &KeychainReference) -> Result<String, AppError> {
    let entry = keychain_entry(keychain_ref)?;
    entry.get_password().map_err(|_| {
        AppError::new(
            "keychain_pat_unavailable",
            "无法从系统钥匙串读取该账号的个人访问令牌。",
        )
    })
}

/// A non-serializable, non-secret environment plan for a Git command.
///
/// `variables` never contains a PAT.  A caller translates this plan to its
/// own Git command wrapper by calling [`Self::environment_pairs`] or
/// [`Self::apply_to_command`].
pub struct CredentialEnvironment {
    variables: Vec<(OsString, OsString)>,
    askpass: Option<AskPassConfig>,
}

