use super::environment::*;
use super::error::*;
use super::validation::*;
use super::*;

const MAX_DISCOVERY_DIRECTORIES: usize = 5_000;
const MAX_DISCOVERY_ENTRIES: usize = 20_000;
const CLONE_STAGING_DIRECTORY_PREFIX: &str = ".bkm-clone-staging-";
const INIT_STAGING_DIRECTORY_PREFIX: &str = ".bkm-init-staging-";

impl GitCore {
    /// Uses `git` from the current process PATH.
    pub fn new() -> Self {
        Self {
            executable: PathBuf::from("git"),
        }
    }

    /// Allows tests or a packaged application to select a concrete Git binary.
    pub fn with_executable(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// Returns the configured executable path without attempting to run it.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Checks that the configured Git executable can be started.
    pub fn version(&self) -> Result<String, AppError> {
        let output = self.run(
            None,
            &[OsString::from("--version")],
            &GitEnvironment::default(),
            "检查 Git 版本",
        )?;
        Ok(output_text(&output.stdout).trim().to_owned())
    }

    /// Resolves a path inside a worktree to its repository root.
    pub fn repository_root(&self, path: &Path) -> Result<PathBuf, AppError> {
        match self.try_repository_root(path)? {
            Some(root) => Ok(root),
            None => Err(AppError::new(
                "not_a_git_repository",
                "所选路径不是 Git 工作区",
            )),
        }
    }

    /// Returns a display-ready record for a repository or a path within one.
    pub fn repository_record(&self, path: &Path) -> Result<RepositoryRecord, AppError> {
        let root = self.repository_root(path)?;
        repository_record_from_root(&root)
    }

    /// Finds worktree repositories below `root`, including `root` itself.
    ///
    /// `.git` directories and symbolic links are never descended into.  This
    /// prevents scanning the Git object database and avoids symlink cycles.
    /// Unreadable nested directories are skipped so a single protected folder
    /// does not hide repositories elsewhere in the selected root.
    pub fn discover_repositories(
        &self,
        root: &Path,
        max_depth: usize,
    ) -> Result<Vec<RepositoryRecord>, AppError> {
        let metadata = fs::symlink_metadata(root)
            .map_err(|_| AppError::new("invalid_repository_root", "无法读取仓库发现目录"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::new(
                "invalid_repository_root",
                "仓库发现目录必须是文件夹",
            ));
        }

        let mut pending = vec![(root.to_path_buf(), 0_usize)];
        let mut seen = HashSet::new();
        let mut repositories = Vec::new();
        let mut inspected_directories = 0_usize;
        let mut inspected_entries = 0_usize;

        while let Some((directory, depth)) = pending.pop() {
            inspected_directories += 1;
            if inspected_directories > MAX_DISCOVERY_DIRECTORIES {
                return Err(AppError::new(
                    "repository_discovery_limit_exceeded",
                    "仓库发现范围过大，请选择更具体的文件夹后重试。",
                ));
            }
            let git_entry = directory.join(".git");
            if matches!(fs::symlink_metadata(&git_entry), Ok(metadata) if !metadata.file_type().is_symlink())
            {
                if let Some(repository_root) = self.try_repository_root(&directory)? {
                    let normalized = normalize_path(&repository_root);
                    if let Ok(record) = repository_record_from_root(&normalized) {
                        if seen.insert(normalized) {
                            repositories.push(record);
                        }
                    }
                }
            }

            if depth >= max_depth {
                continue;
            }

            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries {
                inspected_entries += 1;
                if inspected_entries > MAX_DISCOVERY_ENTRIES {
                    return Err(AppError::new(
                        "repository_discovery_limit_exceeded",
                        "仓库发现范围过大，请选择更具体的文件夹后重试。",
                    ));
                }
                let Ok(entry) = entry else {
                    continue;
                };
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                if !file_type.is_dir()
                    || file_type.is_symlink()
                    || entry.file_name() == ".git"
                    || is_staging_directory_name(&entry.file_name())
                {
                    continue;
                }
                if inspected_directories + pending.len() >= MAX_DISCOVERY_DIRECTORIES {
                    return Err(AppError::new(
                        "repository_discovery_limit_exceeded",
                        "仓库发现范围过大，请选择更具体的文件夹后重试。",
                    ));
                }
                pending.push((entry.path(), depth + 1));
            }
        }

        repositories.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(repositories)
    }

    /// Alias retained for callers that use "list" wording in their command
    /// surface.
    pub fn list_repositories(
        &self,
        root: &Path,
        max_depth: usize,
    ) -> Result<Vec<RepositoryRecord>, AppError> {
        self.discover_repositories(root, max_depth)
    }

    /// Initializes an empty worktree repository at an explicit path.
    pub fn init_repository(
        &self,
        destination: &Path,
        initial_branch: Option<&str>,
    ) -> Result<RepositoryRecord, AppError> {
        if let Some(branch) = initial_branch {
            validate_ref_name(branch, "初始分支名")?;
        }
        let mut init_destination = prepare_init_destination(self, destination)?;
        let mut arguments = vec![OsString::from("init")];
        if let Some(branch) = initial_branch {
            arguments.push(OsString::from("--initial-branch"));
            arguments.push(OsString::from(branch));
        }
        arguments.push(OsString::from("--"));
        arguments.push(init_destination.staging_path().as_os_str().to_os_string());
        self.run(None, &arguments, &GitEnvironment::default(), "初始化仓库")?;
        let destination = init_destination.finalize()?;
        self.repository_record(&destination)
    }

    /// Clones a repository to an explicit destination.  HTTP(S) URLs carrying
    /// embedded credentials are rejected before they could reach a process
    /// argument or a Git config entry.
    pub fn clone_repository(
        &self,
        remote_url: &str,
        destination: &Path,
        options: &CloneOptions,
    ) -> Result<RepositoryRecord, AppError> {
        self.clone_repository_with_environment(
            remote_url,
            destination,
            options,
            &GitEnvironment::default(),
        )
    }

    /// Clones a repository with a backend-only credential environment.  This
    /// is used when the user has explicitly selected an account before the
    /// destination repository exists and can therefore have a remote binding.
    pub fn clone_repository_with_environment(
        &self,
        remote_url: &str,
        destination: &Path,
        options: &CloneOptions,
        environment: &GitEnvironment,
    ) -> Result<RepositoryRecord, AppError> {
        let remote_url = validate_git_remote_url(remote_url)?;
        let mut clone_destination = prepare_clone_destination(destination)?;
        if options.depth == Some(0) {
            return Err(AppError::new("invalid_clone_depth", "克隆深度必须大于 0"));
        }

        // Never let a selected credential continue into a submodule. A
        // top-level clone must also override `clone.recurseSubmodules` from
        // ambient configuration, then an explicit recursive update can run
        // with the ordinary credential environment below.
        let mut arguments = vec![
            OsString::from("clone"),
            OsString::from("--no-recurse-submodules"),
        ];
        if let Some(depth) = options.depth {
            arguments.push(OsString::from("--depth"));
            arguments.push(OsString::from(depth.to_string()));
        }
        if let Some(branch) = options.branch.as_deref() {
            validate_ref_name(branch, "分支名")?;
            arguments.push(OsString::from("--branch"));
            arguments.push(OsString::from(branch));
        }
        arguments.push(OsString::from("--"));
        arguments.push(OsString::from(remote_url));
        arguments.push(clone_destination.staging_path().as_os_str().to_os_string());
        self.run_network(None, &arguments, environment, "克隆仓库")?;

        if options.recursive {
            self.update_submodules(clone_destination.staging_path(), true, true)?;
        }

        // Moving a sibling directory into place is atomic on the same
        // filesystem. It never follows a newly-created destination symlink;
        // an occupied target instead fails without writing through it.
        let destination = clone_destination.finalize()?;

        self.repository_record(&destination)
    }
}

struct InitDestination {
    destination: PathBuf,
    staging: Option<PathBuf>,
    staging_parent: Option<InitStagingParent>,
}

enum InitStagingParent {
    /// The repository directory was created inside the private staging parent
    /// and can be removed recursively if initialization fails.
    AppCreated(OwnedCloneStagingDirectory),
    /// A user-selected existing directory was moved into private staging. On
    /// failure it must be restored when possible, never recursively deleted.
    ExistingDirectory(OwnedCloneStagingDirectory),
}

impl InitStagingParent {
    fn remove_after_success(self) {
        match self {
            Self::AppCreated(directory) => directory.remove_tree_if_still_owned(),
            Self::ExistingDirectory(directory) => {
                if directory.still_matches() {
                    let _ = fs::remove_dir(directory.path);
                }
            }
        }
    }
}

impl InitDestination {
    fn staging_path(&self) -> &Path {
        self.staging
            .as_deref()
            .expect("a prepared init destination always has staging")
    }

