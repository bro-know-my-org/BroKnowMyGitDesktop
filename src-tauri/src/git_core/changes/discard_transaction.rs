#[cfg(target_os = "linux")]
struct DiscardTransaction {
    worktree_parent: fs::File,
    directory_name: CString,
    directory_file: fs::File,
    contains_claimed_user_data: bool,
    completed: bool,
}

#[cfg(target_os = "linux")]
impl DiscardTransaction {
    fn create(worktree: &Path) -> Result<Self, AppError> {
        let worktree_parent =
            open_directory_no_follow(worktree).map_err(|_| discard_filesystem_error())?;
        remove_stale_empty_discard_cleanup_directories(&worktree_parent);
        for _ in 0..16 {
            let name = format!(".bkm-discard-transaction-{}", Uuid::new_v4());
            let directory_name =
                CString::new(name.as_bytes()).map_err(|_| discard_filesystem_error())?;
            if unsafe { libc::mkdirat(worktree_parent.as_raw_fd(), directory_name.as_ptr(), 0o700) }
                != 0
            {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(discard_filesystem_error());
            }
            let descriptor = unsafe {
                libc::openat(
                    worktree_parent.as_raw_fd(),
                    directory_name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if descriptor < 0 {
                let _ = unsafe {
                    libc::unlinkat(
                        worktree_parent.as_raw_fd(),
                        directory_name.as_ptr(),
                        libc::AT_REMOVEDIR,
                    )
                };
                return Err(discard_filesystem_error());
            }
            return Ok(Self {
                worktree_parent,
                directory_name,
                directory_file: unsafe { fs::File::from_raw_fd(descriptor) },
                contains_claimed_user_data: false,
                completed: false,
            });
        }
        Err(discard_filesystem_error())
    }

    fn mark_claimed(&mut self) {
        self.contains_claimed_user_data = true;
    }

    fn anchored_path(directory: &fs::File, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}/{}", directory.as_raw_fd(), name))
    }

    fn create_anchored_directory(&self, name: &CStr) -> Result<(), AppError> {
        if unsafe { libc::mkdirat(self.directory_file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(discard_filesystem_error());
        }
        Ok(())
    }

    fn inheritable_directory_anchor(&self) -> Result<fs::File, AppError> {
        let descriptor = unsafe { libc::fcntl(self.directory_file.as_raw_fd(), libc::F_DUPFD, 3) };
        if descriptor < 0 {
            return Err(discard_filesystem_error());
        }
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }

    fn cleanup(&mut self) -> Result<(), AppError> {
        let original_name = self.directory_name.clone();
        let cleanup_name = CString::new(format!(".bkm-discard-cleanup-{}", Uuid::new_v4()))
            .map_err(|_| discard_transaction_cleanup_error())?;
        rename_at_no_replace(
            self.worktree_parent.as_raw_fd(),
            &self.directory_name,
            self.worktree_parent.as_raw_fd(),
            &cleanup_name,
        )
        .map_err(|_| discard_transaction_cleanup_error())?;
        self.directory_name = cleanup_name;

        let restore_replacement = |transaction: &mut Self| {
            if rename_at_no_replace(
                transaction.worktree_parent.as_raw_fd(),
                &transaction.directory_name,
                transaction.worktree_parent.as_raw_fd(),
                &original_name,
            )
            .is_ok()
            {
                transaction.directory_name = original_name.clone();
            }
        };

        let mut held = std::mem::MaybeUninit::<libc::stat>::uninit();
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        let held_result =
            unsafe { libc::fstat(self.directory_file.as_raw_fd(), held.as_mut_ptr()) };
        let current_result = unsafe {
            libc::fstatat(
                self.worktree_parent.as_raw_fd(),
                self.directory_name.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if held_result != 0 || current_result != 0 {
            restore_replacement(self);
            return Err(discard_transaction_cleanup_error());
        }
        let held = unsafe { held.assume_init() };
        let current = unsafe { current.assume_init() };
        if held.st_dev != current.st_dev
            || held.st_ino != current.st_ino
            || current.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            restore_replacement(self);
            return Err(discard_transaction_cleanup_error());
        }

        remove_directory_contents_at(&self.directory_file)
            .map_err(|_| discard_transaction_cleanup_error())?;
        // Keep the now-empty, randomly named directory rather than unlinking
        // a pathname after a potentially long recursive cleanup. Empty
        // directories are invisible to Git status, and retaining this shell
        // closes the final directory-for-directory substitution race.
        self.completed = true;
        Ok(())
    }

    fn finish(mut self) -> Result<(), AppError> {
        self.cleanup()
    }
}

#[cfg(target_os = "linux")]
struct DiscardIndexLock {
    index_path: PathBuf,
    lock_path: PathBuf,
    prepared_index_path: PathBuf,
    file: Option<fs::File>,
    committed: bool,
}

#[cfg(target_os = "linux")]
impl DiscardIndexLock {
    fn acquire(git: &GitCore, repository: &Path) -> Result<Self, AppError> {
        let index_path = git.git_directory(repository)?.join("index");
        let mut lock_name = index_path
            .file_name()
            .ok_or_else(discard_filesystem_error)?
            .to_os_string();
        lock_name.push(".lock");
        let lock_path = index_path.with_file_name(lock_name);
        let prepared_index_path =
            index_path.with_file_name(format!(".bkm-discard-index-{}", Uuid::new_v4()));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    AppError::new(
                        "git_index_locked",
                        "Git 暂存区正被其他操作修改，请稍后重试。",
                    )
                } else {
                    discard_filesystem_error()
                }
            })?;
        Ok(Self {
            index_path,
            lock_path,
            prepared_index_path,
            file: Some(file),
            committed: false,
        })
    }

