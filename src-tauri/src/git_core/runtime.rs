use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;
use crate::process_tree::attach_process_tree;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const MAX_CONCURRENT_GIT_PIPE_OPERATIONS: usize = 32;
const INITIAL_GIT_PROCESS_POLL_DELAY: Duration = Duration::from_millis(1);
const MAX_GIT_PROCESS_POLL_DELAY: Duration = Duration::from_millis(25);

static ACTIVE_GIT_PIPE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

fn sleep_before_next_git_process_poll(delay: &mut Duration) {
    // Most repository queries finish in a few milliseconds. A fixed 25 ms
    // sleep is multiplied by every Git command used to assemble a snapshot;
    // exponential backoff keeps those short queries responsive while quickly
    // returning long-running operations to the previous low wake-up rate.
    thread::sleep(*delay);
    *delay = std::cmp::min(delay.saturating_mul(2), MAX_GIT_PROCESS_POLL_DELAY);
}

/// A permit remains owned by every pipe worker until it finishes. A hostile
/// hook or transport helper can escape the direct Git process group and keep
/// an inherited pipe open forever; Rust cannot safely cancel that blocking
/// read from another thread. Retaining the permit gives the backend a hard
/// bound on both detached workers and buffered patch input instead of letting
/// repeated IPC calls consume unbounded resources.
struct GitPipeOperationPermit;

impl GitPipeOperationPermit {
    fn acquire() -> Result<Arc<Self>, AppError> {
        let mut active = ACTIVE_GIT_PIPE_OPERATIONS.load(Ordering::Acquire);
        loop {
            if active >= MAX_CONCURRENT_GIT_PIPE_OPERATIONS {
                return Err(AppError::new(
                    "git_operation_busy",
                    "Git 操作过多，请等待正在执行的操作完成。",
                ));
            }
            match ACTIVE_GIT_PIPE_OPERATIONS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Arc::new(Self)),
                Err(observed) => active = observed,
            }
        }
    }
}

impl Drop for GitPipeOperationPermit {
    fn drop(&mut self) {
        ACTIVE_GIT_PIPE_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy)]
struct GitExecutionLimits {
    timeout: Duration,
    maximum_stdout_bytes: usize,
    maximum_stderr_bytes: usize,
    timeout_code: &'static str,
    timeout_message: &'static str,
}

pub(super) struct GitOutputPreview {
    pub stdout: Vec<u8>,
    pub truncated: bool,
}

struct GitOutputPreviewRead {
    preview: GitOutputPreview,
    held_pipe: Option<std::process::ChildStdout>,
}

/// Git serializes configuration writers through sibling .lock files. Retain
/// those locks from the final safety preflight until the network child exits,
/// so another ordinary Git client cannot add a URL rewrite after it is checked.
pub(super) struct NetworkConfigurationLock {
    files: Vec<NetworkConfigurationLockFile>,
}

struct NetworkConfigurationLockFile {
    path: PathBuf,
    file: fs::File,
}

impl NetworkConfigurationLock {
    fn acquire(configurations: Vec<PathBuf>) -> Result<Self, AppError> {
        let mut lock = Self { files: Vec::new() };
        for configuration in configurations {
            let name = configuration
                .file_name()
                .ok_or_else(|| AppError::new("git_command_failed", "无法锁定仓库 Git 配置。"))?;
            let mut lock_name = name.to_os_string();
            lock_name.push(".lock");
            let path = configuration.with_file_name(lock_name);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => lock.files.push(NetworkConfigurationLockFile { path, file }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(AppError::new(
                        "git_config_locked",
                        "仓库 Git 配置正被其他 Git 操作修改，请稍后重试。",
                    ));
                }
                Err(error) => return Err(AppError::io("锁定仓库 Git 配置", error)),
            }
        }
        Ok(lock)
    }
}

impl Drop for NetworkConfigurationLock {
    fn drop(&mut self) {
        while let Some(lock) = self.files.pop() {
            let path = lock.path;
            drop(lock.file);
            let _ = fs::remove_file(path);
        }
    }
}

