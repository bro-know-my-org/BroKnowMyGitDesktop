use super::editor::{
    configure_git_network_process_group, resume_git_network_process, terminate_git_network_child,
};
use super::environment::*;
use super::error::*;
use super::models::*;
use super::remotes::MAX_NETWORK_CONFIGURATION_BYTES;
use super::runtime::GitPipeOperationPermit;
use super::validation::*;
use super::*;
use crate::process_tree::{attach_process_tree, ProcessTreeHandle};
use std::sync::Arc;

struct PushUpstream {
    local_branch: String,
    remote_branch: String,
    pushed_commit: String,
    tracking_reference: Option<String>,
    previous_tracking_commit: Option<String>,
    preserve_existing_upstream: bool,
}

pub(crate) struct PublishedRemoteState {
    pub verified_fetch_url: String,
    pub fetch_urls: Vec<String>,
    pub push_urls: Vec<String>,
}

#[derive(PartialEq, Eq)]
struct BranchUpstream {
    remote: String,
    remote_reference: String,
    tracking_reference: Option<String>,
}

#[cfg(unix)]
struct BranchReferenceLock {
    directories: Vec<fs::File>,
    directory_names: Vec<std::ffi::CString>,
    file: Option<fs::File>,
    name: std::ffi::CString,
    released: bool,
}

#[cfg(unix)]
impl BranchReferenceLock {
    // Safety invariants for the raw-fd operations below:
    // - every CString outlives the libc call that borrows its pointer;
    // - `parent` is always a live directory descriptor opened with O_NOFOLLOW;
    // - every successful open/openat descriptor is transferred exactly once
    //   into File, so early returns close it and no descriptor is double-owned;
    // - cleanup addresses the lock relative to the retained parent descriptor,
    //   never by resolving a repository-controlled pathname again.
    fn acquire(git: &GitCore, repository: &Path, reference: &str) -> Result<Self, AppError> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use std::os::unix::io::FromRawFd;

        validate_ref_name(reference, "本地分支引用")?;
        let common_directory = fs::canonicalize(git.git_common_directory(repository)?)
            .map_err(|error| AppError::io("验证 Git 引用目录", error))?;
        let common_directory = std::ffi::CString::new(common_directory.as_os_str().as_bytes())
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        let descriptor = unsafe {
            libc::open(
                common_directory.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(AppError::io(
                "锁定发布分支",
                std::io::Error::last_os_error(),
            ));
        }
        let mut directories = vec![unsafe { fs::File::from_raw_fd(descriptor) }];
        let mut directory_names = Vec::new();
        let mut components = Path::new(reference)
            .components()
            .map(|component| match component {
                std::path::Component::Normal(component) => Ok(component.to_os_string()),
                _ => Err(push_tracking_setup_incomplete_error()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let leaf = components
            .pop()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        for component in components {
            let component = std::ffi::CString::new(component.into_vec())
                .map_err(|_| push_tracking_setup_incomplete_error())?;
            let parent = directories
                .last()
                .ok_or_else(push_tracking_setup_incomplete_error)?;
            let mut descriptor = unsafe {
                libc::openat(
                    std::os::unix::io::AsRawFd::as_raw_fd(parent),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if descriptor < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(AppError::io("锁定发布分支", error));
                }
                let created = unsafe {
                    libc::mkdirat(
                        std::os::unix::io::AsRawFd::as_raw_fd(parent),
                        component.as_ptr(),
                        0o777,
                    )
                };
                if created < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(AppError::io("锁定发布分支", error));
                    }
                }
                descriptor = unsafe {
                    libc::openat(
                        std::os::unix::io::AsRawFd::as_raw_fd(parent),
                        component.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if descriptor < 0 {
                    return Err(AppError::io(
                        "锁定发布分支",
                        std::io::Error::last_os_error(),
                    ));
                }
            }
            directory_names.push(component);
            directories.push(unsafe { fs::File::from_raw_fd(descriptor) });
        }
        let mut lock_name = leaf;
        lock_name.push(".lock");
        let lock_name = std::ffi::CString::new(lock_name.into_vec())
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        let parent = directories
            .last()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let descriptor = unsafe {
            libc::openat(
                std::os::unix::io::AsRawFd::as_raw_fd(parent),
                lock_name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o666,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
                push_tracking_setup_incomplete_error()
            } else {
                AppError::io("锁定发布分支", error)
            });
        }
        Ok(Self {
            directories,
            directory_names,
            file: Some(unsafe { fs::File::from_raw_fd(descriptor) }),
            name: lock_name,
            released: false,
        })
    }

    fn ensure_attached(&self) -> Result<(), AppError> {
        use std::os::unix::io::AsRawFd;

        if self.directories.len() != self.directory_names.len() + 1 {
            return Err(push_tracking_setup_incomplete_error());
        }
        for (index, name) in self.directory_names.iter().enumerate() {
            let mut owned: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(self.directories[index + 1].as_raw_fd(), &mut owned) } < 0 {
                return Err(AppError::io(
                    "验证发布分支锁目录",
                    std::io::Error::last_os_error(),
                ));
            }
            let mut current: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    self.directories[index].as_raw_fd(),
                    name.as_ptr(),
                    &mut current,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } < 0
                || owned.st_dev != current.st_dev
                || owned.st_ino != current.st_ino
            {
                return Err(push_tracking_setup_incomplete_error());
            }
        }
        let file = self
            .file
            .as_ref()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let parent = self
            .directories
            .last()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let mut owned: libc::stat = unsafe { std::mem::zeroed() };
        let mut current: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(file.as_raw_fd(), &mut owned) } < 0
            || unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    self.name.as_ptr(),
                    &mut current,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } < 0
            || owned.st_dev != current.st_dev
            || owned.st_ino != current.st_ino
        {
            return Err(push_tracking_setup_incomplete_error());
        }
        Ok(())
    }

    fn remove_once(&mut self) -> Result<(), AppError> {
        use std::os::unix::io::AsRawFd;

        if let Err(error) = self.ensure_attached() {
            self.released = true;
            self.file.take();
            self.directories.clear();
            return Err(error);
        }
        let file = self
            .file
            .as_ref()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let parent = self
            .directories
            .last()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let mut owned: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(file.as_raw_fd(), &mut owned) } < 0 {
            self.released = true;
            self.file.take();
            self.directories.clear();
            return Err(AppError::io(
                "验证发布分支锁",
                std::io::Error::last_os_error(),
            ));
        }
        let mut current: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                self.name.as_ptr(),
                &mut current,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            let error = std::io::Error::last_os_error();
            self.released = true;
            self.file.take();
            self.directories.clear();
            return if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(AppError::io("验证发布分支锁", error))
            };
        }
        if owned.st_dev != current.st_dev || owned.st_ino != current.st_ino {
            self.released = true;
            self.file.take();
            self.directories.clear();
            return Ok(());
        }
        let result = unsafe { libc::unlinkat(parent.as_raw_fd(), self.name.as_ptr(), 0) };
        self.released = true;
        self.file.take();
        self.directories.clear();
        if result < 0 {
            return Err(AppError::io(
                "释放发布分支锁",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn release(mut self) -> Result<(), AppError> {
        self.remove_once()
    }
}

#[cfg(unix)]
impl Drop for BranchReferenceLock {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.remove_once();
        }
    }
}

#[cfg(windows)]
struct BranchReferenceLock {
    path: Option<PathBuf>,
    file: Option<fs::File>,
    directories: Vec<fs::File>,
    released: bool,
}

#[cfg(windows)]
fn branch_lock_directory_is_safe(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_type().is_dir()
        && metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            == 0
}

#[cfg(windows)]
fn open_branch_lock_directory(path: &Path) -> Result<fs::File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let directory = fs::OpenOptions::new()
        .read(true)
        // Omitting FILE_SHARE_DELETE keeps this exact directory entry from
        // being replaced while descendants are resolved and the lock lives.
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| AppError::io("锁定发布分支", error))?;
    if !branch_lock_directory_is_safe(
        &directory
            .metadata()
            .map_err(|error| AppError::io("锁定发布分支", error))?,
    ) {
        return Err(push_tracking_setup_incomplete_error());
    }
    Ok(directory)
}

#[cfg(windows)]
impl BranchReferenceLock {
    fn ensure_attached(&self) -> Result<(), AppError> {
        for directory in &self.directories {
            let metadata = directory
                .metadata()
                .map_err(|error| AppError::io("验证发布分支锁目录", error))?;
            if !branch_lock_directory_is_safe(&metadata) {
                return Err(push_tracking_setup_incomplete_error());
            }
        }
        Ok(())
    }

    fn acquire(git: &GitCore, repository: &Path, reference: &str) -> Result<Self, AppError> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };

        const DELETE_ACCESS: u32 = 0x0001_0000;
        const GENERIC_WRITE_ACCESS: u32 = 0x4000_0000;

        validate_ref_name(reference, "本地分支引用")?;
        let common_directory = fs::canonicalize(git.git_common_directory(repository)?)
            .map_err(|error| AppError::io("验证 Git 引用目录", error))?;
        let relative = Path::new(reference);
        let parent = relative
            .parent()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let mut resolved_parent = common_directory;
        let mut directories = vec![open_branch_lock_directory(&resolved_parent)?];
        for component in parent.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(push_tracking_setup_incomplete_error());
            };
            resolved_parent.push(component);
            let directory = match open_branch_lock_directory(&resolved_parent) {
                Ok(directory) => directory,
                Err(error) if !resolved_parent.exists() => {
                    if let Err(create_error) = fs::create_dir(&resolved_parent) {
                        if create_error.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(AppError::io("锁定发布分支", create_error));
                        }
                    }
                    open_branch_lock_directory(&resolved_parent).map_err(|_| error)?
                }
                Err(error) => return Err(error),
            };
            directories.push(directory);
        }
        let mut lock_name = relative
            .file_name()
            .ok_or_else(push_tracking_setup_incomplete_error)?
            .to_os_string();
        lock_name.push(".lock");
        let lock_path = resolved_parent.join(lock_name);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_WRITE_ACCESS | DELETE_ACCESS)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_DELETE_ON_CLOSE)
            .open(&lock_path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    push_tracking_setup_incomplete_error()
                } else {
                    AppError::io("锁定发布分支", error)
                }
            })?;
        Ok(Self {
            path: Some(lock_path),
            file: Some(file),
            directories,
            released: false,
        })
    }

    fn remove_once(&mut self) -> Result<(), AppError> {
        self.released = true;
        // FILE_FLAG_DELETE_ON_CLOSE removes the exact opened file object;
        // never resolve the lock pathname again after ownership is released.
        self.file.take();
        self.path.take();
        self.directories.clear();
        Ok(())
    }

    fn release(mut self) -> Result<(), AppError> {
        self.remove_once()
    }
}

