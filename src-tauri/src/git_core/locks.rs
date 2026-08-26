use super::*;
use crate::git_core::error::normalize_path;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

const SHARED_LOCKS: &[(&str, RepositoryLockKind)] = &[
    ("config.lock", RepositoryLockKind::Configuration),
    ("packed-refs.lock", RepositoryLockKind::Reference),
    ("shallow.lock", RepositoryLockKind::Shallow),
];
const WORKTREE_LOCKS: &[(&str, RepositoryLockKind)] = &[
    ("config.worktree.lock", RepositoryLockKind::Configuration),
    ("index.lock", RepositoryLockKind::Index),
    ("HEAD.lock", RepositoryLockKind::Reference),
    ("AUTO_MERGE.lock", RepositoryLockKind::Reference),
    ("BISECT_HEAD.lock", RepositoryLockKind::Reference),
    ("CHERRY_PICK_HEAD.lock", RepositoryLockKind::Reference),
    ("FETCH_HEAD.lock", RepositoryLockKind::Reference),
    ("MERGE_HEAD.lock", RepositoryLockKind::Reference),
    ("ORIG_HEAD.lock", RepositoryLockKind::Reference),
    ("REBASE_HEAD.lock", RepositoryLockKind::Reference),
    ("REVERT_HEAD.lock", RepositoryLockKind::Reference),
];

struct RepositoryLockCandidate {
    info: RepositoryLockInfo,
    path: PathBuf,
}

impl GitCore {
    /// Lists standard top-level lock files in this worktree and its shared
    /// metadata. Avoiding a recursive `*.lock` walk prevents symlink/reparse
    /// traversal and keeps linked-worktree private state isolated.
    pub fn repository_locks(&self, repository: &Path) -> Result<RepositoryLocksResult, AppError> {
        let (candidates, incomplete) = self.repository_lock_candidates(repository)?;
        Ok(RepositoryLocksResult {
            locks: candidates
                .into_iter()
                .map(|candidate| candidate.info)
                .collect(),
            incomplete,
        })
    }

    pub fn repository_lock_directory(
        &self,
        repository: &Path,
        lock_id: &str,
    ) -> Result<PathBuf, AppError> {
        let candidate = self
            .repository_lock_candidates(repository)?
            .0
            .into_iter()
            .find(|candidate| candidate.info.id == lock_id)
            .ok_or_else(git_lock_changed_error)?;
        candidate
            .path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(git_lock_changed_error)
    }

    pub fn repository_metadata_directories(
        &self,
        repository: &Path,
    ) -> Result<Vec<PathBuf>, AppError> {
        let git_directory = normalize_path(&self.git_directory(repository)?);
        let common_directory = normalize_path(&self.git_common_directory(repository)?);
        let mut directories = BTreeSet::new();
        directories.insert(git_directory);
        directories.insert(common_directory);
        Ok(directories.into_iter().collect())
    }

    fn repository_lock_candidates(
        &self,
        repository: &Path,
    ) -> Result<(Vec<RepositoryLockCandidate>, bool), AppError> {
        let git_directory = normalize_path(&self.git_directory(repository)?);
        let common_directory = normalize_path(&self.git_common_directory(repository)?);
        let mut paths = BTreeMap::<PathBuf, RepositoryLockKind>::new();
        for (name, kind) in SHARED_LOCKS {
            paths.insert(common_directory.join(name), *kind);
        }
        for (name, kind) in WORKTREE_LOCKS {
            paths.insert(git_directory.join(name), *kind);
        }

        let mut candidates = Vec::new();
        let mut first_error = None;
        for (path, kind) in paths {
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    first_error.get_or_insert_with(|| AppError::io("检查 Git 锁文件", error));
                    continue;
                }
            };
            let (id, modified_at) = match lock_id_and_modified(&path, &metadata) {
                Ok(identity) => identity,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            candidates.push(RepositoryLockCandidate {
                info: RepositoryLockInfo {
                    id,
                    path: path.to_string_lossy().into_owned(),
                    kind,
                    modified_at_ms: modified_at.and_then(system_time_millis),
                },
                path,
            });
        }
        candidates.sort_by(|left, right| left.info.path.cmp(&right.info.path));
        Ok((candidates, first_error.is_some()))
    }
}