impl GitCore {
    pub(super) fn try_repository_root(&self, path: &Path) -> Result<Option<PathBuf>, AppError> {
        let arguments = os_arguments(&["rev-parse", "--show-toplevel"]);
        let output = self.execute(Some(path), &arguments, &GitEnvironment::default())?;
        if !output.status.success() {
            return Ok(None);
        }
        let path = git_path_from_output(&output.stdout, "无法读取 Git 工作区路径")?;
        Ok(Some(normalize_path(&path)))
    }

    /// Returns whether this worktree has at least one commit. An unborn
    /// branch is valid for initialization but cannot be amended or reset.
    pub fn has_head(&self, repository: &Path) -> Result<bool, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            return Ok(true);
        }
        if output.status.code() == Some(1) && self.has_unborn_head(repository)? {
            return Ok(false);
        }
        Err(git_command_error("检查当前提交", &output))
    }

    /// Distinguishes an ordinary unborn symbolic branch from a detached or
    /// corrupt HEAD that failed to resolve. Only the former may be presented
    /// as an empty history.
    pub(super) fn has_unborn_head(&self, repository: &Path) -> Result<bool, AppError> {
        let symbolic = self.execute(
            Some(repository),
            &os_arguments(&["symbolic-ref", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        if symbolic.status.code() == Some(1) {
            return Ok(false);
        }
        if !symbolic.status.success() {
            return Err(git_command_error("检查当前分支", &symbolic));
        }
        let reference = output_text(&symbolic.stdout).trim().to_owned();
        if !reference.starts_with("refs/heads/") {
            return Err(AppError::new("git_parse_error", "无法读取当前分支引用。"));
        }
        let exists = self.execute(
            Some(repository),
            &os_arguments(&["show-ref", "--verify", "--quiet", &reference]),
            &GitEnvironment::default(),
        )?;
        if exists.status.success() {
            Ok(false)
        } else if exists.status.code() == Some(1) {
            Ok(true)
        } else {
            Err(git_command_error("检查当前分支", &exists))
        }
    }

    /// Returns the immutable commit currently checked out at `HEAD`. Durable
    /// recovery intents use this to distinguish the original worktree from a
    /// different repository later recreated at the same filesystem path.
    pub fn head_commit_id(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "HEAD^{commit}"]),
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
        let git_dir = git_path_from_output(&output.stdout, "无法读取 Git 元数据目录")?;
        Ok(if git_dir.is_absolute() {
            git_dir
        } else {
            repository.join(git_dir)
        })
    }

    pub(super) fn git_common_directory(&self, repository: &Path) -> Result<PathBuf, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--git-common-dir"]),
            &GitEnvironment::default(),
            "读取 Git 共享元数据目录",
        )?;
        let git_dir = git_path_from_output(&output.stdout, "无法读取 Git 共享元数据目录")?;
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
        self.remote_urls(repository, name, push)?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("git_parse_error", "无法读取远程地址"))
    }

    pub(super) fn remote_urls(
        &self,
        repository: &Path,
        name: &str,
        push: bool,
    ) -> Result<Vec<String>, AppError> {
        validate_remote_name(name)?;
        let mut arguments = os_arguments(&["remote", "get-url"]);
        if push {
            arguments.push(OsString::from("--push"));
        }
        arguments.push(OsString::from("--all"));
        arguments.push(OsString::from(name));
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取远程地址",
        )?;
        let urls = output_text(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if urls.is_empty() {
            return Err(AppError::new("git_parse_error", "无法读取远程地址"));
        }
        Ok(urls)
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
        let _configuration_lock = if let Some(repository) = repository {
            // Recheck after the lock is acquired. A competing Git client may
            // have committed a config update between an earlier preflight and
            // this operation's lock acquisition.
            self.ensure_network_configuration_is_safe(repository)?;
            let lock = self.lock_network_configuration(repository)?;
            self.ensure_network_configuration_is_safe(repository)?;
            Some(lock)
        } else {
            None
        };
        let output = self.execute_network_with_timeout(repository, arguments, environment)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    pub(super) fn lock_network_configuration(
        &self,
        repository: &Path,
    ) -> Result<NetworkConfigurationLock, AppError> {
        let mut configurations = Vec::new();
        for name in ["config", "config.worktree"] {
            let output = self.execute(
                Some(repository),
                &os_arguments(&["rev-parse", "--git-path", name]),
                &GitEnvironment::default(),
            )?;
            if !output.status.success() {
                return Err(git_command_error("读取仓库 Git 配置位置", &output));
            }
            let path = git_path_from_output(&output.stdout, "无法读取仓库 Git 配置位置。")?;
            let path = if path.is_absolute() {
                path
            } else {
                repository.join(path)
            };
            if !configurations.contains(&path) {
                configurations.push(path);
            }
        }
        NetworkConfigurationLock::acquire(configurations)
    }

    /// Executes a local Git query with a bounded stdout capture. This is used
    /// for data returned over IPC, where an arbitrarily large history subject
    /// or patch must not allocate without limit in the desktop backend.
    pub(super) fn run_with_output_limit(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        maximum_stdout_bytes: usize,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let output = self.execute_with_timeout_and_output_limits(
            repository,
            arguments,
            environment,
            GitExecutionLimits {
                timeout: BOUNDED_GIT_COMMAND_TIMEOUT,
                maximum_stdout_bytes,
                maximum_stderr_bytes: MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
                timeout_code: "git_command_timeout",
                timeout_message: "Git 命令超时，请缩小范围后重试。",
            },
        )?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    pub(super) fn run_with_output_preview(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        maximum_stdout_bytes: usize,
        operation: &'static str,
    ) -> Result<GitOutputPreview, AppError> {
        let pipe_operation = GitPipeOperationPermit::acquire()?;
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
        let process_tree = match attach_process_tree(&child) {
            Ok(process_tree) => process_tree,
            Err(error) => {
                terminate_git_network_child(&mut child, process_id, None);
                return Err(AppError::io("配置 Git 子进程树", error));
            }
        };
        if let Err(error) = resume_git_network_process(process_id) {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            return Err(AppError::io("启动 Git 子进程", error));
        }
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            AppError::new("git_command_failed", "无法读取 Git 命令输出。")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            AppError::new("git_command_failed", "无法读取 Git 命令输出。")
        })?;
        let retained_stdout_bytes = maximum_stdout_bytes.checked_add(1).ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            AppError::new("invalid_output_limit", "Git 输出读取范围无效。")
        })?;
        let stdout_reader = spawn_git_preview_pipe_reader(
            stdout,
            retained_stdout_bytes,
            maximum_stdout_bytes,
            Arc::clone(&pipe_operation),
        );
        let stderr_reader = spawn_network_git_pipe_reader(
            stderr,
            MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
            Arc::clone(&pipe_operation),
        );
        let deadline = Instant::now() + BOUNDED_GIT_COMMAND_TIMEOUT;
        let mut stdout_ready = None;
        let mut stderr_ready = None;
        let mut process_poll_delay = INITIAL_GIT_PROCESS_POLL_DELAY;
        let status = loop {
            if let Err(error) = poll_git_preview_pipe(&stdout_reader, &mut stdout_ready) {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(error);
            }
            if let Err(error) = poll_network_git_pipe(&stderr_reader, &mut stderr_ready) {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(error);
            }
            if stdout_ready
                .as_ref()
                .is_some_and(|result| result.preview.truncated)
            {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let pipe_collection_deadline =
                            Instant::now() + NETWORK_GIT_PIPE_COLLECTION_TIMEOUT;
                        let stderr = match collect_network_git_pipe(
                            stderr_reader,
                            stderr_ready,
                            pipe_collection_deadline,
                            "git_command_timeout",
                            "Git 命令超时，请缩小范围后重试。",
                        ) {
                            Ok(stderr) => stderr,
                            Err(error) => {
                                terminate_remaining_git_network_process_tree(
                                    &process_tree,
                                    process_id,
                                );
                                return Err(error);
                            }
                        };
                        let GitOutputPreviewRead { preview, held_pipe } =
                            stdout_ready.expect("truncated preview is present");
                        drop(held_pipe);
                        terminate_remaining_git_network_process_tree(&process_tree, process_id);
                        if status.success() {
                            return Ok(preview);
                        }
                        return Err(git_command_error(
                            operation,
                            &Output {
                                status,
                                stdout: preview.stdout,
                                stderr,
                            },
                        ));
                    }
                    Ok(None) => {
                        // The reader proved overflow while Git was still
                        // running. Stop it deliberately and return the bounded
                        // prefix; a later non-zero status would be induced by
                        // this termination rather than an observed Git error.
                        terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                        let _ = collect_network_git_pipe(
                            stderr_reader,
                            stderr_ready,
                            deadline,
                            "git_command_timeout",
                            "Git 命令超时，请缩小范围后重试。",
                        );
                        let GitOutputPreviewRead { preview, held_pipe } =
                            stdout_ready.expect("truncated preview is present");
                        drop(held_pipe);
                        return Ok(preview);
                    }
                    Err(error) => {
                        terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                        return Err(AppError::io("等待 Git", error));
                    }
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                    return Err(AppError::new(
                        "git_command_timeout",
                        "Git 命令超时，请缩小范围后重试。",
                    ));
                }
                Ok(None) => sleep_before_next_git_process_poll(&mut process_poll_delay),
                Err(error) => {
                    terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                    return Err(AppError::io("等待 Git", error));
                }
            }
        };
        let pipe_collection_deadline = Instant::now() + NETWORK_GIT_PIPE_COLLECTION_TIMEOUT;
        let preview_read = match collect_git_preview_pipe(
            stdout_reader,
            stdout_ready,
            pipe_collection_deadline,
            "git_command_timeout",
            "Git 命令超时，请缩小范围后重试。",
        ) {
            Ok(preview) => preview,
            Err(error) => {
                terminate_remaining_git_network_process_tree(&process_tree, process_id);
                return Err(error);
            }
        };
        let stderr = match collect_network_git_pipe(
            stderr_reader,
            stderr_ready,
            pipe_collection_deadline,
            "git_command_timeout",
            "Git 命令超时，请缩小范围后重试。",
        ) {
            Ok(stderr) => stderr,
            Err(error) => {
                terminate_remaining_git_network_process_tree(&process_tree, process_id);
                return Err(error);
            }
        };
        let GitOutputPreviewRead { preview, held_pipe } = preview_read;
        drop(held_pipe);
        if status.success() {
            return Ok(preview);
        }
        Err(git_command_error(
            operation,
            &Output {
                status,
                stdout: preview.stdout,
                stderr,
            },
        ))
    }

    pub(super) fn run_with_input(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        input: &[u8],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let output = self.execute_with_input_timeout_and_output_limits(
            repository,
            arguments,
            input,
            environment,
            GitExecutionLimits {
                timeout: BOUNDED_GIT_COMMAND_TIMEOUT,
                maximum_stdout_bytes: MAX_LOCAL_GIT_OUTPUT_BYTES,
                maximum_stderr_bytes: MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
                timeout_code: "git_command_timeout",
                timeout_message: "Git 补丁操作超时，请检查仓库状态后重试。",
            },
        )?;
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
        self.execute_with_timeout_and_output_limits(
            repository,
            arguments,
            environment,
            GitExecutionLimits {
                timeout: BOUNDED_GIT_COMMAND_TIMEOUT,
                maximum_stdout_bytes: MAX_LOCAL_GIT_OUTPUT_BYTES,
                maximum_stderr_bytes: MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
                timeout_code: "git_command_timeout",
                timeout_message: "Git 命令超时，请缩小范围后重试。",
            },
        )
    }

    pub(super) fn execute_network_with_timeout(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
    ) -> Result<Output, AppError> {
        self.execute_with_timeout_and_output_limits(
            repository,
            arguments,
            environment,
            GitExecutionLimits {
                timeout: NETWORK_GIT_COMMAND_TIMEOUT,
                maximum_stdout_bytes: MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
                maximum_stderr_bytes: MAX_NETWORK_GIT_DIAGNOSTIC_BYTES,
                timeout_code: "git_network_timeout",
                timeout_message: "Git 网络操作超时，请检查网络、认证和远程服务状态后重试。",
            },
        )
    }

    fn execute_with_timeout_and_output_limits(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        limits: GitExecutionLimits,
    ) -> Result<Output, AppError> {
        self.execute_with_optional_input_timeout_and_output_limits(
            repository,
            arguments,
            None,
            environment,
            limits,
        )
    }

    fn execute_with_input_timeout_and_output_limits(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        input: &[u8],
        environment: &GitEnvironment,
        limits: GitExecutionLimits,
    ) -> Result<Output, AppError> {
        self.execute_with_optional_input_timeout_and_output_limits(
            repository,
            arguments,
            Some(input),
            environment,
            limits,
        )
    }

    fn execute_with_optional_input_timeout_and_output_limits(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        input: Option<&[u8]>,
        environment: &GitEnvironment,
        limits: GitExecutionLimits,
    ) -> Result<Output, AppError> {
        let pipe_operation = GitPipeOperationPermit::acquire()?;
        let mut command = Command::new(&self.executable);
        configure_git_network_process_group(&mut command);
        command
            .args(command_arguments(repository, arguments))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
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
        let process_tree = match attach_process_tree(&child) {
            Ok(process_tree) => process_tree,
            Err(error) => {
                terminate_git_network_child(&mut child, process_id, None);
                return Err(AppError::io("配置 Git 子进程树", error));
            }
        };
        if let Err(error) = resume_git_network_process(process_id) {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            return Err(AppError::io("启动 Git 子进程", error));
        }
        let input_writer = if let Some(input) = input {
            let Some(stdin) = child.stdin.take() else {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"));
            };
            Some(spawn_git_input_writer(
                stdin,
                input.to_vec(),
                Arc::clone(&pipe_operation),
            ))
        } else {
            None
        };
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            AppError::new("git_command_failed", "无法读取 Git 网络命令输出。")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            AppError::new("git_command_failed", "无法读取 Git 网络命令输出。")
        })?;
        let stdout_reader = spawn_network_git_pipe_reader(
            stdout,
            limits.maximum_stdout_bytes,
            Arc::clone(&pipe_operation),
        );
        let stderr_reader = spawn_network_git_pipe_reader(
            stderr,
            limits.maximum_stderr_bytes,
            Arc::clone(&pipe_operation),
        );
        let deadline = Instant::now() + limits.timeout;
        let mut stdout_ready = None;
        let mut stderr_ready = None;
        let mut input_ready = input_writer.is_none();
        let mut process_poll_delay = INITIAL_GIT_PROCESS_POLL_DELAY;
        let status = loop {
            if let Some(writer) = &input_writer {
                if let Err(error) = poll_git_input_writer(writer, &mut input_ready) {
                    terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                    return Err(error);
                }
            }
            if let Err(error) = poll_network_git_pipe(&stdout_reader, &mut stdout_ready) {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(error);
            }
            if let Err(error) = poll_network_git_pipe(&stderr_reader, &mut stderr_ready) {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    break status;
                }
                Ok(None) if Instant::now() >= deadline => {
                    terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                    return Err(AppError::new(limits.timeout_code, limits.timeout_message));
                }
                Ok(None) => sleep_before_next_git_process_poll(&mut process_poll_delay),
                Err(error) => {
                    terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                    return Err(AppError::io("等待 Git", error));
                }
            }
        };
        // `try_wait` above has already reaped the direct Git child. Do not
        // target its numeric PID again when collecting a pipe fails: on
        // Windows it may already identify an unrelated process, and there is
        // no child left for `terminate_git_network_child` to reap.
        if let Some(writer) = input_writer {
            if let Err(error) = collect_git_input_writer(
                writer,
                input_ready,
                deadline,
                limits.timeout_code,
                limits.timeout_message,
            ) {
                terminate_remaining_git_network_process_tree(&process_tree, process_id);
                return Err(error);
            }
        }
        let stdout = match collect_network_git_pipe(
            stdout_reader,
            stdout_ready,
            deadline,
            limits.timeout_code,
            limits.timeout_message,
        ) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_remaining_git_network_process_tree(&process_tree, process_id);
                return Err(error);
            }
        };
        let stderr = match collect_network_git_pipe(
            stderr_reader,
            stderr_ready,
            deadline,
            limits.timeout_code,
            limits.timeout_message,
        ) {
            Ok(stderr) => stderr,
            Err(error) => {
                terminate_remaining_git_network_process_tree(&process_tree, process_id);
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

/// Git prints filesystem paths as bytes. Preserve Unix bytes exactly rather
/// than decoding them lossily and accidentally directing a later operation at
/// a different U+FFFD-containing path.
pub(super) fn git_path_from_output(
    bytes: &[u8],
    message: &'static str,
) -> Result<PathBuf, AppError> {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(AppError::new("git_parse_error", message));
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        let value =
            std::str::from_utf8(bytes).map_err(|_| AppError::new("git_parse_error", message))?;
        Ok(PathBuf::from(value))
    }
}

#[derive(Debug)]
enum NetworkGitPipeError {
    TooLarge,
    Io(std::io::Error),
}

fn spawn_network_git_pipe_reader(
    pipe: impl Read + Send + 'static,
    maximum_bytes: usize,
    pipe_operation: Arc<GitPipeOperationPermit>,
) -> Receiver<Result<Vec<u8>, NetworkGitPipeError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _pipe_operation = pipe_operation;
        let _ = sender.send(read_network_git_pipe(pipe, maximum_bytes));
    });
    receiver
}