#[cfg(windows)]
impl Drop for BranchReferenceLock {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.remove_once();
        }
    }
}

#[cfg(all(not(unix), not(windows)))]
struct BranchReferenceLock;

#[cfg(all(not(unix), not(windows)))]
impl BranchReferenceLock {
    fn acquire(_git: &GitCore, _repository: &Path, reference: &str) -> Result<Self, AppError> {
        validate_ref_name(reference, "本地分支引用")?;
        Err(push_tracking_setup_incomplete_error())
    }

    fn release(self) -> Result<(), AppError> {
        Ok(())
    }

    fn ensure_attached(&self) -> Result<(), AppError> {
        Err(push_tracking_setup_incomplete_error())
    }
}

struct PreparedTrackingReferenceUpdate {
    child: Option<std::process::Child>,
    process_id: u32,
    process_tree: Option<ProcessTreeHandle>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Receiver<Result<String, std::io::Error>>,
    stdout_worker: Option<thread::JoinHandle<()>>,
    stdout_done: Receiver<()>,
    stderr_worker: Option<thread::JoinHandle<()>>,
    stderr_done: Receiver<()>,
    changed: bool,
}

impl PreparedTrackingReferenceUpdate {
    fn prepare(
        git: &GitCore,
        repository: &Path,
        reference: &str,
        pushed_commit: &str,
        previous_commit: Option<&str>,
    ) -> Result<Self, AppError> {
        validate_ref_name(reference, "远程跟踪引用")?;
        parse_commit_id(pushed_commit.as_bytes(), "无法读取首次推送的本地提交标识。")?;
        let expected = previous_commit
            .map(str::to_owned)
            .unwrap_or_else(|| "0".repeat(pushed_commit.len()));
        let update = format!("update {reference} {pushed_commit} {expected}");
        Self::prepare_commands(
            git,
            repository,
            std::slice::from_ref(&update),
            previous_commit != Some(pushed_commit),
        )
    }

    fn prepare_commands(
        git: &GitCore,
        repository: &Path,
        commands: &[String],
        changed: bool,
    ) -> Result<Self, AppError> {
        let pipe_operation = GitPipeOperationPermit::acquire()?;
        let mut command = Command::new(&git.executable);
        command.arg("-C").arg(repository).args([
            "update-ref",
            "--no-deref",
            "--create-reflog",
            "-m",
            "update by push",
            "--stdin",
        ]);
        let mut environment = GitEnvironment::new();
        environment.isolate_network_credentials_and_disable_hooks();
        apply_git_environment(&mut command, &environment);
        configure_git_network_process_group(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| AppError::io("准备远程跟踪引用更新", error))?;
        let process_id = child.id();
        let process_tree = match attach_process_tree(&child) {
            Ok(process_tree) => process_tree,
            Err(error) => {
                terminate_git_network_child(&mut child, process_id, None);
                return Err(AppError::io("配置远程跟踪引用进程树", error));
            }
        };
        if let Err(error) = resume_git_network_process(process_id) {
            terminate_git_network_child(&mut child, process_id, Some(&process_tree));
            return Err(AppError::io("启动远程跟踪引用事务", error));
        }
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (stdin, stdout, stderr) = match pipes {
            (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
            _ => {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(push_tracking_setup_incomplete_error());
            }
        };
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let (stdout_done_sender, stdout_done_receiver) = mpsc::channel();
        let stdout_permit = Arc::clone(&pipe_operation);
        let stdout_worker = match thread::Builder::new()
            .name("git-update-ref-stdout".into())
            .spawn(move || {
                let _permit = stdout_permit;
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if stdout_sender.send(Ok(line)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = stdout_sender.send(Err(error));
                            break;
                        }
                    }
                }
                let _ = stdout_done_sender.send(());
            }) {
            Ok(worker) => worker,
            Err(error) => {
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                return Err(AppError::io("启动远程跟踪引用输出读取", error));
            }
        };
        let (stderr_done_sender, stderr_done_receiver) = mpsc::channel();
        let stderr_permit = Arc::clone(&pipe_operation);
        let stderr_worker = match thread::Builder::new()
            .name("git-update-ref-stderr".into())
            .spawn(move || {
                let _permit = stderr_permit;
                let mut stderr = stderr;
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
                let _ = stderr_done_sender.send(());
            }) {
            Ok(worker) => worker,
            Err(error) => {
                drop(stdin);
                terminate_git_network_child(&mut child, process_id, Some(&process_tree));
                if stdout_done_receiver
                    .recv_timeout(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT)
                    .is_ok()
                {
                    let _ = stdout_worker.join();
                }
                return Err(AppError::io("启动远程跟踪引用错误读取", error));
            }
        };
        drop(pipe_operation);
        let mut prepared = Self {
            child: Some(child),
            process_id,
            process_tree: Some(process_tree),
            stdin: Some(stdin),
            stdout: stdout_receiver,
            stdout_worker: Some(stdout_worker),
            stdout_done: stdout_done_receiver,
            stderr_worker: Some(stderr_worker),
            stderr_done: stderr_done_receiver,
            changed,
        };
        let stdin = prepared
            .stdin
            .as_mut()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        writeln!(stdin, "start").map_err(|error| AppError::io("准备远程跟踪引用更新", error))?;
        for command in commands {
            writeln!(stdin, "{command}")
                .map_err(|error| AppError::io("准备远程跟踪引用更新", error))?;
        }
        writeln!(stdin, "prepare")
            .and_then(|()| stdin.flush())
            .map_err(|error| AppError::io("准备远程跟踪引用更新", error))?;
        prepared.expect_response("start: ok")?;
        prepared.expect_response("prepare: ok")?;
        Ok(prepared)
    }

    fn expect_response(&mut self, expected: &str) -> Result<(), AppError> {
        let response = self
            .stdout
            .recv_timeout(BOUNDED_GIT_COMMAND_TIMEOUT)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => {
                    AppError::new("git_command_timeout", "远程跟踪引用事务超时，请稍后重试。")
                }
                RecvTimeoutError::Disconnected => push_tracking_setup_incomplete_error(),
            })?
            .map_err(|error| AppError::io("读取远程跟踪引用事务", error))?;
        if response.trim_end() == expected {
            Ok(())
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn finish(mut self, command: &str, expected: &str) -> Result<bool, AppError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        writeln!(stdin, "{command}")
            .and_then(|()| stdin.flush())
            .map_err(|error| AppError::io("提交远程跟踪引用事务", error))?;
        self.expect_response(expected)?;
        self.stdin.take();
        let deadline = Instant::now() + BOUNDED_GIT_COMMAND_TIMEOUT;
        let status = loop {
            let status = self
                .child
                .as_mut()
                .ok_or_else(push_tracking_setup_incomplete_error)?
                .try_wait()
                .map_err(|error| AppError::io("等待远程跟踪引用事务", error))?;
            if let Some(status) = status {
                break status;
            }
            if Instant::now() >= deadline {
                self.terminate_child();
                self.collect_workers();
                return Err(AppError::new(
                    "git_command_timeout",
                    "远程跟踪引用事务超时，请稍后重试。",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        self.child.take();
        self.process_tree.take();
        self.collect_workers();
        if status.success() {
            Ok(self.changed)
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn commit(self) -> Result<bool, AppError> {
        self.finish("commit", "commit: ok")
    }

    fn abort(self) -> Result<(), AppError> {
        self.finish("abort", "abort: ok").map(|_| ())
    }

    fn changed(&self) -> bool {
        self.changed
    }

    fn terminate_child(&mut self) {
        if let Some(child) = self.child.as_mut() {
            terminate_git_network_child(child, self.process_id, self.process_tree.as_ref());
        }
        self.child.take();
        self.process_tree.take();
    }

    fn collect_workers(&mut self) {
        if self
            .stdout_done
            .recv_timeout(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT)
            .is_ok()
        {
            if let Some(worker) = self.stdout_worker.take() {
                let _ = worker.join();
            }
        } else {
            self.stdout_worker.take();
        }
        if self
            .stderr_done
            .recv_timeout(NETWORK_GIT_PIPE_COLLECTION_TIMEOUT)
            .is_ok()
        {
            if let Some(worker) = self.stderr_worker.take() {
                let _ = worker.join();
            }
        } else {
            self.stderr_worker.take();
        }
    }
}

impl Drop for PreparedTrackingReferenceUpdate {
    fn drop(&mut self) {
        self.stdin.take();
        self.terminate_child();
        self.collect_workers();
    }
}

struct ConfigurationLockEntry {
    path: PathBuf,
    file: Option<fs::File>,
}

impl ConfigurationLockEntry {
    fn file_still_owns_path(path: &Path, file: &fs::File) -> Result<bool, AppError> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };

            let current = match fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(AppError::io("验证分支上游配置锁", error)),
            };
            return Ok(windows_file_identity(file)? == windows_file_identity(&current)?);
        }
        #[cfg(not(windows))]
        {
            let owned = file
                .metadata()
                .map_err(|error| AppError::io("验证分支上游配置锁", error))?;
            let current = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(AppError::io("验证分支上游配置锁", error)),
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                Ok(owned.dev() == current.dev()
                    && owned.ino() == current.ino()
                    && current.file_type().is_file())
            }
            #[cfg(all(not(unix), not(windows)))]
            {
                Ok(current.file_type().is_file()
                    && owned.len() == current.len()
                    && owned.created().ok() == current.created().ok())
            }
        }
    }

    fn still_owns_path(&self) -> Result<bool, AppError> {
        Self::file_still_owns_path(
            &self.path,
            self.file
                .as_ref()
                .ok_or_else(push_tracking_setup_incomplete_error)?,
        )
    }

    fn release(mut self) -> Result<(), AppError> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
            };

            let file = self
                .file
                .as_ref()
                .ok_or_else(push_tracking_setup_incomplete_error)?;
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            let result = unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle() as _,
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            self.file.take();
            if result == 0 {
                return Err(AppError::io(
                    "释放分支上游配置锁",
                    std::io::Error::last_os_error(),
                ));
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            if !self.still_owns_path()? {
                self.file.take();
                return Ok(());
            }
            // Remove exactly once while the owned handle is still live. If this
            // fails, do not retry the pathname later: ownership may transfer to a
            // new writer after this call returns.
            let result = fs::remove_file(&self.path)
                .map_err(|error| AppError::io("释放分支上游配置锁", error));
            self.file.take();
            result
        }
    }
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> Result<(u64, [u8; 16]), AppError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut identity = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(AppError::io(
            "验证分支上游配置锁",
            std::io::Error::last_os_error(),
        ));
    }
    Ok((identity.VolumeSerialNumber, identity.FileId.Identifier))
}

