use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_EXTERNAL_EDITOR_REAPERS: usize = 4;
static ACTIVE_EXTERNAL_EDITOR_REAPERS: AtomicUsize = AtomicUsize::new(0);

/// A filesystem identity captured with an untracked-file cleanup preview.
/// It stays in memory only; it is neither exposed through IPC nor persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanUntrackedPathFingerprint {
    pub(crate) path: String,
    identity: CleanUntrackedPathIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanUntrackedPathIdentity {
    kind: CleanUntrackedPathKind,
    length: u64,
    modified_at: Option<std::time::SystemTime>,
    created_at: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at: (i64, i64),
    #[cfg(windows)]
    file_attributes: u32,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    last_write_time: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanUntrackedPathKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// An ancestor directory captured immediately before cleanup starts. It is
/// used only on Linux to remove an empty directory after its reviewed leaves
/// have been deleted, without deleting a directory another process created
/// at the same pathname meanwhile.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct CleanUntrackedDirectoryFingerprint {
    path: String,
    device: u64,
    inode: u64,
}

fn clean_untracked_confirmation_changed_error() -> AppError {
    AppError::new(
        "clean_untracked_confirmation_changed",
        "未跟踪文件已变化，请重新预览后再清理。",
    )
}

fn clean_untracked_path_fingerprint(
    repository: &Path,
    relative_path: &str,
) -> Result<CleanUntrackedPathFingerprint, AppError> {
    let metadata = fs::symlink_metadata(repository.join(relative_path))
        .map_err(|_| clean_untracked_confirmation_changed_error())?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_file() {
        CleanUntrackedPathKind::File
    } else if file_type.is_dir() {
        CleanUntrackedPathKind::Directory
    } else if file_type.is_symlink() {
        CleanUntrackedPathKind::Symlink
    } else {
        CleanUntrackedPathKind::Other
    };
    #[cfg(unix)]
    let unix_identity = {
        use std::os::unix::fs::MetadataExt;

        (
            metadata.dev(),
            metadata.ino(),
            (metadata.ctime(), metadata.ctime_nsec()),
        )
    };
    #[cfg(windows)]
    let windows_identity = {
        use std::os::windows::fs::MetadataExt;

        (
            metadata.file_attributes(),
            metadata.creation_time(),
            metadata.last_write_time(),
        )
    };
    Ok(CleanUntrackedPathFingerprint {
        path: relative_path.to_owned(),
        identity: CleanUntrackedPathIdentity {
            kind,
            length: metadata.len(),
            modified_at: metadata.modified().ok(),
            created_at: metadata.created().ok(),
            #[cfg(unix)]
            device: unix_identity.0,
            #[cfg(unix)]
            inode: unix_identity.1,
            #[cfg(unix)]
            changed_at: unix_identity.2,
            #[cfg(windows)]
            file_attributes: windows_identity.0,
            #[cfg(windows)]
            creation_time: windows_identity.1,
            #[cfg(windows)]
            last_write_time: windows_identity.2,
        },
    })
}

#[cfg(target_os = "linux")]
struct CleanUntrackedQuarantine {
    directory: PathBuf,
    directory_file: fs::File,
    preserve_directory: bool,
}

