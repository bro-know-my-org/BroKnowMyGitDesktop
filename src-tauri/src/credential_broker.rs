//! Credential selection and safe Git authentication plumbing.
//!
//! This module deliberately works with account metadata only.  A personal
//! access token is accepted only by [`save_personal_access_token`], stored in
//! the operating system credential store, and read again only by the askpass
//! helper process.  It is never part of a domain model, a serializable type,
//! a command-line argument, or an environment variable.

use crate::app_error::AppError;
use crate::domain::{
    endpoint_for_host, endpoint_matches_authority, git_authority_has_explicit_port,
    hostname_from_git_authority, normalize_git_authority, parse_remote_url, AccountProfile,
    AccountSelection, AuthenticationMethod, AuthenticationStatus, CodeHostEndpoint, CodeHostKind,
    CredentialResolution, CredentialStrategy, KeychainReference, RemoteBinding, RemoteTransport,
};
#[cfg(windows)]
use crate::process_tree::terminate_process_tree;
use crate::process_tree::{attach_process_tree, ProcessTreeHandle};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

/// Stable service name used for application-managed PAT entries.
pub const PAT_KEYCHAIN_SERVICE: &str = "io.github.broknowmyorg.broknowmygitdesktop.pat";

/// Non-secret sentinel used by `main.rs` to enter the private askpass mode
/// before Tauri starts.
pub const ASKPASS_MODE_ENV: &str = "BKM_GIT_ASKPASS_MODE";
const ASKPASS_CAPABILITY_ENV: &str = "BKM_GIT_ASKPASS_CAPABILITY";
const ASKPASS_MODE_KEYCHAIN: &str = "keychain-pat";
const ASKPASS_CAPABILITY_DIRECTORY: &str = "broknowmygitdesktop-askpass";
const ASKPASS_CAPABILITY_TTL: Duration = Duration::from_secs(180);
const PLATFORM_CLI_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PLATFORM_CLI_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PLATFORM_CLI_STDOUT_BYTES: usize = 256 * 1024;
const MAX_PLATFORM_CLI_STDERR_BYTES: usize = 256 * 1024;
const MAX_PLATFORM_CLI_PIPE_OPERATIONS: usize = 4;
const PLATFORM_CLI_DETACHED_PIPE_RELEASE_GRACE: Duration = Duration::from_secs(10);

static ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

/// A permit is retained by each CLI operation while its output workers drain
/// their pipes. If a descendant escapes process-tree termination and retains
/// a pipe, a short expiry releases the concurrency slot rather than leaving
/// gh/glab account detection permanently unavailable.
struct PlatformCliPipeOperationPermit {
    released: AtomicBool,
}

impl PlatformCliPipeOperationPermit {
    fn acquire() -> Result<Arc<Self>, PlatformCliCommandError> {
        let mut active = ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire);
        loop {
            if active >= MAX_PLATFORM_CLI_PIPE_OPERATIONS {
                return Err(PlatformCliCommandError::Busy);
            }
            match ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Arc::new(Self {
                        released: AtomicBool::new(false),
                    }));
                }
                Err(observed) => active = observed,
            }
        }
    }

    /// A terminated child normally closes both pipes immediately. A descendant
    /// may keep one open forever, however, and Rust cannot safely cancel a
    /// blocking `Read` on another thread. Keep the short-term backpressure,
    /// then release the operation slot even if that detached reader survives.
    fn release_after_cleanup_grace(self: &Arc<Self>) {
        let permit = Arc::downgrade(self);
        thread::spawn(move || {
            thread::sleep(PLATFORM_CLI_DETACHED_PIPE_RELEASE_GRACE);
            if let Some(permit) = permit.upgrade() {
                permit.release();
            }
        });
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for PlatformCliPipeOperationPermit {
    fn drop(&mut self) {
        self.release();
    }
}

/// Input to one remote credential resolution.
///
/// This type intentionally does not implement `Debug` or serde traits because
/// `remote_url` can originate outside the application and may contain unsafe
/// userinfo.  The resolver rejects HTTP(S) userinfo before selecting a
/// credential.
pub struct CredentialResolutionRequest<'a> {
    /// Canonical repository path used by persisted `RemoteBinding` records.
    pub repository_path: &'a str,
    pub remote_name: &'a str,
    /// The exact fetch or push URL that Git is about to use.
    pub remote_url: &'a str,
    /// A manually associated endpoint, when one was already selected by UI.
    pub endpoint: Option<&'a CodeHostEndpoint>,
    pub endpoints: &'a [CodeHostEndpoint],
    pub bindings: &'a [RemoteBinding],
    pub accounts: &'a [AccountProfile],
}

/// Stateless broker for remote-to-account resolution.
#[derive(Default)]
pub struct CredentialBroker;

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

impl CredentialEnvironment {
    /// Build the environment plan for a resolved strategy.
    pub fn from_strategy(strategy: &CredentialStrategy) -> Result<Self, AppError> {
        Self::from_strategy_with_account(strategy)
    }

    /// Build the environment plan using the selected account's login for the
    /// Git username prompt.  This is preferred by callers that still have the
    /// resolution and account together.
    pub fn from_resolution(
        resolution: &CredentialResolution,
        account: &AccountProfile,
    ) -> Result<Self, AppError> {
        if resolution.account_profile_id != account.id
            || strategy_account_profile_id(&resolution.strategy) != resolution.account_profile_id
        {
            return Err(AppError::new(
                "credential_resolution_mismatch",
                "认证策略与所选账号不匹配。",
            ));
        }
        Self::from_strategy_with_account(&resolution.strategy)
    }

    fn from_strategy_with_account(strategy: &CredentialStrategy) -> Result<Self, AppError> {
        let askpass = match strategy {
            CredentialStrategy::KeychainPatAskPass {
                account_profile_id,
                keychain_ref,
                authority,
                username,
                ..
            } => Some(AskPassConfig::for_keychain(
                account_profile_id.clone(),
                keychain_ref.clone(),
                authority.clone(),
                username.clone(),
            )?),
        };

        Ok(Self {
            variables: Vec::new(),
            askpass,
        })
    }

    /// Environment pairs that do not depend on the askpass executable path.
    /// These are safe to hand to another backend module; none is a secret.
    pub fn neutral_environment_pairs(&self) -> &[(OsString, OsString)] {
        &self.variables
    }

    /// Returns askpass metadata, not a PAT.  The metadata is only useful to
    /// construct a child-process environment with a trusted helper path.
    pub fn askpass_config(&self) -> Option<&AskPassConfig> {
        self.askpass.as_ref()
    }

