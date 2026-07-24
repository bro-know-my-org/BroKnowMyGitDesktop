use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;

impl GitCore {
    pub(super) fn try_repository_root(&self, path: &Path) -> Result<Option<PathBuf>, AppError> {
        let arguments = os_arguments(&["rev-parse", "--show-toplevel"]);
        let output = self.execute(Some(path), &arguments, &GitEnvironment::default())?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = output_text(&output.stdout).trim().to_owned();
        if value.is_empty() {
            return Ok(None);
        }
        Ok(Some(normalize_path(Path::new(&value))))
    }

    /// Returns whether this worktree has at least one commit. An unborn
    /// branch is valid for initialization but cannot be amended or reset.
    pub fn has_head(&self, repository: &Path) -> Result<bool, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        Ok(output.status.success())
    }

    /// Returns the immutable commit currently checked out at `HEAD`. Durable
    /// recovery intents use this to distinguish the original worktree from a
    /// different repository later recreated at the same filesystem path.
    pub fn head_commit_id(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "HEAD"]),
            &GitEnvironment::default(),
            "读取当前提交",
        )?;
        let output = output_text(&output.stdout);
        let commit_id = output.trim();
        if !(40..=64).contains(&commit_id.len())
            || !commit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取当前提交标识。"));
        }
        Ok(commit_id.to_owned())
    }

    pub(super) fn git_directory(&self, repository: &Path) -> Result<PathBuf, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--git-dir"]),
            &GitEnvironment::default(),
            "读取 Git 操作状态",
        )?;
        let git_dir = PathBuf::from(output_text(&output.stdout).trim());
        if git_dir.as_os_str().is_empty() {
            return Err(AppError::new("git_parse_error", "无法读取 Git 元数据目录"));
        }
        Ok(if git_dir.is_absolute() {
            git_dir
        } else {
            repository.join(git_dir)
        })
    }

    pub(super) fn ensure_no_operation(&self, repository: &Path) -> Result<(), AppError> {
        if self.operation_kind(repository)?.is_some() {
            return Err(AppError::new(
                "git_operation_in_progress",
                "当前已有 Git 操作尚未完成，请先继续或中止它",
            ));
        }
        Ok(())
    }

    pub(super) fn ensure_operation(
        &self,
        repository: &Path,
        expected: &GitOperationKind,
    ) -> Result<(), AppError> {
        if self.operation_kind(repository)?.as_ref() != Some(expected) {
            return Err(AppError::new(
                "git_operation_not_in_progress",
                "对应的 Git 操作已结束或与当前仓库状态不匹配",
            ));
        }
        Ok(())
    }

    pub(super) fn remote_url(
        &self,
        repository: &Path,
        name: &str,
        push: bool,
    ) -> Result<String, AppError> {
        validate_remote_name(name)?;
        let mut arguments = os_arguments(&["remote", "get-url"]);
        if push {
            arguments.push(OsString::from("--push"));
        }
        arguments.push(OsString::from(name));
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取远程地址",
        )?;
        Ok(output_text(&output.stdout).trim().to_owned())
    }

    pub(super) fn run(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let output = self.execute(repository, arguments, environment)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    /// Execute a transport-capable Git command with a finite deadline. Git
    /// network helpers and hooks can otherwise wait indefinitely even when
    /// the Tauri command itself runs on a blocking worker.
    pub(super) fn run_network(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let output = self.execute_network_with_timeout(repository, arguments, environment)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    pub(super) fn run_with_input(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        input: &[u8],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        command
            .args(command_arguments(repository, arguments))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_git_environment(&mut command, environment);
        let mut child = command.spawn().map_err(|_| {
            AppError::with_safe_details(
                "git_unavailable",
                "无法启动系统 Git",
                "请确认 Git 已安装且可从应用进程的 PATH 访问。",
            )
        })?;
        let write_result = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(input),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"));
            }
        };
        if write_result.is_err() {
            // Git can reject a patch before consuming stdin. Reap the child
            // on this early return so repeated failed hunk requests cannot
            // accumulate zombies on Unix.
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"));
        }
        let output = child.wait_with_output().map_err(|_| {
            AppError::new(
                "git_command_failed",
                "Git 未能完成补丁操作；请检查仓库状态。",
            )
        })?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    pub(super) fn execute(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        command.args(command_arguments(repository, arguments));
        apply_git_environment(&mut command, environment);
        command.output().map_err(|_| {
            AppError::with_safe_details(
                "git_unavailable",
                "无法启动系统 Git",
                "请确认 Git 已安装且可从应用进程的 PATH 访问。",
            )
        })
    }

    pub(super) fn execute_network_with_timeout(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        configure_git_network_process_group(&mut command);
        command
            .args(command_arguments(repository, arguments))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_git_environment(&mut command, environment);
        let mut child = command.spawn().map_err(|_| {
            AppError::with_safe_details(
                "git_unavailable",
                "无法启动系统 Git",
                "请确认 Git 已安装且可从应用进程的 PATH 访问。",
            )
        })?;
        let process_id = child.id();
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id);
            AppError::new("git_command_failed", "无法读取 Git 网络命令输出。")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id);
            AppError::new("git_command_failed", "无法读取 Git 网络命令输出。")
        })?;
        let stdout_reader = spawn_network_git_pipe_reader(stdout);
        let stderr_reader = spawn_network_git_pipe_reader(stderr);
        let deadline = Instant::now() + NETWORK_GIT_COMMAND_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break status;
                }
                Ok(None) if Instant::now() >= deadline => {
                    terminate_git_network_child(&mut child, process_id);
                    return Err(AppError::new(
                        "git_network_timeout",
                        "Git 网络操作超时，请检查网络、认证和远程服务状态后重试。",
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    terminate_git_network_child(&mut child, process_id);
                    return Err(AppError::io("等待 Git", error));
                }
            }
        };
        let stdout = match collect_network_git_pipe(stdout_reader, deadline) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_git_network_process_group(process_id);
                return Err(error);
            }
        };
        let stderr = match collect_network_git_pipe(stderr_reader, deadline) {
            Ok(stderr) => stderr,
            Err(error) => {
                terminate_git_network_process_group(process_id);
                return Err(error);
            }
        };
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