#[cfg(target_os = "linux")]
impl CleanUntrackedQuarantine {
    /// The quarantine must share a filesystem with the worktree entries: a
    /// descriptor-relative rename cannot cross from a separately located
    /// `.git` directory onto the worktree filesystem.
    fn create(worktree: &Path) -> Result<Self, AppError> {
        for _ in 0..16 {
            let directory = worktree.join(format!(".bkm-clean-quarantine-{}", Uuid::new_v4()));
            match create_private_clean_untracked_directory(&directory) {
                Ok(()) => {
                    let directory_file = open_directory_no_follow(&directory)?;
                    return Ok(Self {
                        directory,
                        directory_file,
                        preserve_directory: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(AppError::new(
                        "clean_untracked_failed",
                        "无法安全清理未跟踪文件。",
                    ));
                }
            }
        }
        Err(AppError::new(
            "clean_untracked_failed",
            "无法安全清理未跟踪文件。",
        ))
    }

    fn retain(&mut self) {
        self.preserve_directory = true;
    }
}

#[cfg(target_os = "linux")]
fn create_private_clean_untracked_directory(path: &Path) -> std::io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: the path is NUL-terminated and remains live for this call.
    if unsafe { libc::mkdir(path.as_ptr(), 0o700) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
impl Drop for CleanUntrackedQuarantine {
    fn drop(&mut self) {
        if !self.preserve_directory {
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

#[cfg(target_os = "linux")]
type QuarantinedCleanUntrackedPath = (fs::File, CString, CString, CleanUntrackedPathKind);

/// Restores every moved entry in reverse order when the verification phase
/// cannot complete. A replacement at the original path is never overwritten;
/// any entry that cannot be restored stays in the private quarantine instead
/// of being silently deleted.
#[cfg(target_os = "linux")]
fn restore_quarantined_clean_untracked_paths(
    quarantine: &mut CleanUntrackedQuarantine,
    entries: &[QuarantinedCleanUntrackedPath],
) {
    let mut restoration_failed = false;
    for (parent, leaf, temporary_name, _) in entries.iter().rev() {
        if rename_at_no_replace(
            quarantine.directory_file.as_raw_fd(),
            temporary_name,
            parent.as_raw_fd(),
            leaf,
        )
        .is_err()
        {
            restoration_failed = true;
        }
    }
    if restoration_failed {
        quarantine.retain();
    }
}

#[cfg(target_os = "linux")]
fn clean_untracked_filesystem_error() -> AppError {
    AppError::new("clean_untracked_failed", "无法安全清理未跟踪文件。")
}

#[cfg(target_os = "linux")]
fn c_string_from_os_str(value: &std::ffi::OsStr) -> Result<CString, AppError> {
    CString::new(value.as_bytes()).map_err(|_| clean_untracked_confirmation_changed_error())
}

#[cfg(target_os = "linux")]
fn open_directory_no_follow(path: &Path) -> Result<fs::File, AppError> {
    let path = c_string_from_os_str(path.as_os_str())?;
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(clean_untracked_filesystem_error());
    }
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[cfg(target_os = "linux")]
fn open_clean_untracked_parent(
    repository: &Path,
    relative_path: &str,
) -> Result<(fs::File, CString), AppError> {
    validate_worktree_relative_path(relative_path)?;
    let mut components = Path::new(relative_path).components();
    let leaf = match components.next_back() {
        Some(std::path::Component::Normal(component)) => c_string_from_os_str(component)?,
        _ => return Err(clean_untracked_confirmation_changed_error()),
    };
    let mut parent = open_directory_no_follow(repository)?;
    for component in components {
        let std::path::Component::Normal(component) = component else {
            return Err(clean_untracked_confirmation_changed_error());
        };
        let component = c_string_from_os_str(component)?;
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(clean_untracked_confirmation_changed_error());
        }
        parent = unsafe { fs::File::from_raw_fd(descriptor) };
    }
    Ok((parent, leaf))
}

#[cfg(target_os = "linux")]
fn rename_at_no_replace(
    old_directory: std::os::fd::RawFd,
    old_name: &CStr,
    new_directory: std::os::fd::RawFd,
    new_name: &CStr,
) -> std::io::Result<()> {
    // Linux's `renameat2(..., RENAME_NOREPLACE)` atomically moves the exact
    // directory entry only when the destination is vacant. It lets a mismatch
    // be restored without clobbering a concurrently created source pathname.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            old_directory,
            old_name.as_ptr(),
            new_directory,
            new_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn clean_untracked_fingerprint_matches_at(
    directory: std::os::fd::RawFd,
    name: &CStr,
    fingerprint: &CleanUntrackedPathFingerprint,
) -> Result<bool, AppError> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory,
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(clean_untracked_confirmation_changed_error());
    }
    let metadata = unsafe { metadata.assume_init() };
    let kind = match metadata.st_mode & libc::S_IFMT {
        libc::S_IFREG => CleanUntrackedPathKind::File,
        libc::S_IFDIR => CleanUntrackedPathKind::Directory,
        libc::S_IFLNK => CleanUntrackedPathKind::Symlink,
        _ => CleanUntrackedPathKind::Other,
    };
    Ok(fingerprint.identity.kind == kind
        && fingerprint.identity.length == metadata.st_size.max(0) as u64
        && fingerprint.identity.device == metadata.st_dev
        && fingerprint.identity.inode == metadata.st_ino
        // Moving a directory entry into quarantine updates the inode's ctime,
        // so compare the stable identity plus mtime captured before the move.
        && fingerprint.identity.modified_at
            == unix_system_time(metadata.st_mtime, metadata.st_mtime_nsec))
}

#[cfg(target_os = "linux")]
fn clean_untracked_directory_fingerprint_at(
    directory: std::os::fd::RawFd,
    name: &CStr,
    path: String,
) -> Result<Option<CleanUntrackedDirectoryFingerprint>, AppError> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory,
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(clean_untracked_confirmation_changed_error());
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Ok(None);
    }
    Ok(Some(CleanUntrackedDirectoryFingerprint {
        path,
        device: metadata.st_dev,
        inode: metadata.st_ino,
    }))
}

#[cfg(target_os = "linux")]
fn clean_untracked_parent_directory_fingerprints(
    repository: &Path,
    fingerprints: &[CleanUntrackedPathFingerprint],
) -> Result<Vec<CleanUntrackedDirectoryFingerprint>, AppError> {
    let mut paths = std::collections::BTreeSet::new();
    for fingerprint in fingerprints {
        let mut parent = Path::new(&fingerprint.path).parent().map(Path::to_path_buf);
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            let path = path
                .to_str()
                .ok_or_else(clean_untracked_confirmation_changed_error)?;
            validate_worktree_relative_path(path)?;
            paths.insert(path.to_owned());
            parent = Path::new(path).parent().map(Path::to_path_buf);
        }
    }

    let mut directories = Vec::new();
    for path in paths {
        let (parent, leaf) = open_clean_untracked_parent(repository, &path)?;
        if let Some(fingerprint) =
            clean_untracked_directory_fingerprint_at(parent.as_raw_fd(), &leaf, path)?
        {
            directories.push(fingerprint);
        }
    }
    // A child must be removed before an empty parent can become removable.
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.path.matches('/').count()));
    Ok(directories)
}