    /// Return all environment pairs needed by a Git child process.
    ///
    /// `askpass_program` must be the application's trusted askpass entry
    /// point, which calls [`run_keychain_askpass_from_args`].  The resulting
    /// pairs contain keychain references and a username, never the PAT.
    pub fn environment_pairs(
        &self,
        askpass_program: Option<&Path>,
    ) -> Result<Vec<(OsString, OsString)>, AppError> {
        let mut pairs = self.variables.clone();
        if let Some(askpass) = &self.askpass {
            let program = askpass_program.ok_or_else(|| {
                AppError::new(
                    "askpass_program_required",
                    "该认证方式需要应用的安全 askpass helper。",
                )
            })?;
            pairs.extend(askpass.environment_pairs(program)?);
        }
        Ok(pairs)
    }

    /// Apply this plan directly to a `std::process::Command`.  This uses
    /// process environment APIs only; it does not build a shell command.
    pub fn apply_to_command(
        &self,
        command: &mut Command,
        askpass_program: Option<&Path>,
    ) -> Result<(), AppError> {
        remove_inherited_git_credential_environment(command);
        for (key, value) in self.environment_pairs(askpass_program)? {
            command.env(key, value);
        }
        Ok(())
    }
}

fn remove_inherited_git_credential_environment(command: &mut Command) {
    for name in [
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_TERMINAL_PROMPT",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_GRAFT_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
    ] {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("GIT_CONFIG_") || normalized.starts_with("BKM_GIT_ASKPASS_") {
            command.env_remove(name);
        }
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
}

fn strategy_account_profile_id(strategy: &CredentialStrategy) -> &str {
    match strategy {
        CredentialStrategy::KeychainPatAskPass {
            account_profile_id, ..
        } => account_profile_id,
    }
}

/// Configuration for the private askpass helper. It is stored in a
/// short-lived, owner-only capability record rather than trusted directly
/// from a child process environment.
#[derive(Clone)]
pub struct AskPassConfig {
    source: AskPassSecretSource,
    username: String,
}

/// A secret source identifier, never a secret value.
#[derive(Clone)]
enum AskPassSecretSource {
    Keychain {
        account_profile_id: String,
        keychain_ref: KeychainReference,
        authority: String,
    },
}

/// Serializable, non-secret form of an askpass configuration. This exists
/// only inside an owner-only, short-lived capability file; it is never sent to
/// the frontend or passed as mutable configuration through the environment.
#[derive(Serialize, Deserialize)]
struct AskPassCapabilityConfiguration {
    username: String,
    source: AskPassCapabilitySource,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum AskPassCapabilitySource {
    Keychain {
        account_profile_id: String,
        service: String,
        account: String,
        authority: String,
    },
}

#[derive(Serialize, Deserialize)]
struct AskPassCapabilityRecord {
    expires_at_millis: u128,
    configuration: AskPassCapabilityConfiguration,
}

/// An unguessable, one-password-prompt authorization for the askpass binary.
/// The capability is the random filename, not a keychain reference or token.
struct AskPassCapability {
    id: String,
    path: PathBuf,
}

impl AskPassConfig {
    fn for_keychain(
        account_profile_id: String,
        keychain_ref: KeychainReference,
        authority: String,
        username: String,
    ) -> Result<Self, AppError> {
        if !is_safe_metadata_value(&account_profile_id)
            || keychain_ref.service != PAT_KEYCHAIN_SERVICE
            || !is_safe_metadata_value(&keychain_ref.service)
            || !is_safe_metadata_value(&keychain_ref.account)
            || keychain_ref.account != account_profile_id
            || !is_safe_git_authority(&authority)
            || !is_safe_username(&username)
        {
            return Err(AppError::new(
                "invalid_askpass_configuration",
                "无法安全配置该账号的 askpass 认证。",
            ));
        }
        Ok(Self {
            source: AskPassSecretSource::Keychain {
                account_profile_id,
                keychain_ref,
                authority,
            },
            username,
        })
    }

    fn environment_pairs(
        &self,
        askpass_program: &Path,
    ) -> Result<Vec<(OsString, OsString)>, AppError> {
        let capability = AskPassCapability::create(self)?;
        Ok(vec![
            (
                OsString::from("GIT_ASKPASS"),
                askpass_program.as_os_str().to_os_string(),
            ),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
            (
                OsString::from(ASKPASS_MODE_ENV),
                OsString::from(self.mode()),
            ),
            (
                OsString::from(ASKPASS_CAPABILITY_ENV),
                OsString::from(capability.id),
            ),
        ])
    }

    fn mode(&self) -> &'static str {
        match self.source {
            AskPassSecretSource::Keychain { .. } => ASKPASS_MODE_KEYCHAIN,
        }
    }

    fn capability_configuration(&self) -> AskPassCapabilityConfiguration {
        let source = match &self.source {
            AskPassSecretSource::Keychain {
                account_profile_id,
                keychain_ref,
                authority,
            } => AskPassCapabilitySource::Keychain {
                account_profile_id: account_profile_id.clone(),
                service: keychain_ref.service.clone(),
                account: keychain_ref.account.clone(),
                authority: authority.clone(),
            },
        };
        AskPassCapabilityConfiguration {
            username: self.username.clone(),
            source,
        }
    }

    fn from_capability_configuration(
        configuration: AskPassCapabilityConfiguration,
    ) -> Result<Self, AppError> {
        match configuration.source {
            AskPassCapabilitySource::Keychain {
                account_profile_id,
                service,
                account,
                authority,
            } => Self::for_keychain(
                account_profile_id,
                KeychainReference { service, account },
                authority,
                configuration.username,
            ),
        }
    }

    fn from_process_environment(consume_capability: bool) -> Result<Self, AppError> {
        let mode = std::env::var(ASKPASS_MODE_ENV).map_err(|_| {
            AppError::new(
                "askpass_configuration_missing",
                "askpass 未收到认证模式标识。",
            )
        })?;
        let configuration = AskPassCapability::from_process_environment()?
            .load_configuration(consume_capability)?;
        if mode != configuration.mode() {
            return Err(AppError::new(
                "askpass_configuration_invalid",
                "askpass 收到未知的认证模式。",
            ));
        }
        Ok(configuration)
    }
}

impl AskPassCapability {
    fn create(configuration: &AskPassConfig) -> Result<Self, AppError> {
        let directory = askpass_capability_directory()?;
        cleanup_expired_askpass_capabilities(&directory);
        let expires_at_millis =
            askpass_capability_now_millis()?.saturating_add(ASKPASS_CAPABILITY_TTL.as_millis());
        let record = AskPassCapabilityRecord {
            expires_at_millis,
            configuration: configuration.capability_configuration(),
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| {
            AppError::new(
                "askpass_configuration_invalid",
                "无法创建 askpass 认证授权。",
            )
        })?;

        for _ in 0..4 {
            let id = Uuid::new_v4().to_string();
            let path = directory.join(&id);
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    #[cfg(windows)]
                    if let Err(error) = secure_windows_askpass_path(&path, false) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                    let write_result = file.write_all(&bytes).and_then(|_| file.sync_all());
                    if let Err(error) = write_result {
                        let _ = fs::remove_file(&path);
                        return Err(AppError::io("写入 askpass 认证授权", error));
                    }
                    return Ok(Self { id, path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AppError::io("创建 askpass 认证授权", error)),
            }
        }

        Err(AppError::new(
            "askpass_configuration_invalid",
            "无法创建 askpass 认证授权。",
        ))
    }

    fn from_process_environment() -> Result<Self, AppError> {
        let supplied = std::env::var(ASKPASS_CAPABILITY_ENV).map_err(|_| {
            AppError::new(
                "askpass_configuration_missing",
                "askpass 未收到一次性认证授权。",
            )
        })?;
        let parsed = Uuid::parse_str(&supplied).map_err(|_| askpass_capability_invalid())?;
        let id = parsed.to_string();
        if supplied != id {
            return Err(askpass_capability_invalid());
        }
        let path = askpass_capability_directory()?.join(&id);
        Ok(Self { id, path })
    }

    /// The username prompt may inspect a capability, but the password prompt
    /// atomically renames and consumes it before any keychain/CLI secret is
    /// read. A second password helper therefore fails closed.
    fn load_configuration(&self, consume: bool) -> Result<AskPassConfig, AppError> {
        let path = if consume {
            let consumed_path =
                self.path
                    .with_file_name(format!(".{}-{}.consumed", self.id, Uuid::new_v4()));
            fs::rename(&self.path, &consumed_path).map_err(|_| askpass_capability_invalid())?;
            consumed_path
        } else {
            self.path.clone()
        };

        let record = read_askpass_capability_record(&path);
        if consume {
            let _ = fs::remove_file(&path);
        }
        let record = record?;
        if record.expires_at_millis <= askpass_capability_now_millis()? {
            if !consume {
                let _ = fs::remove_file(&self.path);
            }
            return Err(askpass_capability_invalid());
        }

        AskPassConfig::from_capability_configuration(record.configuration)
            .map_err(|_| askpass_capability_invalid())
    }
}

fn askpass_capability_directory() -> Result<PathBuf, AppError> {
    let directory = std::env::temp_dir().join(ASKPASS_CAPABILITY_DIRECTORY);
    let builder = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;

            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
        }
        #[cfg(not(unix))]
        {
            fs::DirBuilder::new()
        }
    };
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(AppError::io("创建 askpass 授权目录", error)),
    }

    let metadata = fs::symlink_metadata(&directory)
        .map_err(|error| AppError::io("检查 askpass 授权目录", error))?;
    if !metadata.file_type().is_dir() {
        return Err(askpass_capability_invalid());
    }
    #[cfg(windows)]
    secure_windows_askpass_path(&directory, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(askpass_capability_invalid());
        }
    }
    Ok(directory)
}