fn spawn_git_preview_pipe_reader(
    mut pipe: std::process::ChildStdout,
    maximum_bytes: usize,
    truncation_threshold: usize,
    pipe_operation: Arc<GitPipeOperationPermit>,
) -> Receiver<std::io::Result<GitOutputPreviewRead>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _pipe_operation = pipe_operation;
        let result =
            read_git_preview_pipe(&mut pipe, maximum_bytes, truncation_threshold).map(|preview| {
                GitOutputPreviewRead {
                    held_pipe: preview.truncated.then_some(pipe),
                    preview,
                }
            });
        let _ = sender.send(result);
    });
    receiver
}

pub(super) fn read_git_preview_pipe(
    mut pipe: impl Read,
    maximum_bytes: usize,
    truncation_threshold: usize,
) -> std::io::Result<GitOutputPreview> {
    let mut stdout = Vec::with_capacity(maximum_bytes.min(64 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            return Ok(GitOutputPreview {
                stdout,
                truncated: false,
            });
        }
        let remaining = maximum_bytes.saturating_sub(stdout.len());
        let retained = count.min(remaining);
        stdout.extend_from_slice(&buffer[..retained]);
        if retained < count || stdout.len() > truncation_threshold {
            return Ok(GitOutputPreview {
                stdout,
                truncated: true,
            });
        }
    }
}