#[cfg(target_os = "linux")]
fn remove_empty_clean_untracked_parent_directories(
    repository: &Path,
    directories: &[CleanUntrackedDirectoryFingerprint],
) {
    for expected in directories {
        let Ok((parent, leaf)) = open_clean_untracked_parent(repository, &expected.path) else {
            continue;
        };
        let Ok(Some(current)) = clean_untracked_directory_fingerprint_at(
            parent.as_raw_fd(),
            &leaf,
            expected.path.clone(),
        ) else {
            continue;
        };
        if current.device != expected.device || current.inode != expected.inode {
            continue;
        }
        // The descriptor pins the parent and the identity check pins the
        // child. `AT_REMOVEDIR` succeeds only for an empty directory, so a
        // concurrently created file is never removed as collateral cleanup.
        let _ = unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) };
    }
}

#[cfg(target_os = "linux")]
fn unix_system_time(seconds: i64, nanoseconds: i64) -> Option<SystemTime> {
    let nanoseconds = u32::try_from(nanoseconds).ok()?;
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanoseconds))
    } else {
        UNIX_EPOCH.checked_sub(Duration::new(seconds.unsigned_abs(), nanoseconds))
    }
}

/// A bounded slot retained until a launched GUI editor has exited. A user can
/// keep an editor open indefinitely, so detached waiters must not grow
/// without limit when repeated IPC calls arrive.
struct ExternalEditorReaperPermit;

impl ExternalEditorReaperPermit {
    fn acquire() -> Result<Self, AppError> {
        let mut active = ACTIVE_EXTERNAL_EDITOR_REAPERS.load(Ordering::Acquire);
        loop {
            if active >= MAX_EXTERNAL_EDITOR_REAPERS {
                return Err(AppError::new(
                    "external_editor_busy",
                    "已有外部编辑器仍在运行，请关闭后再试。",
                ));
            }
            match ACTIVE_EXTERNAL_EDITOR_REAPERS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(observed) => active = observed,
            }
        }
    }
}

