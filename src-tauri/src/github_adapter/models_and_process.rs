#[derive(Debug, Serialize)]
struct GitHubPublishOwnersRequest<'a> {
    query: &'static str,
    variables: GitHubPublishOwnersVariables<'a>,
}

#[derive(Debug, Serialize)]
struct GitHubPublishOwnersVariables<'a> {
    cursor: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersResponse {
    data: Option<GitHubPublishOwnersData>,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersData {
    viewer: GitHubPublishOwnersViewer,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersViewer {
    login: String,
    name: Option<String>,
    organizations: GitHubPublishOrganizationsConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPublishOrganizationsConnection {
    nodes: Vec<GitHubPublishOrganization>,
    page_info: GitHubPageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPublishOrganization {
    login: String,
    name: Option<String>,
    viewer_can_create_repositories: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

struct GitHubPublishOwnersPage {
    viewer_login: String,
    viewer_name: Option<String>,
    organizations: Vec<GitHubPublishOrganization>,
    has_next_page: bool,
    end_cursor: Option<String>,
}

fn parse_publish_owners_page(
    response: &str,
    expected_login: &str,
) -> Result<GitHubPublishOwnersPage, AppError> {
    let response: GitHubPublishOwnersResponse = serde_json::from_str(response).map_err(|_| {
        AppError::new(
            "github_api_invalid_response",
            "无法解析 GitHub 发布所有者列表。",
        )
    })?;
    if !response.errors.is_empty() {
        return Err(AppError::new(
            "github_publish_owners_unavailable",
            "GitHub 无法读取组织发布权限，请检查 gh 的组织访问授权。",
        ));
    }
    let viewer = response
        .data
        .ok_or_else(|| {
            AppError::new(
                "github_api_invalid_response",
                "GitHub 发布所有者响应缺少账号信息。",
            )
        })?
        .viewer;
    if !viewer.login.eq_ignore_ascii_case(expected_login) {
        return Err(AppError::new(
            "cli_auth_unavailable",
            "当前 gh 登录与所选发布账号不匹配或已失效。",
        ));
    }
    Ok(GitHubPublishOwnersPage {
        viewer_login: viewer.login,
        viewer_name: viewer.name,
        organizations: viewer.organizations.nodes,
        has_next_page: viewer.organizations.page_info.has_next_page,
        end_cursor: viewer.organizations.page_info.end_cursor,
    })
}

/// `gh` and Git askpass identify an authenticated host by hostname, whereas
/// Endpoint routing needs the full Git authority. Keep the port out of this
/// CLI argument so it remains subject to the same hostname validation as the
/// credential broker.
fn gh_hostname(endpoint: &CodeHostEndpoint) -> Result<String, AppError> {
    if git_authority_has_explicit_port(&endpoint.git_host) {
        return Err(AppError::new(
            "github_ported_endpoint_unsupported",
            "GitHub 平台功能暂不支持带端口的 Endpoint；请使用 PAT、SSH 或系统 Git 凭据执行 Git 操作。",
        ));
    }
    if !github_endpoint_has_gh_compatible_api(endpoint) {
        return Err(AppError::new(
            "github_api_endpoint_unsupported",
            "GitHub Endpoint 的 API 地址与 gh 的 host 路由不兼容；请使用 PAT、SSH 或系统 Git 凭据执行 Git 操作。",
        ));
    }
    hostname_from_git_authority(&endpoint.git_host).ok_or_else(|| {
        AppError::new(
            "adapter_endpoint_mismatch",
            "GitHub Endpoint 的 Git host 无效。",
        )
    })
}

/// Run `gh` with bounded output and a hard deadline. Network and
/// credential-helper stalls must not leave a desktop command pending forever.
fn run_gh_with_timeout(
    mut command: Command,
    input: Option<&[u8]>,
    deadline: Instant,
) -> Result<Output, AppError> {
    if Instant::now() >= deadline {
        return Err(github_api_timeout());
    }
    let operation_permit = GhPipeOperationPermit::acquire()?;
    configure_gh_process_group(&mut command);
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new(
                "gh_unavailable",
                "未找到 gh；安装并登录 GitHub CLI 后可使用 GitHub 平台功能。",
            )
        } else {
            AppError::io("启动 gh", error)
        }
    })?;
    let process_id = child.id();
    let process_tree = match attach_process_tree(&child) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            terminate_gh_child(&mut child, process_id);
            return Err(AppError::io("配置 gh 子进程树", error));
        }
    };

    let stdin_writer = if let Some(input) = input {
        let Some(stdin) = child.stdin.take() else {
            terminate_gh_child(&mut child, process_id);
            return Err(AppError::new(
                "github_api_failed",
                "无法向 GitHub CLI 写入请求。",
            ));
        };
        let input = input.to_vec();
        Some(spawn_gh_stdin_writer(
            stdin,
            input,
            Arc::clone(&operation_permit),
        ))
    } else {
        None
    };

    let Some(stdout) = child.stdout.take() else {
        terminate_gh_child(&mut child, process_id);
        operation_permit.release_after_cleanup_grace();
        return Err(AppError::new(
            "github_api_failed",
            "无法读取 GitHub CLI 输出。",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_gh_child(&mut child, process_id);
        operation_permit.release_after_cleanup_grace();
        return Err(AppError::new(
            "github_api_failed",
            "无法读取 GitHub CLI 诊断。",
        ));
    };
    let stdout_reader =
        spawn_gh_pipe_reader(stdout, MAX_GH_RESPONSE_BYTES, Arc::clone(&operation_permit));
    let stderr_reader = spawn_gh_pipe_reader(
        stderr,
        MAX_GH_DIAGNOSTIC_BYTES,
        Arc::clone(&operation_permit),
    );

    let mut stdout_ready = None;
    let mut stderr_ready = None;
    let status = loop {
        if let Err(error) = poll_gh_pipe(&stdout_reader, &mut stdout_ready) {
            terminate_gh_child(&mut child, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
        if let Err(error) = poll_gh_pipe(&stderr_reader, &mut stderr_ready) {
            terminate_gh_child(&mut child, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                terminate_gh_child(&mut child, process_id);
                // Do not join helper threads here. A descendant process can
                // retain a pipe even after `gh` exits; detached readers
                // cannot keep this UI command pending.
                operation_permit.release_after_cleanup_grace();
                return Err(github_api_timeout());
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_gh_child(&mut child, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(AppError::io("等待 gh", error));
            }
        }
    };

    let stdout = match collect_gh_pipe(stdout_reader, stdout_ready, deadline) {
        Ok(stdout) => stdout,
        Err(error) => {
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    let stderr = match collect_gh_pipe(stderr_reader, stderr_ready, deadline) {
        Ok(stderr) => stderr,
        Err(error) => {
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    if let Some(writer) = stdin_writer {
        if status.success() {
            if let Err(error) = collect_gh_stdin_writer(writer, deadline) {
                terminate_remaining_gh_process_tree(&process_tree, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(error);
            }
        } else {
            // A failed gh process can leave a descendant holding stdin even
            // after both output readers complete. Do not let it pin this
            // operation's permit forever.
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
        }
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn terminate_gh_child(child: &mut Child, process_id: u32) {
    terminate_gh_process_group(process_id);
    let _ = child.kill();
    let _ = child.wait();
}

/// The direct child has already exited, so its numeric PID must not be used
/// as a Windows tree identity. Unix process groups retain their kernel-backed
/// PGID while descendants exist; Windows uses the Job Object attached at
/// spawn time instead.
fn terminate_remaining_gh_process_tree(_process_tree: &ProcessTreeHandle, _process_id: u32) {
    #[cfg(unix)]
    terminate_gh_process_group(_process_id);
    #[cfg(windows)]
    terminate_process_tree(_process_tree);
}

#[derive(Debug)]
enum GhPipeError {
    TooLarge,
    Io(std::io::Error),
}

fn spawn_gh_pipe_reader(
    pipe: impl Read + Send + 'static,
    maximum_bytes: usize,
    operation_permit: Arc<GhPipeOperationPermit>,
) -> Receiver<Result<Vec<u8>, GhPipeError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _permit = operation_permit;
        let _ = sender.send(read_gh_pipe(pipe, maximum_bytes));
    });
    receiver
}

fn spawn_gh_stdin_writer(
    mut stdin: std::process::ChildStdin,
    input: Vec<u8>,
    operation_permit: Arc<GhPipeOperationPermit>,
) -> Receiver<std::io::Result<()>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _permit = operation_permit;
        let _ = sender.send(stdin.write_all(&input));
    });
    receiver
}

fn read_gh_pipe(mut pipe: impl Read, maximum_bytes: usize) -> Result<Vec<u8>, GhPipeError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(GhPipeError::Io)?;
        if count == 0 {
            break;
        }
        let remaining = maximum_bytes.saturating_sub(bytes.len());
        if count > remaining {
            // Notify the parent immediately. Continuing to drain a hostile
            // pipe would turn this memory limit into a 45-second wait.
            return Err(GhPipeError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn poll_gh_pipe(
    reader: &Receiver<Result<Vec<u8>, GhPipeError>>,
    ready: &mut Option<Result<Vec<u8>, GhPipeError>>,
) -> Result<(), AppError> {
    if ready.is_some() {
        return Ok(());
    }
    match reader.try_recv() {
        Ok(Ok(bytes)) => {
            *ready = Some(Ok(bytes));
            Ok(())
        }
        Ok(Err(error)) => Err(gh_pipe_error(error)),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AppError::new(
            "github_api_failed",
            "GitHub CLI 输出任务未能完成。",
        )),
    }
}

fn collect_gh_pipe(
    reader: Receiver<Result<Vec<u8>, GhPipeError>>,
    ready: Option<Result<Vec<u8>, GhPipeError>>,
    deadline: Instant,
) -> Result<Vec<u8>, AppError> {
    let result = match ready {
        Some(result) => result,
        None => match reader.recv_timeout(remaining_gh_collection_timeout(deadline)?) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                return Err(AppError::new(
                    "github_api_pipe_timeout",
                    "GitHub CLI 输出未能及时关闭。",
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AppError::new(
                    "github_api_failed",
                    "GitHub CLI 输出任务未能完成。",
                ));
            }
        },
    };
    match result {
        Ok(bytes) => Ok(bytes),
        Err(error) => Err(gh_pipe_error(error)),
    }
}

fn gh_pipe_error(error: GhPipeError) -> AppError {
    match error {
        GhPipeError::TooLarge => AppError::new(
            "github_api_response_too_large",
            "GitHub CLI 返回的数据过大，已停止处理。",
        ),
        GhPipeError::Io(error) => AppError::io("读取 gh 输出", error),
    }
}

fn collect_gh_stdin_writer(
    writer: Receiver<std::io::Result<()>>,
    deadline: Instant,
) -> Result<(), AppError> {
    match writer.recv_timeout(remaining_gh_collection_timeout(deadline)?) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(AppError::new("github_api_failed", "GitHub 请求未能发送。")),
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(
            "github_api_pipe_timeout",
            "GitHub CLI 请求输入未能及时关闭。",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
            "github_api_failed",
            "GitHub 请求发送任务未能完成。",
        )),
    }
}

fn remaining_gh_collection_timeout(deadline: Instant) -> Result<Duration, AppError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(github_api_timeout());
    }
    Ok(remaining.min(GH_PIPE_COLLECTION_TIMEOUT))
}

fn github_api_timeout() -> AppError {
    AppError::new(
        "github_api_timeout",
        "GitHub 请求超时，请检查网络和 gh 登录状态后重试。",
    )
}

#[cfg(unix)]
fn configure_gh_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Delegate the post-fork setup to the standard library rather than
    // invoking Rust code from a `pre_exec` closure in this multithreaded app.
    command.process_group(0);
}

#[cfg(windows)]
fn configure_gh_process_group(_command: &mut Command) {}

#[cfg(all(not(unix), not(windows)))]
fn configure_gh_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_gh_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the process group created by
    // configure_gh_process_group; failing to find an already-exited group is
    // harmless and the direct child is also killed by the caller when present.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_gh_process_group(process_id: u32) {
    // `taskkill /T` is the Windows process-tree primitive available on every
    // supported desktop installation. Wait for it before the caller reaps
    // the direct child so `/T` can still locate and terminate descendants.
    let process_id = process_id.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", process_id.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_gh_process_group(_process_id: u32) {}

