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
        let arguments = os_arguments(&[
            "-c",
            "core.quotePath=true",
            "submodule",
            "status",
            "--recursive",
        ]);
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
        let repository_root = self.repository_root(repository)?;
        self.ensure_network_configuration_is_safe(&repository_root)?;
        self.ensure_initialized_submodule_network_configurations_are_safe(
            &repository_root,
            recursive,
        )?;
        let mut arguments = os_arguments(&["submodule", "update"]);
        if initialize {
            arguments.push(OsString::from("--init"));
        }
        if recursive {
            arguments.push(OsString::from("--recursive"));
        }
        // Submodule update procedures and URLs are repository-controlled.
        // Do not inherit one parent account's single-use askpass capability;
        // Git may use the user's ordinary credential source, but repository
        // transport, hook, and custom-protocol settings are neutralized for
        // the spawned process.
        let environment = self.unbound_network_environment()?;
        self.run_network(
            Some(&repository_root),
            &arguments,
            &environment,
            "初始化或更新子模块",
        )?;
        Ok(())
    }

    /// Returns the repository roots that an update can mutate, including the
    /// parent worktree and every initialized configured submodule reachable at
    /// the requested depth. Command handlers use these canonical paths for a
    /// single operation lease before starting Git.
    pub fn initialized_submodule_repository_paths(
        &self,
        repository: &Path,
        recursive: bool,
    ) -> Result<Vec<String>, AppError> {
        Ok(self
            .initialized_submodule_roots(repository, recursive)?
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect())
    }

    /// Returns every repository path a submodule update may mutate. In
    /// addition to initialized child repositories, this reserves configured
    /// destinations that `--init` can create while the update is running.
    /// Their lexical paths are rooted at an already canonical parent, so a
    /// command addressed to the child after initialization acquires the same
    /// operation-lease key.
    pub fn submodule_operation_repository_paths(
        &self,
        repository: &Path,
        recursive: bool,
    ) -> Result<Vec<String>, AppError> {
        let roots = self.initialized_submodule_roots(repository, recursive)?;
        let mut paths = roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        for (index, root) in roots.iter().enumerate() {
            // A non-recursive update only touches the requested superproject
            // and its direct children. With --recursive, every initialized
            // child can initialize its own configured descendants as well.
            if index != 0 && !recursive {
                continue;
            }
            for relative_path in self.configured_submodule_paths(root)? {
                paths.push(
                    self.submodule_operation_destination(root, &relative_path)?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        paths.sort_unstable();
        paths.dedup();
        Ok(paths)
    }

    /// A recursive update runs Git inside already initialized nested
    /// repositories. Read configured paths from `.gitmodules` instead of
    /// parsing `git submodule status`'s display text, then check each
    /// initialized child's local configuration before an app-selected
    /// credential environment can reach those child processes.
    fn ensure_initialized_submodule_network_configurations_are_safe(
        &self,
        repository: &Path,
        recursive: bool,
    ) -> Result<(), AppError> {
        for submodule_root in self
            .initialized_submodule_roots(repository, recursive)?
            .into_iter()
            .skip(1)
        {
            self.ensure_network_configuration_is_safe(&submodule_root)?;
        }
        Ok(())
    }

    fn initialized_submodule_roots(
        &self,
        repository: &Path,
        recursive: bool,
    ) -> Result<Vec<PathBuf>, AppError> {
        let repository_root = self.repository_root(repository)?;
        let canonical_repository = fs::canonicalize(&repository_root)
            .map_err(|_| AppError::new("unsafe_git_config", "无法安全检查子模块网络配置。"))?;
        let mut roots = vec![canonical_repository.clone()];
        let mut seen = HashSet::from([canonical_repository]);
        self.collect_initialized_submodule_roots(&mut roots, &mut seen, recursive)?;
        Ok(roots)
    }

    fn collect_initialized_submodule_roots(
        &self,
        roots: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        recursive: bool,
    ) -> Result<(), AppError> {
        let Some(repository) = roots.last().cloned() else {
            return Ok(());
        };
        let configured_paths = self.configured_submodule_paths(&repository)?;
        for relative_path in configured_paths {
            validate_worktree_relative_path(&relative_path)?;
            let submodule_path = repository.join(&relative_path);
            let metadata = match fs::symlink_metadata(&submodule_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err(AppError::new(
                        "unsafe_git_config",
                        "无法安全检查子模块网络配置。",
                    ));
                }
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::new(
                    "unsafe_git_config",
                    "子模块路径不安全，拒绝使用其网络配置。",
                ));
            }
            let canonical_submodule = fs::canonicalize(&submodule_path)
                .map_err(|_| AppError::new("unsafe_git_config", "无法安全检查子模块网络配置。"))?;
            if !canonical_submodule.starts_with(&repository) {
                return Err(AppError::new(
                    "unsafe_git_config",
                    "子模块路径不安全，拒绝使用其网络配置。",
                ));
            }
            let Some(submodule_root) = self
                .try_repository_root(&canonical_submodule)
                .map_err(|_| AppError::new("unsafe_git_config", "无法安全检查子模块网络配置。"))?
            else {
                // `git submodule deinit` can leave an empty directory behind.
                continue;
            };
            if submodule_root != canonical_submodule || !seen.insert(submodule_root.clone()) {
                // A plain directory inside its parent resolves back to the
                // parent's root. Do not recurse into it or create a cycle.
                continue;
            }
            roots.push(submodule_root.clone());
            if recursive {
                self.collect_initialized_submodule_roots(roots, seen, true)?;
            }
        }
        Ok(())
    }

    fn configured_submodule_paths(&self, repository: &Path) -> Result<Vec<String>, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "config",
                "--no-includes",
                "--null",
                "--file",
                ".gitmodules",
                "--get-regexp",
                "^submodule\\..*\\.path$",
            ]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            return parse_submodule_config_paths(&output.stdout);
        }
        if output.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        Err(git_command_error("读取子模块配置", &output))
    }

    fn submodule_operation_destination(
        &self,
        repository: &Path,
        relative_path: &str,
    ) -> Result<PathBuf, AppError> {
        validate_worktree_relative_path(relative_path)?;
        let destination = repository.join(relative_path);
        let mut current = repository.to_path_buf();
        let components = Path::new(relative_path)
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(component) => Some(component),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(AppError::new(
                        "unsafe_git_config",
                        "子模块路径不安全，拒绝初始化。",
                    ));
                }
                Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                    return Err(AppError::new(
                        "unsafe_git_config",
                        "子模块路径不安全，拒绝初始化。",
                    ));
                }
                Ok(metadata) if index + 1 == components.len() && !metadata.is_dir() => {
                    return Err(AppError::new(
                        "unsafe_git_config",
                        "子模块路径不安全，拒绝初始化。",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // Missing components will be created by `git submodule
                    // update --init`; the canonical superproject root makes
                    // this future lease key stable.
                    return Ok(destination);
                }
                Err(_) => {
                    return Err(AppError::new(
                        "unsafe_git_config",
                        "无法安全检查子模块路径。",
                    ));
                }
            }
        }
        let canonical = fs::canonicalize(&destination)
            .map_err(|_| AppError::new("unsafe_git_config", "无法安全检查子模块路径。"))?;
        if !canonical.starts_with(repository) {
            return Err(AppError::new(
                "unsafe_git_config",
                "子模块路径不安全，拒绝初始化。",
            ));
        }
        Ok(canonical)
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
        let initialized = if config_output.status.success() {
            true
        } else if config_output.status.code() == Some(1) {
            false
        } else {
            return Err(git_command_error("检查 Git LFS 配置", &config_output));
        };
        let files_output = self.run(
            Some(repository),
            &os_arguments(&["lfs", "ls-files", "--name-only"]),
            &GitEnvironment::default(),
            "读取 Git LFS 文件",
        )?;
        Ok(LfsStatus {
            available: true,
            initialized,
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