impl Drop for ExternalEditorReaperPermit {
    fn drop(&mut self) {
        ACTIVE_EXTERNAL_EDITOR_REAPERS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl GitCore {
    /// Reads status using porcelain v2, which is designed for machine parsing.
    pub fn status(&self, repository: &Path) -> Result<GitStatus, AppError> {
        let arguments = os_arguments(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取仓库状态",
        )?;
        let mut status = parse_status_porcelain_v2(&output.stdout)?;
        if let (Some(head_commit), Some(upstream)) =
            (status.head_commit.as_deref(), status.upstream.as_deref())
        {
            let branch_name = status
                .branch
                .as_deref()
                .ok_or_else(|| AppError::new("git_parse_error", "无法读取当前分支。"))?;
            status.upstream_commit =
                self.commit_for_upstream(repository, branch_name, head_commit, upstream)?;
        }
        Ok(status)
    }

    /// Opens every unresolved conflict file in the application's explicitly
    /// configured external editor. The editor executable and each argument
    /// are passed directly to the operating system; no repository setting,
    /// shell command, or user-controlled path is interpolated into a shell.
    ///
    /// This intentionally opens files rather than invoking Git's mergetool.
    /// A repository-local mergetool configuration can contain an arbitrary
    /// shell command, whereas this application-local configuration is saved
    /// only after the user explicitly selects it.
    pub fn open_conflicted_files_in_external_editor(
        &self,
        repository: &Path,
        editor: &ExternalEditorConfig,
    ) -> Result<Vec<String>, AppError> {
        let editor = normalize_external_editor_config(editor.clone())?;
        let status = self.status(repository)?;
        let paths: Vec<String> = status
            .entries
            .iter()
            .filter(|entry| is_conflicted_status_entry(entry))
            .map(|entry| entry.path.clone())
            .collect();
        if paths.is_empty() {
            return Err(AppError::new(
                "no_conflicted_files",
                "当前仓库没有需要在外部编辑器中处理的冲突文件。",
            ));
        }

        let canonical_repository = fs::canonicalize(repository).map_err(|_| {
            AppError::new(
                "external_editor_repository_unavailable",
                "无法访问当前仓库，未启动外部编辑器。",
            )
        })?;
        let mut files = Vec::with_capacity(paths.len());
        for path in &paths {
            validate_worktree_relative_path(path)?;
            let file = fs::canonicalize(repository.join(path)).map_err(|_| {
                AppError::new(
                    "conflicted_file_unavailable",
                    "冲突文件已不存在或无法访问；请刷新仓库状态后重试。",
                )
            })?;
            if !file.starts_with(&canonical_repository) {
                return Err(AppError::new(
                    "conflicted_file_outside_repository",
                    "拒绝在仓库外部打开冲突文件。",
                ));
            }
            files.push(file);
        }
        if files.iter().any(|file| file.to_str().is_none()) {
            return Err(AppError::new(
                "conflicted_file_unavailable",
                "冲突文件路径包含外部编辑器不支持的字符。",
            ));
        }
        validate_external_editor_launch_size(
            std::iter::once(editor.executable.as_str())
                .chain(editor.arguments.iter().map(String::as_str))
                .chain(files.iter().filter_map(|file| file.to_str())),
        )?;

        // Reserve the reaper before starting a process so repeated calls
        // cannot create an unbounded collection of blocked `Child::wait`
        // threads for editors left open indefinitely.
        let reaper_permit = ExternalEditorReaperPermit::acquire()?;
        let mut command = Command::new(&editor.executable);
        command
            .args(&editor.arguments)
            .args(&files)
            .current_dir(repository)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| {
            AppError::new(
                "external_editor_launch_failed",
                "无法启动外部编辑器；请检查可执行文件路径和参数。",
            )
        })?;
        // A GUI editor may remain open for a long time. Reap it in a detached
        // worker so the desktop process never accumulates zombie children and
        // the IPC call can return immediately for the user to resolve files.
        let (sender, receiver) = std::sync::mpsc::sync_channel::<std::process::Child>(1);
        let reaper = thread::Builder::new()
            .name("bkm-external-editor-reaper".into())
            .spawn(move || {
                let _reaper_permit = reaper_permit;
                if let Ok(mut child) = receiver.recv() {
                    let _ = child.wait();
                }
            });
        match reaper {
            Ok(_) => {
                if let Err(error) = sender.send(child) {
                    let mut child = error.0;
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AppError::new(
                        "external_editor_reaper_failed",
                        "无法监控外部编辑器进程，请稍后重试。",
                    ));
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new(
                    "external_editor_reaper_failed",
                    "无法监控外部编辑器进程，请稍后重试。",
                ));
            }
        }
        Ok(paths)
    }

    /// Reads an unstaged or staged patch.  File paths are placed after `--` so
    /// they cannot be interpreted as Git options. For one untracked file,
    /// ordinary `git diff` is empty, so we render it as an addition from
    /// `/dev/null` while preserving the file's executable bit.
    pub fn diff(&self, repository: &Path, options: &DiffOptions) -> Result<String, AppError> {
        for path in &options.paths {
            // Normal Git pathspecs are safe after `--`, but the untracked
            // fallback below reads the filesystem through `--no-index` and
            // must prove the path cannot escape the worktree.
            validate_worktree_relative_path(path)?;
        }
        let mut arguments = os_arguments(&["diff", "--no-ext-diff"]);
        if options.staged {
            arguments.push(OsString::from("--cached"));
        }
        if let Some(context_lines) = options.context_lines {
            arguments.push(OsString::from(format!("--unified={context_lines}")));
        }
        arguments.push(OsString::from("--"));
        arguments.extend(options.paths.iter().map(|path| literal_pathspec(path)));

        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取改动内容",
        )?;
        let diff = output_text(&output.stdout);
        if !diff.is_empty() || options.staged || options.paths.len() != 1 {
            return Ok(diff);
        }