/// Replace a capability path's owner and DACL with the current user's SID.
/// Windows does not honor Unix-style creation modes, so a restrictive DACL is
/// required before a capability record receives any secret-bearing metadata.
#[cfg(windows)]
fn secure_windows_askpass_path(path: &Path, inherit_to_children: bool) -> Result<(), AppError> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, GENERIC_ALL};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
        NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let security_error = || {
        AppError::new(
            "askpass_capability_invalid",
            "无法验证 askpass 授权目录的 Windows 访问权限。",
        )
    };
    let mut token = ptr::null_mut();
    // SAFETY: the current process pseudo-handle is valid, and `token` points
    // to storage for the token handle returned by the synchronous API.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(security_error());
    }

    let result = (|| {
        let mut required_bytes = 0_u32;
        // SAFETY: querying with a null buffer obtains the required size.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required_bytes);
        }
        if required_bytes == 0 {
            return Err(security_error());
        }
        let mut token_user_bytes = vec![0_u8; required_bytes as usize];
        // SAFETY: the buffer is exactly the size Windows requested and is
        // retained while the returned SID pointer is used below.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                token_user_bytes.as_mut_ptr().cast(),
                required_bytes,
                &mut required_bytes,
            )
        } == 0
        {
            return Err(security_error());
        }
        // SAFETY: `GetTokenInformation(TokenUser)` populated this buffer with
        // a `TOKEN_USER` at its beginning.
        let token_user = unsafe { &*(token_user_bytes.as_ptr().cast::<TOKEN_USER>()) };
        let mut access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: if inherit_to_children {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT
            } else {
                NO_INHERITANCE
            },
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: token_user.User.Sid.cast(),
            },
        };
        let mut acl = ptr::null_mut();
        // SAFETY: `access` contains the current user's SID and `acl` receives
        // the ACL allocation that Windows requires us to free with LocalFree.
        if unsafe { SetEntriesInAclW(1, &mut access, ptr::null(), &mut acl) } != 0 {
            return Err(security_error());
        }
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        // SAFETY: the path is NUL-terminated and the SID/ACL remain valid for
        // this call. Replacing both owner and protected DACL prevents inherited
        // or pre-existing principals from reading future capability records.
        let set_result = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                token_user.User.Sid,
                ptr::null_mut(),
                acl,
                ptr::null(),
            )
        };
        // SAFETY: SetEntriesInAclW allocated this local-memory block.
        unsafe {
            let _ = LocalFree(acl.cast());
        }
        if set_result != 0 {
            return Err(security_error());
        }
        Ok(())
    })();

    // SAFETY: OpenProcessToken returned this owned handle.
    unsafe {
        let _ = CloseHandle(token);
    }
    result
}

fn read_askpass_capability_record(path: &Path) -> Result<AskPassCapabilityRecord, AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| askpass_capability_invalid())?;
    if !metadata.file_type().is_file() || metadata.len() > 16 * 1024 {
        return Err(askpass_capability_invalid());
    }
    let mut file = fs::File::open(path).map_err(|_| askpass_capability_invalid())?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| askpass_capability_invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| askpass_capability_invalid())
}

fn cleanup_expired_askpass_capabilities(directory: &Path) {
    let Ok(now) = askpass_capability_now_millis() else {
        return;
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let expired = read_askpass_capability_record(&path)
            .map(|record| record.expires_at_millis <= now)
            .unwrap_or(false);
        if expired {
            let _ = fs::remove_file(path);
        }
    }
}

fn askpass_capability_now_millis() -> Result<u128, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| askpass_capability_invalid())
}

fn askpass_capability_invalid() -> AppError {
    AppError::new(
        "askpass_capability_invalid",
        "askpass 未收到有效的一次性认证授权。",
    )
}

/// Used by the binary entrypoint to avoid treating an ordinary desktop launch
/// with a stray mode variable as an askpass process.
pub fn askpass_invocation_requested() -> bool {
    std::env::var_os(ASKPASS_MODE_ENV).is_some()
        && std::env::var_os(ASKPASS_CAPABILITY_ENV).is_some()
}

/// Askpass entry point for a dedicated child-process mode of the application.
///
/// The surrounding executable should invoke this function before initializing
/// Tauri, pass Git's first argument as the prompt, and write no diagnostics to
/// stdout. On a password prompt, the selected keychain PAT flows directly to
/// Git's askpass pipe.
pub fn run_keychain_askpass_from_args() -> Result<(), AppError> {
    let prompt = std::env::args_os().nth(1).ok_or_else(|| {
        AppError::new("askpass_prompt_missing", "askpass 未收到 Git 的认证提示。")
    })?;
    run_keychain_askpass(&prompt)
}