#[cfg(windows)]
fn replace_configuration_file_by_handle(
    file: &fs::File,
    destination: &Path,
) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileRenameInfoEx, SetFileInformationByHandle, FILE_RENAME_INFO,
    };

    const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 1;
    let destination = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    let header_bytes = std::mem::size_of::<FILE_RENAME_INFO>() - std::mem::size_of::<u16>();
    let buffer_bytes = header_bytes + destination.len() * std::mem::size_of::<u16>();
    let units = buffer_bytes.div_ceil(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![FILE_RENAME_INFO::default(); units.max(1)];
    let information = &mut buffer[0];
    information.Anonymous.Flags = FILE_RENAME_FLAG_REPLACE_IF_EXISTS;
    information.RootDirectory = std::ptr::null_mut();
    information.FileNameLength = (destination.len() * std::mem::size_of::<u16>()) as u32;
    unsafe {
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            information.FileName.as_mut_ptr(),
            destination.len(),
        );
    }
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle() as _,
            FileRenameInfoEx,
            buffer.as_ptr().cast(),
            buffer_bytes as u32,
        )
    };
    if result == 0 {
        return Err(AppError::io(
            "提交分支上游配置",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ConfigurationLockCleanup {
    locks: Vec<ConfigurationLockEntry>,
}

impl ConfigurationLockCleanup {
    fn release(&mut self) -> Result<(), AppError> {
        let mut first_error = None;
        for lock in self.locks.drain(..) {
            if let Err(error) = lock.release() {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn into_locks(mut self) -> Vec<ConfigurationLockEntry> {
        std::mem::take(&mut self.locks)
    }
}

impl Drop for ConfigurationLockCleanup {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

struct BranchUpstreamConfigTransaction {
    configuration_path: PathBuf,
    configuration_files: Vec<(PathBuf, fs::File)>,
    configuration_paths: Vec<PathBuf>,
    lock_path: PathBuf,
    lock_file: Option<fs::File>,
    additional_locks: Vec<ConfigurationLockEntry>,
    committed: bool,
}

enum BranchConfigCommitError {
    Before(AppError),
    BeforeCleanup,
    AfterDurability(AppError),
    AfterCleanup(AppError),
}

impl BranchUpstreamConfigTransaction {
    fn ensure_configuration_is_owned(&self) -> Result<(), AppError> {
        for (path, file) in &self.configuration_files {
            if !ConfigurationLockEntry::file_still_owns_path(path, file)? {
                return Err(push_tracking_setup_incomplete_error());
            }
        }
        for path in &self.configuration_paths {
            if self
                .configuration_files
                .iter()
                .any(|(opened, _)| opened == path)
            {
                continue;
            }
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(push_tracking_setup_incomplete_error()),
            }
        }
        Ok(())
    }

    fn ensure_primary_lock_is_owned(&self) -> Result<(), AppError> {
        let file = self
            .lock_file
            .as_ref()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        if ConfigurationLockEntry::file_still_owns_path(&self.lock_path, file)? {
            Ok(())
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn acquire(git: &GitCore, repository: &Path) -> Result<Self, AppError> {
        let configurations = git.network_configuration_paths(repository)?;
        let configuration_path = configurations
            .first()
            .cloned()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let mut lock_cleanup = ConfigurationLockCleanup {
            locks: Vec::with_capacity(configurations.len()),
        };
        for configuration in &configurations {
            let name = configuration
                .file_name()
                .ok_or_else(push_tracking_setup_incomplete_error)?;
            let mut lock_name = name.to_os_string();
            lock_name.push(".lock");
            let lock_path = configuration.with_file_name(lock_name);
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
                    FILE_SHARE_WRITE,
                };
                const DELETE_ACCESS: u32 = 0x0001_0000;
                const GENERIC_READ_ACCESS: u32 = 0x8000_0000;
                const GENERIC_WRITE_ACCESS: u32 = 0x4000_0000;
                options
                    .access_mode(GENERIC_READ_ACCESS | GENERIC_WRITE_ACCESS | DELETE_ACCESS)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            }
            match options.open(&lock_path) {
                Ok(file) => {
                    lock_cleanup.locks.push(ConfigurationLockEntry {
                        path: lock_path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let original = push_tracking_setup_incomplete_error();
                    return Err(lock_cleanup.release().err().unwrap_or(original));
                }
                Err(error) => {
                    let original = AppError::io("锁定分支上游配置", error);
                    return Err(lock_cleanup.release().err().unwrap_or(original));
                }
            }
        }
        let mut locks = lock_cleanup.into_locks();
        let primary_lock = if locks.is_empty() {
            None
        } else {
            Some(locks.remove(0))
        }
        .ok_or_else(push_tracking_setup_incomplete_error)?;
        Ok(Self {
            configuration_path,
            configuration_files: Vec::new(),
            configuration_paths: configurations,
            lock_path: primary_lock.path,
            lock_file: primary_lock.file,
            additional_locks: locks,
            committed: false,
        })
    }

    fn prepare_staging_config(&mut self) -> Result<(), AppError> {
        let mut source_options = fs::OpenOptions::new();
        source_options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            source_options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            source_options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        let mut configuration_files = Vec::new();
        for path in &self.configuration_paths {
            let file = match source_options.open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(AppError::io("检查分支上游配置", error)),
            };
            let metadata = file
                .metadata()
                .map_err(|error| AppError::io("检查分支上游配置", error))?;
            #[cfg(windows)]
            let is_reparse_point = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    != 0
            };
            #[cfg(not(windows))]
            let is_reparse_point = false;
            if !metadata.file_type().is_file()
                || is_reparse_point
                || metadata.len() > MAX_NETWORK_CONFIGURATION_BYTES as u64
            {
                return Err(AppError::new(
                    "unsafe_git_config",
                    "仓库 Git 配置不是可安全更新的普通文件。",
                ));
            }
            configuration_files.push((path.clone(), file));
        }
        let primary = configuration_files
            .iter()
            .find(|(path, _)| path == &self.configuration_path)
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        let mut source = primary
            .1
            .try_clone()
            .map_err(|error| AppError::io("检查分支上游配置", error))?;
        let metadata = source
            .metadata()
            .map_err(|error| AppError::io("检查分支上游配置", error))?;
        let staging = self
            .lock_file
            .as_mut()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        staging
            .set_len(0)
            .map_err(|error| AppError::io("准备分支上游配置", error))?;
        std::io::Seek::seek(staging, std::io::SeekFrom::Start(0))
            .map_err(|error| AppError::io("准备分支上游配置", error))?;
        let staging_metadata = staging
            .metadata()
            .map_err(|error| AppError::io("准备分支上游配置", error))?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if staging_metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(AppError::new(
                    "unsafe_git_config",
                    "仓库 Git 配置暂存文件无法安全更新。",
                ));
            }
        }
        if !staging_metadata.file_type().is_file() {
            return Err(AppError::new(
                "unsafe_git_config",
                "仓库 Git 配置暂存文件无法安全更新。",
            ));
        }
        std::io::copy(&mut source, staging)
            .map_err(|error| AppError::io("准备分支上游配置", error))?;
        staging
            .set_permissions(metadata.permissions())
            .map_err(|error| AppError::io("准备分支上游配置权限", error))?;
        self.configuration_files = configuration_files;
        self.ensure_configuration_is_owned()?;
        Ok(())
    }

    fn ensure_upstream_is_compatible(
        &self,
        git: &GitCore,
        branch: &str,
        remote_name: &str,
        remote_reference: &str,
    ) -> Result<bool, AppError> {
        self.ensure_configuration_is_owned()?;
        let compatible = ensure_branch_config_values_are_compatible(
            git,
            &self.configuration_paths,
            branch,
            remote_name,
            remote_reference,
        )?;
        self.ensure_configuration_is_owned()?;
        Ok(compatible)
    }

    fn set_upstream(
        &mut self,
        git: &GitCore,
        branch: &str,
        remote_name: &str,
        remote_reference: &str,
    ) -> Result<(), AppError> {
        self.ensure_primary_lock_is_owned()?;
        let staging = self
            .lock_file
            .as_mut()
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        std::io::Seek::seek(staging, std::io::SeekFrom::End(0))
            .map_err(|error| AppError::io("准备分支上游配置", error))?;
        writeln!(
            staging,
            "\n[branch \"{}\"]\n\tremote = \"{}\"\n\tmerge = \"{}\"",
            escape_git_config_value(branch),
            escape_git_config_value(remote_name),
            escape_git_config_value(remote_reference),
        )
        .and_then(|()| staging.flush())
        .map_err(|error| AppError::io("准备分支上游配置", error))?;
        self.ensure_primary_lock_is_owned()?;
        if ensure_branch_config_values_are_compatible(
            git,
            std::slice::from_ref(&self.lock_path),
            branch,
            remote_name,
            remote_reference,
        )? {
            self.ensure_primary_lock_is_owned()?;
            self.ensure_configuration_is_owned()
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn abort(mut self) -> Result<(), AppError> {
        self.committed = true;
        self.configuration_files.clear();
        let mut locks = std::mem::take(&mut self.additional_locks);
        if let Some(file) = self.lock_file.take() {
            locks.insert(
                0,
                ConfigurationLockEntry {
                    path: self.lock_path.clone(),
                    file: Some(file),
                },
            );
        }
        ConfigurationLockCleanup { locks }.release()
    }

    fn abort_with_error(self, error: AppError) -> BranchConfigCommitError {
        match self.abort() {
            Ok(()) => BranchConfigCommitError::Before(error),
            Err(_) => BranchConfigCommitError::BeforeCleanup,
        }
    }

    fn commit(mut self) -> Result<(), BranchConfigCommitError> {
        if let Err(error) = self
            .lock_file
            .as_ref()
            .ok_or_else(push_tracking_setup_incomplete_error)
            .and_then(|file| {
                file.sync_all()
                    .map_err(|error| AppError::io("同步分支上游配置", error))
            })
            .and_then(|()| self.ensure_primary_lock_is_owned())
            .and_then(|()| self.ensure_configuration_is_owned())
        {
            return Err(self.abort_with_error(error));
        }
        #[cfg(windows)]
        let replacement = match self.lock_file.as_ref() {
            Some(file) => replace_configuration_file_by_handle(file, &self.configuration_path),
            None => Err(push_tracking_setup_incomplete_error()),
        };
        #[cfg(not(windows))]
        let replacement =
            crate::persistence::replace_file(&self.lock_path, &self.configuration_path)
                .map_err(|error| AppError::io("提交分支上游配置", error));
        if let Err(error) = replacement {
            return Err(self.abort_with_error(error));
        }
        self.configuration_files.clear();
        self.lock_file.take();
        self.committed = true;
        let durability_error = crate::persistence::sync_parent_directory(&self.configuration_path)
            .err()
            .map(|error| AppError::io("同步分支上游配置目录", error));
        let cleanup_error = ConfigurationLockCleanup {
            locks: std::mem::take(&mut self.additional_locks),
        }
        .release()
        .err();
        if let Some(error) = cleanup_error {
            return Err(BranchConfigCommitError::AfterCleanup(error));
        }
        if let Some(error) = durability_error {
            return Err(BranchConfigCommitError::AfterDurability(error));
        }
        Ok(())
    }
}

fn escape_git_config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn branch_config_values(
    git: &GitCore,
    configuration: &Path,
    key: &str,
) -> Result<Vec<String>, AppError> {
    let output = git.execute(
        None,
        &[
            OsString::from("config"),
            OsString::from("--file"),
            configuration.as_os_str().to_owned(),
            OsString::from("--get-all"),
            OsString::from(key),
        ],
        &GitEnvironment::default(),
    )?;
    if output.status.code() == Some(1) {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        return Err(git_command_error("读取分支上游配置", &output));
    }
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|value| !value.is_empty())
        .map(|value| {
            std::str::from_utf8(value)
                .map(str::to_owned)
                .map_err(|_| AppError::new("git_parse_error", "无法读取分支上游配置。"))
        })
        .collect()
}

fn ensure_branch_config_values_are_compatible(
    git: &GitCore,
    configurations: &[PathBuf],
    branch: &str,
    remote_name: &str,
    remote_reference: &str,
) -> Result<bool, AppError> {
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let mut configured_remote = None;
    let mut configured_merge = None;
    for configuration in configurations {
        if !configuration.exists() {
            continue;
        }
        let remotes = branch_config_values(git, configuration, &remote_key)?;
        let merges = branch_config_values(git, configuration, &merge_key)?;
        if remotes.len() > 1 || merges.len() > 1 {
            return Err(push_tracking_setup_incomplete_error());
        }
        if let Some(remote) = remotes.into_iter().next() {
            match configured_remote.as_deref() {
                Some(configured) if configured != remote => {
                    return Err(push_tracking_setup_incomplete_error())
                }
                None => configured_remote = Some(remote),
                _ => {}
            }
        }
        if let Some(merge) = merges.into_iter().next() {
            match configured_merge.as_deref() {
                Some(configured) if configured != merge => {
                    return Err(push_tracking_setup_incomplete_error())
                }
                None => configured_merge = Some(merge),
                _ => {}
            }
        }
    }
    match (configured_remote.as_deref(), configured_merge.as_deref()) {
        (None, None) => Ok(false),
        (Some(configured_remote), Some(configured_merge))
            if configured_remote == remote_name && configured_merge == remote_reference =>
        {
            Ok(true)
        }
        _ => Err(push_tracking_setup_incomplete_error()),
    }
}

impl Drop for BranchUpstreamConfigTransaction {
    fn drop(&mut self) {
        self.configuration_files.clear();
        let mut locks = std::mem::take(&mut self.additional_locks);
        if !self.committed {
            if let Some(file) = self.lock_file.take() {
                locks.insert(
                    0,
                    ConfigurationLockEntry {
                        path: self.lock_path.clone(),
                        file: Some(file),
                    },
                );
            }
        }
        let _ = ConfigurationLockCleanup { locks }.release();
    }
}

impl GitCore {
    /// Reads the current tip of one remote branch without updating local refs.
    /// Publish reconciliation uses this to prove that a successful-looking
    /// `gh repo create --push` really delivered the local commit before it
    /// clears its durable recovery intent.
    pub(crate) fn remote_branch_tip(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
        environment: &GitEnvironment,
    ) -> Result<(Option<String>, String), AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(branch, "远程分支名")?;
        // Publish reconciliation is a network operation too. In particular,
        // it must not let a repository-local credential helper or transport
        // setting run just because it is only reading a remote branch tip.
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = self.remote_url_for_operation(repository, remote_name, false)?;
        let remote_url = validate_git_remote_url(&remote_url)?;
        let reference = format!("refs/heads/{branch}");
        let arguments = network_execution_arguments(os_arguments(&[
            "ls-remote",
            "--heads",
            "--refs",
            &remote_url,
            &reference,
        ]));
        let output = self.run_network(
            Some(repository),
            &arguments,
            environment,
            "验证远程发布状态",
        )?;
        let output = output_text(&output.stdout);
        let Some(line) = output.lines().next() else {
            return Ok((None, remote_url));
        };
        let Some((commit, returned_reference)) = line.split_once('\t') else {
            return Err(AppError::new("git_parse_error", "无法读取远程分支提交。"));
        };
        if returned_reference != reference
            || !(40..=64).contains(&commit.len())
            || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取远程分支提交。"));
        }
        Ok((Some(commit.to_owned()), remote_url))
    }

    /// Fetches exactly one remote.  The caller must resolve its credentials
    /// before calling this method.
    pub fn fetch(&self, repository: &Path, options: &FetchOptions) -> Result<(), AppError> {
        self.fetch_with_environment(repository, options, &GitEnvironment::default())
    }

    /// Backend-only equivalent of [`Self::fetch`] with a controlled process
    /// environment, for example an askpass helper selected by CredentialBroker.
    pub fn fetch_with_environment(
        &self,
        repository: &Path,
        options: &FetchOptions,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_configured_remote(repository, &options.remote)?;
        let refspecs = if options.refspecs.is_empty() {
            self.fetch_refspecs_for_remote(repository, &options.remote)?
        } else {
            options.refspecs.clone()
        };
        let remote_url = self.remote_url_for_operation(repository, &options.remote, false)?;
        self.fetch_with_environment_at_url(
            repository,
            &FetchOptions {
                remote: options.remote.clone(),
                refspecs,
                prune: options.prune,
            },
            &remote_url,
            environment,
        )
    }

    /// Captures the refspecs that a normal fetch would use before credentials
    /// are selected. The all-branches mapping replaces restrictive
    /// single-branch mappings, while non-branch namespaces such as Gerrit
    /// changes remain part of the operation.
    pub(crate) fn fetch_refspecs_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<Vec<String>, AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let mut refspecs = vec![all_remote_branches_fetch_refspec(remote_name)?];
        for refspec in self.repository_config_values(repository, &config_key)? {
            let refspec = refspec.trim();
            if refspec.is_empty()
                || (is_remote_branch_fetch_refspec(refspec) && !refspec.starts_with('^'))
            {
                continue;
            }
            validate_refspec(refspec)?;
            refspecs.push(refspec.to_owned());
        }
        Ok(refspecs)
    }

    /// Executes a fetch against the exact URL validated during credential
    /// resolution. This prevents a later rewrite of `remote.<name>.url` from
    /// changing where an app-selected credential is offered.
    pub(crate) fn fetch_with_environment_at_url(
        &self,
        repository: &Path,
        options: &FetchOptions,
        remote_url: &str,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = validate_git_remote_url(remote_url)?;
        let arguments = network_execution_arguments(fetch_arguments_for_url(options, &remote_url)?);
        self.run_network(Some(repository), &arguments, environment, "获取远程更新")?;
        Ok(())
    }

    /// Pulls from exactly one remote.  Rebase/merge policy remains under the
    /// user's existing Git configuration.
    pub fn pull(&self, repository: &Path, options: &PullOptions) -> Result<(), AppError> {
        self.pull_with_environment(repository, options, &GitEnvironment::default())
    }

    /// Backend-only equivalent of [`Self::pull`] with a controlled process
    /// environment.
    pub fn pull_with_environment(
        &self,
        repository: &Path,
        options: &PullOptions,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_configured_remote(repository, &options.remote)?;
        let branch = self.current_branch_name(repository)?;
        let refspec = match options.refspec.clone() {
            Some(refspec) => refspec,
            None => self.pull_refspec_for_remote(repository, &options.remote)?,
        };
        let remote_url = self.remote_url_for_operation(repository, &options.remote, false)?;
        if self.current_branch_name(repository)? != branch {
            return Err(AppError::new(
                "repository_changed",
                "当前分支已变化，请重新执行拉取。",
            ));
        }
        self.pull_with_environment_at_url(
            repository,
            &PullOptions {
                remote: options.remote.clone(),
                refspec: Some(refspec),
            },
            &remote_url,
            environment,
        )
    }

    /// Captures the current branch's configured upstream before a
    /// credential-bearing operation is prepared. Passing a raw URL to
    /// `git pull` bypasses `remote.<name>.fetch`, so use an explicit
    /// source:destination refspec: the source prevents Git from selecting the
    /// URL's advertised default branch, while the destination keeps the
    /// selected remote-tracking branch in sync just like a named-remote pull.
    pub(crate) fn pull_refspec_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<String, AppError> {
        let upstream = self.upstream_for_remote(repository, remote_name)?;
        let upstream_reference = upstream.remote_reference;
        let remote_branch = upstream_reference
            .strip_prefix("refs/heads/")
            .ok_or_else(|| {
                AppError::new("pull_upstream_not_a_branch", "当前分支上游不是远程分支。")
            })?;
        validate_ref_name(remote_branch, "远程分支名")?;
        let tracking_reference = upstream
            .tracking_reference
            .ok_or_else(pull_upstream_fetch_mapping_unavailable_error)?;
        let force = self.fetch_mapping_is_forced(
            repository,
            remote_name,
            &upstream_reference,
            &tracking_reference,
        )?;
        let refspec = format!(
            "{}{upstream_reference}:{tracking_reference}",
            if force { "+" } else { "" }
        );
        validate_refspec(&refspec)?;
        Ok(refspec)
    }

    fn upstream_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<BranchUpstream, AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let branch = self.current_branch_name(repository)?;
        let Some(upstream) = self.branch_upstream(repository, &branch)? else {
            return Err(pull_upstream_not_configured_error());
        };
        if upstream.remote != remote_name {
            return Err(AppError::new(
                "pull_upstream_remote_mismatch",
                "当前分支没有跟踪所选 remote。",
            ));
        }
        validate_refspec(&upstream.remote_reference)?;
        if let Some(tracking_reference) = &upstream.tracking_reference {
            validate_refspec(tracking_reference)?;
        }
        Ok(upstream)
    }

    fn fetch_mapping_is_forced(
        &self,
        repository: &Path,
        remote_name: &str,
        source_reference: &str,
        tracking_reference: &str,
    ) -> Result<bool, AppError> {
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let configured_refspecs = self.repository_config_values(repository, &config_key)?;
        if configured_refspecs.is_empty() {
            return Err(pull_upstream_fetch_mapping_unavailable_error());
        }

        let mut matching_force = None;
        for configured_refspec in configured_refspecs.iter().map(|refspec| refspec.trim()) {
            if configured_refspec.is_empty() {
                continue;
            }
            validate_refspec(configured_refspec)?;
            if let Some(exclusion) = configured_refspec.strip_prefix('^') {
                if refspec_source_capture(exclusion, source_reference).is_some() {
                    return Err(pull_upstream_fetch_mapping_unavailable_error());
                }
                continue;
            }
            let (forced, configured_refspec) = configured_refspec
                .strip_prefix('+')
                .map_or((false, configured_refspec), |refspec| (true, refspec));
            let Some((source_pattern, destination_pattern)) = configured_refspec.split_once(':')
            else {
                continue;
            };
            let Some(capture) = refspec_source_capture(source_pattern, source_reference) else {
                continue;
            };
            if expand_refspec_destination(destination_pattern, capture.as_deref()).as_deref()
                != Some(tracking_reference)
            {
                continue;
            }
            matching_force = Some(matching_force.unwrap_or(false) || forced);
        }
        matching_force.ok_or_else(pull_upstream_fetch_mapping_unavailable_error)
    }

    fn repository_config_values(
        &self,
        repository: &Path,
        config_key: &str,
    ) -> Result<Vec<String>, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--null", "--show-scope", "--get-all", config_key]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        if !output.status.success() {
            return Err(git_command_error("读取远程分支获取规则", &output));
        }
        parse_repository_config_values(&output.stdout)
    }

    /// Builds a deterministic current-branch push plan for the selected
    /// remote. Every path uses an explicit refspec, so user `push.default`
    /// configuration and a concurrent checkout cannot redirect the push.
    pub(crate) fn push_plan_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<(Vec<String>, bool), AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let branch = self.current_branch_name(repository)?;
        let local_reference = format!("refs/heads/{branch}");
        let (destination, set_upstream) = match self.branch_upstream(repository, &branch)? {
            Some(upstream)
                if upstream.remote == remote_name
                    && upstream.remote_reference.starts_with("refs/heads/") =>
            {
                (upstream.remote_reference, false)
            }
            Some(upstream) if upstream.remote == remote_name => {
                return Err(AppError::new(
                    "push_upstream_not_a_branch",
                    "当前分支上游不是远程分支。",
                ));
            }
            Some(_) => (local_reference.clone(), false),
            None => (local_reference.clone(), true),
        };
        let refspec = format!("{local_reference}:{destination}");
        validate_refspec(&refspec)?;
        Ok((vec![refspec], set_upstream))
    }

    fn branch_upstream(
        &self,
        repository: &Path,
        branch: &str,
    ) -> Result<Option<BranchUpstream>, AppError> {
        let branch_reference = format!("refs/heads/{branch}");
        let output = self.execute(
            Some(repository),
            &[
                OsString::from("for-each-ref"),
                OsString::from(
                    "--format=%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
                ),
                OsString::from(&branch_reference),
            ],
            &GitEnvironment::default(),
        )?;
        if !output.status.success() {
            return Err(git_command_error("读取当前分支上游", &output));
        }

        let mut matching_record = None;
        for record in output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
        {
            let Some(separator) = record.iter().position(|byte| *byte == b'\0') else {
                return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
            };
            if record[..separator] != *branch_reference.as_bytes() {
                continue;
            }
            if matching_record.replace(&record[separator + 1..]).is_some() {
                return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
            }
        }
        let record = match matching_record {
            Some(record) => record,
            None if self.current_branch_name(repository).ok().as_deref() == Some(branch)
                && self.has_unborn_head(repository)? =>
            {
                return Ok(None);
            }
            None => {
                return Err(AppError::new(
                    "repository_changed",
                    "仓库引用正在变化，请稍后重试。",
                ));
            }
        };
        let fields = record.split(|byte| *byte == b'\0').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
        }
        let configured_remote = std::str::from_utf8(fields[0])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        let remote_reference = std::str::from_utf8(fields[1])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        let tracking_reference = std::str::from_utf8(fields[2])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        if configured_remote.is_empty() || remote_reference.is_empty() {
            return Ok(None);
        }
        validate_remote_name(configured_remote)?;
        validate_refspec(remote_reference)?;
        if !tracking_reference.is_empty() {
            validate_refspec(tracking_reference)?;
        }
        Ok(Some(BranchUpstream {
            remote: configured_remote.to_owned(),
            remote_reference: remote_reference.to_owned(),
            tracking_reference: (!tracking_reference.is_empty())
                .then(|| tracking_reference.to_owned()),
        }))
    }

    /// Pulls from an immutable, previously validated operation URL.
    pub(crate) fn pull_with_environment_at_url(
        &self,
        repository: &Path,
        options: &PullOptions,
        remote_url: &str,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = validate_git_remote_url(remote_url)?;
        let arguments = network_execution_arguments(pull_arguments_for_url(options, &remote_url)?);
        self.run_network(Some(repository), &arguments, environment, "拉取远程改动")?;
        Ok(())
    }

    /// Pushes to exactly one remote.  The default mode intentionally does not
    /// enable force-push behavior.
    pub fn push(&self, repository: &Path, options: &PushOptions) -> Result<(), AppError> {
        self.push_with_environment(repository, options, &GitEnvironment::default())
    }

    /// Backend-only equivalent of [`Self::push`] with a controlled process
    /// environment.
    pub fn push_with_environment(
        &self,
        repository: &Path,
        options: &PushOptions,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_configured_remote(repository, &options.remote)?;
        let remote_urls = self.remote_urls_for_operation(repository, &options.remote, true)?;
        self.push_with_environment_at_urls(repository, options, &remote_urls, environment)
    }

    /// Pushes to every URL captured before credential resolution. Git treats
    /// multiple push URLs as mirrors; running them one at a time preserves
    /// that behavior while avoiding a mutable remote-name lookup after the
    /// selected credential environment exists.
    pub(crate) fn push_with_environment_at_urls(
        &self,
        repository: &Path,
        options: &PushOptions,
        remote_urls: &[String],
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        if remote_urls.is_empty() {
            return Err(AppError::new("remote_not_found", "找不到该 remote。"));
        }
        for refspec in &options.refspecs {
            validate_refspec(refspec)?;
        }
        let (refspecs, upstream) = self.push_refspecs_for_url(repository, options)?;
        for remote_url in remote_urls {
            let remote_url = validate_git_remote_url(remote_url)?;
            let mut arguments = vec![
                OsString::from("push"),
                OsString::from("--recurse-submodules=no"),
            ];
            if options.force_with_lease {
                arguments.push(OsString::from("--force-with-lease"));
            }
            if options.follow_tags {
                arguments.push(OsString::from("--follow-tags"));
            }
            arguments.push(OsString::from(remote_url.as_str()));
            arguments.extend(refspecs.iter().map(OsString::from));
            let arguments = network_execution_arguments(arguments);
            self.run_network(Some(repository), &arguments, environment, "推送改动")?;
        }
        if let Some(upstream) = upstream {
            self.configure_push_upstream(repository, &options.remote, &upstream, None)?;
        }
        Ok(())
    }

    /// Passing a URL in place of a remote name preserves the credential
    /// binding, but Git cannot associate that URL with `branch.*.remote`.
    /// `push.default=upstream` therefore rejects an otherwise valid push.
    /// Capture the configured upstream and express it as an explicit refspec
    /// before using the immutable URL. A `--set-upstream` request is also
    /// converted to an explicit immutable-commit refspec: the network child
    /// holds the config lock, then this process establishes the named remote's
    /// official tracking relationship only after the push has completed.
    fn push_refspecs_for_url(
        &self,
        repository: &Path,
        options: &PushOptions,
    ) -> Result<(Vec<String>, Option<PushUpstream>), AppError> {
        if options.set_upstream {
            let mut upstream = self.push_upstream_for_url(repository, options)?;
            if let Some(tracking_reference) = self.configured_tracking_reference(
                repository,
                &options.remote,
                &upstream.remote_branch,
            )? {
                validate_ref_name(&tracking_reference, "远程跟踪引用")
                    .map_err(|_| push_upstream_fetch_mapping_unavailable_error())?;
                upstream.previous_tracking_commit =
                    self.reference_commit(repository, &tracking_reference)?;
                upstream.tracking_reference = Some(tracking_reference);
            }
            let refspec = format!(
                "{}:refs/heads/{}",
                upstream.pushed_commit, upstream.remote_branch
            );
            validate_refspec(&refspec)?;
            return Ok((vec![refspec], Some(upstream)));
        }
        if !options.refspecs.is_empty() || !self.push_default_is_upstream(repository)? {
            return Ok((options.refspecs.clone(), None));
        }
        let local_branch = self.current_branch_name(repository)?;
        let upstream_branch = self
            .upstream_for_remote(repository, &options.remote)?
            .remote_reference;
        let refspec = format!("refs/heads/{local_branch}:{upstream_branch}");
        validate_refspec(&refspec)?;
        Ok((vec![refspec], None))
    }

    fn push_upstream_for_url(
        &self,
        repository: &Path,
        options: &PushOptions,
    ) -> Result<PushUpstream, AppError> {
        match options.refspecs.as_slice() {
            [] => {
                let local_branch = self.current_branch_name(repository)?;
                let pushed_commit = self.commit_for_local_branch(repository, &local_branch)?;
                Ok(PushUpstream {
                    remote_branch: local_branch.clone(),
                    local_branch,
                    pushed_commit,
                    tracking_reference: None,
                    previous_tracking_commit: None,
                    preserve_existing_upstream: false,
                })
            }
            [refspec] => {
                let (local_branch, remote_branch) = push_branches_for_refspec(refspec)?;
                let local_reference = format!("refs/heads/{local_branch}^{{commit}}");
                let commit = self.execute(
                    Some(repository),
                    &[
                        OsString::from("rev-parse"),
                        OsString::from("--verify"),
                        OsString::from("--quiet"),
                        OsString::from(local_reference),
                    ],
                    &GitEnvironment::default(),
                )?;
                if !commit.status.success() {
                    return Err(AppError::new(
                        "set_upstream_requires_local_branch",
                        "设置上游必须指定一个存在的本地分支。",
                    ));
                }
                Ok(PushUpstream {
                    local_branch,
                    remote_branch,
                    pushed_commit: parse_commit_id(
                        &commit.stdout,
                        "无法读取首次推送的本地提交标识。",
                    )?,
                    tracking_reference: None,
                    previous_tracking_commit: None,
                    preserve_existing_upstream: false,
                })
            }
            _ => Err(AppError::new(
                "set_upstream_requires_single_branch",
                "设置上游只能推送当前分支到一个远程分支。",
            )),
        }
    }

    /// Finalizes the local tracking state after a publish push was performed
    /// from an isolated clone. The caller must first prove that the named
    /// remote branch resolves to `published_commit`; this method independently
    /// verifies that the original repository still has that exact commit
    /// checked out before recording the remote-tracking ref and upstream.
    #[cfg(test)]
    pub(crate) fn configure_published_branch_upstream(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
        published_commit: &str,
        remote_state: &PublishedRemoteState,
    ) -> Result<(), AppError> {
        self.configure_published_branch_upstream_and_then(
            repository,
            remote_name,
            branch,
            published_commit,
            remote_state,
            || Ok(()),
        )
    }

    pub(crate) fn configure_published_branch_upstream_and_then<T>(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
        published_commit: &str,
        remote_state: &PublishedRemoteState,
        after_configuration: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(branch, "发布分支名")?;
        parse_commit_id(published_commit.as_bytes(), "无法读取已发布提交标识。")?;
        self.ensure_network_configuration_is_safe(repository)?;
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_files_reference_storage(repository)?;
        let local_reference = format!("refs/heads/{branch}");
        let branch_lock = BranchReferenceLock::acquire(self, repository, &local_reference)?;
        let result = (|| {
            self.ensure_direct_branch_reference(repository, &local_reference)?;
            if self.commit_for_local_branch(repository, branch)? != published_commit {
                return Err(push_tracking_setup_incomplete_error());
            }
            self.ensure_published_upstream_is_compatible(repository, remote_name, branch)?;

            let tracking_reference = self
                .configured_tracking_reference(repository, remote_name, branch)?
                .ok_or_else(push_upstream_fetch_mapping_unavailable_error)?;
            let previous_tracking_commit =
                self.reference_commit(repository, &tracking_reference)?;
            self.configure_push_upstream(
                repository,
                remote_name,
                &PushUpstream {
                    local_branch: branch.to_owned(),
                    remote_branch: branch.to_owned(),
                    pushed_commit: published_commit.to_owned(),
                    tracking_reference: Some(tracking_reference.clone()),
                    previous_tracking_commit,
                    preserve_existing_upstream: true,
                },
                Some(remote_state),
            )?;
            let tracking_lock =
                BranchReferenceLock::acquire(self, repository, &tracking_reference)?;
            branch_lock.ensure_attached()?;
            tracking_lock.ensure_attached()?;
            if self.commit_for_local_branch(repository, branch)? != published_commit {
                return Err(push_tracking_setup_incomplete_error());
            }
            self.verify_published_branch_upstream(
                repository,
                remote_name,
                branch,
                published_commit,
            )?;
            let callback_result = after_configuration();
            match (callback_result, tracking_lock.release()) {
                (Ok(value), Ok(())) => Ok(value),
                (Ok(_), Err(_)) => Err(push_finalized_cleanup_failed_error()),
                (Err(_), Err(_)) => Err(push_finalization_cleanup_blocked_error()),
                (Err(error), Ok(())) => Err(error),
            }
        })();
        match result {
            Ok(value) => match branch_lock.release() {
                Ok(()) => Ok(value),
                Err(_) => Err(push_finalized_cleanup_failed_error()),
            },
            Err(error) => match branch_lock.release() {
                Ok(()) => Err(error),
                Err(_) if error.code() == "push_finalized_cleanup_failed" => {
                    Err(push_finalized_cleanup_failed_error())
                }
                // A callback failure means the durable pending journal may
                // still require reconciliation. Preserve that primary error
                // even if releasing the local branch lock also fails; the
                // do-not-retry diagnostic is only valid after the callback
                // itself completed.
                Err(_) => Err(error),
            },
        }
    }

    fn ensure_direct_branch_reference(
        &self,
        repository: &Path,
        reference: &str,
    ) -> Result<(), AppError> {
        let output = self.execute(
            Some(repository),
            &[
                OsString::from("symbolic-ref"),
                OsString::from("--quiet"),
                OsString::from(reference),
            ],
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(());
        }
        Err(push_tracking_setup_incomplete_error())
    }

    pub(crate) fn verify_published_branch_upstream(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
        published_commit: &str,
    ) -> Result<(), AppError> {
        let tracking_reference = self
            .configured_tracking_reference(repository, remote_name, branch)?
            .ok_or_else(push_tracking_setup_incomplete_error)?;
        self.verify_push_upstream(
            repository,
            remote_name,
            &PushUpstream {
                local_branch: branch.to_owned(),
                remote_branch: branch.to_owned(),
                pushed_commit: published_commit.to_owned(),
                tracking_reference: Some(tracking_reference.clone()),
                previous_tracking_commit: None,
                preserve_existing_upstream: true,
            },
            &tracking_reference,
        )
    }

    fn ensure_files_reference_storage(&self, repository: &Path) -> Result<(), AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--get", "extensions.refStorage"]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1)
            || (output.status.success()
                && output_text(&output.stdout)
                    .trim()
                    .eq_ignore_ascii_case("files"))
        {
            return Ok(());
        }
        Err(push_tracking_setup_incomplete_error())
    }

    fn ensure_tracking_reference_is_not_symbolic(
        &self,
        repository: &Path,
        reference: &str,
    ) -> Result<(), AppError> {
        let output = self.execute(
            Some(repository),
            &[
                OsString::from("symbolic-ref"),
                OsString::from("--quiet"),
                OsString::from(reference),
            ],
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(());
        }
        Err(push_tracking_setup_incomplete_error())
    }

    fn ensure_published_upstream_is_compatible(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
    ) -> Result<(), AppError> {
        let expected_remote_reference = format!("refs/heads/{branch}");
        ensure_branch_config_values_are_compatible(
            self,
            &self.network_configuration_paths(repository)?,
            branch,
            remote_name,
            &expected_remote_reference,
        )
        .map(|_| ())
    }

    fn configure_push_upstream(
        &self,
        repository: &Path,
        remote_name: &str,
        upstream: &PushUpstream,
        published_remote_state: Option<&PublishedRemoteState>,
    ) -> Result<(), AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(&upstream.local_branch, "本地分支名")?;
        validate_ref_name(&upstream.remote_branch, "远程分支名")?;
        let created_tracking_reference;
        let tracking_reference = if let Some(reference) = upstream.tracking_reference.as_deref() {
            reference
        } else if upstream.preserve_existing_upstream {
            return Err(push_upstream_fetch_mapping_unavailable_error());
        } else {
            created_tracking_reference = self.ensure_push_remote_tracking_mapping(
                repository,
                remote_name,
                &upstream.remote_branch,
            )?;
            created_tracking_reference.as_str()
        };
        validate_ref_name(tracking_reference, "远程跟踪引用")
            .map_err(|_| push_tracking_setup_incomplete_error())?;

        if upstream.preserve_existing_upstream {
            let mut transaction = BranchUpstreamConfigTransaction::acquire(self, repository)?;
            let expected_remote_reference = format!("refs/heads/{}", upstream.remote_branch);
            let mut tracking_changed = false;
            let mut upstream_needs_configuration = true;
            let preparation = (|| {
                self.ensure_files_reference_storage(repository)?;
                self.ensure_network_configuration_is_safe(repository)?;
                transaction.prepare_staging_config()?;
                let remote_state =
                    published_remote_state.ok_or_else(push_tracking_setup_incomplete_error)?;
                let current_fetch_urls =
                    self.remote_urls_for_operation(repository, remote_name, false)?;
                let current_push_urls =
                    self.remote_urls_for_operation(repository, remote_name, true)?;
                let current_fetch_url =
                    self.remote_url_for_operation(repository, remote_name, false)?;
                if current_fetch_url != remote_state.verified_fetch_url
                    || current_fetch_urls != remote_state.fetch_urls
                    || current_push_urls != remote_state.push_urls
                {
                    return Err(push_tracking_setup_incomplete_error());
                }
                if self
                    .configured_tracking_reference(
                        repository,
                        remote_name,
                        &upstream.remote_branch,
                    )?
                    .as_deref()
                    != Some(tracking_reference)
                {
                    return Err(push_tracking_setup_incomplete_error());
                }
                upstream_needs_configuration = !transaction.ensure_upstream_is_compatible(
                    self,
                    &upstream.local_branch,
                    remote_name,
                    &expected_remote_reference,
                )?;
                let prepared_tracking = PreparedTrackingReferenceUpdate::prepare(
                    self,
                    repository,
                    tracking_reference,
                    &upstream.pushed_commit,
                    upstream.previous_tracking_commit.as_deref(),
                )?;
                if let Err(error) =
                    self.ensure_tracking_reference_is_not_symbolic(repository, tracking_reference)
                {
                    let abort_error = prepared_tracking.abort().err();
                    return Err(abort_error.unwrap_or(error));
                }
                let commit_would_change_tracking = prepared_tracking.changed();
                match prepared_tracking.commit() {
                    Ok(changed) => tracking_changed = changed,
                    Err(error) => {
                        // The child may have committed before a timeout or a
                        // lost response. Read the ref back before choosing a
                        // compensating CAS; if it is still unchanged, preserve
                        // the primary process error without a bogus rollback.
                        tracking_changed = commit_would_change_tracking
                            && self
                                .reference_commit(repository, tracking_reference)
                                .ok()
                                .flatten()
                                .as_deref()
                                == Some(upstream.pushed_commit.as_str());
                        return Err(error);
                    }
                }
                if upstream_needs_configuration {
                    transaction.set_upstream(
                        self,
                        &upstream.local_branch,
                        remote_name,
                        &expected_remote_reference,
                    )?;
                }
                Ok(())
            })();
            if let Err(error) = preparation {
                let rollback_error = self
                    .rollback_tracking_reference_change(
                        repository,
                        tracking_reference,
                        upstream,
                        tracking_changed,
                    )
                    .err();
                let cleanup_error = transaction.abort().err();
                if cleanup_error.is_some() {
                    return Err(push_finalization_cleanup_blocked_error());
                }
                return Err(rollback_error.unwrap_or(error));
            }
            if !upstream_needs_configuration {
                let cleanup_error = transaction.abort().err();
                let verification = self.verify_push_upstream(
                    repository,
                    remote_name,
                    upstream,
                    tracking_reference,
                );
                return match cleanup_error {
                    Some(_) => Err(push_finalization_cleanup_blocked_error()),
                    None => verification,
                };
            }
            match transaction.commit() {
                Ok(()) => {
                    return self.verify_push_upstream(
                        repository,
                        remote_name,
                        upstream,
                        tracking_reference,
                    )
                }
                Err(BranchConfigCommitError::Before(error)) => {
                    self.rollback_tracking_reference_change(
                        repository,
                        tracking_reference,
                        upstream,
                        tracking_changed,
                    )?;
                    return Err(error);
                }
                Err(BranchConfigCommitError::BeforeCleanup) => {
                    let _ = self.rollback_tracking_reference_change(
                        repository,
                        tracking_reference,
                        upstream,
                        tracking_changed,
                    );
                    return Err(push_finalization_cleanup_blocked_error());
                }
                Err(BranchConfigCommitError::AfterDurability(_error)) => {
                    // The staged config has already replaced the primary
                    // file. As with profile persistence, a later directory
                    // sync warning is classified from the effective state.
                    return self.verify_push_upstream(
                        repository,
                        remote_name,
                        upstream,
                        tracking_reference,
                    );
                }
                Err(BranchConfigCommitError::AfterCleanup(_error)) => {
                    return Err(push_finalization_cleanup_blocked_error())
                }
            }
        } else {
            self.update_tracking_reference_after_push(
                repository,
                tracking_reference,
                &upstream.pushed_commit,
                upstream.previous_tracking_commit.as_deref(),
            )?;

            let set_upstream_argument = format!("--set-upstream-to={tracking_reference}");
            self.run(
                Some(repository),
                &[
                    OsString::from("branch"),
                    OsString::from(set_upstream_argument),
                    OsString::from(&upstream.local_branch),
                ],
                &GitEnvironment::default(),
                "设置当前分支上游",
            )?;
        }
        self.verify_push_upstream(repository, remote_name, upstream, tracking_reference)
    }

    fn verify_push_upstream(
        &self,
        repository: &Path,
        remote_name: &str,
        upstream: &PushUpstream,
        tracking_reference: &str,
    ) -> Result<(), AppError> {
        let tracking_commit = self
            .reference_commit(repository, tracking_reference)
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        let configured = self
            .branch_upstream(repository, &upstream.local_branch)
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        let expected_remote_reference = format!("refs/heads/{}", upstream.remote_branch);
        if tracking_commit.as_deref() != Some(upstream.pushed_commit.as_str())
            || !configured.is_some_and(|configured| {
                configured.remote == remote_name
                    && configured.remote_reference == expected_remote_reference
                    && configured.tracking_reference.as_deref() == Some(tracking_reference)
            })
        {
            return Err(push_tracking_setup_incomplete_error());
        }
        Ok(())
    }

    fn rollback_tracking_reference_change(
        &self,
        repository: &Path,
        tracking_reference: &str,
        upstream: &PushUpstream,
        tracking_changed: bool,
    ) -> Result<(), AppError> {
        if tracking_changed {
            self.restore_tracking_reference_after_failed_upstream(
                repository,
                tracking_reference,
                &upstream.pushed_commit,
                upstream.previous_tracking_commit.as_deref(),
            )?;
        }
        Ok(())
    }

    pub(super) fn update_tracking_reference_after_push(
        &self,
        repository: &Path,
        tracking_reference: &str,
        pushed_commit: &str,
        previous_tracking_commit: Option<&str>,
    ) -> Result<bool, AppError> {
        validate_ref_name(tracking_reference, "远程跟踪引用")
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        parse_commit_id(pushed_commit.as_bytes(), "无法读取首次推送的本地提交标识。")?;
        let expected = previous_tracking_commit.unwrap_or("");
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "update-ref",
                "--no-deref",
                "--create-reflog",
                "-m",
                "update by push",
                tracking_reference,
                pushed_commit,
                expected,
            ]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            return Ok(previous_tracking_commit != Some(pushed_commit));
        }
        let verification = self.execute(
            Some(repository),
            &os_arguments(&[
                "update-ref",
                "--no-deref",
                tracking_reference,
                pushed_commit,
                pushed_commit,
            ]),
            &GitEnvironment::default(),
        )?;
        if verification.status.success() {
            Ok(false)
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn restore_tracking_reference_after_failed_upstream(
        &self,
        repository: &Path,
        tracking_reference: &str,
        pushed_commit: &str,
        previous_tracking_commit: Option<&str>,
    ) -> Result<(), AppError> {
        let mut arguments = os_arguments(&["update-ref", "--no-deref"]);
        if let Some(previous) = previous_tracking_commit {
            arguments.extend(os_arguments(&[tracking_reference, previous, pushed_commit]));
        } else {
            arguments.extend(os_arguments(&["-d", tracking_reference, pushed_commit]));
        }
        let output = self.execute(Some(repository), &arguments, &GitEnvironment::default())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn commit_for_local_branch(&self, repository: &Path, branch: &str) -> Result<String, AppError> {
        validate_ref_name(branch, "本地分支名")?;
        let revision = format!("refs/heads/{branch}^{{commit}}");
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", &revision]),
            &GitEnvironment::default(),
            "读取首次推送的本地提交",
        )?;
        parse_commit_id(&output.stdout, "无法读取首次推送的本地提交标识。")
    }

    fn reference_commit(
        &self,
        repository: &Path,
        reference: &str,
    ) -> Result<Option<String>, AppError> {
        validate_refspec(reference)?;
        let revision = format!("{reference}^{{commit}}");
        let output = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", &revision]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(git_command_error("读取远程跟踪分支", &output));
        }
        parse_commit_id(&output.stdout, "无法读取远程跟踪分支提交标识。").map(Some)
    }

    /// Ensure Git can reverse-map the pushed remote branch to a local
    /// remote-tracking ref. `git remote set-branches --add` is the porcelain
    /// operation Git provides for extending a restrictive single-branch
    /// clone without replacing the user's existing tracked branch set.
    fn ensure_push_remote_tracking_mapping(
        &self,
        repository: &Path,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<String, AppError> {
        if let Some(reference) =
            self.configured_tracking_reference(repository, remote_name, remote_branch)?
        {
            return Ok(reference);
        }
        self.run(
            Some(repository),
            &[
                OsString::from("remote"),
                OsString::from("set-branches"),
                OsString::from("--add"),
                OsString::from(remote_name),
                OsString::from(remote_branch),
            ],
            &GitEnvironment::default(),
            "扩展远程分支跟踪范围",
        )?;
        self.configured_tracking_reference(repository, remote_name, remote_branch)?
            .ok_or_else(push_upstream_fetch_mapping_unavailable_error)
    }

    pub(super) fn configured_tracking_reference(
        &self,
        repository: &Path,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<Option<String>, AppError> {
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let source_reference = format!("refs/heads/{remote_branch}");
        let mut tracking_reference = None;
        let mut excluded = false;
        for configured_refspec in self
            .repository_config_values(repository, &config_key)?
            .iter()
            .map(|refspec| refspec.trim())
        {
            if configured_refspec.is_empty() {
                continue;
            }
            validate_refspec(configured_refspec)?;
            if let Some(exclusion) = configured_refspec.strip_prefix('^') {
                excluded |= refspec_source_capture(exclusion, &source_reference).is_some();
                continue;
            }
            let configured_refspec = configured_refspec
                .strip_prefix('+')
                .unwrap_or(configured_refspec);
            let Some((source_pattern, destination_pattern)) = configured_refspec.split_once(':')
            else {
                continue;
            };
            let Some(capture) = refspec_source_capture(source_pattern, &source_reference) else {
                continue;
            };
            let Some(destination) =
                expand_refspec_destination(destination_pattern, capture.as_deref())
            else {
                continue;
            };
            validate_ref_name(&destination, "远程跟踪引用")
                .map_err(|_| push_upstream_fetch_mapping_unavailable_error())?;
            let expected_prefix = format!("refs/remotes/{remote_name}/");
            if !destination.starts_with(&expected_prefix) {
                return Err(push_upstream_fetch_mapping_unavailable_error());
            }
            match tracking_reference.as_deref() {
                Some(existing) if existing != destination => {
                    return Err(push_upstream_fetch_mapping_unavailable_error());
                }
                Some(_) => {}
                None => tracking_reference = Some(destination),
            }
        }
        if excluded {
            return Err(push_upstream_fetch_mapping_unavailable_error());
        }
        Ok(tracking_reference)
    }

    fn push_default_is_upstream(&self, repository: &Path) -> Result<bool, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--get", "push.default"]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        if !output.status.success() {
            return Err(git_command_error("读取推送默认规则", &output));
        }
        Ok(output_text(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("upstream"))
    }

    /// Git accepts a URL in the positional repository slot for fetch, pull,
    /// and push. The desktop API accepts a remote *name* only, because its
    /// credential resolution is bound to one persisted remote configuration.
    fn ensure_configured_remote(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        if self
            .remotes(repository)?
            .iter()
            .any(|remote| remote.name == name)
        {
            // The name came from a configured remote, but it will later be
            // interpolated into refspecs. Reject names that Git can store yet
            // cannot safely use in that generated namespace.
            validate_remote_name(name)?;
            Ok(())
        } else {
            Err(AppError::new("remote_not_found", "找不到该 remote。"))
        }
    }
}

