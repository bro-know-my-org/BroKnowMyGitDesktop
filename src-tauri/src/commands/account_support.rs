use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};

const MAX_SSH_AGENT_STATUS_REAPERS: usize = 4;
static ACTIVE_SSH_AGENT_STATUS_REAPERS: AtomicUsize = AtomicUsize::new(0);
type SshAgentStatusReaperJob = (Child, SshAgentStatusReaperPermit);
static SSH_AGENT_STATUS_REAPER_SENDER: OnceLock<Option<mpsc::SyncSender<SshAgentStatusReaperJob>>> =
    OnceLock::new();
static SSH_AGENT_STATUS_ORPHANED_JOBS: OnceLock<Mutex<Vec<SshAgentStatusReaperJob>>> =
    OnceLock::new();
static SSH_AGENT_STATUS_PROBE: OnceLock<(Mutex<SshAgentStatusProbe>, Condvar)> = OnceLock::new();

struct SshAgentStatusProbe {
    in_flight: bool,
    last_status: AuthenticationStatus,
}

/// A bounded slot retained until a timed-out `ssh-add` child has exited. The
/// status probe is normally short-lived, but a broken helper must not create
/// an unbounded number of detached waiters.
struct SshAgentStatusReaperPermit;

impl SshAgentStatusReaperPermit {
    fn acquire() -> Option<Self> {
        let mut active = ACTIVE_SSH_AGENT_STATUS_REAPERS.load(Ordering::Acquire);
        loop {
            if active >= MAX_SSH_AGENT_STATUS_REAPERS {
                return None;
            }
            match ACTIVE_SSH_AGENT_STATUS_REAPERS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self),
                Err(observed) => active = observed,
            }
        }
    }
}

impl Drop for SshAgentStatusReaperPermit {
    fn drop(&mut self) {
        ACTIVE_SSH_AGENT_STATUS_REAPERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Starts a fixed reaper pool before the first status probe is spawned. This
/// lets timeout cleanup hand a child to an already-owned queue and never fall
/// back to a synchronous `wait` when all probes are busy.
fn ssh_agent_status_reaper_sender() -> Option<mpsc::SyncSender<SshAgentStatusReaperJob>> {
    SSH_AGENT_STATUS_REAPER_SENDER
        .get_or_init(|| {
            let (sender, receiver) =
                mpsc::sync_channel::<SshAgentStatusReaperJob>(MAX_SSH_AGENT_STATUS_REAPERS);
            let receiver = Arc::new(Mutex::new(receiver));
            for worker in 0..MAX_SSH_AGENT_STATUS_REAPERS {
                let receiver = Arc::clone(&receiver);
                if std::thread::Builder::new()
                    .name(format!("bkm-ssh-agent-status-reaper-{worker}"))
                    .spawn(move || loop {
                        let job = {
                            let Ok(receiver) = receiver.lock() else {
                                return;
                            };
                            let Ok(job) = receiver.recv() else {
                                return;
                            };
                            job
                        };
                        let (mut child, _permit) = job;
                        let _ = child.wait();
                    })
                    .is_err()
                {
                    // Dropping the local sender wakes any workers already
                    // created for this failed initialization. Future probes
                    // return an unavailable status without spawning a child.
                    return None;
                }
            }
            Some(sender)
        })
        .as_ref()
        .cloned()
}

/// Retains a killed child if the pre-started queue has unexpectedly become
/// unavailable. Its permit remains held, so this path is bounded and future
/// probes fail closed instead of blocking the caller or creating more waiters.
fn retain_orphaned_ssh_agent_status_job(job: SshAgentStatusReaperJob) {
    let orphans = SSH_AGENT_STATUS_ORPHANED_JOBS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut orphans) = orphans.lock() {
        orphans.push(job);
    } else {
        // A poisoned fallback queue is an internal catastrophic failure. Keep
        // the child and permit leaked rather than dropping the permit and
        // allowing unbounded status probes to accumulate.
        std::mem::forget(job);
    }
}

fn ssh_agent_status_probe() -> &'static (Mutex<SshAgentStatusProbe>, Condvar) {
    SSH_AGENT_STATUS_PROBE.get_or_init(|| {
        (
            Mutex::new(SshAgentStatusProbe {
                in_flight: false,
                last_status: AuthenticationStatus::Unavailable,
            }),
            Condvar::new(),
        )
    })
}

pub(super) fn endpoint_from_state(
    state: &AppState,
    endpoint_id: &str,
) -> Result<CodeHostEndpoint, AppError> {
    endpoint_from_store(&state.snapshot()?, endpoint_id)
}