    fn finalize(&mut self) -> Result<PathBuf, AppError> {
        let staging = self
            .staging
            .as_ref()
            .expect("a prepared init destination always has staging");
        rename_directory_without_replacing(staging, &self.destination).map_err(|_| {
            AppError::new(
                "invalid_init_destination",
                "初始化期间目标目录已变化，已取消写入。",
            )
        })?;
        self.staging = None;
        if let Some(staging_parent) = self.staging_parent.take() {
            staging_parent.remove_after_success();
        }
        Ok(self.destination.clone())
    }
}

impl Drop for InitDestination {
    fn drop(&mut self) {
        let Some(staging_parent) = self.staging_parent.take() else {
            return;
        };
        match staging_parent {
            InitStagingParent::AppCreated(directory) => directory.remove_tree_if_still_owned(),
            InitStagingParent::ExistingDirectory(directory) => {
                if let Some(staging) = self.staging.take() {
                    // The final name may have been recreated by another
                    // process. No-replace restoration preserves both that
                    // entry and the original directory when recovery cannot
                    // be completed safely.
                    if rename_directory_without_replacing(&staging, &self.destination).is_err() {
                        return;
                    }
                }
                if directory.still_matches() {
                    let _ = fs::remove_dir(directory.path);
                }
            }
        }
    }
}

struct CloneDestination {
    destination: PathBuf,
    staging: Option<PathBuf>,
    staging_parent: Option<OwnedCloneStagingDirectory>,
}

/// Records the directory identity created by this process before Git is
/// allowed to write below it. Failed clones may leave a non-empty worktree, so
/// cleanup is recursive only when this identity still matches the directory
/// we created.
struct OwnedCloneStagingDirectory {
    path: PathBuf,
    identity: CloneStagingDirectoryIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneStagingDirectoryIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    owner: u32,
    #[cfg(not(unix))]
    created_at: Option<std::time::SystemTime>,
}

impl OwnedCloneStagingDirectory {
    fn capture(path: PathBuf) -> std::io::Result<Self> {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "clone staging path is not an owned directory",
            ));
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;

