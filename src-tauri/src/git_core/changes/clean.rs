fn clean_untracked_confirmation_changed_error() -> AppError {
    AppError::new(
        "clean_untracked_confirmation_changed",
        "未跟踪文件已变化，请重新预览后再清理。",
    )
}

fn discard_confirmation_changed_error() -> AppError {
    AppError::new(
        "discard_confirmation_changed",
        "所选改动已变化，请重新确认后再丢弃。",
    )
}

fn discard_index_contains_gitlink(index_entries: &[u8]) -> Result<bool, AppError> {
    for record in index_entries
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(AppError::new(
                "git_parse_error",
                "无法读取待丢弃的暂存区状态。",
            ));
        };
        if record[..tab].starts_with(b"160000 ") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn discard_worktree_path_fingerprint(
    repository: &Path,
    relative_path: &str,
) -> Result<DiscardWorktreePathFingerprint, AppError> {
    match fs::symlink_metadata(repository.join(relative_path)) {
        Ok(_) => clean_untracked_path_fingerprint(repository, relative_path)
            .map(DiscardWorktreePathFingerprint::Present)
            .map_err(|_| discard_confirmation_changed_error()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(
            DiscardWorktreePathFingerprint::Missing(relative_path.to_owned()),
        ),
        Err(_) => Err(discard_confirmation_changed_error()),
    }
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
fn discard_transaction_cleanup_error() -> AppError {
    AppError::with_safe_details(
        "discard_cleanup_required",
        "改动已丢弃，但临时备份无法自动清理。",
        "请在仓库根目录检查并清理 .bkm-discard-transaction-* 或 .bkm-discard-cleanup-* 目录。",
    )
}

#[cfg(target_os = "linux")]
fn claim_directory_entry_for_cleanup(
    directory: &fs::File,
    name: &CStr,
    expected: &libc::stat,
) -> std::io::Result<CString> {
    let claimed = CString::new(format!("entry-{}", Uuid::new_v4()))
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    rename_at_no_replace(directory.as_raw_fd(), name, directory.as_raw_fd(), &claimed)?;
    let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
    let current_result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            claimed.as_ptr(),
            current.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if current_result == 0 {
        let current = unsafe { current.assume_init() };
        if current.st_dev == expected.st_dev && current.st_ino == expected.st_ino {
            return Ok(claimed);
        }
    }
    let _ = rename_at_no_replace(directory.as_raw_fd(), &claimed, directory.as_raw_fd(), name);
    Err(std::io::Error::from(std::io::ErrorKind::NotFound))
}

#[cfg(target_os = "linux")]
fn remove_directory_contents_at(directory: &fs::File) -> std::io::Result<()> {
    let duplicated = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicated < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stream = unsafe { libc::fdopendir(duplicated) };
    if stream.is_null() {
        unsafe { libc::close(duplicated) };
        return Err(std::io::Error::last_os_error());
    }

    let result = (|| -> std::io::Result<()> {
        loop {
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                return if error.raw_os_error() == Some(0) {
                    Ok(())
                } else {
                    Err(error)
                };
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    metadata.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let metadata = unsafe { metadata.assume_init() };
            if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
                let child = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if child < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let child = unsafe { fs::File::from_raw_fd(child) };
                let mut opened = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe { libc::fstat(child.as_raw_fd(), opened.as_mut_ptr()) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let opened = unsafe { opened.assume_init() };
                if opened.st_dev != metadata.st_dev || opened.st_ino != metadata.st_ino {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                }
                remove_directory_contents_at(&child)?;
                let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
                if unsafe {
                    libc::fstatat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        current.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                let current = unsafe { current.assume_init() };
                if current.st_dev != opened.st_dev || current.st_ino != opened.st_ino {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                }
                let claimed = claim_directory_entry_for_cleanup(directory, name, &opened)?;
                if unsafe {
                    libc::unlinkat(directory.as_raw_fd(), claimed.as_ptr(), libc::AT_REMOVEDIR)
                } != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            } else {
                let claimed = claim_directory_entry_for_cleanup(directory, name, &metadata)?;
                if unsafe { libc::unlinkat(directory.as_raw_fd(), claimed.as_ptr(), 0) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }
    })();
    unsafe { libc::closedir(stream) };
    result
}

#[cfg(target_os = "linux")]
fn remove_stale_empty_discard_cleanup_directories(worktree: &fs::File) {
    let duplicated = unsafe { libc::dup(worktree.as_raw_fd()) };
    if duplicated < 0 {
        return;
    }
    let stream = unsafe { libc::fdopendir(duplicated) };
    if stream.is_null() {
        unsafe { libc::close(duplicated) };
        return;
    }
    let mut inspected = 0usize;
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() || inspected >= 256 {
            break;
        }
        inspected += 1;
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if !name.to_bytes().starts_with(b".bkm-discard-cleanup-") {
            continue;
        }
        // rmdir semantics are intentional: a retained recovery directory or
        // any concurrently populated replacement is never removed.
        let _ = unsafe { libc::unlinkat(worktree.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
    }
    unsafe { libc::closedir(stream) };
}