/// Looks up an Endpoint while a profile-store mutation is in progress. Commands
/// that perform a slow external action first must call this again immediately
/// before persisting an account: the Endpoint may have been removed or edited
/// after their initial snapshot was taken.
pub(super) fn endpoint_from_store(
    store: &ProfileStore,
    endpoint_id: &str,
) -> Result<CodeHostEndpoint, AppError> {
    store
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

/// Endpoint, case-insensitive login, and authentication method form one
/// logical account identity. Keeping it unique makes manual saves agree with
/// CLI import reconciliation and avoids ambiguous automatic credential picks.
pub(super) fn account_profile_identity_matches(
    left: &AccountProfile,
    right: &AccountProfile,
) -> bool {
    left.endpoint_id == right.endpoint_id
        && left.login.eq_ignore_ascii_case(&right.login)
        && left.auth_method == right.auth_method
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

fn current_account_status_with_ssh_agent_status(
    account: &AccountProfile,
    endpoints: &[CodeHostEndpoint],
    ssh_agent: Option<&AuthenticationStatus>,
) -> AuthenticationStatus {
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.id == account.endpoint_id);
    match (&account.auth_method, endpoint) {
        (AuthenticationMethod::PersonalAccessToken, _) => personal_access_token_reference(account)
            .map(personal_access_token_status)
            .unwrap_or(AuthenticationStatus::NeedsReauthentication),
        (AuthenticationMethod::SshAgent, _) => ssh_agent.cloned().unwrap_or_else(ssh_agent_status),
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
            if status.status == AuthenticationStatus::Unavailable {
                AuthenticationStatus::Unavailable
            } else if status.is_usable()
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
            if status.status == AuthenticationStatus::Unavailable {
                AuthenticationStatus::Unavailable
            } else if status.is_usable()
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
    let mut accounts = accounts;
    refresh_account_statuses_in_place(&mut accounts, endpoints);
    accounts
}

/// SSH-agent availability is process-wide. A bulk refresh therefore probes it
/// once and reuses the result for every SSH profile instead of serially
/// spending the bounded probe timeout for each account.
pub(super) fn refresh_account_statuses_in_place(
    accounts: &mut [AccountProfile],
    endpoints: &[CodeHostEndpoint],
) {
    let ssh_agent = accounts
        .iter()
        .any(|account| account.auth_method == AuthenticationMethod::SshAgent)
        .then(ssh_agent_status);
    for account in accounts {
        account.status =
            current_account_status_with_ssh_agent_status(account, endpoints, ssh_agent.as_ref());
    }
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
    let (probe, completion) = ssh_agent_status_probe();
    let mut state = match probe.lock() {
        Ok(state) => state,
        Err(_) => return AuthenticationStatus::Unavailable,
    };
    if state.in_flight {
        let wait =
            completion.wait_timeout_while(state, SSH_AGENT_STATUS_TIMEOUT, |state| state.in_flight);
        let Ok((state, _)) = wait else {
            return AuthenticationStatus::Unavailable;
        };
        return if !state.in_flight {
            state.last_status.clone()
        } else {
            AuthenticationStatus::Unavailable
        };
    }
    state.in_flight = true;
    drop(state);

    let status = probe_ssh_agent_status();
    if let Ok(mut state) = probe.lock() {
        state.in_flight = false;
        state.last_status = status.clone();
        completion.notify_all();
    }
    status
}

/// Runs the single shared SSH-agent probe. Concurrent refreshes wait for this
/// bounded result instead of each reserving a reaper slot while healthy
/// `ssh-add` children are still polling.
fn probe_ssh_agent_status() -> AuthenticationStatus {
    // `ssh-add -l` only asks the already configured agent for public key
    // fingerprints; it neither creates keys nor changes the agent state.
    let mut command = Command::new("ssh-add");
    command
        .arg("-l")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Some(reaper_sender) = ssh_agent_status_reaper_sender() else {
        return AuthenticationStatus::Unavailable;
    };
    // Reserve a bounded reaper slot before spawning. If every prior timed-out
    // probe is still owned by a worker, do not start another child that would
    // later need an unbounded or synchronous cleanup path.
    let Some(reaper_permit) = SshAgentStatusReaperPermit::acquire() else {
        return AuthenticationStatus::Unavailable;
    };
    match command.spawn() {
        Ok(child) => match wait_for_child_status_with_reaper(
            child,
            SSH_AGENT_STATUS_TIMEOUT,
            reaper_sender,
            reaper_permit,
        ) {
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
/// queued for reaping on timeout so repeated account refreshes cannot
/// accumulate zombies or block the caller.
fn wait_for_child_status_with_reaper(
    mut child: Child,
    timeout: Duration,
    reaper_sender: mpsc::SyncSender<SshAgentStatusReaperJob>,
    reaper_permit: SshAgentStatusReaperPermit,
) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                reap_ssh_agent_status_child(child, reaper_sender, reaper_permit);
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                reap_ssh_agent_status_child(child, reaper_sender, reaper_permit);
                return None;
            }
        }
    }
}

fn reap_ssh_agent_status_child(
    child: Child,
    reaper_sender: mpsc::SyncSender<SshAgentStatusReaperJob>,
    reaper_permit: SshAgentStatusReaperPermit,
) {
    if let Err(mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) =
        reaper_sender.try_send((child, reaper_permit))
    {
        retain_orphaned_ssh_agent_status_job(job);
    }
}

#[cfg(test)]
pub(super) fn wait_for_child_status(child: Child, timeout: Duration) -> Option<ExitStatus> {
    let reaper_sender = ssh_agent_status_reaper_sender()?;
    let reaper_permit = SshAgentStatusReaperPermit::acquire()?;
    wait_for_child_status_with_reaper(child, timeout, reaper_sender, reaper_permit)
}
