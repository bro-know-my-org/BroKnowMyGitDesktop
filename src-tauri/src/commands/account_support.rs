use super::*;

pub(super) fn endpoint_from_state(
    state: &AppState,
    endpoint_id: &str,
) -> Result<CodeHostEndpoint, AppError> {
    state
        .snapshot()?
        .endpoint(endpoint_id)
        .ok_or_else(|| AppError::new("endpoint_not_found", "找不到该代码托管 Endpoint。"))
}

pub(super) fn non_empty_trimmed(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn required_login(value: &str) -> Result<String, AppError> {
    let login = value.trim();
    if login.is_empty() || login.len() > 256 || login.chars().any(char::is_whitespace) {
        return Err(AppError::new(
            "invalid_account_login",
            "账号登录名不能为空且不能包含空格。",
        ));
    }
    Ok(login.to_owned())
}

pub(super) fn validate_method_for_endpoint(
    method: &AuthenticationMethod,
    endpoint: &CodeHostEndpoint,
) -> Result<(), AppError> {
    match (method, &endpoint.kind) {
        (AuthenticationMethod::GhCli, CodeHostKind::GitHub)
            if !git_authority_has_explicit_port(&endpoint.git_host)
                && github_endpoint_has_gh_compatible_api(endpoint) =>
        {
            Ok(())
        }
        (AuthenticationMethod::GlabCli, CodeHostKind::GitLab)
            if !git_authority_has_explicit_port(&endpoint.git_host) =>
        {
            Ok(())
        }
        (AuthenticationMethod::SshAgent, _)
        | (AuthenticationMethod::SystemGitCredentialHelper, _) => Ok(()),
        _ => Err(AppError::new(
            "authentication_method_mismatch",
            "该认证方式不适用于所选 Endpoint。",
        )),
    }
}

pub(super) fn current_account_status(
    account: &AccountProfile,
    endpoints: &[CodeHostEndpoint],
) -> AuthenticationStatus {
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.id == account.endpoint_id);
    match (&account.auth_method, endpoint) {
        (AuthenticationMethod::PersonalAccessToken, _) => personal_access_token_reference(account)
            .map(personal_access_token_status)
            .unwrap_or(AuthenticationStatus::NeedsReauthentication),
        (AuthenticationMethod::SshAgent, _) => ssh_agent_status(),
        (AuthenticationMethod::SystemGitCredentialHelper, _) => system_git_helper_status(),
        (AuthenticationMethod::GhCli, Some(endpoint)) => {
            if git_authority_has_explicit_port(&endpoint.git_host)
                || !github_endpoint_has_gh_compatible_api(endpoint)
            {
                return AuthenticationStatus::Unavailable;
            }
            let Some(hostname) = hostname_from_git_authority(&endpoint.git_host) else {
                return AuthenticationStatus::Unavailable;
            };
            let status = gh_auth_status(&hostname);
            if status.is_usable()
                && status
                    .login
                    .as_deref()
                    .is_some_and(|login| login.eq_ignore_ascii_case(&account.login))
            {
                AuthenticationStatus::Valid
            } else {
                AuthenticationStatus::NeedsReauthentication
            }
        }
        (AuthenticationMethod::GlabCli, Some(endpoint)) => {
            if git_authority_has_explicit_port(&endpoint.git_host) {
                return AuthenticationStatus::Unavailable;
            }
            let Some(hostname) = hostname_from_git_authority(&endpoint.git_host) else {
                return AuthenticationStatus::Unavailable;
            };
            let status = glab_auth_status(&hostname);
            if status.is_usable()
                && status
                    .login
                    .as_deref()
                    .is_some_and(|login| login.eq_ignore_ascii_case(&account.login))
            {
                AuthenticationStatus::Valid
            } else {
                AuthenticationStatus::NeedsReauthentication
            }
        }
        _ => AuthenticationStatus::Unavailable,
    }
}

pub(super) fn refresh_account_profiles(
    accounts: Vec<AccountProfile>,
    endpoints: &[CodeHostEndpoint],
) -> Vec<AccountProfile> {
    accounts
        .into_iter()
        .map(|mut account| {
            account.status = current_account_status(&account, endpoints);
            account
        })
        .collect()
}

pub(super) fn merge_refreshed_account_statuses(
    store: &mut ProfileStore,
    refreshed: &[AccountProfile],
) {
    for account in refreshed {
        // Profiles are immutable after creation. If a user removed one while
        // the background worker ran, silently leave it removed.
        if let Some(existing) = store
            .accounts
            .iter_mut()
            .find(|existing| existing.id == account.id)
        {
            existing.status = account.status.clone();
        }
    }
}

pub(super) fn system_git_helper_status() -> AuthenticationStatus {
    // Credential helpers can be configured at repository or URL scope, or be
    // supplied by a platform Git integration.  Looking only at global config
    // would incorrectly disable valid profiles.  This status intentionally
    // means "Git may use the user's existing helper"; Git remains the final
    // authority and the application never asks it to reveal a credential.
    AuthenticationStatus::Available
}

pub(super) fn ssh_agent_status() -> AuthenticationStatus {
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        return AuthenticationStatus::Unavailable;
    }
    // `ssh-add -l` only asks the already configured agent for public key
    // fingerprints; it neither creates keys nor changes the agent state.
    let mut command = Command::new("ssh-add");
    command
        .arg("-l")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(child) => match wait_for_child_status(child, SSH_AGENT_STATUS_TIMEOUT) {
            Some(status) if status.success() => AuthenticationStatus::Available,
            Some(_) | None => AuthenticationStatus::Unavailable,
        },
        // The socket still gives Git a chance to use a platform-specific
        // agent when ssh-add itself is unavailable.
        Err(_) => AuthenticationStatus::Available,
    }
}

/// Wait for a small status probe without allowing a broken agent or an
/// executable replaced on PATH to block the caller indefinitely. The child is
/// reaped on timeout so repeated account refreshes cannot accumulate zombies.
pub(super) fn wait_for_child_status(mut child: Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                // `wait` has no deadline. Reap outside the status probe so a
                // stuck child cannot turn this bounded check into a permanent
                // IPC worker stall.
                let _ = std::thread::Builder::new()
                    .name("bkm-ssh-agent-status-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = std::thread::Builder::new()
                    .name("bkm-ssh-agent-status-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return None;
            }
        }
    }
}
