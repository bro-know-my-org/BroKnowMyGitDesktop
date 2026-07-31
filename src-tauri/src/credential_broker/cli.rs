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

