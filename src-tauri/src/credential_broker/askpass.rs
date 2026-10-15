/// A secret source identifier, never a secret value.
#[derive(Clone)]
enum AskPassSecretSource {
    GhAccount {
        account_profile_id: String,
        authority: String,
    },
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
    GhAccount {
        account_profile_id: String,
        authority: String,
    },
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
    fn for_gh_account(
        account_profile_id: String,
        authority: String,
        username: String,
    ) -> Result<Self, AppError> {
        if !is_safe_metadata_value(&account_profile_id)
            || !is_safe_git_authority(&authority)
            || !is_safe_hostname(&authority)
            || git_authority_has_explicit_port(&authority)
            || !is_safe_username(&username)
        {
            return Err(AppError::new(
                "invalid_askpass_configuration",
                "无法安全配置所选 gh 账号的 askpass 认证。",
            ));
        }
        Ok(Self {
            source: AskPassSecretSource::GhAccount {
                account_profile_id,
                authority,
            },
            username,
        })
    }

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
            AskPassSecretSource::GhAccount { .. } => ASKPASS_MODE_GH,
            AskPassSecretSource::Keychain { .. } => ASKPASS_MODE_KEYCHAIN,
        }
    }

    fn capability_configuration(&self) -> AskPassCapabilityConfiguration {
        let source = match &self.source {
            AskPassSecretSource::GhAccount {
                account_profile_id,
                authority,
            } => AskPassCapabilitySource::GhAccount {
                account_profile_id: account_profile_id.clone(),
                authority: authority.clone(),
            },
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
            AskPassCapabilitySource::GhAccount {
                account_profile_id,
                authority,
            } => Self::for_gh_account(account_profile_id, authority, configuration.username),
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
/// stdout. On a password prompt, the selected keychain/CLI token flows directly to
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
            AskPassSecretSource::GhAccount { authority, .. }
                if authority.eq_ignore_ascii_case(&prompt_authority) =>
            {
                read_gh_account_token(&authority, &configuration.username)
            }
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
        AskPassSecretSource::GhAccount { authority, .. }
        | AskPassSecretSource::Keychain { authority, .. } => {
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
