use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

impl GitCore {
    /// Lists the main and linked worktrees that share this repository's Git
    /// metadata. Porcelain with NUL separators preserves paths containing
    /// whitespace or newlines.
    pub fn worktrees(&self, repository: &Path) -> Result<Vec<WorktreeInfo>, AppError> {
        let arguments = os_arguments(&["worktree", "list", "--porcelain", "-z"]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取工作树",
        )?;
        parse_worktrees(&output.stdout)
    }

    /// Adds a linked worktree at an explicit destination. A caller may create
    /// a new branch at the current HEAD, or check out one explicit revision.
    pub fn add_worktree(
        &self,
        repository: &Path,
        path: &Path,
        create_branch: Option<&str>,
        commitish: Option<&str>,
    ) -> Result<(), AppError> {
        validate_worktree_path(path)?;
        if create_branch.is_none() && commitish.is_none() {
            return Err(AppError::new(
                "invalid_worktree_config",
                "请指定要创建的新分支或要检出的分支",
            ));
        }
        if let Some(branch) = create_branch {
            validate_ref_name(branch, "新工作树分支名")?;
        }
        if let Some(revision) = commitish {
            validate_revision(revision, "工作树引用")?;
        }
        let mut arguments = os_arguments(&["worktree", "add"]);
        if let Some(branch) = create_branch {
            arguments.push(OsString::from("-b"));
            arguments.push(OsString::from(branch));
        }
        arguments.push(path.as_os_str().to_os_string());
        if let Some(revision) = commitish {
            arguments.push(OsString::from(revision));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建工作树",
        )?;
        Ok(())
    }

    /// Removes a linked worktree. The main worktree and the active worktree
    /// are deliberately blocked even if a stale UI tried to request them.
    pub fn remove_worktree(
        &self,
        repository: &Path,
        path: &Path,
        force: bool,
    ) -> Result<(), AppError> {
        validate_worktree_path(path)?;
        let worktree = self
            .worktrees(repository)?
            .into_iter()
            .find(|worktree| paths_match(Path::new(&worktree.path), path))
            .ok_or_else(|| AppError::new("worktree_not_found", "找不到要移除的工作树"))?;
        if worktree.is_main {
            return Err(AppError::new(
                "cannot_remove_main_worktree",
                "不能移除主工作树",
            ));
        }
        let active_worktree_root = self.repository_root(repository)?;
        if paths_match(&active_worktree_root, path) {
            return Err(AppError::new(
                "cannot_remove_active_worktree",
                "请先切换到其他工作树，再移除当前工作树",
            ));
        }
        let mut arguments = os_arguments(&["worktree", "remove"]);
        if force {
            arguments.push(OsString::from("--force"));
        }
        arguments.push(path.as_os_str().to_os_string());
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "移除工作树",
        )?;
        Ok(())
    }

    /// Lists initialized, uninitialized, stale, and conflicted submodule
    /// checkouts. Git's leading status marker is retained as a typed state.
    pub fn submodules(&self, repository: &Path) -> Result<Vec<SubmoduleInfo>, AppError> {
        let arguments = os_arguments(&["submodule", "status", "--recursive"]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取子模块",
        )?;
        parse_submodules(&output.stdout)
    }

    /// Initializes and/or updates all submodules recursively. This preserves
    /// the repository's configured submodule URLs and normal Git credential
    /// behavior, including any explicit remote binding needed for fetches.
    pub fn update_submodules(
        &self,
        repository: &Path,
        initialize: bool,
        recursive: bool,
    ) -> Result<(), AppError> {
        self.update_submodules_with_environment(
            repository,
            initialize,
            recursive,
            &GitEnvironment::default(),
        )
    }

    /// Initializes and/or updates submodules with the credential environment
    /// selected by the command layer. Askpass additionally verifies the host
    /// in every prompt before returning a credential.
    pub fn update_submodules_with_environment(
        &self,
        repository: &Path,
        initialize: bool,
        recursive: bool,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let mut arguments = os_arguments(&["submodule", "update"]);
        if initialize {
            arguments.push(OsString::from("--init"));
        }
        if recursive {
            arguments.push(OsString::from("--recursive"));
        }
        self.run_network(
            Some(repository),
            &arguments,
            environment,
            "初始化或更新子模块",
        )?;
        Ok(())
    }

    /// Detects Git LFS without treating an optional LFS installation as a Git
    /// error. When available, the result includes paths currently managed by
    /// LFS and whether its filter is configured for this repository.
    pub fn lfs_status(&self, repository: &Path) -> Result<LfsStatus, AppError> {
        let version_output = self.execute(
            Some(repository),
            &os_arguments(&["lfs", "version"]),
            &GitEnvironment::default(),
        )?;
        if !version_output.status.success() && lfs_is_unavailable(&version_output) {
            return Ok(LfsStatus {
                available: false,
                initialized: false,
                version: None,
                files: Vec::new(),
            });
        }
        if !version_output.status.success() {
            return Err(git_command_error("检查 Git LFS", &version_output));
        }
        let config_output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--get", "filter.lfs.process"]),
            &GitEnvironment::default(),
        )?;
        let files_output = self.run(
            Some(repository),
            &os_arguments(&["lfs", "ls-files", "--name-only"]),
            &GitEnvironment::default(),
            "读取 Git LFS 文件",
        )?;
        Ok(LfsStatus {
            available: true,
            initialized: config_output.status.success(),
            version: Some(output_text(&version_output.stdout).trim().to_owned())
                .filter(|version| !version.is_empty()),
            files: parse_lfs_files(&files_output.stdout),
        })
    }

    /// Configures Git LFS for this repository only, leaving global Git
    /// configuration unchanged.
    pub fn install_lfs(&self, repository: &Path) -> Result<(), AppError> {
        if !self.lfs_status(repository)?.available {
            return Err(AppError::new("lfs_unavailable", "系统未安装可用的 Git LFS"));
        }
        self.run(
            Some(repository),
            &os_arguments(&["lfs", "install", "--local"]),
            &GitEnvironment::default(),
            "初始化 Git LFS",
        )?;
        Ok(())
    }

    /// Registers one path pattern with Git LFS. Git LFS updates
    /// .gitattributes; the caller refreshes the normal repository snapshot so
    /// that change can be reviewed and committed by the user.
    pub fn track_lfs_pattern(&self, repository: &Path, pattern: &str) -> Result<(), AppError> {
        if !self.lfs_status(repository)?.available {
            return Err(AppError::new("lfs_unavailable", "系统未安装可用的 Git LFS"));
        }
        validate_lfs_pattern(pattern)?;
        let arguments = os_arguments(&["lfs", "track", pattern.trim()]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "添加 Git LFS 跟踪规则",
        )?;
        Ok(())
    }
}

fn lfs_is_unavailable(output: &Output) -> bool {
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    contains_any(
        &diagnostic,
        &[
            "is not a git command",
            "unknown subcommand: lfs",
            "git-lfs: command not found",
        ],
    )
}