fn pull_upstream_fetch_mapping_unavailable_error() -> AppError {
    AppError::new(
        "pull_upstream_fetch_mapping_unavailable",
        "当前分支上游没有可用的远程跟踪引用映射。",
    )
}

fn push_upstream_fetch_mapping_unavailable_error() -> AppError {
    AppError::new(
        "push_upstream_fetch_mapping_unavailable",
        "无法为首次推送的远程分支建立唯一的远程跟踪引用映射。",
    )
}

fn push_tracking_setup_incomplete_error() -> AppError {
    AppError::new(
        "push_tracking_setup_incomplete",
        "远程推送已成功，但本地远程跟踪引用同时发生变化，上游设置未能完成。",
    )
}

fn push_finalized_cleanup_failed_error() -> AppError {
    AppError::with_safe_details(
        "push_finalized_cleanup_failed",
        "发布与本地绑定已完成，但相关 Git 锁未能清理。",
        "请勿直接重试发布；关闭应用并检查 Git 锁文件后再继续操作。",
    )
}

fn push_finalization_cleanup_blocked_error() -> AppError {
    AppError::with_safe_details(
        "push_finalization_cleanup_blocked",
        "发布分支的本地跟踪状态已写入，但账户绑定尚未完成。",
        "请关闭应用并检查残留的 Git 配置锁；清理后重新发布会继续恢复，不会重复创建远程仓库。",
    )
}