/// Same as [`run_keychain_askpass_from_args`], with a prompt supplied by the
/// embedding executable.  This remains backend-only and must never be wired
/// to a Tauri command.
pub fn run_keychain_askpass(prompt: &OsStr) -> Result<(), AppError> {
    let response = askpass_response(prompt)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(response.as_bytes()).map_err(|_| {
        AppError::new(
            "askpass_output_failed",
            "无法向 Git 的 askpass 通道写入认证响应。",
        )
    })?;
    stdout.write_all(b"\n").map_err(|_| {
        AppError::new(
            "askpass_output_failed",
            "无法完成 Git 的 askpass 认证响应。",
        )
    })?;
    stdout.flush().map_err(|_| {
        AppError::new(
            "askpass_output_failed",
            "无法完成 Git 的 askpass 认证响应。",
        )
    })
}

fn askpass_response(prompt: &OsStr) -> Result<String, AppError> {
    let prompt = prompt.to_string_lossy();
    let prompt_lower = prompt.to_ascii_lowercase();
    let prompt_kind = askpass_prompt_kind(&prompt_lower)?;
    // A password prompt is the only point at which a secret can be read, so
    // consuming the one-shot capability here prevents a second helper process
    // from replaying the same keychain/CLI request.
    let configuration = AskPassConfig::from_process_environment(matches!(
        prompt_kind,
        AskPassPromptKind::Password
    ))?;
    let prompt_authority = match prompt_kind {
        AskPassPromptKind::Username => askpass_prompt_authority(&prompt)?,
        AskPassPromptKind::Password => {
            askpass_password_prompt_authority(&prompt, &configuration.username)?
        }
    };
    hostname_from_git_authority(&prompt_authority)
        .filter(|hostname| is_safe_hostname(hostname))
        .ok_or_else(|| {
            AppError::new(
                "askpass_prompt_unsupported",
                "askpass 未收到可验证的 HTTPS 主机认证提示。",
            )
        })?;

    match prompt_kind {
        AskPassPromptKind::Username => {
            askpass_username_for_prompt(&configuration, &prompt_authority)
        }
        AskPassPromptKind::Password => match configuration.source {
            AskPassSecretSource::Keychain {
                account_profile_id,
                keychain_ref,
                authority,
            } if keychain_ref.account == account_profile_id
                && authority.eq_ignore_ascii_case(&prompt_authority) =>
            {
                read_personal_access_token(&keychain_ref)
            }
            _ => Err(askpass_prompt_host_mismatch()),
        },
    }
}

#[derive(Clone, Copy)]
enum AskPassPromptKind {
    Username,
    Password,
}

fn askpass_prompt_kind(prompt_lower: &str) -> Result<AskPassPromptKind, AppError> {
    let prompt_lower = prompt_lower.trim_start();
    if prompt_lower.starts_with("username for ") {
        return Ok(AskPassPromptKind::Username);
    }
    if prompt_lower.starts_with("password for ") {
        return Ok(AskPassPromptKind::Password);
    }
    Err(AppError::new(
        "askpass_prompt_unsupported",
        "askpass 收到不受支持的 Git 认证提示。",
    ))
}

fn askpass_username_for_prompt(
    configuration: &AskPassConfig,
    prompt_authority: &str,
) -> Result<String, AppError> {
    let matches_prompt = match &configuration.source {
        AskPassSecretSource::Keychain { authority, .. } => {
            authority.eq_ignore_ascii_case(prompt_authority)
        }
    };
    if matches_prompt {
        Ok(configuration.username.clone())
    } else {
        Err(askpass_prompt_host_mismatch())
    }
}

fn askpass_prompt_host_mismatch() -> AppError {
    AppError::new(
        "askpass_prompt_host_mismatch",
        "askpass 拒绝向不匹配的主机提供认证信息。",
    )
}

/// Git askpass prompts quote the URL that needs authentication, e.g.
/// `Password for 'https://github.com':`. Preserve the parsed authority so a
/// PAT selected for one non-default HTTPS port cannot be offered to another.
fn askpass_prompt_authority(prompt: &str) -> Result<String, AppError> {
    askpass_prompt_authority_with_username(prompt, None)
}

/// Git commonly repeats the username in its password prompt, e.g.
/// `Password for 'https://work-login@gitee.com':`. That username is normal
/// protocol state, not remote URL credentials, but it must exactly match the
/// account selected by the parent Git operation before a secret is released.
fn askpass_password_prompt_authority(
    prompt: &str,
    expected_username: &str,
) -> Result<String, AppError> {
    askpass_prompt_authority_with_username(prompt, Some(expected_username))
}

fn askpass_prompt_authority_with_username(
    prompt: &str,
    expected_username: Option<&str>,
) -> Result<String, AppError> {
    for candidate in prompt.split(['\'', '"']) {
        let candidate = candidate.trim();
        if !candidate.starts_with("https://") {
            continue;
        }
        let Ok(url) = Url::parse(candidate) else {
            continue;
        };
        if url.scheme() != "https"
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            continue;
        }
        if !url.username().is_empty()
            && !expected_username
                .is_some_and(|username| askpass_prompt_username_matches(&url, username))
        {
            continue;
        }
        let parsed = parse_remote_url(candidate);
        if matches!(parsed.transport, RemoteTransport::Https) {
            if let Some(authority) = parsed
                .authority()
                .filter(|authority| is_safe_git_authority(authority))
            {
                return Ok(authority);
            }
        }
    }

    Err(AppError::new(
        "askpass_prompt_unsupported",
        "askpass 未收到可验证的 HTTPS 主机认证提示。",
    ))
}

fn askpass_prompt_username_matches(url: &Url, expected_username: &str) -> bool {
    if !is_safe_username(expected_username) {
        return false;
    }
    let Ok(mut expected_url) = Url::parse("https://bkm.invalid/") else {
        return false;
    };
    expected_url.set_username(expected_username).is_ok()
        && url.username() == expected_url.username()
}

/// A supported platform CLI.  Its output is reduced to safe account metadata
/// and never returned verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformCli {
    Gh,
    Glab,
}

impl PlatformCli {
    fn executable(self) -> &'static str {
        match self {
            Self::Gh => "gh",
            Self::Glab => "glab",
        }
    }

    fn authentication_method(self) -> AuthenticationMethod {
        match self {
            Self::Gh => AuthenticationMethod::GhCli,
            Self::Glab => AuthenticationMethod::GlabCli,
        }
    }
}

/// A deliberately small error surface for CLI probes. Their diagnostics can
/// contain local paths, hostnames, or credential-helper text and must never be
/// sent through IPC.
#[derive(Debug)]
enum PlatformCliCommandError {
    NotFound,
    Failed,
    TimedOut,
    Busy,
}