    fn prepared_index_path(&self) -> &Path {
        &self.prepared_index_path
    }

    fn copy_current_to_prepared(&self) -> Result<(), AppError> {
        if self.index_path.exists() {
            fs::copy(&self.index_path, &self.prepared_index_path)
                .map_err(|_| discard_filesystem_error())?;
        }
        Ok(())
    }

    fn commit(mut self) -> Result<(), AppError> {
        let file = self.file.as_mut().ok_or_else(discard_filesystem_error)?;
        file.set_len(0).map_err(|_| discard_filesystem_error())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| discard_filesystem_error())?;
        let mut prepared =
            fs::File::open(&self.prepared_index_path).map_err(|_| discard_filesystem_error())?;
        std::io::copy(&mut prepared, file).map_err(|_| discard_filesystem_error())?;
        file.sync_all().map_err(|_| discard_filesystem_error())?;
        self.file.take();
        fs::rename(&self.lock_path, &self.index_path).map_err(|_| discard_filesystem_error())?;
        let _ = fs::remove_file(&self.prepared_index_path);
        self.committed = true;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for DiscardIndexLock {
    fn drop(&mut self) {
        if !self.committed {
            self.file.take();
            let _ = fs::remove_file(&self.lock_path);
        }
        let _ = fs::remove_file(&self.prepared_index_path);
    }
}

#[cfg(target_os = "linux")]
struct DiscardReferenceLocks {
    locks: Vec<(PathBuf, fs::File)>,
}

#[cfg(target_os = "linux")]
impl DiscardReferenceLocks {
    fn acquire(
        git: &GitCore,
        repository: &Path,
        head_reference: Option<&str>,
    ) -> Result<Self, AppError> {
        let mut targets = vec![git.git_directory(repository)?.join("HEAD")];
        if let Some(reference) = head_reference {
            let output = git.run(
                Some(repository),
                &[
                    OsString::from("rev-parse"),
                    OsString::from("--git-path"),
                    OsString::from(reference),
                ],
                &GitEnvironment::default(),
                "读取当前分支位置",
            )?;
            let path = git_path_from_output(&output.stdout, "无法读取当前分支位置。")?;
            targets.push(if path.is_absolute() {
                path
            } else {
                repository.join(path)
            });
        }

        let mut guard = Self { locks: Vec::new() };
        for target in targets {
            let parent = target.parent().ok_or_else(discard_filesystem_error)?;
            fs::create_dir_all(parent).map_err(|_| discard_filesystem_error())?;
            let mut lock_name = target
                .file_name()
                .ok_or_else(discard_filesystem_error)?
                .to_os_string();
            lock_name.push(".lock");
            let lock_path = target.with_file_name(lock_name);
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        AppError::new(
                            "git_reference_locked",
                            "当前分支正被其他 Git 操作修改，请稍后重试。",
                        )
                    } else {
                        discard_filesystem_error()
                    }
                })?;
            guard.locks.push((lock_path, file));
        }
        Ok(guard)
    }
}