            // SAFETY: `geteuid` has no arguments, does not dereference memory,
            // and cannot violate Rust memory safety.
            let effective_user_id = unsafe { libc::geteuid() };
            if metadata.uid() != effective_user_id {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "clone staging directory has an unexpected owner",
                ));
            }
            CloneStagingDirectoryIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                owner: metadata.uid(),
            }
        };
        #[cfg(not(unix))]
        let identity = CloneStagingDirectoryIdentity {
            // A staging parent gains and later loses its `repository` child,
            // both of which change its modification timestamp. Creation time
            // remains stable across those expected mutations and lets normal
            // cleanup proceed instead of leaking every staging directory.
            created_at: metadata.created().ok(),
        };
        Ok(Self { path, identity })
    }

    fn still_matches(&self) -> bool {
        #[cfg(unix)]
        {
            Self::capture(self.path.clone())
                .map(|current| current.identity == self.identity)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            // A missing creation timestamp is not a stable identity. Keeping
            // a private staging directory in that uncommon case is safer
            // than recursively deleting a pathname an attacker replaced on a
            // filesystem that cannot report its creation time.
            matches!(
                (&self.identity.created_at, Self::capture(self.path.clone())),
                (Some(expected), Ok(current))
                    if current.identity.created_at.as_ref() == Some(expected)
            )
        }
    }

    fn remove_tree_if_still_owned(self) {
        if self.still_matches() {
            // This directory was created with a random name and owner-only
            // permissions. `remove_dir_all` removes a nested symlink itself
            // rather than following it, and identity verification above keeps
            // it from being applied to a replacement directory.
            let _ = fs::remove_dir_all(self.path);
        }
    }
}

impl CloneDestination {
    fn staging_path(&self) -> &Path {
        self.staging
            .as_deref()
            .expect("a prepared clone destination always has staging")
    }