/// Execute a gh/glab probe with a finite lifetime. Account status checks can
/// involve a keychain helper or a network request, so `Command::output()` is
/// not safe on an IPC path.
fn run_platform_cli_command(
    cli: PlatformCli,
    arguments: &[&str],
) -> Result<Output, PlatformCliCommandError> {
    run_platform_cli_program(OsStr::new(cli.executable()), arguments)
}

fn run_platform_cli_program(
    executable: &OsStr,
    arguments: &[&str],
) -> Result<Output, PlatformCliCommandError> {
    let operation_permit = PlatformCliPipeOperationPermit::acquire()?;
    let mut command = Command::new(executable);
    configure_platform_cli_process_group(&mut command);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PlatformCliCommandError::NotFound
        } else {
            PlatformCliCommandError::Failed
        }
    })?;
    let process_id = child.id();
    let process_tree = match attach_process_tree(&child) {
        Ok(process_tree) => process_tree,
        Err(_) => {
            terminate_platform_cli_child(&mut child, process_id);
            return Err(PlatformCliCommandError::Failed);
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_platform_cli_child(&mut child, process_id);
            return Err(PlatformCliCommandError::Failed);
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_platform_cli_child(&mut child, process_id);
            return Err(PlatformCliCommandError::Failed);
        }
    };
    let stdout_reader = spawn_platform_cli_pipe_reader(
        stdout,
        MAX_PLATFORM_CLI_STDOUT_BYTES,
        Arc::clone(&operation_permit),
    );
    let stderr_reader = spawn_platform_cli_pipe_reader(
        stderr,
        MAX_PLATFORM_CLI_STDERR_BYTES,
        Arc::clone(&operation_permit),
    );
    let mut stdout_ready = None;
    let mut stderr_ready = None;
    let deadline = Instant::now() + PLATFORM_CLI_COMMAND_TIMEOUT;
    let status = loop {
        if poll_platform_cli_pipe(&stdout_reader, &mut stdout_ready).is_err()
            || poll_platform_cli_pipe(&stderr_reader, &mut stderr_ready).is_err()
        {
            terminate_platform_cli_child(&mut child, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(PlatformCliCommandError::Failed);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                terminate_platform_cli_child(&mut child, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(PlatformCliCommandError::TimedOut);
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                terminate_platform_cli_child(&mut child, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(PlatformCliCommandError::Failed);
            }
        }
    };
    let stdout = match collect_platform_cli_pipe(stdout_reader, stdout_ready, deadline) {
        Ok(stdout) => stdout,
        Err(error) => {
            terminate_remaining_platform_cli_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    let stderr = match collect_platform_cli_pipe(stderr_reader, stderr_ready, deadline) {
        Ok(stderr) => stderr,
        Err(error) => {
            terminate_remaining_platform_cli_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Debug)]
enum PlatformCliPipeError {
    TooLarge,
    Io(std::io::Error),
}

fn spawn_platform_cli_pipe_reader(
    pipe: impl Read + Send + 'static,
    maximum_bytes: usize,
    operation_permit: Arc<PlatformCliPipeOperationPermit>,
) -> Receiver<Result<Vec<u8>, PlatformCliPipeError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _permit = operation_permit;
        let _ = sender.send(read_platform_cli_pipe(pipe, maximum_bytes));
    });
    receiver
}

fn read_platform_cli_pipe(
    mut pipe: impl Read,
    maximum_bytes: usize,
) -> Result<Vec<u8>, PlatformCliPipeError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(PlatformCliPipeError::Io)?;
        if count == 0 {
            break;
        }
        let remaining = maximum_bytes.saturating_sub(bytes.len());
        if count > remaining {
            return Err(PlatformCliPipeError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn poll_platform_cli_pipe(
    reader: &Receiver<Result<Vec<u8>, PlatformCliPipeError>>,
    ready: &mut Option<Result<Vec<u8>, PlatformCliPipeError>>,
) -> Result<(), PlatformCliCommandError> {
    if ready.is_some() {
        return Ok(());
    }
    match reader.try_recv() {
        Ok(Ok(bytes)) => {
            *ready = Some(Ok(bytes));
            Ok(())
        }
        Ok(Err(PlatformCliPipeError::TooLarge)) => Err(PlatformCliCommandError::Failed),
        Ok(Err(PlatformCliPipeError::Io(error))) => {
            let _ = error.kind();
            Err(PlatformCliCommandError::Failed)
        }
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(PlatformCliCommandError::Failed),
    }
}

fn collect_platform_cli_pipe(
    reader: Receiver<Result<Vec<u8>, PlatformCliPipeError>>,
    ready: Option<Result<Vec<u8>, PlatformCliPipeError>>,
    deadline: Instant,
) -> Result<Vec<u8>, PlatformCliCommandError> {
    let result = match ready {
        Some(result) => result,
        None => match reader.recv_timeout(remaining_platform_cli_collection_timeout(deadline)?) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => return Err(PlatformCliCommandError::TimedOut),
            Err(RecvTimeoutError::Disconnected) => return Err(PlatformCliCommandError::Failed),
        },
    };
    match result {
        Ok(bytes) => Ok(bytes),
        Err(PlatformCliPipeError::TooLarge) => Err(PlatformCliCommandError::Failed),
        Err(PlatformCliPipeError::Io(error)) => {
            let _ = error.kind();
            Err(PlatformCliCommandError::Failed)
        }
    }
}

fn remaining_platform_cli_collection_timeout(
    deadline: Instant,
) -> Result<Duration, PlatformCliCommandError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(PlatformCliCommandError::TimedOut);
    }
    Ok(remaining.min(PLATFORM_CLI_PIPE_COLLECTION_TIMEOUT))
}

fn terminate_platform_cli_child(child: &mut Child, process_id: u32) {
    terminate_platform_cli_process_group(process_id);
    let _ = child.kill();
    let _ = child.wait();
}

/// Once the direct CLI child is reaped, use a retained Job Object on Windows
/// rather than a numeric PID. Unix can still address its dedicated process
/// group while any descendant remains in it.
fn terminate_remaining_platform_cli_process_tree(
    _process_tree: &ProcessTreeHandle,
    _process_id: u32,
) {
    #[cfg(unix)]
    terminate_platform_cli_process_group(_process_id);
    #[cfg(windows)]
    terminate_process_tree(_process_tree);
}

#[cfg(unix)]
fn configure_platform_cli_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Delegate the post-fork setup to the standard library rather than
    // invoking Rust code from a `pre_exec` closure in this multithreaded app.
    command.process_group(0);
}

#[cfg(windows)]
fn configure_platform_cli_process_group(_command: &mut Command) {}

#[cfg(all(not(unix), not(windows)))]
fn configure_platform_cli_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_platform_cli_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the dedicated child process group.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_platform_cli_process_group(process_id: u32) {
    let process_id = process_id.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", process_id.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_platform_cli_process_group(_process_id: u32) {}

/// Safe result of checking a platform CLI's currently active account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliAuthenticationStatus {
    pub cli: PlatformCli,
    pub hostname: String,
    pub login: Option<String>,
    pub status: AuthenticationStatus,
}