#[cfg(not(windows))]
fn lock_id_and_modified(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(String, Option<SystemTime>), AppError> {
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    digest.update(metadata.len().to_le_bytes());
    update_system_time_digest(&mut digest, metadata.modified().ok());
    update_system_time_digest(&mut digest, metadata.created().ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
        digest.update(metadata.ctime().to_le_bytes());
        digest.update(metadata.ctime_nsec().to_le_bytes());
    }
    Ok((format!("{:x}", digest.finalize()), metadata.modified().ok()))
}

#[cfg(windows)]
fn lock_id_and_modified(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(String, Option<SystemTime>), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide_path` is NUL-terminated and remains alive for the call.
    // No security attributes or template handle are needed, so those pointers
    // are null. The returned handle is checked before ownership is transferred.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(AppError::io(
            "检查 Git 锁文件身份",
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: this is the unique ownership transfer for the validated handle;
    // `file` closes it exactly once when it leaves scope.
    let file = unsafe { fs::File::from_raw_handle(handle) };
    let metadata = file
        .metadata()
        .map_err(|error| AppError::io("检查 Git 锁文件身份", error))?;
    if !metadata.file_type().is_file() {
        return Err(git_lock_changed_error());
    }
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    digest.update(metadata.len().to_le_bytes());
    update_system_time_digest(&mut digest, metadata.modified().ok());
    update_system_time_digest(&mut digest, metadata.created().ok());
    digest.update(metadata.file_attributes().to_le_bytes());
    digest.update(metadata.creation_time().to_le_bytes());
    digest.update(metadata.last_write_time().to_le_bytes());

    let mut identity = FILE_ID_INFO::default();
    // SAFETY: `identity` is initialized writable storage and the byte count is
    // exactly its size. The handle remains valid for the duration of the call.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result != 0 {
        digest.update([1]);
        digest.update(identity.VolumeSerialNumber.to_le_bytes());
        digest.update(identity.FileId.Identifier);
        return Ok((format!("{:x}", digest.finalize()), metadata.modified().ok()));
    }

    let mut fallback = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `fallback` is initialized writable storage of the exact type
    // required by the API, and `file` still owns a valid handle.
    let fallback_result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut fallback) };
    if fallback_result == 0 {
        return Err(AppError::io(
            "检查 Git 锁文件身份",
            std::io::Error::last_os_error(),
        ));
    }
    digest.update([0]);
    digest.update(fallback.dwVolumeSerialNumber.to_le_bytes());
    digest.update(fallback.nFileIndexHigh.to_le_bytes());
    digest.update(fallback.nFileIndexLow.to_le_bytes());
    Ok((format!("{:x}", digest.finalize()), metadata.modified().ok()))
}

fn update_system_time_digest(digest: &mut Sha256, value: Option<SystemTime>) {
    let Some(value) = value else {
        digest.update([0]);
        return;
    };
    digest.update([1]);
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            digest.update([1]);
            digest.update(duration.as_secs().to_le_bytes());
            digest.update(duration.subsec_nanos().to_le_bytes());
        }
        Err(error) => {
            digest.update([0]);
            let duration = error.duration();
            digest.update(duration.as_secs().to_le_bytes());
            digest.update(duration.subsec_nanos().to_le_bytes());
        }
    }
}

fn system_time_millis(value: SystemTime) -> Option<u64> {
    let millis = value.duration_since(UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}

fn git_lock_changed_error() -> AppError {
    AppError::new("git_lock_changed", "Git 锁文件已经变化，请重新检查。")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use uuid::Uuid;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_git_template_dir() -> &'static Path {
        static TEMPLATE_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
        TEMPLATE_DIRECTORY
            .get_or_init(|| {
                let directory = std::env::temp_dir().join(format!(
                    "bkm-locks-empty-template-{}-{}",
                    std::process::id(),
                    Uuid::new_v4()
                ));
                fs::create_dir(&directory).unwrap();
                directory
            })
            .as_path()
    }

    fn test_git_command() -> Command {
        let mut command = Command::new("git");
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("GIT_") {
                command.env_remove(name);
            }
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH)
            .env("GIT_TEMPLATE_DIR", test_git_template_dir())
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-c")
            .arg(format!("core.hooksPath={DISABLED_GIT_HOOKS_PATH}"));
        command
    }

    fn test_repository() -> (TestDirectory, PathBuf) {
        let root = std::env::temp_dir().join(format!("bkm-locks-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        assert!(test_git_command()
            .args(["-C", root.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap()
            .success());
        (TestDirectory(root.clone()), root)
    }

    #[test]
    fn inventories_standard_regular_git_lock_files() {
        let (_cleanup, repository) = test_repository();
        let git_directory = repository.join(".git");
        fs::write(git_directory.join("config.lock"), b"").unwrap();
        fs::write(git_directory.join("index.lock"), b"index").unwrap();
        fs::write(git_directory.join("HEAD.lock"), b"head").unwrap();
        fs::write(git_directory.join("REBASE_HEAD.lock"), b"rebase").unwrap();
        fs::create_dir_all(git_directory.join("refs/heads")).unwrap();
        fs::write(git_directory.join("refs/heads/main.lock"), b"ref").unwrap();

        let result = GitCore::new().repository_locks(&repository).unwrap();
        assert!(!result.incomplete);
        let locks = result.locks;
        assert_eq!(locks.len(), 4);
        assert!(locks
            .iter()
            .any(|lock| lock.kind == RepositoryLockKind::Configuration));
        assert!(locks
            .iter()
            .any(|lock| lock.kind == RepositoryLockKind::Index));
        assert!(locks
            .iter()
            .any(|lock| lock.kind == RepositoryLockKind::Reference));
        assert!(locks.iter().all(|lock| lock.id.len() == 64));
        assert!(!locks.iter().any(|lock| lock.path.ends_with("main.lock")));
    }

    #[cfg(unix)]
    #[test]
    fn inventory_does_not_follow_lock_symlinks() {
        use std::os::unix::fs::symlink;

        let (_cleanup, repository) = test_repository();
        let target = repository.join("target");
        fs::write(&target, b"keep").unwrap();
        symlink(&target, repository.join(".git/config.lock")).unwrap();

        assert!(GitCore::new()
            .repository_locks(&repository)
            .unwrap()
            .locks
            .is_empty());
        assert_eq!(fs::read(target).unwrap(), b"keep");
    }

    #[test]
    fn stale_lock_id_does_not_resolve_a_replacement_at_the_same_path() {
        let (_cleanup, repository) = test_repository();
        let lock_path = repository.join(".git/index.lock");
        let replaced_path = repository.join(".git/index.lock.replaced");
        fs::write(&lock_path, b"first").unwrap();
        let lock_id = GitCore::new().repository_locks(&repository).unwrap().locks[0]
            .id
            .clone();

        fs::rename(&lock_path, replaced_path).unwrap();
        fs::write(&lock_path, b"second").unwrap();

        let error = GitCore::new()
            .repository_lock_directory(&repository, &lock_id)
            .unwrap_err();
        assert_eq!(error.code(), "git_lock_changed");
    }

    #[test]
    fn linked_worktree_inventory_excludes_the_main_worktrees_private_locks() {
        let (_cleanup, repository) = test_repository();
        let linked = repository.join("linked");
        for (key, value) in [
            ("user.email", "locks@example.test"),
            ("user.name", "Lock Tests"),
        ] {
            assert!(test_git_command()
                .args(["-C", repository.to_str().unwrap(), "config", key, value])
                .status()
                .unwrap()
                .success());
        }
        fs::write(repository.join("tracked"), b"content").unwrap();
        assert!(test_git_command()
            .args(["-C", repository.to_str().unwrap(), "add", "tracked"])
            .status()
            .unwrap()
            .success());
        assert!(test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "commit",
                "-qm",
                "initial"
            ])
            .status()
            .unwrap()
            .success());
        assert!(test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "worktree",
                "add",
                "-q",
                "-b",
                "linked",
                linked.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success());

        let main_git_directory = repository.join(".git");
        fs::write(main_git_directory.join("index.lock"), b"main").unwrap();
        let git = GitCore::new();
        let linked_git_directory = git.git_directory(&linked).unwrap();
        fs::write(linked_git_directory.join("index.lock"), b"linked").unwrap();
        fs::write(main_git_directory.join("config.lock"), b"shared").unwrap();

        let linked_locks = git.repository_locks(&linked).unwrap().locks;
        assert_eq!(linked_locks.len(), 2);
        assert!(linked_locks
            .iter()
            .any(|lock| lock.path == linked_git_directory.join("index.lock").to_string_lossy()));
        assert!(linked_locks
            .iter()
            .any(|lock| lock.path == main_git_directory.join("config.lock").to_string_lossy()));
        assert!(!linked_locks
            .iter()
            .any(|lock| { lock.path == main_git_directory.join("index.lock").to_string_lossy() }));
    }
}