    fn finalize(&mut self) -> Result<PathBuf, AppError> {
        let staging = self
            .staging
            .as_ref()
            .expect("a prepared clone destination always has staging");
        match fs::symlink_metadata(&self.destination) {
            Ok(_) => {
                return Err(AppError::new(
                    "invalid_clone_destination",
                    "克隆目标路径在操作期间被创建，已取消写入。",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(AppError::new(
                    "invalid_clone_destination",
                    "无法访问克隆目标目录。",
                ));
            }
        }
        rename_directory_without_replacing(staging, &self.destination).map_err(|_| {
            AppError::new(
                "invalid_clone_destination",
                "无法安全完成克隆目标目录的写入。",
            )
        })?;
        self.staging = None;
        if let Some(staging_parent) = self.staging_parent.take() {
            staging_parent.remove_tree_if_still_owned();
        }
        Ok(self.destination.clone())
    }
}

/// Atomically publishes an app-created staging directory without ever
/// replacing a name recreated by another process. `std::fs::rename` replaces
/// an empty directory on Unix, which would turn the check just above into a
/// check-then-act race.
#[cfg(target_os = "linux")]
pub(super) fn rename_directory_without_replacing(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are NUL-terminated and remain live for the call.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn rename_directory_without_replacing(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renamex_np(
            from: *const libc::c_char,
            to: *const libc::c_char,
            flags: u32,
        ) -> libc::c_int;
    }
    const RENAME_EXCL: u32 = 0x0000_0004;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are NUL-terminated and remain live for the call.
    let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn rename_directory_without_replacing(
    source: &Path,
    destination: &Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // No `MOVEFILE_REPLACE_EXISTING` flag: the OS must reject a destination
    // that appeared after our validation instead of replacing it.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn rename_directory_without_replacing(
    _source: &Path,
    _destination: &Path,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace directory rename is unavailable on this platform",
    ))
}

impl Drop for CloneDestination {
    fn drop(&mut self) {
        self.staging.take();
        if let Some(staging_parent) = self.staging_parent.take() {
            staging_parent.remove_tree_if_still_owned();
        }
    }
}

fn prepare_clone_destination(destination: &Path) -> Result<CloneDestination, AppError> {
    if destination.as_os_str().is_empty() {
        return Err(AppError::new(
            "invalid_clone_destination",
            "克隆目标目录不能为空",
        ));
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = absolute_path(parent)?;
    ensure_no_symlink_path_components(&parent)?;
    let parent = fs::canonicalize(parent)
        .map_err(|_| AppError::new("invalid_clone_destination", "无法访问克隆目标的父目录。"))?;
    let parent_metadata = fs::symlink_metadata(&parent)
        .map_err(|_| AppError::new("invalid_clone_destination", "无法访问克隆目标的父目录。"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(AppError::new(
            "invalid_clone_destination",
            "克隆目标的父路径必须是文件夹。",
        ));
    }
    ensure_clone_parent_is_safe(&parent)?;
    let name = destination
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "invalid_clone_destination",
                "克隆目标必须是父目录下的一个文件夹。",
            )
        })?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(AppError::new(
                "invalid_clone_destination",
                "克隆目标路径必须是文件夹，且不能是符号链接",
            ));
        }
        Ok(_) => {
            return Err(AppError::new(
                "invalid_clone_destination",
                "克隆目标目录必须不存在。",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(AppError::new(
                "invalid_clone_destination",
                "无法访问克隆目标目录",
            ))
        }
    }
    for _ in 0..16 {
        // Keep the clone target inside a private, app-created sibling
        // directory for the whole Git subprocess. A parent selected from a
        // shared location can otherwise let another user replace the empty
        // staging name with a symlink between validation and `git clone`.
        let staging_parent_path = parent.join(format!(
            "{CLONE_STAGING_DIRECTORY_PREFIX}{}",
            uuid::Uuid::new_v4()
        ));
        match create_private_staging_directory(&staging_parent_path) {
            Ok(()) => {
                let staging_parent =
                    match OwnedCloneStagingDirectory::capture(staging_parent_path.clone()) {
                        Ok(staging_parent) => staging_parent,
                        Err(_) => {
                            // Identity capture failed before Git could write any
                            // content. Only try a non-recursive removal here: a
                            // replacement must never be deleted on this path.
                            let _ = fs::remove_dir(&staging_parent_path);
                            continue;
                        }
                    };
                let staging = staging_parent.path.join("repository");
                if create_private_staging_directory(&staging).is_err() {
                    staging_parent.remove_tree_if_still_owned();
                    continue;
                }
                return Ok(CloneDestination {
                    destination,
                    staging: Some(staging),
                    staging_parent: Some(staging_parent),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(AppError::new(
                    "invalid_clone_destination",
                    "无法创建安全的克隆暂存目录。",
                ));
            }
        }
    }
    Err(AppError::new(
        "invalid_clone_destination",
        "无法创建安全的克隆暂存目录。",
    ))
}

/// Resolves an initialization target into a private staging directory. Git is
/// never given the user-controlled final pathname: for a new destination the
/// initialized staging directory is published atomically, while an existing
/// directory is first moved into private staging and identity-checked.
fn prepare_init_destination(
    git: &GitCore,
    destination: &Path,
) -> Result<InitDestination, AppError> {
    if destination.as_os_str().is_empty() {
        return Err(AppError::new(
            "invalid_init_destination",
            "新仓库目录不能为空",
        ));
    }

    // `git init .` is a normal existing-directory workflow. Resolve it to a
    // canonical directory before separating its parent and final name so it
    // receives the same staging protection as an explicit absolute path.
    let destination = if destination == Path::new(".") {
        let absolute = absolute_path(destination).map_err(|_| {
            AppError::new("invalid_init_destination", "无法访问新仓库目录的父路径。")
        })?;
        ensure_no_symlink_path_components(&absolute).map_err(|_| {
            AppError::new(
                "invalid_init_destination",
                "新仓库的父路径不能包含符号链接。",
            )
        })?;
        fs::canonicalize(absolute)
            .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录。"))?
    } else {
        destination.to_path_buf()
    };
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = absolute_path(parent)
        .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录的父路径。"))?;
    ensure_no_symlink_path_components(&parent).map_err(|_| {
        AppError::new(
            "invalid_init_destination",
            "新仓库的父路径不能包含符号链接。",
        )
    })?;
    let parent = fs::canonicalize(&parent)
        .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录的父路径。"))?;
    ensure_init_parent_is_safe(&parent)?;
    let name = destination
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "invalid_init_destination",
                "新仓库必须位于一个已存在的父目录下。",
            )
        })?;
    let destination = parent.join(name);

    match fs::symlink_metadata(&destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::new(
                    "invalid_init_destination",
                    "新仓库路径必须是文件夹",
                ));
            }
            if git.try_repository_root(&destination)?.is_some() {
                return Err(AppError::new(
                    "already_git_repository",
                    "所选目录已经是 Git 仓库",
                ));
            }
            let original =
                OwnedCloneStagingDirectory::capture(destination.clone()).map_err(|_| {
                    AppError::new("invalid_init_destination", "无法安全锁定新仓库目录。")
                })?;
            let staging_parent = create_init_staging_parent(&parent)?;
            let staging = staging_parent.path.join("repository");
            fs::rename(&destination, &staging).map_err(|_| {
                AppError::new("invalid_init_destination", "新仓库目录在初始化期间已变化。")
            })?;
            let moved = OwnedCloneStagingDirectory::capture(staging.clone());
            if !matches!(moved, Ok(ref moved) if moved.identity == original.identity) {
                let _ = rename_directory_without_replacing(&staging, &destination);
                if staging_parent.still_matches() {
                    let _ = fs::remove_dir(&staging_parent.path);
                }
                return Err(AppError::new(
                    "invalid_init_destination",
                    "新仓库目录在初始化期间已变化。",
                ));
            }
            Ok(InitDestination {
                destination,
                staging: Some(staging),
                staging_parent: Some(InitStagingParent::ExistingDirectory(staging_parent)),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let staging_parent = create_init_staging_parent(&parent)?;
            let staging = staging_parent.path.join("repository");
            if create_private_staging_directory(&staging).is_err() {
                staging_parent.remove_tree_if_still_owned();
                return Err(AppError::new(
                    "invalid_init_destination",
                    "无法创建安全的初始化暂存目录。",
                ));
            }
            Ok(InitDestination {
                destination,
                staging: Some(staging),
                staging_parent: Some(InitStagingParent::AppCreated(staging_parent)),
            })
        }
        Err(_) => Err(AppError::new(
            "invalid_init_destination",
            "无法访问新仓库目录",
        )),
    }
}