impl CliAuthenticationStatus {
    pub fn is_usable(&self) -> bool {
        matches!(
            &self.status,
            AuthenticationStatus::Available | AuthenticationStatus::Valid
        )
    }
}

/// Check the current `gh` authentication state without changing it.
pub fn gh_auth_status(hostname: &str) -> CliAuthenticationStatus {
    cli_auth_status(PlatformCli::Gh, hostname)
}

/// Check the current `glab` authentication state without changing it.
pub fn glab_auth_status(hostname: &str) -> CliAuthenticationStatus {
    cli_auth_status(PlatformCli::Glab, hostname)
}

fn cli_auth_status(cli: PlatformCli, hostname: &str) -> CliAuthenticationStatus {
    let hostname = hostname.trim().to_ascii_lowercase();
    if !is_safe_hostname(&hostname) {
        return CliAuthenticationStatus {
            cli,
            hostname,
            login: None,
            status: AuthenticationStatus::Unavailable,
        };
    }

    let output = run_platform_cli_command(cli, &["auth", "status", "--hostname", &hostname]);

    match output {
        Ok(output) if output.status.success() => CliAuthenticationStatus {
            cli,
            hostname,
            login: parse_login_from_status_output(&output),
            status: AuthenticationStatus::Valid,
        },
        Ok(_) => CliAuthenticationStatus {
            cli,
            hostname,
            login: None,
            status: AuthenticationStatus::NeedsReauthentication,
        },
        Err(PlatformCliCommandError::NotFound) => CliAuthenticationStatus {
            cli,
            hostname,
            login: None,
            status: AuthenticationStatus::Unavailable,
        },
        Err(_) => CliAuthenticationStatus {
            cli,
            hostname,
            login: None,
            status: AuthenticationStatus::Unavailable,
        },
    }
}

/// Import the account currently active in `gh` into the common account model.
/// The caller owns persistence and may choose a deterministic profile ID.
pub fn import_gh_current_account(
    endpoint: &CodeHostEndpoint,
    account_profile_id: impl Into<String>,
) -> Result<AccountProfile, AppError> {
    import_current_cli_account(PlatformCli::Gh, endpoint, account_profile_id.into())
}

/// Import the account currently active in `glab` into the common account model.
/// The caller owns persistence and may choose a deterministic profile ID.
pub fn import_glab_current_account(
    endpoint: &CodeHostEndpoint,
    account_profile_id: impl Into<String>,
) -> Result<AccountProfile, AppError> {
    import_current_cli_account(PlatformCli::Glab, endpoint, account_profile_id.into())
}

/// Convenience import helper for callers that do not yet have a profile ID.
pub fn import_gh_current_account_new(
    endpoint: &CodeHostEndpoint,
) -> Result<AccountProfile, AppError> {
    import_gh_current_account(endpoint, uuid::Uuid::new_v4().to_string())
}

/// Convenience import helper for callers that do not yet have a profile ID.
pub fn import_glab_current_account_new(
    endpoint: &CodeHostEndpoint,
) -> Result<AccountProfile, AppError> {
    import_glab_current_account(endpoint, uuid::Uuid::new_v4().to_string())
}

fn import_current_cli_account(
    cli: PlatformCli,
    endpoint: &CodeHostEndpoint,
    account_profile_id: String,
) -> Result<AccountProfile, AppError> {
    let supported = matches!(
        (cli, &endpoint.kind),
        (PlatformCli::Gh, CodeHostKind::GitHub) | (PlatformCli::Glab, CodeHostKind::GitLab)
    );
    let hostname = hostname_from_git_authority(&endpoint.git_host);
    if !supported
        || git_authority_has_explicit_port(&endpoint.git_host)
        || !hostname.as_deref().is_some_and(is_safe_hostname)
        || account_profile_id.trim().is_empty()
    {
        return Err(AppError::new(
            "cli_account_import_invalid",
            "该 Endpoint 不能使用所选 CLI 账号导入方式。",
        ));
    }

    let hostname = hostname.ok_or_else(|| {
        AppError::new(
            "cli_account_import_invalid",
            "该 Endpoint 不能使用所选 CLI 账号导入方式。",
        )
    })?;
    let status = cli_auth_status(cli, &hostname);
    if !status.is_usable() {
        return Err(AppError::new(
            "cli_auth_unavailable",
            "当前 CLI 没有可用的认证账号。",
        ));
    }

    // The API query asks the CLI for the account that is active now.  It does
    // not request or expose its token, and no command changes CLI state.
    let login = current_cli_login(cli, &hostname).ok_or_else(|| {
        AppError::new(
            "cli_account_import_failed",
            "无法读取当前 CLI 认证账号的登录名。",
        )
    })?;

    Ok(AccountProfile {
        id: account_profile_id,
        endpoint_id: endpoint.id.clone(),
        login,
        display_name: None,
        auth_method: cli.authentication_method(),
        secret_ref: None,
        status: AuthenticationStatus::Valid,
        is_default_candidate: false,
    })
}

fn current_cli_login(cli: PlatformCli, hostname: &str) -> Option<String> {
    let output = match cli {
        PlatformCli::Gh => run_platform_cli_command(
            cli,
            &["api", "user", "--hostname", hostname, "--jq", ".login"],
        )
        .ok()?,
        PlatformCli::Glab => {
            run_platform_cli_command(cli, &["api", "user", "--hostname", hostname]).ok()?
        }
    };

    if !output.status.success() {
        return None;
    }

    match cli {
        PlatformCli::Gh => normalized_login(std::str::from_utf8(&output.stdout).ok()?),
        PlatformCli::Glab => {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            value
                .get("username")
                .and_then(serde_json::Value::as_str)
                .or_else(|| value.get("login").and_then(serde_json::Value::as_str))
                .and_then(normalized_login)
        }
    }
}

fn parse_login_from_status_output(output: &Output) -> Option<String> {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for marker in [" account ", " as "] {
        if let Some((_, remainder)) = combined.rsplit_once(marker) {
            if let Some(candidate) = remainder.split_whitespace().next() {
                if let Some(login) = normalized_login(candidate) {
                    return Some(login);
                }
            }
        }
    }
    None
}

fn normalized_login(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| matches!(character, '(' | ')' | '[' | ']' | ',' | ':'));
    is_safe_username(value).then(|| value.to_owned())
}

fn is_safe_hostname(hostname: &str) -> bool {
    is_safe_metadata_value(hostname)
        && !hostname.contains(['/', '\\', '@', '[', ']'])
        && (hostname.parse::<IpAddr>().is_ok()
            || (!hostname.contains(':')
                && hostname.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
                })))
}