#[derive(Debug)]
enum NetworkGitPipeError {
    TooLarge,
    Io(std::io::Error),
}

fn spawn_network_git_pipe_reader(
    pipe: impl Read + Send + 'static,
) -> Receiver<Result<Vec<u8>, NetworkGitPipeError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(read_network_git_pipe(pipe));
    });
    receiver
}

fn read_network_git_pipe(mut pipe: impl Read) -> Result<Vec<u8>, NetworkGitPipeError> {
    let mut bytes = Vec::new();
    let mut exceeded_limit = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(NetworkGitPipeError::Io)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_NETWORK_GIT_DIAGNOSTIC_BYTES.saturating_sub(bytes.len());
        let retained = count.min(remaining);
        bytes.extend_from_slice(&buffer[..retained]);
        exceeded_limit |= retained != count;
    }
    if exceeded_limit {
        Err(NetworkGitPipeError::TooLarge)
    } else {
        Ok(bytes)
    }
}

fn collect_network_git_pipe(
    reader: Receiver<Result<Vec<u8>, NetworkGitPipeError>>,
    deadline: Instant,
) -> Result<Vec<u8>, AppError> {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT);
    if timeout.is_zero() {
        return Err(AppError::new(
            "git_network_timeout",
            "Git 网络操作超时，请检查网络、认证和远程服务状态后重试。",
        ));
    }
    match reader.recv_timeout(timeout) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(NetworkGitPipeError::TooLarge)) => Err(AppError::new(
            "git_command_failed",
            "Git 网络命令输出过大，已停止该操作。",
        )),
        Ok(Err(NetworkGitPipeError::Io(error))) => {
            let _ = error.kind();
            Err(AppError::new(
                "git_command_failed",
                "无法读取 Git 网络命令输出。",
            ))
        }
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(
            "git_network_timeout",
            "Git 网络操作超时，请检查网络、认证和远程服务状态后重试。",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
            "git_command_failed",
            "无法读取 Git 网络命令输出。",
        )),
    }
}