fn parse_commit_id(output: &[u8], message: &'static str) -> Result<String, AppError> {
    let output = output_text(output);
    let commit_id = output.trim();
    if !(40..=64).contains(&commit_id.len())
        || !commit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::new("git_parse_error", message));
    }
    Ok(commit_id.to_owned())
}

pub(super) fn parse_repository_config_values(output: &[u8]) -> Result<Vec<String>, AppError> {
    let mut fields = output.split(|byte| *byte == b'\0').collect::<Vec<_>>();
    if fields.last() == Some(&&[][..]) {
        fields.pop();
    }
    if fields.len() % 2 != 0 {
        return Err(AppError::new(
            "git_parse_error",
            "无法读取远程分支获取规则。",
        ));
    }

    fields
        .chunks_exact(2)
        .filter(|fields| fields[0] == b"local" || fields[0] == b"worktree")
        .map(|fields| {
            std::str::from_utf8(fields[1])
                .map(str::to_owned)
                .map_err(|_| AppError::new("git_parse_error", "无法读取远程分支获取规则。"))
        })
        .collect()
}

fn refspec_source_capture(pattern: &str, reference: &str) -> Option<Option<String>> {
    let Some(wildcard_index) = pattern.find('*') else {
        return (pattern == reference).then_some(None);
    };
    if pattern[wildcard_index + 1..].contains('*') {
        return None;
    }
    let prefix = &pattern[..wildcard_index];
    let suffix = &pattern[wildcard_index + 1..];
    if !reference.starts_with(prefix)
        || !reference.ends_with(suffix)
        || reference.len() < prefix.len() + suffix.len()
    {
        return None;
    }
    Some(Some(
        reference[prefix.len()..reference.len() - suffix.len()].to_owned(),
    ))
}