#[cfg(target_os = "linux")]
impl Drop for DiscardReferenceLocks {
    fn drop(&mut self) {
        while let Some((path, file)) = self.locks.pop() {
            drop(file);
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for DiscardTransaction {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if self.contains_claimed_user_data {
            // A failed rollback must leave every claimed object in the private
            // transaction directory for manual recovery rather than deleting it.
            return;
        }
        let _ = self.cleanup();
    }
}

#[cfg(target_os = "linux")]
struct AppliedDiscardPath {
    target_parent: fs::File,
    target_leaf: CString,
    backup_name: Option<CString>,
    desired_parent: Option<fs::File>,
    desired_leaf: Option<CString>,
    desired_fingerprint: Option<CleanUntrackedPathFingerprint>,
    desired_installed: bool,
}

#[cfg(target_os = "linux")]
fn discard_filesystem_error() -> AppError {
    AppError::new("discard_failed", "无法在文件变化保护下安全丢弃所选改动。")
}

#[cfg(target_os = "linux")]
fn discard_recovery_required_error(_transaction: &DiscardTransaction) -> AppError {
    AppError::with_safe_details(
        "discard_recovery_required",
        "丢弃失败且部分文件无法自动恢复。",
        "请从仓库根目录的 .bkm-discard-transaction-* 目录恢复文件。",
    )
}

#[cfg(target_os = "linux")]
fn open_or_create_discard_parent(
    repository: &Path,
    relative_path: &str,
    created_directories: &mut Vec<CleanUntrackedDirectoryFingerprint>,
) -> Result<(fs::File, CString), AppError> {
    validate_worktree_relative_path(relative_path)?;
    let mut components = Path::new(relative_path).components();
    let leaf = match components.next_back() {
        Some(std::path::Component::Normal(component)) => {
            c_string_from_os_str(component).map_err(|_| discard_confirmation_changed_error())?
        }
        _ => return Err(discard_confirmation_changed_error()),
    };
    let mut parent =
        open_directory_no_follow(repository).map_err(|_| discard_filesystem_error())?;
    let mut relative_parent = PathBuf::new();
    for component in components {
        let std::path::Component::Normal(component) = component else {
            return Err(discard_confirmation_changed_error());
        };
        relative_parent.push(component);
        let component =
            c_string_from_os_str(component).map_err(|_| discard_confirmation_changed_error())?;
        let mut was_created = false;
        let mut descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            let created = unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o777) };
            if created != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(discard_filesystem_error());
            }
            was_created = created == 0;
            descriptor = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
        }
        if descriptor < 0 {
            return Err(discard_confirmation_changed_error());
        }
        if was_created {
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(descriptor, metadata.as_mut_ptr()) } != 0 {
                unsafe { libc::close(descriptor) };
                return Err(discard_filesystem_error());
            }
            let metadata = unsafe { metadata.assume_init() };
            created_directories.push(CleanUntrackedDirectoryFingerprint {
                path: relative_parent.to_string_lossy().into_owned(),
                device: metadata.st_dev,
                inode: metadata.st_ino,
            });
        }
        parent = unsafe { fs::File::from_raw_fd(descriptor) };
    }
    Ok((parent, leaf))
}

#[cfg(target_os = "linux")]
fn rollback_applied_discard_paths(
    transaction: &DiscardTransaction,
    applied: &[AppliedDiscardPath],
) -> bool {
    let mut rollback_succeeded = true;
    for entry in applied.iter().rev() {
        if entry.desired_installed {
            let Some(desired_parent) = entry.desired_parent.as_ref() else {
                rollback_succeeded = false;
                continue;
            };
            let Some(desired_leaf) = entry.desired_leaf.as_ref() else {
                rollback_succeeded = false;
                continue;
            };
            if rename_at_no_replace(
                entry.target_parent.as_raw_fd(),
                &entry.target_leaf,
                desired_parent.as_raw_fd(),
                desired_leaf,
            )
            .is_err()
            {
                rollback_succeeded = false;
            } else if let Some(fingerprint) = &entry.desired_fingerprint {
                match clean_untracked_fingerprint_matches_at(
                    desired_parent.as_raw_fd(),
                    desired_leaf,
                    fingerprint,
                ) {
                    Ok(true) => {}
                    Ok(false) | Err(_) => rollback_succeeded = false,
                }
            }
        }
        if let Some(backup_name) = &entry.backup_name {
            if rename_at_no_replace(
                transaction.directory_file.as_raw_fd(),
                backup_name,
                entry.target_parent.as_raw_fd(),
                &entry.target_leaf,
            )
            .is_err()
            {
                rollback_succeeded = false;
            }
        }
    }
    rollback_succeeded
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

