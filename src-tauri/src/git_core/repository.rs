use super::environment::*;
use super::error::*;
use super::validation::*;
use super::*;

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
        Ok(repository_record_from_root(&root))
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
        let metadata = fs::metadata(root)
            .map_err(|_| AppError::new("invalid_repository_root", "无法读取仓库发现目录"))?;
        if !metadata.is_dir() {
            return Err(AppError::new(
                "invalid_repository_root",
                "仓库发现目录必须是文件夹",
            ));
        }

        let mut pending = vec![(root.to_path_buf(), 0_usize)];
        let mut seen = HashSet::new();
        let mut repositories = Vec::new();

        while let Some((directory, depth)) = pending.pop() {
            if directory.join(".git").exists() {
                if let Some(repository_root) = self.try_repository_root(&directory)? {
                    let normalized = normalize_path(&repository_root);
                    if seen.insert(normalized.clone()) {
                        repositories.push(repository_record_from_root(&normalized));
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
            for entry in entries.flatten() {
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(_) => continue,
                };
                if !file_type.is_dir() || file_type.is_symlink() || entry.file_name() == ".git" {
                    continue;
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
        if destination.as_os_str().is_empty() {
            return Err(AppError::new(
                "invalid_init_destination",
                "新仓库目录不能为空",
            ));
        }
        if destination.exists() {
            let metadata = fs::metadata(destination)
                .map_err(|_| AppError::new("invalid_init_destination", "无法访问新仓库目录"))?;
            if !metadata.is_dir() {
                return Err(AppError::new(
                    "invalid_init_destination",
                    "新仓库路径必须是文件夹",
                ));
            }
            if self.try_repository_root(destination)?.is_some() {
                return Err(AppError::new(
                    "already_git_repository",
                    "所选目录已经是 Git 仓库",
                ));
            }
        }

        let mut arguments = vec![OsString::from("init")];
        if let Some(branch) = initial_branch {
            validate_ref_name(branch, "初始分支名")?;
            arguments.push(OsString::from("--initial-branch"));
            arguments.push(OsString::from(branch));
        }
        arguments.push(OsString::from("--"));
        arguments.push(destination.as_os_str().to_os_string());
        self.run(None, &arguments, &GitEnvironment::default(), "初始化仓库")?;
        self.repository_record(destination)
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
        validate_git_remote_url(remote_url)?;
        if destination.as_os_str().is_empty() {
            return Err(AppError::new(
                "invalid_clone_destination",
                "克隆目标目录不能为空",
            ));
        }
        if options.depth == Some(0) {
            return Err(AppError::new("invalid_clone_depth", "克隆深度必须大于 0"));
        }

        let mut arguments = vec![OsString::from("clone")];
        if options.recursive {
            arguments.push(OsString::from("--recurse-submodules"));
        }
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
        arguments.push(destination.as_os_str().to_os_string());
        self.run_network(None, &arguments, environment, "克隆仓库")?;

        self.repository_record(destination)
    }
}