fn poll_git_preview_pipe(
    reader: &Receiver<std::io::Result<GitOutputPreviewRead>>,
    ready: &mut Option<GitOutputPreviewRead>,
) -> Result<(), AppError> {
    if ready.is_some() {
        return Ok(());
    }
    match reader.try_recv() {
        Ok(Ok(preview)) => {
            *ready = Some(preview);
            Ok(())
        }
        Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(AppError::new(
            "git_command_failed",
            "无法读取 Git 命令输出。",
        )),
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
    }
}

fn collect_git_preview_pipe(
    reader: Receiver<std::io::Result<GitOutputPreviewRead>>,
    ready: Option<GitOutputPreviewRead>,
    deadline: Instant,
    timeout_code: &'static str,
    timeout_message: &'static str,
) -> Result<GitOutputPreviewRead, AppError> {
    if let Some(preview) = ready {
        return Ok(preview);
    }
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT);
    if timeout.is_zero() {
        return Err(AppError::new(timeout_code, timeout_message));
    }
    match reader.recv_timeout(timeout) {
        Ok(Ok(preview)) => Ok(preview),
        Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
            "git_command_failed",
            "无法读取 Git 命令输出。",
        )),
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(timeout_code, timeout_message)),
    }
}

fn spawn_git_input_writer(
    mut stdin: std::process::ChildStdin,
    input: Vec<u8>,
    pipe_operation: Arc<GitPipeOperationPermit>,
) -> Receiver<std::io::Result<()>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _pipe_operation = pipe_operation;
        let result = stdin.write_all(&input);
        drop(stdin);
        let _ = sender.send(result);
    });
    receiver
}