fn is_safe_git_authority(authority: &str) -> bool {
    normalize_git_authority(authority).as_deref() == Some(authority)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> CodeHostEndpoint {
        CodeHostEndpoint {
            id: "gitee.com".into(),
            kind: CodeHostKind::Gitee,
            display_name: "Gitee.com".into(),
            web_url: "https://gitee.com".into(),
            api_url: "https://gitee.com/api/v5".into(),
            git_host: "gitee.com".into(),
            is_builtin: true,
        }
    }

    fn account(
        id: &str,
        auth_method: AuthenticationMethod,
        status: AuthenticationStatus,
        is_default_candidate: bool,
    ) -> AccountProfile {
        let secret_ref =
            matches!(&auth_method, AuthenticationMethod::PersonalAccessToken).then(|| {
                KeychainReference {
                    service: PAT_KEYCHAIN_SERVICE.into(),
                    account: id.into(),
                }
            });
        AccountProfile {
            id: id.into(),
            endpoint_id: "gitee.com".into(),
            login: format!("{id}-login"),
            display_name: None,
            auth_method,
            secret_ref,
            status,
            is_default_candidate,
        }
    }

    fn resolve(
        endpoints: &[CodeHostEndpoint],
        bindings: &[RemoteBinding],
        accounts: &[AccountProfile],
        remote_url: &str,
    ) -> Result<CredentialResolution, AppError> {
        CredentialBroker.resolve(CredentialResolutionRequest {
            repository_path: "/work/demo/",
            remote_name: "origin",
            remote_url,
            endpoint: None,
            endpoints,
            bindings,
            accounts,
        })
    }

    #[test]
    fn explicit_binding_wins_over_default_candidate() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::SystemGitCredentialHelper,
                AuthenticationStatus::Valid,
                true,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                false,
            ),
        ];
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "gitee.com".into(),
            account_selection: AccountSelection::Profile("work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("explicit binding should resolve");

        assert_eq!(resolved.account_profile_id, "work");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass {
                ref account_profile_id,
                ..
            } if account_profile_id == "work"
        ));
    }

    #[test]
    fn auto_uses_its_only_usable_candidate() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "expired",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::NeedsReauthentication,
                false,
            ),
            account(
                "usable",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Available,
                false,
            ),
        ];

        let resolved = resolve(
            &endpoints,
            &[],
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("only one usable account should resolve");

        assert_eq!(resolved.account_profile_id, "usable");
    }

    #[test]
    fn auto_uses_the_only_valid_default_when_multiple_accounts_exist() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::SystemGitCredentialHelper,
                AuthenticationStatus::Valid,
                false,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                true,
            ),
        ];
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "gitee.com".into(),
            account_selection: AccountSelection::Auto,
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("one valid default candidate should resolve");

        assert_eq!(resolved.account_profile_id, "work");
    }

    #[test]
    fn auto_never_guesses_between_multiple_candidates() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                false,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Available,
                false,
            ),
        ];

        let error = resolve(
            &endpoints,
            &[],
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect_err("ambiguous auto selection must fail");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn ssh_agent_profiles_are_not_pinnable_to_remote_bindings() {
        let endpoints = vec![endpoint()];
        let accounts = vec![account(
            "ssh-account",
            AuthenticationMethod::SshAgent,
            AuthenticationStatus::Valid,
            false,
        )];

        let error = resolve(
            &endpoints,
            &[RemoteBinding {
                repository_path: "/work/demo".into(),
                remote_name: "origin".into(),
                endpoint_id: "gitee.com".into(),
                account_selection: AccountSelection::Profile("ssh-account".into()),
            }],
            &accounts,
            "git@gitee.com:team/project.git",
        )
        .expect_err("an SSH agent profile cannot prove a selected identity");
        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn credential_resolution_rejects_plain_http() {
        let endpoints = vec![endpoint()];
        let accounts = vec![account(
            "helper-account",
            AuthenticationMethod::SystemGitCredentialHelper,
            AuthenticationStatus::Valid,
            false,
        )];

        let error = resolve(
            &endpoints,
            &[],
            &accounts,
            "http://gitee.com/team/project.git",
        )
        .expect_err("credentials must never be sent over plain HTTP");
        assert_eq!(error.code(), "unsupported_remote_transport");
    }

    #[test]
    fn credential_url_validation_rejects_token_bearing_queries_and_fragments() {
        assert_eq!(
            validate_remote_url_for_credentials(
                "https://gitee.com/team/project.git?access_token=hidden",
                &RemoteTransport::Https,
            )
            .expect_err("HTTPS remote queries can retain tokens")
            .code(),
            "unsafe_remote_url"
        );
        assert_eq!(
            validate_remote_url_for_credentials(
                "ssh://git@gitee.com/team/project.git#token",
                &RemoteTransport::Ssh,
            )
            .expect_err("SSH URL fragments are not repository paths")
            .code(),
            "unsafe_remote_url"
        );
        assert_eq!(
            validate_remote_url_for_credentials(
                "git@gitee.com:team/project.git?access_token=hidden",
                &RemoteTransport::Ssh,
            )
            .expect_err("SCP-style SSH remotes cannot retain token-like query data")
            .code(),
            "unsafe_remote_url"
        );
    }

    #[test]
    fn generic_git_host_resolves_an_explicit_https_pat_binding() {
        let endpoints = vec![CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        }];
        let mut profile = account(
            "example-work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile.endpoint_id = "git.example.test".into();
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "git.example.test".into(),
            account_selection: AccountSelection::Profile("example-work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &[profile],
            "https://git.example.test/team/project.git",
        )
        .expect("a generic Git endpoint should support HTTPS PAT authentication");

        assert_eq!(resolved.endpoint_id, "git.example.test");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass { .. }
        ));
    }

    #[test]
    fn non_default_endpoint_port_requires_an_exact_remote_and_keeps_its_askpass_authority() {
        let endpoints = vec![CodeHostEndpoint {
            id: "git.example.test-8443".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test:8443".into(),
            is_builtin: false,
        }];
        let mut profile = account(
            "example-work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile.endpoint_id = "git.example.test-8443".into();
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "git.example.test-8443".into(),
            account_selection: AccountSelection::Profile("example-work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &[profile.clone()],
            "https://git.example.test:8443/team/project.git",
        )
        .expect("matching non-default port should resolve");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass { ref authority, .. }
                if authority == "git.example.test:8443"
        ));

        let error = resolve(
            &endpoints,
            &bindings,
            &[profile],
            "https://git.example.test/team/project.git",
        )
        .expect_err("a port-scoped binding must not apply to the default port");
        assert_eq!(error.code(), "endpoint_association_required");
    }

    #[test]
    fn askpass_environment_contains_only_a_one_shot_capability() {
        let strategy = CredentialStrategy::KeychainPatAskPass {
            account_profile_id: "work".into(),
            keychain_ref: KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            authority: "gitee.com:8443".into(),
            username: "work-login".into(),
        };

        let environment = CredentialEnvironment::from_strategy(&strategy).unwrap();
        let pairs = environment
            .environment_pairs(Some(Path::new("/trusted/bkm-askpass")))
            .unwrap();
        let rendered = pairs
            .iter()
            .map(|(_, value)| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join("|");

        assert!(!rendered.contains(PAT_KEYCHAIN_SERVICE));
        assert!(!rendered.contains("not-a-personal-access-token"));
        assert!(pairs.iter().all(|(key, _)| key != "GIT_PASSWORD"));
        assert!(pairs
            .iter()
            .any(|(key, value)| { key == ASKPASS_MODE_ENV && value == ASKPASS_MODE_KEYCHAIN }));
        let capability_id = pairs
            .iter()
            .find(|(key, _)| key == ASKPASS_CAPABILITY_ENV)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .expect("askpass capability is present");
        let capability = AskPassCapability {
            path: askpass_capability_directory().unwrap().join(&capability_id),
            id: capability_id,
        };
        let restored = capability
            .load_configuration(true)
            .expect("capability restores the selected askpass configuration");
        assert!(matches!(
            restored.source,
            AskPassSecretSource::Keychain { authority, .. } if authority == "gitee.com:8443"
        ));
    }

    #[test]
    fn cli_profiles_are_not_repurposed_as_git_https_credentials() {
        let endpoint = CodeHostEndpoint {
            id: "github.com".into(),
            kind: CodeHostKind::GitHub,
            display_name: "GitHub.com".into(),
            web_url: "https://github.com".into(),
            api_url: "https://api.github.com".into(),
            git_host: "github.com".into(),
            is_builtin: true,
        };
        let profile = AccountProfile {
            id: "octo".into(),
            endpoint_id: endpoint.id.clone(),
            login: "octo".into(),
            display_name: None,
            auth_method: AuthenticationMethod::GhCli,
            secret_ref: None,
            status: AuthenticationStatus::Valid,
            is_default_candidate: true,
        };

        assert!(strategy_for_account(&profile, &endpoint, &RemoteTransport::Https).is_none());
    }

    #[test]
    fn system_helper_profiles_are_not_explicit_git_credential_candidates() {
        let endpoint = endpoint();
        let profile = account(
            "work",
            AuthenticationMethod::SystemGitCredentialHelper,
            AuthenticationStatus::Valid,
            true,
        );

        assert!(strategy_for_account(&profile, &endpoint, &RemoteTransport::Https).is_none());
        assert!(!CredentialBroker::has_explicit_account_candidate(
            &[profile],
            &endpoint,
            &RemoteTransport::Https,
        ));
    }

    #[test]
    fn askpass_capability_rejects_a_rebound_keychain_account() {
        let configuration = AskPassCapabilityConfiguration {
            username: "work-login".into(),
            source: AskPassCapabilitySource::Keychain {
                account_profile_id: "work".into(),
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "personal".into(),
                authority: "gitee.com".into(),
            },
        };

        let error = match AskPassConfig::from_capability_configuration(configuration) {
            Ok(_) => panic!("a capability must not read another profile's PAT"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "invalid_askpass_configuration");
    }

    #[test]
    fn askpass_capability_is_consumed_by_one_password_prompt() {
        let configuration = AskPassConfig::for_keychain(
            "work".into(),
            KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            "gitee.com".into(),
            "work-login".into(),
        )
        .unwrap();
        let capability = AskPassCapability::create(&configuration).unwrap();

        assert!(capability.load_configuration(false).is_ok());
        assert!(capability.load_configuration(true).is_ok());
        let error = match capability.load_configuration(true) {
            Ok(_) => panic!("a consumed capability must not be replayed"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "askpass_capability_invalid");
    }

    #[test]
    fn platform_cli_pipe_permit_can_release_before_detached_workers_finish() {
        let before = ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire);
        let permit = PlatformCliPipeOperationPermit::acquire().expect("a test permit is available");
        let worker = Arc::clone(&permit);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );

        drop(permit);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );
        worker.release();
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before
        );
        drop(worker);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before
        );
    }

    #[cfg(unix)]
    #[test]
    fn platform_cli_runner_drains_stdout_and_stderr_while_the_child_runs() {
        let output = run_platform_cli_program(
            OsStr::new("sh"),
            &[
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
            ],
        )
        .expect("large concurrent CLI output must not deadlock the runner");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131_072);
        assert_eq!(output.stderr.len(), 131_072);
    }

    #[test]
    fn tampered_profile_cannot_target_another_keychain_service() {
        let endpoints = vec![endpoint()];
        let mut profile = account(
            "work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile
            .secret_ref
            .as_mut()
            .expect("PAT profile has a reference")
            .service = "unrelated.system.service".into();

        let error = resolve(
            &endpoints,
            &[],
            &[profile],
            "https://gitee.com/team/project.git",
        )
        .expect_err("only application-owned keychain entries may be used");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn tampered_profile_cannot_target_another_profile_pat() {
        let endpoints = vec![endpoint()];
        let mut profile = account(
            "work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile
            .secret_ref
            .as_mut()
            .expect("PAT profile has a reference")
            .account = "personal".into();

        let error = resolve(
            &endpoints,
            &[],
            &[profile],
            "https://gitee.com/team/project.git",
        )
        .expect_err("a profile may only resolve its own keychain reference");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn askpass_only_accepts_a_matching_https_prompt_authority() {
        let configuration = AskPassConfig::for_keychain(
            "work".into(),
            KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            "gitee.com:8443".into(),
            "work-login".into(),
        )
        .unwrap();
        assert_eq!(
            askpass_prompt_authority("Password for 'https://gitee.com:8443':").unwrap(),
            "gitee.com:8443"
        );
        assert_eq!(
            askpass_username_for_prompt(&configuration, "gitee.com:8443").unwrap(),
            "work-login"
        );
        assert_eq!(
            askpass_username_for_prompt(&configuration, "gitee.com:9443")
                .unwrap_err()
                .code(),
            "askpass_prompt_host_mismatch"
        );
        assert_eq!(
            askpass_password_prompt_authority(
                "Password for 'https://work-login@gitee.com:8443/team/project.git':",
                "work-login",
            )
            .unwrap(),
            "gitee.com:8443"
        );
        assert!(askpass_password_prompt_authority(
            "Password for 'https://other-login@gitee.com:8443/team/project.git':",
            "work-login",
        )
        .is_err());
        assert!(askpass_prompt_authority(
            "Username for 'https://work-login@gitee.com:8443/team/project.git':"
        )
        .is_err());
        assert!(askpass_prompt_authority("Password for gitee.com:").is_err());
    }

    #[test]
    fn askpass_prompt_authority_keeps_bracketed_ipv6_ports() {
        let authority =
            askpass_prompt_authority("Password for 'https://[2001:db8::1]:8443/team/project.git':")
                .expect("IPv6 HTTPS prompt parses");

        assert_eq!(authority, "[2001:db8::1]:8443");
        assert_eq!(
            hostname_from_git_authority(&authority).as_deref(),
            Some("2001:db8::1")
        );
        assert!(is_safe_git_authority(&authority));
    }
}