fn create_init_staging_parent(parent: &Path) -> Result<OwnedCloneStagingDirectory, AppError> {
    for _ in 0..16 {
        let path = parent.join(format!(
            "{INIT_STAGING_DIRECTORY_PREFIX}{}",
            uuid::Uuid::new_v4()
        ));
        match create_private_staging_directory(&path) {
            Ok(()) => match OwnedCloneStagingDirectory::capture(path.clone()) {
                Ok(directory) => return Ok(directory),
                Err(_) => {
                    let _ = fs::remove_dir(&path);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => break,
        }
    }
    Err(AppError::new(
        "invalid_init_destination",
        "无法创建安全的初始化暂存目录。",
    ))
}

fn is_staging_directory_name(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        [
            CLONE_STAGING_DIRECTORY_PREFIX,
            INIT_STAGING_DIRECTORY_PREFIX,
        ]
        .iter()
        .filter_map(|prefix| name.strip_prefix(prefix))
        .any(|suffix| uuid::Uuid::parse_str(suffix).is_ok())
    })
}

/// On Unix, another user who can rename entries in the selected parent could
/// replace a private staging directory with a symlink before Git resolves the
/// destination path. A sticky shared directory protects an entry only when
/// its owner is trusted (this user or root); its untrusted owner could still
/// rename any child entry.
#[cfg(unix)]
fn ensure_clone_parent_is_safe(parent: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(parent)
        .map_err(|_| AppError::new("invalid_clone_destination", "无法访问克隆目标目录。"))?;
    let mode = metadata.permissions().mode();
    let writable_by_other_users = mode & 0o022 != 0;
    let sticky_directory = mode & 0o1000 != 0;
    // SAFETY: `geteuid` has no arguments and cannot violate Rust memory safety.
    let effective_user_id = unsafe { libc::geteuid() };
    let trusted_owner = metadata.uid() == effective_user_id || metadata.uid() == 0;
    if writable_by_other_users && (!sticky_directory || !trusted_owner) {
        return Err(AppError::new(
            "invalid_clone_destination",
            "克隆目标的父目录可被其他用户写入，请选择私有目录。",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_clone_parent_is_safe(_parent: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(unix)]
fn ensure_init_parent_is_safe(parent: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(parent)
        .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录。"))?;
    let mode = metadata.permissions().mode();
    // SAFETY: `geteuid` has no arguments and cannot violate Rust memory safety.
    let effective_user_id = unsafe { libc::geteuid() };
    let trusted_owner = metadata.uid() == effective_user_id || metadata.uid() == 0;
    if mode & 0o022 != 0 && (mode & 0o1000 == 0 || !trusted_owner) {
        return Err(AppError::new(
            "invalid_init_destination",
            "新仓库的父目录可被其他用户写入，请选择私有目录。",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_init_parent_is_safe(_parent: &Path) -> Result<(), AppError> {
    Ok(())
}

/// Creates a staging directory that is owner-only from the creation syscall
/// onward. This protects the path passed to Git from other local users while
/// it is still empty; publication still uses an atomic no-replace rename.
#[cfg(unix)]
fn create_private_staging_directory(path: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `path` is NUL-terminated and remains live for this syscall.
    if unsafe { libc::mkdir(path.as_ptr(), 0o700) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn create_private_staging_directory(path: &Path) -> std::io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    };
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // A protected DACL with an Owner Rights ACE gives only the account that
    // created the directory full access. Creating it with this descriptor is
    // atomic, so an inherited permissive ACL can never expose the empty path
    // to a different local user before Git starts writing into it.
    let descriptor_text = "D:P(A;;GA;;;OW)\0".encode_utf16().collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: both output pointers are valid for the duration of the call;
    // Windows allocates the returned descriptor and LocalFree below releases
    // it after CreateDirectoryW has consumed it.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: the NUL-terminated path and security descriptor remain valid
    // through this synchronous directory-creation call.
    let created = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
    let error = if created == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated
    // this local-memory descriptor exactly once above.
    unsafe {
        let _ = LocalFree(descriptor);
    }
    error.map_or(Ok(()), Err)
}

#[cfg(all(not(unix), not(windows)))]
fn create_private_staging_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

fn absolute_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current_directory| current_directory.join(path))
            .map_err(|_| AppError::new("invalid_clone_destination", "无法访问克隆目标的父目录。"))
    }
}

/// Reject a user-selected destination whose original parent traversal enters a
/// symbolic link. Canonicalizing first would hide that link and let clone
/// publish beneath a different hierarchy than the user selected.
fn ensure_no_symlink_path_components(path: &Path) -> Result<(), AppError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            std::path::Component::RootDir => current.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                current.pop();
            }
            std::path::Component::Normal(part) => {
                current.push(part);
                let metadata = fs::symlink_metadata(&current).map_err(|_| {
                    AppError::new("invalid_clone_destination", "无法访问克隆目标的父目录。")
                })?;
                if metadata.file_type().is_symlink() {
                    return Err(AppError::new(
                        "invalid_clone_destination",
                        "克隆目标的父路径不能包含符号链接。",
                    ));
                }
            }
        }
    }
    Ok(())
}