fn poll_git_input_writer(
    writer: &Receiver<std::io::Result<()>>,
    ready: &mut bool,
) -> Result<(), AppError> {
    if *ready {
        return Ok(());
    }
    match writer.try_recv() {
        Ok(Ok(())) => {
            *ready = true;
            Ok(())
        }
        Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"))
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
    }
}

fn collect_git_input_writer(
    writer: Receiver<std::io::Result<()>>,
    ready: bool,
    deadline: Instant,
    timeout_code: &'static str,
    timeout_message: &'static str,
) -> Result<(), AppError> {
    if ready {
        return Ok(());
    }
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT);
    if timeout.is_zero() {
        return Err(AppError::new(timeout_code, timeout_message));
    }
    match writer.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
            Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"))
        }
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(timeout_code, timeout_message)),
    }
}

fn read_network_git_pipe(
    mut pipe: impl Read,
    maximum_bytes: usize,
) -> Result<Vec<u8>, NetworkGitPipeError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(NetworkGitPipeError::Io)?;
        if count == 0 {
            break;
        }
        let remaining = maximum_bytes.saturating_sub(bytes.len());
        if count > remaining {
            return Err(NetworkGitPipeError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn poll_network_git_pipe(
    reader: &Receiver<Result<Vec<u8>, NetworkGitPipeError>>,
    ready: &mut Option<Vec<u8>>,
) -> Result<(), AppError> {
    if ready.is_some() {
        return Ok(());
    }
    match reader.try_recv() {
        Ok(Ok(bytes)) => {
            *ready = Some(bytes);
            Ok(())
        }
        Ok(Err(error)) => Err(network_git_pipe_error(error)),
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(AppError::new(
            "git_command_failed",
            "无法读取 Git 网络命令输出。",
        )),
    }
}

fn collect_network_git_pipe(
    reader: Receiver<Result<Vec<u8>, NetworkGitPipeError>>,
    ready: Option<Vec<u8>>,
    deadline: Instant,
    timeout_code: &'static str,
    timeout_message: &'static str,
) -> Result<Vec<u8>, AppError> {
    if let Some(bytes) = ready {
        return Ok(bytes);
    }
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT);
    if timeout.is_zero() {
        return Err(AppError::new(timeout_code, timeout_message));
    }
    match reader.recv_timeout(timeout) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(network_git_pipe_error(error)),
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(timeout_code, timeout_message)),
        Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
            "git_command_failed",
            "无法读取 Git 网络命令输出。",
        )),
    }
}

fn network_git_pipe_error(error: NetworkGitPipeError) -> AppError {
    match error {
        NetworkGitPipeError::TooLarge => {
            AppError::new("git_output_too_large", "Git 命令输出过大，已停止该操作。")
        }
        NetworkGitPipeError::Io(error) => {
            let _ = error.kind();
            AppError::new("git_command_failed", "无法读取 Git 网络命令输出。")
        }
    }
}