fn expand_refspec_destination(pattern: &str, capture: Option<&str>) -> Option<String> {
    match (pattern.find('*'), capture) {
        (None, None) => Some(pattern.to_owned()),
        (Some(wildcard_index), Some(capture)) if !pattern[wildcard_index + 1..].contains('*') => {
            Some(format!(
                "{}{capture}{}",
                &pattern[..wildcard_index],
                &pattern[wildcard_index + 1..]
            ))
        }
        _ => None,
    }
}

fn push_branches_for_refspec(refspec: &str) -> Result<(String, String), AppError> {
    let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
    let (source, destination) = refspec
        .split_once(':')
        .map_or((refspec, None), |(source, destination)| {
            (source, Some(destination))
        });
    let local_branch = source.strip_prefix("refs/heads/").unwrap_or(source);
    if source == "HEAD"
        || source.is_empty()
        || source.starts_with("refs/") && !source.starts_with("refs/heads/")
    {
        return Err(AppError::new(
            "set_upstream_requires_current_branch",
            "设置上游必须明确指定一个本地分支。",
        ));
    }
    validate_ref_name(local_branch, "本地分支名")?;
    let destination = destination.unwrap_or(local_branch);
    if destination.starts_with("refs/") && !destination.starts_with("refs/heads/") {
        return Err(AppError::new(
            "set_upstream_requires_remote_branch",
            "设置上游必须指定一个远程分支。",
        ));
    }
    let remote_branch = destination
        .strip_prefix("refs/heads/")
        .unwrap_or(destination);
    validate_ref_name(remote_branch, "远程分支名")?;
    Ok((local_branch.to_owned(), remote_branch.to_owned()))
}