        let path = &options.paths[0];
        if self.is_untracked_path(repository, path)? {
            return self.untracked_file_diff(repository, path, options.context_lines);
        }
        Ok(diff)
    }

    /// Returns Git's standard blame output for one tracked file. The caller
    /// receives text rather than an error diagnostic so source lines and
    /// commit subjects remain data, never part of an IPC error message.
    pub fn blame(&self, repository: &Path, path: &str) -> Result<String, AppError> {
        validate_worktree_relative_path(path)?;
        if !self.has_head(repository)? {
            return Err(AppError::new(
                "blame_unavailable",
                "当前仓库还没有可用于 Blame 的提交历史。",
            ));
        }
        if self.is_untracked_path(repository, path)? {
            return Err(AppError::new(
                "blame_unavailable",
                "未跟踪文件没有可用于 Blame 的提交历史。",
            ));
        }
        // Unlike diff, blame does not accept a magic-pathspec token such as
        // :(literal)path. Use Git's global literal-pathspec switch instead,
        // then pass the already validated path as one direct argument.
        let mut arguments = os_arguments(&["--literal-pathspecs", "blame", "--abbrev=12", "--"]);
        arguments.push(OsString::from(path));
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取文件 Blame",
        )?;
        Ok(output_text(&output.stdout))
    }

    /// Shows recent HEAD movements in Git's native reflog presentation. A
    /// bounded count prevents a very old repository from sending an
    /// unbounded recovery history through IPC.
    pub fn reflog(&self, repository: &Path, limit: usize) -> Result<String, AppError> {
        let limit = limit.clamp(1, 500);
        let arguments = os_arguments(&[
            "reflog",
            "show",
            "--date=iso-strict",
            &format!("--max-count={limit}"),
            "--format=%h %gd %gs (%an, %ad)",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取 Reflog",
        )?;
        Ok(output_text(&output.stdout))
    }

    /// Previews exactly the untracked leaf paths that a confirmation token
    /// will later authorize. Ignored files are deliberately excluded; cleaning
    /// them needs a separate, more explicit workflow.
    pub fn preview_clean_untracked_paths(
        &self,
        repository: &Path,
        paths: &[String],
    ) -> Result<String, AppError> {
        self.ensure_no_operation(repository)?;
        if paths.is_empty() {
            return Ok(String::new());
        }
        validate_paths(paths)?;
        let environment = GitEnvironment::new()
            .with_variable("LC_ALL", "C")
            .with_variable("LANG", "C");
        let mut arguments = os_arguments(&["clean", "-n", "-d", "--"]);
        arguments.extend(paths.iter().map(|path| literal_pathspec(path)));
        let output = self.run(
            Some(repository),
            &arguments,
            &environment,
            "预览清理未跟踪文件",
        )?;
        Ok(output_text(&output.stdout))
    }

    /// Captures the exact untracked paths and their filesystem identities
    /// immediately after a cleanup preview.  The identity lets the command
    /// reject a later confirmation when another process has replaced one of
    /// the reviewed entries at the same path.
    pub(crate) fn capture_clean_untracked_path_fingerprints(
        &self,
        repository: &Path,
        paths: &[String],
    ) -> Result<Vec<CleanUntrackedPathFingerprint>, AppError> {
        validate_paths(paths)?;
        for path in paths {
            validate_worktree_relative_path(path)?;
        }
        let untracked_paths = self
            .status(repository)?
            .entries
            .into_iter()
            .filter(|entry| entry.index_status.is_empty() && entry.worktree_status == "?")
            .map(|entry| entry.path)
            .collect::<HashSet<_>>();
        paths
            .iter()
            .map(|path| {
                if !untracked_paths.contains(path) {
                    return Err(clean_untracked_confirmation_changed_error());
                }
                clean_untracked_path_fingerprint(repository, path)
            })
            .collect()
    }

    /// Revalidates a confirmation directly before beginning the secure
    /// descriptor-relative deletion sequence.
    /// Re-checking both Git status and the captured identity prevents a stale
    /// preview from authorizing a different untracked entry that reused the
    /// same pathname.
    pub(crate) fn validate_clean_untracked_path_fingerprints(
        &self,
        repository: &Path,
        fingerprints: &[CleanUntrackedPathFingerprint],
    ) -> Result<Vec<String>, AppError> {
        let paths = fingerprints
            .iter()
            .map(|fingerprint| fingerprint.path.clone())
            .collect::<Vec<_>>();
        let current = self.capture_clean_untracked_path_fingerprints(repository, &paths)?;
        if current != fingerprints {
            return Err(clean_untracked_confirmation_changed_error());
        }
        Ok(paths)
    }

    /// Deletes exactly the previewed filesystem objects. A final Git status
    /// check is necessary but insufficient on its own: another local process
    /// can replace a pathname before a later path-based `git clean` resolves
    /// it. Linux moves each entry through directory descriptors into a private
    /// quarantine, verifies the captured identity there, then unlinks it.
    pub(crate) fn clean_untracked_paths_with_fingerprints(
        &self,
        repository: &Path,
        fingerprints: &[CleanUntrackedPathFingerprint],
    ) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
        if fingerprints.is_empty() {
            return Ok(());
        }
        self.validate_clean_untracked_path_fingerprints(repository, fingerprints)?;
        #[cfg(target_os = "linux")]
        {
            self.clean_untracked_paths_with_linux_quarantine(repository, fingerprints)
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Descriptor-relative deletion is not available in the standard
            // library on these platforms. Keep the reviewed literal paths
            // after the final identity/status validation and delegate their
            // removal to Git, whose clean implementation removes a symlink
            // entry itself rather than traversing it.
            let mut arguments = os_arguments(&["clean", "-f", "-d", "--"]);
            arguments.extend(
                fingerprints
                    .iter()
                    .map(|fingerprint| literal_pathspec(&fingerprint.path)),
            );
            self.run(
                Some(repository),
                &arguments,
                &GitEnvironment::default(),
                "清理未跟踪文件",
            )?;
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn clean_untracked_paths_with_linux_quarantine(
        &self,
        repository: &Path,
        fingerprints: &[CleanUntrackedPathFingerprint],
    ) -> Result<(), AppError> {
        let parent_directories =
            clean_untracked_parent_directory_fingerprints(repository, fingerprints)?;
        let mut quarantine = CleanUntrackedQuarantine::create(repository)?;
        let mut quarantined = Vec::<QuarantinedCleanUntrackedPath>::new();
        for fingerprint in fingerprints {
            let (parent, leaf) = match open_clean_untracked_parent(repository, &fingerprint.path) {
                Ok(value) => value,
                Err(error) => {
                    restore_quarantined_clean_untracked_paths(&mut quarantine, &quarantined);
                    return Err(error);
                }
            };
            let temporary_name = CString::new(format!("entry-{}", Uuid::new_v4()))
                .map_err(|_| clean_untracked_confirmation_changed_error())?;
            if let Err(error) = rename_at_no_replace(
                parent.as_raw_fd(),
                &leaf,
                quarantine.directory_file.as_raw_fd(),
                &temporary_name,
            ) {
                restore_quarantined_clean_untracked_paths(&mut quarantine, &quarantined);
                return Err(if error.kind() == std::io::ErrorKind::NotFound {
                    clean_untracked_confirmation_changed_error()
                } else {
                    AppError::new("clean_untracked_failed", "无法安全清理未跟踪文件。")
                });
            }
            quarantined.push((parent, leaf, temporary_name, fingerprint.identity.kind));
            let (_, _, temporary_name, _) = quarantined
                .last()
                .expect("the just-moved clean entry is retained in quarantine");

            let identity_matches = match clean_untracked_fingerprint_matches_at(
                quarantine.directory_file.as_raw_fd(),
                temporary_name,
                fingerprint,
            ) {
                Ok(matches) => matches,
                Err(error) => {
                    restore_quarantined_clean_untracked_paths(&mut quarantine, &quarantined);
                    return Err(error);
                }
            };
            if !identity_matches {
                restore_quarantined_clean_untracked_paths(&mut quarantine, &quarantined);
                return Err(clean_untracked_confirmation_changed_error());
            }
        }

        // All paths are now verified in one private directory. Final deletion
        // is deliberately best effort: an unlink failure cannot restore a
        // worktree path that has already been removed, so keep any leftovers
        // quarantined and still report the user-visible cleanup as complete.
        for (_, _, temporary_name, kind) in quarantined {
            let flags = match kind {
                CleanUntrackedPathKind::Directory => libc::AT_REMOVEDIR,
                CleanUntrackedPathKind::File
                | CleanUntrackedPathKind::Symlink
                | CleanUntrackedPathKind::Other => 0,
            };
            // The entry is now in an app-private, randomly named directory;
            // no repository path is resolved during this final unlink.
            let removed = unsafe {
                libc::unlinkat(
                    quarantine.directory_file.as_raw_fd(),
                    temporary_name.as_ptr(),
                    flags,
                )
            };
            if removed != 0 {
                quarantine.retain();
            }
        }
        remove_empty_clean_untracked_parent_directories(repository, &parent_directories);
        Ok(())
    }

    pub(super) fn is_untracked_path(
        &self,
        repository: &Path,
        path: &str,
    ) -> Result<bool, AppError> {
        let mut arguments =
            os_arguments(&["ls-files", "--others", "--exclude-standard", "-z", "--"]);
        arguments.push(literal_pathspec(path));
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取未跟踪文件",
        )?;
        Ok(!output.stdout.is_empty())
    }

    pub(super) fn untracked_file_diff(
        &self,
        repository: &Path,
        path: &str,
        _context_lines: Option<u32>,
    ) -> Result<String, AppError> {
        let content = read_untracked_file_safely(repository, path)?;
        if content.bytes.is_empty() {
            return Ok(render_empty_untracked_file_diff(
                path,
                content.is_executable,
                &self.empty_blob_object_id(repository)?,
            ));
        }
        Ok(render_untracked_file_diff(path, &content))
    }

    /// A new empty file has no unified-diff hunk. Ask Git for the repository's
    /// empty-blob object ID so the resulting patch is valid for both SHA-1 and
    /// SHA-256 repositories instead of hard-coding a SHA-1-only index line.
    fn empty_blob_object_id(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.run_with_input(
            Some(repository),
            &os_arguments(&["hash-object", "--stdin"]),
            b"",
            &GitEnvironment::default(),
            "读取空文件对象",
        )?;
        let output = output_text(&output.stdout);
        let object_id = output.trim();
        if !(40..=64).contains(&object_id.len())
            || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取空文件对象标识。"));
        }
        Ok(object_id.to_owned())
    }

    /// Stages one or more paths.
    pub fn stage_paths(&self, repository: &Path, paths: &[String]) -> Result<(), AppError> {
        validate_paths(paths)?;
        let mut arguments = os_arguments(&["add", "--"]);
        arguments.extend(paths.iter().map(|path| literal_pathspec(path)));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "暂存改动",
        )?;
        Ok(())
    }

    /// Removes one or more paths from the index without discarding worktree
    /// changes.
    pub fn unstage_paths(&self, repository: &Path, paths: &[String]) -> Result<(), AppError> {
        validate_paths(paths)?;
        // `git restore --staged` reads HEAD to restore the index. An unborn
        // repository has no HEAD yet, so use `git rm --cached` there: it
        // removes only the index entries and deliberately leaves the files in
        // the worktree as untracked changes.
        let mut arguments = if self.has_head(repository)? {
            os_arguments(&["restore", "--staged", "--"])
        } else {
            os_arguments(&["rm", "--cached", "--ignore-unmatch", "--"])
        };
        arguments.extend(paths.iter().map(|path| literal_pathspec(path)));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "取消暂存",
        )?;
        Ok(())
    }

    /// Applies a Git-generated unified diff to the index only. This is the
    /// backend primitive for staging or unstaging one selected hunk; patch
    /// text is size-limited and is never sent to a shell or reflected in an
    /// error response.
    pub fn apply_patch(
        &self,
        repository: &Path,
        path: &str,
        patch: &str,
        reverse: bool,
    ) -> Result<(), AppError> {
        validate_git_patch(patch)?;
        validate_worktree_relative_path(path)?;
        self.ensure_patch_targets_path(repository, patch, path)?;
        let mut arguments = os_arguments(&["apply", "--cached", "--whitespace=nowarn"]);
        if reverse {
            arguments.push(OsString::from("--reverse"));
        }
        self.run_with_input(
            Some(repository),
            &arguments,
            patch.as_bytes(),
            &GitEnvironment::default(),
            if reverse {
                "取消暂存代码块"
            } else {
                "暂存代码块"
            },
        )?;
        Ok(())
    }

    /// `git apply --numstat` parses the patch without changing the index. Its
    /// NUL-delimited path output is Git's own interpretation of every file
    /// affected by the patch, including quoted or unusual filenames. Require
    /// exactly one matching UI path before the later `git apply --cached`
    /// receives the same bytes.
    fn ensure_patch_targets_path(
        &self,
        repository: &Path,
        patch: &str,
        expected_path: &str,
    ) -> Result<(), AppError> {
        let output = self.run_with_input(
            Some(repository),
            &os_arguments(&["apply", "--numstat", "-z"]),
            patch.as_bytes(),
            &GitEnvironment::default(),
            "验证代码块补丁",
        )?;
        let mut records = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty());
        let Some(record) = records.next() else {
            return Err(AppError::new(
                "invalid_git_patch",
                "代码块补丁未包含可应用的文件改动。",
            ));
        };
        if records.next().is_some() {
            return Err(AppError::new(
                "invalid_git_patch",
                "代码块补丁只能修改当前选中的一个文件。",
            ));
        }
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let _added = fields.next();
        let _deleted = fields.next();
        let Some(path) = fields.next().filter(|path| !path.is_empty()) else {
            return Err(AppError::new(
                "invalid_git_patch",
                "代码块补丁包含不支持的重命名或路径格式。",
            ));
        };
        let path = std::str::from_utf8(path).map_err(|_| {
            AppError::new(
                "invalid_git_patch",
                "代码块补丁包含无法安全处理的文件路径。",
            )
        })?;
        if path != expected_path {
            return Err(AppError::new(
                "invalid_git_patch",
                "代码块补丁与当前选中的文件不匹配。",
            ));
        }
        Ok(())
    }

    /// Discards all indexed and working-tree changes for the selected status
    /// entries. Purely untracked entries are removed directly, while tracked
    /// entries go through Git so renames, the index, and file modes remain
    /// consistent. Callers must ask the user for confirmation before this.
    pub fn discard_paths(&self, repository: &Path, paths: &[String]) -> Result<(), AppError> {
        validate_paths(paths)?;
        for path in paths {
            validate_worktree_relative_path(path)?;
        }
        // `restore --staged --worktree` can erase conflict stages while a
        // merge, rebase, or cherry-pick is paused. A UI-side confirmation and
        // the in-process lease cannot make that externally persisted Git
        // state safe, so destructive discard must wait for the operation to
        // be continued or explicitly aborted.
        self.ensure_no_operation(repository)?;

        let status = self.status(repository)?;
        let entries_by_path = status
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        let mut tracked_seen = HashSet::new();
        let mut tracked_paths = Vec::new();
        let mut paths_to_remove = Vec::new();
        for path in paths {
            if !seen.insert(path.as_str()) {
                continue;
            }
            let entry = entries_by_path.get(path.as_str()).copied().ok_or_else(|| {
                AppError::new("nothing_to_discard", "所选文件已经没有可丢弃的改动")
            })?;
            if entry.index_status.is_empty() && entry.worktree_status == "?" {
                paths_to_remove.push(path.clone());
            } else {
                // A porcelain-v2 copy also supplies `original_path`, but it
                // leaves that source in place. Restore the source only for a
                // true rename so a selected copy cannot discard independent
                // edits on its original file.
                let related_path = (entry.index_status == "R" || entry.worktree_status == "R")
                    .then_some(entry.original_path.as_deref())
                    .flatten();
                for tracked_path in std::iter::once(entry.path.as_str()).chain(related_path) {
                    validate_worktree_relative_path(tracked_path)?;
                    if tracked_seen.insert(tracked_path) {
                        tracked_paths.push(tracked_path.to_owned());
                    }
                }
            }
        }

        if !tracked_paths.is_empty() {
            // Another Git client can begin a merge, rebase, or cherry-pick
            // after the earlier status read. Check immediately before each
            // destructive Git mutation so we never knowingly erase conflict
            // stages from a newly paused operation.
            self.ensure_no_operation(repository)?;
            if self.has_head(repository)? {
                let mut arguments =
                    os_arguments(&["restore", "--source=HEAD", "--staged", "--worktree", "--"]);
                arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
                self.run(
                    Some(repository),
                    &arguments,
                    &GitEnvironment::default(),
                    "丢弃改动",
                )?;
            } else {
                // An unborn repository has no HEAD for `git restore` to use.
                // Remove its index entries first, then remove the matching
                // worktree paths below.
                let mut arguments = os_arguments(&["rm", "--cached", "--ignore-unmatch", "--"]);
                arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
                self.run(
                    Some(repository),
                    &arguments,
                    &GitEnvironment::default(),
                    "丢弃改动",
                )?;
                paths_to_remove.extend(tracked_paths);
            }
        }

        if !paths_to_remove.is_empty() {
            self.ensure_no_operation(repository)?;
            // Delegate removal to Git with literal pathspecs rather than
            // reopening repository-relative paths in this process. Git's
            // worktree machinery handles symlinks as entries and avoids the
            // check-then-use filesystem traversal that direct removal needs.
            let mut arguments = os_arguments(&["clean", "-f", "--"]);
            arguments.extend(paths_to_remove.iter().map(|path| literal_pathspec(path)));
            self.run(
                Some(repository),
                &arguments,
                &GitEnvironment::default(),
                "丢弃未跟踪改动",
            )?;
        }
        Ok(())
    }
}