/// Command-scope overrides complement the preflight: a repository config may
/// still change after it has been inspected. The remote URL itself is passed
/// positionally, while these transport and hook settings prevent that raced
/// config from selecting a proxy, SSH command, protocol, or hook helper.
fn network_execution_arguments(mut arguments: Vec<OsString>) -> Vec<OsString> {
    let mut protected = Vec::with_capacity(arguments.len() + 30);
    for (key, value) in [
        ("http.extraheader", ""),
        ("http.proxy", ""),
        ("http.sslverify", "true"),
        ("core.hooksPath", DISABLED_GIT_HOOKS_PATH),
        ("core.gitProxy", ""),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
        ("protocol.http.allow", "always"),
        ("protocol.https.allow", "always"),
        ("protocol.ssh.allow", "always"),
        ("protocol.git.allow", "always"),
        ("protocol.file.allow", "always"),
        ("protocol.ext.allow", "never"),
    ] {
        protected.push(OsString::from("-c"));
        protected.push(OsString::from(format!("{key}={value}")));
    }
    protected.append(&mut arguments);
    protected
}

pub(super) fn fetch_arguments_for_url(
    options: &FetchOptions,
    remote_url: &str,
) -> Result<Vec<OsString>, AppError> {
    validate_remote_name(&options.remote)?;
    let remote_url = validate_git_remote_url(remote_url)?;
    for refspec in &options.refspecs {
        validate_refspec(refspec)?;
    }
    let mut arguments = vec![
        OsString::from("fetch"),
        OsString::from("--no-recurse-submodules"),
    ];
    if options.prune {
        arguments.push(OsString::from("--prune"));
    }
    arguments.push(OsString::from(remote_url));
    if options.refspecs.is_empty() {
        return Err(AppError::new(
            "fetch_refspec_required",
            "获取远程更新需要已确认的引用规格。",
        ));
    }
    arguments.extend(options.refspecs.iter().map(OsString::from));
    Ok(arguments)
}

fn remote_fetch_refspec_config_key(remote_name: &str) -> Result<String, AppError> {
    validate_remote_name(remote_name)?;
    Ok(format!("remote.{remote_name}.fetch"))
}

fn is_remote_branch_fetch_refspec(refspec: &str) -> bool {
    let source = refspec.strip_prefix('+').unwrap_or(refspec);
    source.starts_with("refs/heads/") || source.starts_with("^refs/heads/")
}

pub(super) fn pull_arguments_for_url(
    options: &PullOptions,
    remote_url: &str,
) -> Result<Vec<OsString>, AppError> {
    validate_remote_name(&options.remote)?;
    let remote_url = validate_git_remote_url(remote_url)?;
    let refspec = options
        .refspec
        .as_deref()
        .ok_or_else(pull_upstream_not_configured_error)?;
    validate_refspec(refspec)?;
    Ok(os_arguments(&[
        "pull",
        "--no-edit",
        "--no-recurse-submodules",
        &remote_url,
        refspec,
    ]))
}

fn pull_upstream_not_configured_error() -> AppError {
    AppError::new(
        "pull_upstream_not_configured",
        "当前分支未配置可拉取的上游。",
    )
}
