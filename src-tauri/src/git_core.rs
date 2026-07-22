//! Safe, system-Git based repository operations.
//!
//! `GitCore` deliberately has no knowledge of CodeHost accounts or tokens.  A
//! caller that has resolved a [`crate::domain::CredentialStrategy`] can pass a
//! backend-only [`GitEnvironment`] to the network methods.  The environment is
//! not serializable and is never logged by this module.

use crate::app_error::AppError;
use crate::domain::{
    endpoint_for_host, parse_remote_url, sanitize_remote_url, BranchInfo, CodeHostEndpoint,
    CodeHostKind, CommitSummary, GitOperationKind, GitStatus, GitStatusEntry, LfsStatus,
    RemoteProfile, RepositoryRecord, ResetMode, StashEntry, SubmoduleInfo, SubmoduleStatus,
    TagInfo, WorktreeInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const NETWORK_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[cfg(windows)]
const DISABLED_GIT_HOOKS_PATH: &str = "NUL";
#[cfg(not(windows))]
const DISABLED_GIT_HOOKS_PATH: &str = "/dev/null";

/// A small wrapper around the system `git` executable.
///
/// Every invocation is built with `Command` arguments.  This type never
/// invokes a shell, which keeps repository paths, commit messages, refspecs,
/// and remote names from being interpreted as shell code.
#[derive(Debug, Clone)]
pub struct GitCore {
    executable: PathBuf,
}

impl Default for GitCore {
    fn default() -> Self {
        Self::new()
    }
}

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
        parse_status_porcelain_v2(&output.stdout)
    }

    /// Reads an unstaged or staged patch.  File paths are placed after `--` so
    /// they cannot be interpreted as Git options. For one untracked file,
    /// ordinary `git diff` is empty, so we render it as an addition from
    /// `/dev/null` using Git's `--no-index` mode.
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

    fn is_untracked_path(&self, repository: &Path, path: &str) -> Result<bool, AppError> {
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

    fn untracked_file_diff(
        &self,
        repository: &Path,
        path: &str,
        _context_lines: Option<u32>,
    ) -> Result<String, AppError> {
        let bytes = read_untracked_file_safely(repository, path)?;
        Ok(render_untracked_file_diff(path, &bytes))
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
        patch: &str,
        reverse: bool,
    ) -> Result<(), AppError> {
        validate_git_patch(patch)?;
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

    /// Discards all indexed and working-tree changes for the selected status
    /// entries. Purely untracked entries are removed directly, while tracked
    /// entries go through Git so renames, the index, and file modes remain
    /// consistent. Callers must ask the user for confirmation before this.
    pub fn discard_paths(&self, repository: &Path, paths: &[String]) -> Result<(), AppError> {
        validate_paths(paths)?;
        for path in paths {
            validate_worktree_relative_path(path)?;
        }

        let status = self.status(repository)?;
        let mut seen = HashSet::new();
        let mut tracked_seen = HashSet::new();
        let mut tracked_paths = Vec::new();
        let mut paths_to_remove = Vec::new();
        for path in paths {
            if !seen.insert(path.as_str()) {
                continue;
            }
            let entry = status
                .entries
                .iter()
                .find(|entry| entry.path == *path)
                .ok_or_else(|| {
                    AppError::new("nothing_to_discard", "所选文件已经没有可丢弃的改动")
                })?;
            if entry.index_status.is_empty() && entry.worktree_status == "?" {
                paths_to_remove.push(path.clone());
            } else {
                // A porcelain-v2 rename reports the destination in `path`
                // and the source in `original_path`. Restoring only the
                // destination would leave the source deleted instead of
                // returning the selected rename to HEAD.
                for tracked_path in
                    std::iter::once(entry.path.as_str()).chain(entry.original_path.as_deref())
                {
                    validate_worktree_relative_path(tracked_path)?;
                    if tracked_seen.insert(tracked_path) {
                        tracked_paths.push(tracked_path.to_owned());
                    }
                }
            }
        }

        if !tracked_paths.is_empty() {
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
            // Delegate removal to Git with literal pathspecs rather than
            // reopening repository-relative paths in this process. Git's
            // worktree machinery handles symlinks as entries and avoids the
            // check-then-use filesystem traversal that direct removal needs.
            let mut arguments = os_arguments(&["clean", "-f", "-d", "--"]);
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

    /// Creates a commit while preserving the user's normal Git hooks and
    /// credential/configuration behavior.
    pub fn commit(
        &self,
        repository: &Path,
        message: &str,
        amend: bool,
    ) -> Result<CommitResult, AppError> {
        if message.trim().is_empty() {
            return Err(AppError::new("invalid_commit_message", "提交信息不能为空"));
        }
        if amend && !self.has_head(repository)? {
            return Err(AppError::new(
                "no_commit_to_amend",
                "当前分支还没有可修改的提交",
            ));
        }
        let mut arguments = vec![OsString::from("commit")];
        if amend {
            arguments.push(OsString::from("--amend"));
        }
        arguments.push(OsString::from("-m"));
        arguments.push(OsString::from(message));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建提交",
        )?;

        let revision = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "HEAD"]),
            &GitEnvironment::default(),
            "读取新提交",
        )?;
        Ok(CommitResult {
            commit_id: output_text(&revision.stdout).trim().to_owned(),
        })
    }

    /// Lists recent commits using a NUL-delimited format so user-authored
    /// subjects cannot confuse the parser.
    pub fn commits(&self, repository: &Path, limit: usize) -> Result<Vec<CommitSummary>, AppError> {
        if limit == 0 || limit > 1_000 {
            return Err(AppError::new(
                "invalid_history_limit",
                "提交历史读取数量必须在 1 到 1000 之间",
            ));
        }
        let format = "%H%x00%h%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00";
        let arguments = vec![
            OsString::from("log"),
            OsString::from("--no-ext-diff"),
            OsString::from(format!("--max-count={limit}")),
            // `tformat:` appends a newline after every record. With our
            // explicit record separator that trailing newline becomes a
            // spurious final record, which makes valid histories fail the
            // strict parser below. `format:` avoids that final record, but
            // Git still inserts one newline between adjacent records; the
            // parser explicitly removes that separator before reading a SHA.
            OsString::from(format!("--format=format:{format}")),
        ];
        // An unborn branch has no HEAD to pass to `git log`. Checking it via
        // the quiet plumbing command is locale-independent and intentionally
        // treats that normal state as an empty history.
        let head = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        if !head.status.success() {
            return Ok(Vec::new());
        }
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取提交历史",
        )?;
        parse_commits(&output.stdout)
    }

    /// Reads a complete patch for one existing commit without exposing Git's
    /// raw diagnostic output on failure.
    pub fn commit_diff(&self, repository: &Path, revision: &str) -> Result<String, AppError> {
        validate_revision(revision, "提交引用")?;
        let arguments = vec![
            OsString::from("show"),
            OsString::from("--no-ext-diff"),
            OsString::from("--format="),
            OsString::from("--find-renames"),
            OsString::from("--unified=3"),
            OsString::from(revision),
            OsString::from("--"),
        ];
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取提交改动",
        )?;
        Ok(output_text(&output.stdout))
    }

    /// Lists local branches and their configured upstreams.
    pub fn branches(&self, repository: &Path) -> Result<Vec<BranchInfo>, AppError> {
        let arguments = os_arguments(&[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)",
            "refs/heads",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取分支",
        )?;
        parse_branches(&output.stdout)
    }

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
        if paths_match(repository, path) {
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
        if !version_output.status.success() {
            return Ok(LfsStatus {
                available: false,
                initialized: false,
                version: None,
                files: Vec::new(),
            });
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

    /// Creates a local branch without switching to it.
    pub fn create_branch(
        &self,
        repository: &Path,
        branch: &str,
        start_point: Option<&str>,
    ) -> Result<(), AppError> {
        validate_ref_name(branch, "分支名")?;
        if let Some(start_point) = start_point {
            validate_ref_name(start_point, "起点引用")?;
        }
        let mut arguments = os_arguments(&["branch", "--", branch]);
        if let Some(start_point) = start_point {
            arguments.push(OsString::from(start_point));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建分支",
        )?;
        Ok(())
    }

    /// Switches the worktree to an existing local branch.
    pub fn switch_branch(&self, repository: &Path, branch: &str) -> Result<(), AppError> {
        validate_ref_name(branch, "分支名")?;
        let arguments = os_arguments(&["switch", "--", branch]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "切换分支",
        )?;
        Ok(())
    }

    /// Renames a local branch. Git itself enforces that the destination does
    /// not collide with another branch.
    pub fn rename_branch(
        &self,
        repository: &Path,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), AppError> {
        validate_ref_name(old_name, "原分支名")?;
        validate_ref_name(new_name, "新分支名")?;
        let arguments = os_arguments(&["branch", "-m", old_name, new_name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "重命名分支",
        )?;
        Ok(())
    }

    /// Deletes a local branch. The caller must make the force choice
    /// explicit; normal deletion retains Git's merged-branch safety check.
    pub fn delete_branch(
        &self,
        repository: &Path,
        name: &str,
        force: bool,
    ) -> Result<(), AppError> {
        validate_ref_name(name, "分支名")?;
        let option = if force { "-D" } else { "-d" };
        let arguments = os_arguments(&["branch", option, name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "删除分支",
        )?;
        Ok(())
    }

    /// Creates and switches to a new branch atomically from the user's point
    /// of view.
    pub fn create_and_switch_branch(
        &self,
        repository: &Path,
        branch: &str,
        start_point: Option<&str>,
    ) -> Result<(), AppError> {
        validate_ref_name(branch, "分支名")?;
        if let Some(start_point) = start_point {
            validate_ref_name(start_point, "起点引用")?;
        }
        let mut arguments = os_arguments(&["switch", "-c", branch]);
        if let Some(start_point) = start_point {
            arguments.push(OsString::from(start_point));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建并切换分支",
        )?;
        Ok(())
    }

    /// Merges a branch using Git's generated merge message.  If a conflict is
    /// reported, the worktree remains available for the UI to show its status.
    pub fn merge_branch(&self, repository: &Path, branch: &str) -> Result<(), AppError> {
        validate_ref_name(branch, "分支名")?;
        let arguments = os_arguments(&["merge", "--no-edit", "--", branch]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "合并分支",
        )?;
        Ok(())
    }

    /// Starts a non-interactive rebase of the current branch onto `base`.
    /// Should a conflict occur, Git's on-disk state is retained and exposed by
    /// [`Self::operation_kind`] so the user can resolve, continue, or abort.
    pub fn start_rebase(&self, repository: &Path, base: &str) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
        validate_revision(base, "变基目标")?;
        let arguments = os_arguments(&["rebase", base]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "变基",
        )?;
        Ok(())
    }

    /// Cherry-picks one or more validated revisions onto the current branch.
    /// `--no-edit` prevents a hidden terminal editor from blocking the GUI.
    pub fn cherry_pick(&self, repository: &Path, revisions: &[String]) -> Result<(), AppError> {
        if revisions.is_empty() || revisions.len() > 100 {
            return Err(AppError::new(
                "invalid_cherry_pick",
                "请选择 1 到 100 个要拣选的提交",
            ));
        }
        self.ensure_no_operation(repository)?;
        for revision in revisions {
            validate_revision(revision, "要拣选的提交")?;
        }
        let mut arguments = os_arguments(&["cherry-pick", "--no-edit"]);
        arguments.extend(revisions.iter().map(OsString::from));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "拣选提交",
        )?;
        Ok(())
    }

    /// Detects an operation that Git has paused in this worktree. This uses
    /// Git's own git-dir resolution so linked worktrees are handled correctly.
    pub fn operation_kind(&self, repository: &Path) -> Result<Option<GitOperationKind>, AppError> {
        let git_dir = self.git_directory(repository)?;
        if git_dir.join("rebase-merge").exists()
            || git_dir.join("rebase-apply").join("rebasing").exists()
        {
            return Ok(Some(GitOperationKind::Rebase));
        }
        if git_dir.join("CHERRY_PICK_HEAD").exists() {
            return Ok(Some(GitOperationKind::CherryPick));
        }
        if git_dir.join("REVERT_HEAD").exists() {
            return Ok(Some(GitOperationKind::Revert));
        }
        if git_dir.join("MERGE_HEAD").exists() {
            return Ok(Some(GitOperationKind::Merge));
        }
        Ok(None)
    }

    /// Continues the currently paused operation after all conflicts are
    /// resolved and staged. The operation kind is checked against Git's state
    /// to avoid accidentally continuing a different workflow.
    pub fn continue_operation(
        &self,
        repository: &Path,
        kind: GitOperationKind,
    ) -> Result<(), AppError> {
        self.ensure_operation(repository, &kind)?;
        let arguments = match &kind {
            GitOperationKind::Merge => os_arguments(&["commit", "--no-edit"]),
            GitOperationKind::Rebase => os_arguments(&["rebase", "--continue"]),
            GitOperationKind::CherryPick => os_arguments(&["cherry-pick", "--continue"]),
            GitOperationKind::Revert => os_arguments(&["revert", "--continue"]),
        };
        let environment = match &kind {
            GitOperationKind::Merge => GitEnvironment::default(),
            GitOperationKind::Rebase | GitOperationKind::CherryPick | GitOperationKind::Revert => {
                GitEnvironment::new().with_variable("GIT_EDITOR", "true")
            }
        };
        self.run(Some(repository), &arguments, &environment, "继续 Git 操作")?;
        Ok(())
    }

    /// Aborts a paused merge, rebase, cherry-pick, or revert and restores the
    /// repository to the state Git recorded before that operation started.
    pub fn abort_operation(
        &self,
        repository: &Path,
        kind: GitOperationKind,
    ) -> Result<(), AppError> {
        self.ensure_operation(repository, &kind)?;
        let arguments = match kind {
            GitOperationKind::Merge => os_arguments(&["merge", "--abort"]),
            GitOperationKind::Rebase => os_arguments(&["rebase", "--abort"]),
            GitOperationKind::CherryPick => os_arguments(&["cherry-pick", "--abort"]),
            GitOperationKind::Revert => os_arguments(&["revert", "--abort"]),
        };
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "中止 Git 操作",
        )?;
        Ok(())
    }

    /// Lists stashes in Git's user-visible order, newest first.
    pub fn stashes(&self, repository: &Path) -> Result<Vec<StashEntry>, AppError> {
        let arguments = os_arguments(&["stash", "list", "--format=format:%gd%x00%gs%x00"]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取暂存栈",
        )?;
        parse_stashes(&output.stdout)
    }

    /// Saves tracked changes and, optionally, untracked files to a stash.
    pub fn stash_push(
        &self,
        repository: &Path,
        message: Option<&str>,
        include_untracked: bool,
    ) -> Result<(), AppError> {
        let mut arguments = os_arguments(&["stash", "push"]);
        if include_untracked {
            arguments.push(OsString::from("--include-untracked"));
        }
        if let Some(message) = message.map(str::trim).filter(|message| !message.is_empty()) {
            arguments.push(OsString::from("--message"));
            arguments.push(OsString::from(message));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "保存暂存栈",
        )?;
        Ok(())
    }

    /// Applies and removes one stash entry.
    pub fn stash_pop(&self, repository: &Path, reference: &str) -> Result<(), AppError> {
        validate_revision(reference, "暂存栈引用")?;
        let arguments = os_arguments(&["stash", "pop", reference]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "恢复暂存栈",
        )?;
        Ok(())
    }

    /// Permanently removes one stash entry after the UI has confirmed it.
    pub fn stash_drop(&self, repository: &Path, reference: &str) -> Result<(), AppError> {
        validate_revision(reference, "暂存栈引用")?;
        let arguments = os_arguments(&["stash", "drop", reference]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "删除暂存栈",
        )?;
        Ok(())
    }

    /// Resets the current branch to a selected revision. The reset mode is
    /// explicit so the frontend can present a proportionate confirmation.
    pub fn reset(&self, repository: &Path, target: &str, mode: ResetMode) -> Result<(), AppError> {
        validate_revision(target, "重置目标")?;
        let option = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        let arguments = os_arguments(&["reset", option, target]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "重置分支",
        )?;
        Ok(())
    }

    /// Rewrites the most recent commits into one commit while retaining the
    /// earlier base commit. A clean worktree is required so unrelated staged
    /// or unstaged changes can never be accidentally included in the squash.
    pub fn squash_recent_commits(
        &self,
        repository: &Path,
        count: usize,
        message: &str,
    ) -> Result<(), AppError> {
        if !(2..=1_000).contains(&count) {
            return Err(AppError::new(
                "invalid_squash_count",
                "压缩提交数量必须在 2 到 1000 之间",
            ));
        }
        if message.trim().is_empty() {
            return Err(AppError::new("invalid_commit_message", "提交信息不能为空"));
        }
        self.ensure_no_operation(repository)?;
        if !self.has_head(repository)? {
            return Err(AppError::new(
                "invalid_squash_count",
                "当前分支没有足够的提交可压缩",
            ));
        }
        if !self.status(repository)?.entries.is_empty() {
            return Err(AppError::new(
                "working_tree_not_clean",
                "压缩提交前请先提交、暂存或丢弃当前改动",
            ));
        }
        let base = format!("HEAD~{count}");
        let base_exists = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", &base]),
            &GitEnvironment::default(),
        )?;
        if !base_exists.status.success() {
            return Err(AppError::new(
                "invalid_squash_count",
                "压缩提交后必须保留一个更早的基线提交",
            ));
        }
        let original_head = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "HEAD"]),
            &GitEnvironment::default(),
            "读取压缩前提交",
        )?;
        let original_head = output_text(&original_head.stdout).trim().to_owned();
        self.run(
            Some(repository),
            &os_arguments(&["reset", "--soft", &base]),
            &GitEnvironment::default(),
            "准备压缩提交",
        )?;
        if let Err(error) = self.commit(repository, message, false) {
            // `--soft` deliberately leaves the combined changes staged. A
            // mixed reset restores both HEAD and the index when commit hooks,
            // identity, or signing configuration reject the replacement.
            if self
                .run(
                    Some(repository),
                    &os_arguments(&["reset", "--mixed", &original_head]),
                    &GitEnvironment::default(),
                    "恢复压缩前提交",
                )
                .is_err()
            {
                return Err(AppError::new(
                    "squash_recovery_failed",
                    "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Creates a new revert commit for a selected commit. Git's normal hooks
    /// and conflict behavior are preserved.
    pub fn revert_commit(&self, repository: &Path, revision: &str) -> Result<(), AppError> {
        validate_revision(revision, "提交引用")?;
        let arguments = os_arguments(&["revert", "--no-edit", revision]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "还原提交",
        )?;
        Ok(())
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
        self.ensure_network_configuration_is_safe(repository)?;
        let arguments = fetch_arguments(options)?;
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
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        if let Some(refspec) = options.refspec.as_deref() {
            validate_refspec(refspec)?;
        }
        let mut arguments = os_arguments(&["pull", &options.remote]);
        if let Some(refspec) = options.refspec.as_deref() {
            arguments.push(OsString::from(refspec));
        }
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
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        for refspec in &options.refspecs {
            validate_refspec(refspec)?;
        }
        let mut arguments = vec![OsString::from("push")];
        if options.set_upstream {
            arguments.push(OsString::from("--set-upstream"));
        }
        if options.force_with_lease {
            arguments.push(OsString::from("--force-with-lease"));
        }
        if options.follow_tags {
            arguments.push(OsString::from("--follow-tags"));
        }
        arguments.push(OsString::from(&options.remote));
        arguments.extend(options.refspecs.iter().map(OsString::from));
        self.run_network(Some(repository), &arguments, environment, "推送改动")?;
        Ok(())
    }

    /// Lists tags with their object IDs, newest first according to Git's
    /// creator date ordering.
    pub fn tags(&self, repository: &Path) -> Result<Vec<TagInfo>, AppError> {
        let arguments = os_arguments(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname:short)%00%(objectname)",
            "refs/tags",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取标签",
        )?;
        parse_tags(&output.stdout)
    }

    /// Creates a lightweight tag, or an annotated tag when `message` is set.
    pub fn create_tag(
        &self,
        repository: &Path,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), AppError> {
        validate_ref_name(name, "标签名")?;
        if let Some(target) = target {
            validate_ref_name(target, "标签目标")?;
        }
        if let Some(message) = message {
            if message.trim().is_empty() {
                return Err(AppError::new("invalid_tag_message", "注释标签信息不能为空"));
            }
        }

        let mut arguments = vec![OsString::from("tag")];
        if let Some(message) = message {
            arguments.push(OsString::from("-a"));
            arguments.push(OsString::from(name));
            arguments.push(OsString::from("-m"));
            arguments.push(OsString::from(message));
        } else {
            arguments.push(OsString::from(name));
        }
        if let Some(target) = target {
            arguments.push(OsString::from(target));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建标签",
        )?;
        Ok(())
    }

    /// Deletes a local tag only; it does not delete the corresponding remote
    /// tag.
    pub fn delete_tag(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        validate_ref_name(name, "标签名")?;
        let arguments = os_arguments(&["tag", "-d", name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "删除标签",
        )?;
        Ok(())
    }

    /// Lists all configured remotes with credential-safe display URLs and
    /// built-in endpoint recognition.
    pub fn remotes(&self, repository: &Path) -> Result<Vec<RemoteProfile>, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["remote"]),
            &GitEnvironment::default(),
            "读取远程仓库",
        )?;

        let endpoints = CodeHostEndpoint::builtin();
        let mut remotes = Vec::new();
        for name in output_text(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let fetch_url = self.remote_url(repository, name, false)?;
            let push_url = self.remote_url(repository, name, true)?;
            let parsed = parse_remote_url(&fetch_url);
            let endpoint = parsed
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints));
            remotes.push(RemoteProfile {
                name: name.to_owned(),
                fetch_url: safe_remote_url(&fetch_url),
                push_url: safe_remote_url(&push_url),
                parsed,
                endpoint_id: endpoint.map(|endpoint| endpoint.id.clone()),
                code_host_kind: endpoint
                    .map(|endpoint| endpoint.kind.clone())
                    .unwrap_or(CodeHostKind::Unknown),
            });
        }
        remotes.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(remotes)
    }

    /// Returns the configured URL for backend credential resolution only.
    ///
    /// Unlike [`Self::remotes`], this can contain userinfo from an unsafe
    /// pre-existing Git configuration, so it must never be serialized, logged,
    /// or returned through a Tauri command. CredentialBroker validates and
    /// rejects unsafe HTTP(S) values before a network operation proceeds.
    pub fn remote_url_for_operation(
        &self,
        repository: &Path,
        name: &str,
        push: bool,
    ) -> Result<String, AppError> {
        self.remote_url(repository, name, push)
    }

    /// Adds a remote.  URLs containing an HTTP(S) username, password, query,
    /// or fragment are rejected to ensure a PAT cannot become Git config.
    pub fn add_remote(&self, repository: &Path, name: &str, url: &str) -> Result<(), AppError> {
        validate_remote_name(name)?;
        validate_git_remote_url(url)?;
        let arguments = os_arguments(&["remote", "add", name, url]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "添加远程仓库",
        )?;
        Ok(())
    }

    /// Replaces either the fetch URL or push URL of an existing remote.
    pub fn set_remote_url(
        &self,
        repository: &Path,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
    ) -> Result<(), AppError> {
        validate_remote_name(name)?;
        validate_git_remote_url(url)?;
        let mut arguments = os_arguments(&["remote", "set-url"]);
        if kind == RemoteUrlKind::Push {
            arguments.push(OsString::from("--push"));
        }
        arguments.push(OsString::from(name));
        arguments.push(OsString::from(url));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "更新远程地址",
        )?;
        Ok(())
    }

    /// Reject repository-local configuration that can execute a credential
    /// helper or inject transport credentials. Local `.git/config` belongs to
    /// the opened repository, so it is not a trusted source for an
    /// application-selected account. Global/system helpers remain available
    /// only for an unscoped URL-based fallback, never for a selected profile.
    pub fn ensure_network_configuration_is_safe(&self, repository: &Path) -> Result<(), AppError> {
        let arguments = os_arguments(&[
            "config",
            "--local",
            "--null",
            "--name-only",
            "--get-regexp",
            ".",
        ]);
        let output = self.execute(Some(repository), &arguments, &GitEnvironment::default())?;
        if output.status.success() {
            let unsafe_key = output
                .stdout
                .split(|byte| *byte == b'\0')
                .filter_map(|key| std::str::from_utf8(key).ok())
                .any(is_unsafe_network_config_key);
            return if unsafe_key {
                Err(AppError::new(
                    "unsafe_git_config",
                    "仓库本地 Git 配置包含不安全的凭据或网络设置；请移除后再执行网络操作。",
                ))
            } else {
                Ok(())
            };
        }

        // `git config --get-regexp` exits with 1 when there are no matching
        // keys. That is a safe, ordinary empty-configuration result.
        if output.status.code() == Some(1) {
            Ok(())
        } else {
            Err(git_command_error("检查仓库网络配置", &output))
        }
    }

    /// Renames a local remote configuration entry.
    pub fn rename_remote(
        &self,
        repository: &Path,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), AppError> {
        validate_remote_name(old_name)?;
        validate_remote_name(new_name)?;
        let arguments = os_arguments(&["remote", "rename", old_name, new_name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "重命名远程仓库",
        )?;
        Ok(())
    }

    /// Removes a local remote configuration entry only.
    pub fn remove_remote(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        validate_remote_name(name)?;
        let arguments = os_arguments(&["remote", "remove", name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "移除远程仓库",
        )?;
        Ok(())
    }

    /// Loads the fields most repository pages need in one backend call.
    pub fn snapshot(&self, repository: &Path) -> Result<RepositorySnapshot, AppError> {
        Ok(RepositorySnapshot {
            repository: self.repository_record(repository)?,
            status: self.status(repository)?,
            branches: self.branches(repository)?,
            remotes: self.remotes(repository)?,
            tags: self.tags(repository)?,
        })
    }

    fn try_repository_root(&self, path: &Path) -> Result<Option<PathBuf>, AppError> {
        let arguments = os_arguments(&["rev-parse", "--show-toplevel"]);
        let output = self.execute(Some(path), &arguments, &GitEnvironment::default())?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = output_text(&output.stdout).trim().to_owned();
        if value.is_empty() {
            return Ok(None);
        }
        Ok(Some(normalize_path(Path::new(&value))))
    }

    /// Returns whether this worktree has at least one commit. An unborn
    /// branch is valid for initialization but cannot be amended or reset.
    pub fn has_head(&self, repository: &Path) -> Result<bool, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        Ok(output.status.success())
    }

    /// Returns the immutable commit currently checked out at `HEAD`. Durable
    /// recovery intents use this to distinguish the original worktree from a
    /// different repository later recreated at the same filesystem path.
    pub fn head_commit_id(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "HEAD"]),
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

    fn git_directory(&self, repository: &Path) -> Result<PathBuf, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--git-dir"]),
            &GitEnvironment::default(),
            "读取 Git 操作状态",
        )?;
        let git_dir = PathBuf::from(output_text(&output.stdout).trim());
        if git_dir.as_os_str().is_empty() {
            return Err(AppError::new("git_parse_error", "无法读取 Git 元数据目录"));
        }
        Ok(if git_dir.is_absolute() {
            git_dir
        } else {
            repository.join(git_dir)
        })
    }

    fn ensure_no_operation(&self, repository: &Path) -> Result<(), AppError> {
        if self.operation_kind(repository)?.is_some() {
            return Err(AppError::new(
                "git_operation_in_progress",
                "当前已有 Git 操作尚未完成，请先继续或中止它",
            ));
        }
        Ok(())
    }

    fn ensure_operation(
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

    fn remote_url(&self, repository: &Path, name: &str, push: bool) -> Result<String, AppError> {
        validate_remote_name(name)?;
        let mut arguments = os_arguments(&["remote", "get-url"]);
        if push {
            arguments.push(OsString::from("--push"));
        }
        arguments.push(OsString::from(name));
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取远程地址",
        )?;
        Ok(output_text(&output.stdout).trim().to_owned())
    }

    fn run(
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
    fn run_network(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let output = self.execute_network_with_timeout(repository, arguments, environment)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    fn run_with_input(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        input: &[u8],
        environment: &GitEnvironment,
        operation: &'static str,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        command
            .args(command_arguments(repository, arguments))
            .stdin(Stdio::piped())
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
        let write_result = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(input),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"));
            }
        };
        if write_result.is_err() {
            // Git can reject a patch before consuming stdin. Reap the child
            // on this early return so repeated failed hunk requests cannot
            // accumulate zombies on Unix.
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new("git_command_failed", "无法向 Git 写入补丁。"));
        }
        let output = child.wait_with_output().map_err(|_| {
            AppError::new(
                "git_command_failed",
                "Git 未能完成补丁操作；请检查仓库状态。",
            )
        })?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(git_command_error(operation, &output))
        }
    }

    fn execute(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        command.args(command_arguments(repository, arguments));
        apply_git_environment(&mut command, environment);
        command.output().map_err(|_| {
            AppError::with_safe_details(
                "git_unavailable",
                "无法启动系统 Git",
                "请确认 Git 已安装且可从应用进程的 PATH 访问。",
            )
        })
    }

    fn execute_network_with_timeout(
        &self,
        repository: Option<&Path>,
        arguments: &[OsString],
        environment: &GitEnvironment,
    ) -> Result<Output, AppError> {
        let mut command = Command::new(&self.executable);
        configure_git_network_process_group(&mut command);
        command
            .args(command_arguments(repository, arguments))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        apply_git_environment(&mut command, environment);
        let mut child = command.spawn().map_err(|_| {
            AppError::with_safe_details(
                "git_unavailable",
                "无法启动系统 Git",
                "请确认 Git 已安装且可从应用进程的 PATH 访问。",
            )
        })?;
        let process_id = child.id();
        let deadline = Instant::now() + NETWORK_GIT_COMMAND_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(Output {
                        status,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    });
                }
                Ok(None) if Instant::now() >= deadline => {
                    terminate_git_network_child(&mut child, process_id);
                    return Err(AppError::new(
                        "git_network_timeout",
                        "Git 网络操作超时，请检查网络、认证和远程服务状态后重试。",
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    terminate_git_network_child(&mut child, process_id);
                    return Err(AppError::io("等待 Git", error));
                }
            }
        }
    }
}

fn terminate_git_network_child(child: &mut std::process::Child, process_id: u32) {
    terminate_git_network_process_group(process_id);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn configure_git_network_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: the child only creates its own process group before exec. It
    // lets a timeout terminate Git together with SSH, hooks, and helpers it
    // started rather than leaving a descendant behind.
    unsafe {
        command.pre_exec(|| {
            // SAFETY: setpgid is async-signal-safe and uses only the child PID.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn configure_git_network_process_group(_command: &mut Command) {}

#[cfg(all(not(unix), not(windows)))]
fn configure_git_network_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_git_network_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the dedicated child process group.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_git_network_process_group(process_id: u32) {
    let process_id = process_id.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", process_id.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_git_network_process_group(_process_id: u32) {}

/// Environment variables supplied only by backend code for one Git process.
/// It intentionally has no `Serialize` or `Debug` implementation so values
/// such as askpass configuration are not accidentally sent to the frontend or
/// emitted by standard debug logging.
#[derive(Default)]
pub struct GitEnvironment {
    variables: Vec<(OsString, OsString)>,
}

impl GitEnvironment {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one environment variable for a Git child process.
    pub fn set(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        let name = name.as_ref().to_os_string();
        let value = value.as_ref().to_os_string();
        if let Some((_, existing_value)) = self
            .variables
            .iter_mut()
            .find(|(existing_name, _)| *existing_name == name)
        {
            *existing_value = value;
        } else {
            self.variables.push((name, value));
        }
    }

    pub fn with_variable(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.set(name, value);
        self
    }

    /// Disable credential/configuration values inherited from an opened
    /// repository for an operation that uses the app-selected askpass source.
    /// Command-scope config has higher precedence than local/global/system
    /// config; an empty helper resets the configured helper list before Git
    /// could execute a repository-provided `credential.helper = !...` value.
    pub fn isolate_network_credentials(&mut self) {
        const CONFIGURATION: [(&str, &str); 6] = [
            ("credential.helper", ""),
            ("http.extraheader", ""),
            ("http.proxy", ""),
            ("http.sslverify", "true"),
            ("http.cookiefile", ""),
            ("http.savecookies", "false"),
        ];
        self.set_network_credential_configuration(&CONFIGURATION);
    }

    /// Isolate an app-managed HTTPS credential and prevent repository or
    /// global Git hooks from inheriting its one-shot askpass capability. A
    /// hook runs inside Git's process tree and is not a trusted recipient of
    /// a PAT authorization token.
    pub fn isolate_network_credentials_and_disable_hooks(&mut self) {
        const CONFIGURATION: [(&str, &str); 7] = [
            ("credential.helper", ""),
            ("http.extraheader", ""),
            ("http.proxy", ""),
            ("http.sslverify", "true"),
            ("http.cookiefile", ""),
            ("http.savecookies", "false"),
            ("core.hooksPath", DISABLED_GIT_HOOKS_PATH),
        ];
        self.set_network_credential_configuration(&CONFIGURATION);
    }

    fn set_network_credential_configuration(&mut self, configuration: &[(&str, &str)]) {
        self.variables.retain(|(name, _)| {
            let name = name.to_string_lossy();
            name != "GIT_CONFIG_COUNT"
                && !name.starts_with("GIT_CONFIG_KEY_")
                && !name.starts_with("GIT_CONFIG_VALUE_")
        });
        self.set("GIT_CONFIG_COUNT", configuration.len().to_string());
        for (index, (key, value)) in configuration.iter().enumerate() {
            self.set(format!("GIT_CONFIG_KEY_{index}"), key);
            self.set(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
}

/// Every Git child starts from a credential-neutral environment. A parent
/// shell, IDE, or another app process must not silently choose an askpass
/// program or inject an alternate credential helper into a resolved operation.
fn apply_git_environment(command: &mut Command, environment: &GitEnvironment) {
    for name in [
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_TERMINAL_PROMPT",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_GRAFT_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
    ] {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("GIT_CONFIG_") || normalized.starts_with("BKM_GIT_ASKPASS_") {
            command.env_remove(name);
        }
    }
    // Git commands launched from the GUI cannot safely service an interactive
    // terminal prompt. CredentialEnvironment may replace this with the app's
    // trusted askpass helper for the selected account.
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        // Askpass prompt parsing is intentionally narrow. Force Git's stable
        // C locale so localized Username/Password prompts cannot make a
        // selected HTTPS credential fail unpredictably.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    for (name, value) in &environment.variables {
        command.env(name, value);
    }
}

/// Options for cloning a worktree repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloneOptions {
    pub branch: Option<String>,
    pub depth: Option<u32>,
    pub recursive: bool,
}

/// Selection for an unstaged or staged diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffOptions {
    pub staged: bool,
    pub paths: Vec<String>,
    pub context_lines: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub commit_id: String,
}

/// Fetch only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FetchOptions {
    pub remote: String,
    pub refspecs: Vec<String>,
    pub prune: bool,
}

/// Pull only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullOptions {
    pub remote: String,
    pub refspec: Option<String>,
}

/// Push only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushOptions {
    pub remote: String,
    pub refspecs: Vec<String>,
    pub set_upstream: bool,
    pub force_with_lease: bool,
    pub follow_tags: bool,
}

/// Selects which local remote URL setting is updated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoteUrlKind {
    Fetch,
    Push,
}

/// The data needed by the repository overview page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryRecord,
    pub status: GitStatus,
    pub branches: Vec<BranchInfo>,
    pub remotes: Vec<RemoteProfile>,
    pub tags: Vec<TagInfo>,
}

fn fetch_arguments(options: &FetchOptions) -> Result<Vec<OsString>, AppError> {
    validate_remote_name(&options.remote)?;
    for refspec in &options.refspecs {
        validate_refspec(refspec)?;
    }
    let mut arguments = vec![OsString::from("fetch")];
    if options.prune {
        arguments.push(OsString::from("--prune"));
    }
    arguments.push(OsString::from(&options.remote));
    arguments.extend(options.refspecs.iter().map(OsString::from));
    Ok(arguments)
}

fn command_arguments(repository: Option<&Path>, arguments: &[OsString]) -> Vec<OsString> {
    let mut all_arguments =
        Vec::with_capacity(arguments.len() + usize::from(repository.is_some()) * 2);
    if let Some(repository) = repository {
        all_arguments.push(OsString::from("-C"));
        all_arguments.push(repository.as_os_str().to_os_string());
    }
    all_arguments.extend(arguments.iter().cloned());
    all_arguments
}

fn os_arguments(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

fn parse_status_porcelain_v2(raw: &[u8]) -> Result<GitStatus, AppError> {
    let mut status = GitStatus {
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
    };
    let mut cursor = 0;

    while cursor < raw.len() {
        if raw[cursor] == 0 {
            cursor += 1;
            continue;
        }

        if raw[cursor..].starts_with(b"# ") {
            let end = raw[cursor..]
                .iter()
                .position(|byte| *byte == b'\n' || *byte == 0)
                .map(|offset| cursor + offset)
                .unwrap_or(raw.len());
            parse_status_header(&raw[cursor..end], &mut status)?;
            cursor = end.saturating_add(1);
            continue;
        }

        let end = raw[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| cursor + offset)
            .unwrap_or(raw.len());
        let record = &raw[cursor..end];
        cursor = end.saturating_add(1);
        if record.is_empty() {
            continue;
        }

        match record.first().copied() {
            Some(b'1') => status.entries.push(parse_status_ordinary(record)?),
            Some(b'2') => {
                if cursor > raw.len() {
                    return Err(status_parse_error());
                }
                let original_end = raw[cursor..]
                    .iter()
                    .position(|byte| *byte == 0)
                    .map(|offset| cursor + offset)
                    .unwrap_or(raw.len());
                if original_end == raw.len() && !raw.ends_with(&[0]) {
                    return Err(status_parse_error());
                }
                let original_path = lossy(&raw[cursor..original_end]);
                cursor = original_end.saturating_add(1);
                status
                    .entries
                    .push(parse_status_renamed(record, original_path)?);
            }
            Some(b'u') => status.entries.push(parse_status_unmerged(record)?),
            // A short untracked record has no index side. Keep the worktree
            // marker but normalize its absent index status to the same empty
            // value used by ordinary `.<status>` records.
            Some(b'?') => status.entries.push(parse_status_short(record, "", "?")?),
            Some(b'!') => status.entries.push(parse_status_short(record, "", "!")?),
            _ => return Err(status_parse_error()),
        }
    }

    Ok(status)
}

fn parse_status_header(record: &[u8], status: &mut GitStatus) -> Result<(), AppError> {
    let value = lossy(record);
    let value = value.strip_prefix("# ").ok_or_else(status_parse_error)?;
    if let Some(branch) = value.strip_prefix("branch.head ") {
        status.branch = match branch {
            "(detached)" | "(unknown)" => None,
            "" => return Err(status_parse_error()),
            branch => Some(branch.to_owned()),
        };
    } else if let Some(upstream) = value.strip_prefix("branch.upstream ") {
        status.upstream = (!upstream.is_empty()).then(|| upstream.to_owned());
    } else if let Some(ahead_behind) = value.strip_prefix("branch.ab ") {
        let mut values = ahead_behind.split_whitespace();
        let ahead = values.next().and_then(|value| value.strip_prefix('+'));
        let behind = values.next().and_then(|value| value.strip_prefix('-'));
        if values.next().is_some() {
            return Err(status_parse_error());
        }
        status.ahead = ahead
            .and_then(|value| value.parse().ok())
            .ok_or_else(status_parse_error)?;
        status.behind = behind
            .and_then(|value| value.parse().ok())
            .ok_or_else(status_parse_error)?;
    }
    Ok(())
}

fn parse_status_ordinary(record: &[u8]) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 9)?;
    status_entry_from_xy(fields[1], fields[8], None)
}

fn parse_status_renamed(record: &[u8], original_path: String) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 10)?;
    status_entry_from_xy(fields[1], fields[9], Some(original_path))
}

fn parse_status_unmerged(record: &[u8]) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 11)?;
    status_entry_from_xy(fields[1], fields[10], None)
}

fn parse_status_short(
    record: &[u8],
    index_status: &str,
    worktree_status: &str,
) -> Result<GitStatusEntry, AppError> {
    if record.len() < 3 || record[1] != b' ' {
        return Err(status_parse_error());
    }
    Ok(GitStatusEntry {
        path: lossy(&record[2..]),
        original_path: None,
        index_status: index_status.to_owned(),
        worktree_status: worktree_status.to_owned(),
    })
}

fn split_fields(record: &[u8], count: usize) -> Result<Vec<&[u8]>, AppError> {
    let mut fields = Vec::with_capacity(count);
    let mut remaining = record;
    for _ in 0..count.saturating_sub(1) {
        let Some(separator) = remaining.iter().position(|byte| *byte == b' ') else {
            return Err(status_parse_error());
        };
        fields.push(&remaining[..separator]);
        remaining = &remaining[separator + 1..];
    }
    if remaining.is_empty() {
        return Err(status_parse_error());
    }
    fields.push(remaining);
    Ok(fields)
}

fn status_entry_from_xy(
    xy: &[u8],
    path: &[u8],
    original_path: Option<String>,
) -> Result<GitStatusEntry, AppError> {
    if xy.len() != 2 {
        return Err(status_parse_error());
    }
    Ok(GitStatusEntry {
        path: lossy(path),
        original_path,
        index_status: status_side_from_porcelain(xy[0]),
        worktree_status: status_side_from_porcelain(xy[1]),
    })
}

/// Git porcelain v2 uses `.` for a clean side of an ordinary status record.
/// The application model represents an absent index/worktree change as an
/// empty string, which lets all UI consumers use one consistent check rather
/// than treating `.` as a real modification.
fn status_side_from_porcelain(status: u8) -> String {
    if status == b'.' {
        String::new()
    } else {
        (status as char).to_string()
    }
}

fn parse_branches(raw: &[u8]) -> Result<Vec<BranchInfo>, AppError> {
    let mut branches = Vec::new();
    for line in output_text(raw).lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 3 || fields[1].is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 分支信息"));
        }
        branches.push(BranchInfo {
            name: fields[1].to_owned(),
            is_current: fields[0].trim() == "*",
            upstream: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
        });
    }
    Ok(branches)
}

fn parse_worktrees(raw: &[u8]) -> Result<Vec<WorktreeInfo>, AppError> {
    let mut worktrees = Vec::new();
    let mut fields = Vec::new();
    for field in raw.split(|byte| *byte == 0) {
        if field.is_empty() {
            if !fields.is_empty() {
                worktrees.push(parse_worktree_fields(&fields, worktrees.is_empty())?);
                fields.clear();
            }
        } else {
            fields.push(field);
        }
    }
    if !fields.is_empty() {
        worktrees.push(parse_worktree_fields(&fields, worktrees.is_empty())?);
    }
    Ok(worktrees)
}

fn parse_worktree_fields(fields: &[&[u8]], is_main: bool) -> Result<WorktreeInfo, AppError> {
    let mut path = None;
    let mut head = String::new();
    let mut branch = None;
    let mut is_detached = false;
    let mut is_locked = false;
    let mut is_prunable = false;

    for field in fields {
        let field = lossy(field);
        if let Some(value) = field.strip_prefix("worktree ") {
            if value.is_empty() || path.replace(value.to_owned()).is_some() {
                return Err(AppError::new("git_parse_error", "无法解析 Git 工作树信息"));
            }
        } else if let Some(value) = field.strip_prefix("HEAD ") {
            head = value.to_owned();
        } else if let Some(value) = field.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_owned(),
            );
        } else if field == "detached" {
            is_detached = true;
        } else if field == "locked" || field.starts_with("locked ") {
            is_locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            is_prunable = true;
        }
    }

    Ok(WorktreeInfo {
        path: path.ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 工作树信息"))?,
        head,
        branch,
        is_main,
        is_detached,
        is_locked,
        is_prunable,
    })
}

fn parse_submodules(raw: &[u8]) -> Result<Vec<SubmoduleInfo>, AppError> {
    let mut submodules = Vec::new();
    for line in output_text(raw)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let (marker, remainder) = line.split_at(1);
        let status = match marker {
            " " => SubmoduleStatus::Initialized,
            "-" => SubmoduleStatus::Uninitialized,
            "+" => SubmoduleStatus::NeedsUpdate,
            "U" => SubmoduleStatus::Conflicted,
            _ => return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态")),
        };
        let mut fields = remainder.trim_start().splitn(2, char::is_whitespace);
        let commit = fields
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
        let path_with_description = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
        // Git appends an optional parenthesized description after the path.
        // Paths containing a literal " (" are uncommon but remain usable for
        // update-all, which never round-trips the displayed path to Git.
        let path = path_with_description
            .rsplit_once(" (")
            .map(|(path, _)| path)
            .unwrap_or(path_with_description)
            .trim();
        if path.is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"));
        }
        submodules.push(SubmoduleInfo {
            path: path.to_owned(),
            commit: commit.to_owned(),
            status,
        });
    }
    Ok(submodules)
}

fn parse_lfs_files(raw: &[u8]) -> Vec<String> {
    output_text(raw)
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_commits(raw: &[u8]) -> Result<Vec<CommitSummary>, AppError> {
    let mut fields: Vec<_> = raw.split(|byte| *byte == 0).collect();
    if matches!(fields.last(), Some(field) if field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 7 != 0 {
        return Err(AppError::new("git_parse_error", "无法解析 Git 提交历史"));
    }

    let mut commits = Vec::with_capacity(fields.len() / 7);
    for (record_index, fields) in fields.chunks_exact(7).enumerate() {
        // `git log --format=format:...` inserts one LF between adjacent
        // records. It precedes the next SHA, outside the NUL-delimited data.
        let sha = if record_index == 0 {
            fields[0]
        } else {
            fields[0].strip_prefix(b"\n").unwrap_or(fields[0])
        };
        if sha.is_empty() || [1, 3, 4, 5].iter().any(|index| fields[*index].is_empty()) {
            return Err(AppError::new("git_parse_error", "无法解析 Git 提交历史"));
        }
        commits.push(CommitSummary {
            sha: lossy(sha),
            short_sha: lossy(fields[1]),
            parents: lossy(fields[2])
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect(),
            author_name: lossy(fields[3]),
            author_email: lossy(fields[4]),
            authored_at: lossy(fields[5]),
            summary: lossy(fields[6]),
        });
    }
    Ok(commits)
}

fn parse_stashes(raw: &[u8]) -> Result<Vec<StashEntry>, AppError> {
    let mut fields: Vec<_> = raw.split(|byte| *byte == 0).collect();
    if matches!(fields.last(), Some(field) if field.is_empty()) {
        fields.pop();
    }
    if matches!(fields.last(), Some(field) if *field == b"\n") {
        fields.pop();
    }
    if fields.len() % 2 != 0 {
        return Err(AppError::new("git_parse_error", "无法解析 Git 暂存栈"));
    }

    let mut stashes = Vec::with_capacity(fields.len() / 2);
    for (index, fields) in fields.chunks_exact(2).enumerate() {
        let reference = if index == 0 {
            fields[0]
        } else {
            fields[0].strip_prefix(b"\n").unwrap_or(fields[0])
        };
        if reference.is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 暂存栈"));
        }
        stashes.push(StashEntry {
            reference: lossy(reference),
            message: lossy(fields[1]),
        });
    }
    Ok(stashes)
}

fn parse_tags(raw: &[u8]) -> Result<Vec<TagInfo>, AppError> {
    let mut tags = Vec::new();
    for line in output_text(raw).lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 2 || fields[0].is_empty() || fields[1].is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 标签信息"));
        }
        tags.push(TagInfo {
            name: fields[0].to_owned(),
            target: fields[1].to_owned(),
        });
    }
    Ok(tags)
}

fn validate_paths(paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Err(AppError::new(
            "invalid_pathspec",
            "请选择至少一个有效文件路径",
        ));
    }
    for path in paths {
        if path.is_empty() || path.contains('\0') {
            return Err(AppError::new(
                "invalid_pathspec",
                "请选择至少一个有效文件路径",
            ));
        }
        validate_worktree_relative_path(path)?;
    }
    Ok(())
}

/// UI paths identify one status entry, never a Git path pattern. Prefix them
/// with Git's literal magic after validation so names such as `:(glob)**` are
/// treated as filenames rather than a request to affect the whole worktree.
fn literal_pathspec(path: &str) -> OsString {
    OsString::from(format!(":(literal){path}"))
}

/// Extra validation used before filesystem removal. Local deletion must prove
/// that a user-provided path cannot escape the worktree; Git-facing calls add
/// [`literal_pathspec`] separately to prevent pathspec magic expansion.
fn validate_worktree_relative_path(value: &str) -> Result<(), AppError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\0')
        || value.contains(['\r', '\n'])
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AppError::new(
            "invalid_pathspec",
            "要丢弃的文件路径必须位于当前仓库内",
        ));
    }
    Ok(())
}

fn validate_worktree_path(path: &Path) -> Result<(), AppError> {
    let value = path.to_string_lossy();
    if value.trim().is_empty() || !path.is_absolute() || value.starts_with('-') {
        return Err(AppError::new(
            "invalid_worktree_path",
            "工作树路径必须是有效的绝对路径",
        ));
    }
    Ok(())
}

fn validate_lfs_pattern(value: &str) -> Result<(), AppError> {
    let pattern = value.trim();
    if pattern.is_empty()
        || pattern.starts_with('-')
        || pattern.contains('\0')
        || pattern.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_lfs_pattern", "Git LFS 跟踪规则无效"));
    }
    Ok(())
}

fn validate_git_patch(patch: &str) -> Result<(), AppError> {
    const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
    if patch.is_empty()
        || patch.len() > MAX_PATCH_BYTES
        || patch.contains('\0')
        || !patch.starts_with("diff --git ")
    {
        return Err(AppError::new(
            "invalid_git_patch",
            "代码块补丁无效或超过大小限制",
        ));
    }
    Ok(())
}

fn paths_match(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

const MAX_UNTRACKED_DIFF_BYTES: usize = 4 * 1024 * 1024;

fn render_untracked_file_diff(path: &str, bytes: &[u8]) -> String {
    let header = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    let Ok(text) = std::str::from_utf8(bytes) else {
        return format!("{header}Binary files /dev/null and b/{path} differ\n");
    };
    if text.is_empty() {
        return header;
    }

    let mut diff = format!("{header}@@ -0,0 +1,{} @@\n", text.lines().count());
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        diff.push('+');
        diff.push_str(body);
        diff.push('\n');
    }
    if !text.ends_with('\n') {
        diff.push_str("\\ No newline at end of file\n");
    }
    diff
}

#[cfg(unix)]
fn read_untracked_file_safely(repository: &Path, relative_path: &str) -> Result<Vec<u8>, AppError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    validate_worktree_relative_path(relative_path)?;
    let repository_path = CString::new(repository.as_os_str().as_bytes())
        .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
    // SAFETY: `repository_path` is NUL-terminated and remains live during the
    // call. O_NOFOLLOW pins the repository root to a real directory instead
    // of resolving a symlink supplied by another local process.
    let root_fd = unsafe {
        libc::open(
            repository_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "无法安全读取未跟踪文件的改动内容。",
        ));
    }
    // SAFETY: open returned a new owned descriptor on success.
    let mut parent = unsafe { fs::File::from_raw_fd(root_fd) };
    let components: Vec<_> = Path::new(relative_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => Some(component),
            _ => None,
        })
        .collect();
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(AppError::new("invalid_pathspec", "未跟踪文件路径无效"));
    };

    for component in parent_components {
        let component = CString::new(component.as_bytes())
            .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
        // SAFETY: parent is an open directory descriptor and component is a
        // NUL-terminated single path component. O_NOFOLLOW prevents an
        // intermediate replacement from escaping the worktree.
        let directory_fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_fd < 0 {
            return Err(AppError::new(
                "untracked_diff_unavailable",
                "无法安全读取未跟踪文件的改动内容。",
            ));
        }
        // SAFETY: openat returned a new owned descriptor on success.
        parent = unsafe { fs::File::from_raw_fd(directory_fd) };
    }

    let file_name = CString::new(file_name.as_bytes())
        .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
    // SAFETY: parent is pinned above and file_name is a NUL-terminated final
    // component. O_NOFOLLOW ensures this final lookup cannot follow a swapped
    // symlink outside the opened worktree hierarchy.
    let file_fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if file_fd < 0 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "无法安全读取未跟踪文件的改动内容。",
        ));
    }
    // SAFETY: openat returned a new owned descriptor on success.
    let mut file = unsafe { fs::File::from_raw_fd(file_fd) };
    let metadata = file
        .metadata()
        .map_err(|_| AppError::new("untracked_diff_unavailable", "无法读取未跟踪文件。"))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_UNTRACKED_DIFF_BYTES as u64 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "未跟踪文件不是可安全显示的常规文本文件，或内容过大。",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| AppError::new("untracked_diff_unavailable", "无法读取未跟踪文件。"))?;
    if bytes.len() > MAX_UNTRACKED_DIFF_BYTES {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "未跟踪文件内容过大，无法安全显示。",
        ));
    }
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_untracked_file_safely(
    _repository: &Path,
    _relative_path: &str,
) -> Result<Vec<u8>, AppError> {
    // The standard library does not provide descriptor-relative no-follow
    // traversal on this platform. Do not fall back to a path check followed by
    // a second path open, which could expose data outside the worktree.
    Err(AppError::new(
        "untracked_diff_unavailable",
        "当前平台无法安全读取未跟踪文件的改动内容。",
    ))
}

fn validate_remote_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty()
        || name.starts_with('-')
        || name.contains('\0')
        || name.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_remote_name", "远程名称无效"));
    }
    Ok(())
}

fn validate_ref_name(value: &str, _label: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_git_reference", "Git 引用无效。"));
    }
    Ok(())
}

/// Revision expressions such as `HEAD~2` and `stash@{0}` are valid Git
/// inputs but still must never be interpreted as an option or span lines.
fn validate_revision(value: &str, _label: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_git_revision", "Git 修订版本无效。"));
    }
    Ok(())
}

fn validate_refspec(value: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_refspec", "引用规格无效"));
    }
    Ok(())
}

/// Validate every URL form accepted by `git clone` and remote configuration.
/// This intentionally permits public HTTP, `git://`, file, and local-path
/// remotes while rejecting values that could inject options or credentials.
pub(crate) fn validate_git_remote_url(raw: &str) -> Result<(), AppError> {
    let value = raw.trim();
    if value.is_empty() || value.starts_with('-') || value.contains(['\r', '\n', '\0']) {
        return Err(AppError::new("invalid_remote_url", "远程地址无效"));
    }
    if raw_http_url_contains_unsafe_metadata(value) {
        return Err(AppError::new(
            "credential_in_remote_url",
            "远程地址不能包含认证信息或令牌",
        ));
    }

    if has_supported_url_scheme(value)
        && (has_invalid_percent_escape(value) || Url::parse(value).is_err())
    {
        return Err(AppError::new("invalid_remote_url", "远程地址无效"));
    }
    if let Ok(url) = Url::parse(value) {
        let scheme = url.scheme().to_ascii_lowercase();
        let http = matches!(scheme.as_str(), "http" | "https");
        let unauthenticated = matches!(scheme.as_str(), "git" | "file");
        if !matches!(
            scheme.as_str(),
            "https" | "http" | "ssh" | "git+ssh" | "git" | "file"
        ) {
            return Err(AppError::new(
                "invalid_remote_url",
                "远程地址使用了不受支持的协议",
            ));
        }
        if !matches!(scheme.as_str(), "file") && url.host_str().is_none() {
            return Err(AppError::new("invalid_remote_url", "远程地址无效"));
        }
        if url.password().is_some()
            || (http && !url.username().is_empty())
            || (http && (url.query().is_some() || url.fragment().is_some()))
            || (matches!(scheme.as_str(), "ssh" | "git+ssh")
                && (url.query().is_some() || url.fragment().is_some()))
            || (unauthenticated
                && (!url.username().is_empty()
                    || url.query().is_some()
                    || url.fragment().is_some()))
        {
            return Err(AppError::new(
                "credential_in_remote_url",
                "远程地址不能包含认证信息或令牌",
            ));
        }
    }
    Ok(())
}

fn has_supported_url_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "https" | "http" | "ssh" | "git+ssh" | "git" | "file"
    )
}

fn has_invalid_percent_escape(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return true;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    false
}

/// Inspect the raw HTTP authority before generic URL parsing. A malformed URL
/// such as `https://token@%zz/repo` can fail parsing but would otherwise be
/// passed through to Git configuration with its userinfo intact.
fn raw_http_url_contains_unsafe_metadata(value: &str) -> bool {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@') || remainder.contains(['?', '#'])
}

fn is_unsafe_network_config_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    let unsafe_http_key = key.starts_with("http.")
        && (matches!(
            key.as_str(),
            "http.extraheader"
                | "http.proxy"
                | "http.sslverify"
                | "http.cookiefile"
                | "http.savecookies"
        ) || key.ends_with(".extraheader")
            || key.ends_with(".proxy")
            || key.ends_with(".sslverify")
            || key.ends_with(".cookiefile")
            || key.ends_with(".savecookies"));
    let unsafe_remote_key = key.starts_with("remote.")
        && (key.ends_with(".sshcommand")
            || key.ends_with(".uploadpack")
            || key.ends_with(".receivepack"));
    let unsafe_url_rewrite_key =
        key.starts_with("url.") && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof"));
    key == "core.sshcommand"
        || key == "core.gitproxy"
        || key == "include.path"
        || key.starts_with("includeif.") && key.ends_with(".path")
        || key == "credential.helper"
        || key.starts_with("credential.")
        || unsafe_http_key
        || unsafe_remote_key
        || unsafe_url_rewrite_key
}

fn git_command_error(operation: &'static str, output: &Output) -> AppError {
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    git_error_from_diagnostic(operation, &diagnostic)
}

/// Classifies known Git failures but never returns original process output.
/// Git may echo a URL or a server response containing a token, so forwarding
/// raw stdout/stderr would violate the credential boundary.
fn git_error_from_diagnostic(operation: &'static str, diagnostic: &str) -> AppError {
    let (code, guidance) = if contains_any(
        diagnostic,
        &[
            "authentication failed",
            "could not read username",
            "terminal prompts disabled",
            "permission denied (publickey)",
            "http basic: access denied",
            "invalid username or password",
            "access denied",
        ],
    ) {
        (
            "git_authentication_failed",
            "请检查 remote 绑定的账号、SSH agent 或系统 Git credential helper。",
        )
    } else if contains_any(
        diagnostic,
        &[
            "could not resolve host",
            "failed to connect",
            "network is unreachable",
            "connection timed out",
        ],
    ) {
        ("git_network_error", "请检查网络连接、代理和远程服务状态。")
    } else if contains_any(
        diagnostic,
        &[
            "repository not found",
            "does not appear to be a git repository",
            "remote: not found",
        ],
    ) {
        (
            "git_remote_not_found",
            "请检查远程地址、访问权限和账号绑定。",
        )
    } else if diagnostic.contains("not a git repository") {
        (
            "not_a_git_repository",
            "请确认所选目录仍是有效的 Git 工作区。",
        )
    } else if diagnostic.contains("conflict") {
        ("git_merge_conflict", "Git 检测到冲突；请处理冲突后再继续。")
    } else {
        (
            "git_command_failed",
            "Git 未能完成操作；请检查仓库状态和 Git 配置。",
        )
    };
    let _ = operation;
    AppError::with_safe_details(code, "Git 命令失败。", guidance)
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn safe_remote_url(raw: &str) -> String {
    // The common domain sanitizer is always the first pass, as required for
    // every remote URL sent over IPC.  The second pass removes HTTP query and
    // fragment values from pre-existing user config, which could otherwise
    // carry an access token.
    let sanitized = sanitize_remote_url(raw);
    if let Ok(mut url) = Url::parse(&sanitized) {
        if matches!(url.scheme(), "http" | "https") {
            url.set_query(None);
            url.set_fragment(None);
        }
        return url.to_string();
    }
    sanitized
}

fn repository_record_from_root(root: &Path) -> RepositoryRecord {
    let root = normalize_path(root);
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    RepositoryRecord {
        path: root.to_string_lossy().into_owned(),
        name,
        group: None,
        host: None,
        namespace: None,
        last_opened_at: None,
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn status_parse_error() -> AppError {
    AppError::new("git_parse_error", "无法解析 Git 仓库状态")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn as_strings(arguments: &[OsString]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn builds_repository_command_arguments_without_a_shell() {
        let arguments = command_arguments(
            Some(Path::new("/workspace/repository with spaces")),
            &os_arguments(&["status", "--porcelain=v2", "-z"]),
        );
        assert_eq!(
            as_strings(&arguments),
            vec![
                "-C",
                "/workspace/repository with spaces",
                "status",
                "--porcelain=v2",
                "-z",
            ]
        );
    }

    #[test]
    fn git_children_replace_inherited_credential_environment() {
        let environment = GitEnvironment::new().with_variable("BKM_GIT_ASKPASS_MODE", "trusted");
        let mut command = Command::new("git");
        command.env("GIT_ASKPASS", "/untrusted/askpass");
        command.env("GIT_CONFIG_COUNT", "1");
        command.env("GIT_DIR", "/untrusted/git-dir");
        apply_git_environment(&mut command, &environment);
        let values: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
            .collect();

        assert_eq!(values.get(OsStr::new("GIT_ASKPASS")), Some(&None));
        assert_eq!(values.get(OsStr::new("GIT_CONFIG_COUNT")), Some(&None));
        assert_eq!(values.get(OsStr::new("GIT_DIR")), Some(&None));
        assert_eq!(
            values.get(OsStr::new("GIT_TERMINAL_PROMPT")),
            Some(&Some(OsString::from("0")))
        );
        assert_eq!(
            values.get(OsStr::new("BKM_GIT_ASKPASS_MODE")),
            Some(&Some(OsString::from("trusted")))
        );
        assert_eq!(
            values.get(OsStr::new("LC_ALL")),
            Some(&Some(OsString::from("C")))
        );
        assert_eq!(
            values.get(OsStr::new("LANG")),
            Some(&Some(OsString::from("C")))
        );
    }

    #[test]
    fn explicit_credentials_clear_repository_helper_configuration() {
        let mut environment = GitEnvironment::new();
        environment.isolate_network_credentials();
        let mut command = Command::new("git");
        apply_git_environment(&mut command, &environment);
        let values: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
            .collect();

        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_COUNT")),
            Some(&Some(OsString::from("6")))
        );
        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_KEY_0")),
            Some(&Some(OsString::from("credential.helper")))
        );
        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_VALUE_0")),
            Some(&Some(OsString::new()))
        );
    }

    #[test]
    fn askpass_credentials_disable_hooks_before_git_spawns() {
        let mut environment = GitEnvironment::new();
        environment.isolate_network_credentials_and_disable_hooks();
        let mut command = Command::new("git");
        apply_git_environment(&mut command, &environment);
        let values: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
            .collect();

        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_COUNT")),
            Some(&Some(OsString::from("7")))
        );
        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_KEY_6")),
            Some(&Some(OsString::from("core.hooksPath")))
        );
        assert_eq!(
            values.get(OsStr::new("GIT_CONFIG_VALUE_6")),
            Some(&Some(OsString::from(DISABLED_GIT_HOOKS_PATH)))
        );
    }

    #[test]
    fn builds_single_remote_fetch_arguments() {
        let arguments = fetch_arguments(&FetchOptions {
            remote: "origin".into(),
            refspecs: vec!["refs/heads/main:refs/remotes/origin/main".into()],
            prune: true,
        })
        .unwrap();
        assert_eq!(
            as_strings(&arguments),
            vec![
                "fetch",
                "--prune",
                "origin",
                "refs/heads/main:refs/remotes/origin/main",
            ]
        );
    }

    #[test]
    fn rejects_credential_bearing_https_remote_urls() {
        let error = validate_git_remote_url("https://token-value@gitee.com/team/repository.git")
            .unwrap_err();
        assert_eq!(error.code(), "credential_in_remote_url");
        assert!(!error.message().contains("token-value"));
        assert_eq!(
            validate_git_remote_url("https://token-value@%zz/team/repository.git")
                .expect_err("malformed HTTP userinfo must never reach Git config")
                .code(),
            "credential_in_remote_url"
        );
        assert_eq!(
            validate_git_remote_url("https://gitee.com/%zz/team/repository.git")
                .expect_err("malformed HTTP URLs must be rejected")
                .code(),
            "invalid_remote_url"
        );
        assert!(validate_git_remote_url("git@gitee.com:team/repository.git").is_ok());
        assert!(
            validate_git_remote_url("git://token-value@gitee.com/team/repository.git").is_err()
        );
        assert!(validate_git_remote_url("ext::unsafe-helper %S").is_err());
        assert!(validate_git_remote_url("--mirror=fetch").is_err());
    }

    #[test]
    fn parses_porcelain_v2_status_with_rename_and_untracked_path() {
        let raw = concat!(
            "# branch.head main\n",
            "# branch.upstream origin/main\n",
            "# branch.ab +2 -3\n",
            "1 M. N... 100644 100644 100644 abcdef1 abcdef2 README.md\0",
            "2 R. N... 100644 100644 100644 abcdef1 abcdef2 R100 renamed file.txt\0",
            "original file.txt\0",
            "? untracked file.txt\0"
        )
        .as_bytes();

        let status = parse_status_porcelain_v2(raw).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 3));
        assert_eq!(status.entries.len(), 3);
        assert_eq!(status.entries[0].index_status, "M");
        assert_eq!(status.entries[0].worktree_status, "");
        assert_eq!(status.entries[1].path, "renamed file.txt");
        assert_eq!(
            status.entries[1].original_path.as_deref(),
            Some("original file.txt")
        );
        assert_eq!(status.entries[2].path, "untracked file.txt");
        assert_eq!(status.entries[2].index_status, "");
        assert_eq!(status.entries[2].worktree_status, "?");
    }

    #[test]
    fn parses_nul_terminated_status_headers_too() {
        let raw = concat!(
            "# branch.head feature\0",
            "# branch.ab +0 -1\0",
            "1 A. N... 100644 100644 100644 a b src/lib.rs\0",
        )
        .as_bytes();
        let status = parse_status_porcelain_v2(raw).unwrap();
        assert_eq!(status.branch.as_deref(), Some("feature"));
        assert_eq!((status.ahead, status.behind), (0, 1));
        assert_eq!(status.entries[0].path, "src/lib.rs");
        assert_eq!(status.entries[0].index_status, "A");
        assert_eq!(status.entries[0].worktree_status, "");
    }

    #[test]
    fn parses_branch_and_tag_output() {
        let branches = parse_branches(b"*\0main\0origin/main\n \0topic\0\n").unwrap();
        assert_eq!(branches.len(), 2);
        assert!(branches[0].is_current);
        assert_eq!(branches[1].upstream, None);

        let tags = parse_tags(b"v1.0.0\0abc123\nv0.9.0\0def456\n").unwrap();
        assert_eq!(tags[0].name, "v1.0.0");
        assert_eq!(tags[1].target, "def456");
    }

    #[test]
    fn parses_worktree_porcelain_with_linked_state() {
        let raw = concat!(
            "worktree /workspace/main\0",
            "HEAD abcdef123\0",
            "branch refs/heads/main\0\0",
            "worktree /workspace/topic\0",
            "HEAD fedcba987\0",
            "detached\0",
            "locked maintenance\0",
            "prunable stale metadata\0\0",
        )
        .as_bytes();
        let worktrees = parse_worktrees(raw).unwrap();
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees[0].is_main);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(!worktrees[1].is_main);
        assert!(worktrees[1].is_detached);
        assert!(worktrees[1].is_locked);
        assert!(worktrees[1].is_prunable);
    }

    #[test]
    fn parses_submodule_status_markers() {
        let raw = concat!(
            " abcdef0123456789 modules/ready (heads/main)\n",
            "-fedcba9876543210 modules/missing\n",
            "+0011223344556677 modules/stale (new commits)\n",
            "U8899aabbccddeeff modules/conflicted\n",
        )
        .as_bytes();
        let submodules = parse_submodules(raw).unwrap();
        assert_eq!(submodules.len(), 4);
        assert_eq!(submodules[0].path, "modules/ready");
        assert_eq!(submodules[0].status, SubmoduleStatus::Initialized);
        assert_eq!(submodules[1].status, SubmoduleStatus::Uninitialized);
        assert_eq!(submodules[2].status, SubmoduleStatus::NeedsUpdate);
        assert_eq!(submodules[3].status, SubmoduleStatus::Conflicted);
    }

    #[test]
    fn parses_lfs_file_names_and_validates_patterns() {
        assert_eq!(
            parse_lfs_files(b"assets/video.mov\nassets/model.bin\n\n"),
            vec!["assets/video.mov", "assets/model.bin"]
        );
        assert!(validate_lfs_pattern("*.psd").is_ok());
        assert!(validate_lfs_pattern("-unsafe").is_err());
        assert!(validate_lfs_pattern("*.zip\n--help").is_err());
    }

    #[test]
    fn patch_validation_accepts_unified_git_diff_only() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        assert!(validate_git_patch(patch).is_ok());
        assert!(validate_git_patch("@@ -1 +1 @@\n-old\n+new\n").is_err());
        assert!(validate_git_patch("diff --git a/a b/a\0").is_err());
    }

    #[test]
    fn applies_and_reverses_a_generated_index_patch() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-patch-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let run_git = |arguments: &[&str]| {
            let status = Command::new("git")
                .args(["-C", repository.to_str().unwrap()])
                .args(arguments)
                .status()
                .unwrap();
            assert!(status.success());
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "test@example.invalid"]);
        run_git(&["config", "user.name", "Patch Test"]);
        fs::write(repository.join("file.txt"), "before\n").unwrap();
        run_git(&["add", "file.txt"]);
        run_git(&["commit", "-qm", "initial"]);
        fs::write(repository.join("file.txt"), "after\n").unwrap();

        let git = GitCore::new();
        let head = git.head_commit_id(&repository).unwrap();
        assert_eq!(head.len(), 40);
        assert!(head.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let patch = git
            .diff(
                &repository,
                &DiffOptions {
                    staged: false,
                    paths: vec!["file.txt".into()],
                    context_lines: Some(3),
                },
            )
            .unwrap();
        git.apply_patch(&repository, &patch, false).unwrap();
        let staged = git.status(&repository).unwrap();
        assert_eq!(staged.entries[0].index_status, "M");
        assert_eq!(staged.entries[0].worktree_status, "");

        let cached_patch = git
            .diff(
                &repository,
                &DiffOptions {
                    staged: true,
                    paths: vec!["file.txt".into()],
                    context_lines: Some(3),
                },
            )
            .unwrap();
        git.apply_patch(&repository, &cached_patch, true).unwrap();
        let unstaged = git.status(&repository).unwrap();
        assert_eq!(unstaged.entries[0].index_status, "");
        assert_eq!(unstaged.entries[0].worktree_status, "M");

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn unstages_files_without_head_in_an_unborn_repository() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-unstage-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let initialized = Command::new("git")
            .args(["-C", repository.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success());
        fs::write(repository.join("first.txt"), "first\n").unwrap();

        let git = GitCore::new();
        git.stage_paths(&repository, &["first.txt".into()]).unwrap();
        let staged = git.status(&repository).unwrap();
        assert_eq!(staged.entries[0].index_status, "A");
        assert_eq!(staged.entries[0].worktree_status, "");

        git.unstage_paths(&repository, &["first.txt".into()])
            .unwrap();
        let unstaged = git.status(&repository).unwrap();
        assert_eq!(unstaged.entries[0].index_status, "");
        assert_eq!(unstaged.entries[0].worktree_status, "?");
        assert!(repository.join("first.txt").is_file());

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn discard_staged_rename_restores_both_original_and_destination_paths() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-discard-rename-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let run_git = |arguments: &[&str]| {
            let status = Command::new("git")
                .args(["-C", repository.to_str().unwrap()])
                .args(arguments)
                .status()
                .unwrap();
            assert!(status.success());
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "discard@example.invalid"]);
        run_git(&["config", "user.name", "Discard Test"]);
        fs::write(repository.join("old-name.txt"), "original\n").unwrap();
        run_git(&["add", "old-name.txt"]);
        run_git(&["commit", "-qm", "initial"]);
        run_git(&["mv", "old-name.txt", "new-name.txt"]);

        GitCore::new()
            .discard_paths(&repository, &["new-name.txt".into()])
            .expect("discarding a staged rename restores HEAD");

        assert_eq!(
            fs::read_to_string(repository.join("old-name.txt")).unwrap(),
            "original\n"
        );
        assert!(!repository.join("new-name.txt").exists());

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn local_credential_helper_configuration_is_rejected_before_network_git() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-unsafe-config-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let initialized = Command::new("git")
            .args(["-C", repository.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success());
        let configured = Command::new("git")
            .args([
                "-C",
                repository.to_str().unwrap(),
                "config",
                "credential.helper",
                "!echo should-not-run",
            ])
            .status()
            .unwrap();
        assert!(configured.success());

        assert_eq!(
            GitCore::new()
                .ensure_network_configuration_is_safe(&repository)
                .expect_err("repository-local helper must not run for a selected account")
                .code(),
            "unsafe_git_config"
        );

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn local_git_config_includes_are_rejected_before_network_git() {
        assert!(is_unsafe_network_config_key("include.path"));
        assert!(is_unsafe_network_config_key(
            "includeIf.gitdir:/work/**.path"
        ));
        assert!(is_unsafe_network_config_key(
            "url.https://replacement.example/.insteadOf"
        ));
        assert!(is_unsafe_network_config_key(
            "url.ssh://replacement.example/.pushInsteadOf"
        ));
    }

    #[test]
    fn renders_untracked_file_content_as_an_addition() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-untracked-diff-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let initialized = Command::new("git")
            .args(["-C", repository.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success());
        fs::write(repository.join("new file.txt"), "first line\nsecond line\n").unwrap();

        let git = GitCore::new();
        let diff = git
            .diff(
                &repository,
                &DiffOptions {
                    staged: false,
                    paths: vec!["new file.txt".into()],
                    context_lines: Some(3),
                },
            )
            .expect("an untracked text file has a displayable diff");

        assert!(diff.contains("new file mode 100644"));
        assert!(diff.contains("+++ b/new file.txt"));
        assert!(diff.contains("+first line"));
        assert!(diff.contains("+second line"));
        assert_eq!(
            git.diff(
                &repository,
                &DiffOptions {
                    staged: false,
                    paths: vec!["../outside.txt".into()],
                    context_lines: Some(3),
                },
            )
            .expect_err("no-index fallback must not read outside the worktree")
            .code(),
            "invalid_pathspec"
        );

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn parses_commit_history_and_stash_output() {
        let raw = concat!(
            "0123456789abcdef\0",
            "0123456\0",
            "parent-one parent-two\0",
            "Ada Lovelace\0",
            "ada@example.test\0",
            "2026-07-19T10:20:30+08:00\0",
            "Implement \x1ehistory\0",
            "\n",
            "fedcba9876543210\0",
            "fedcba9\0",
            "\0",
            "Grace Hopper\0",
            "grace@example.test\0",
            "2026-07-18T09:00:00+08:00\0",
            "Initial commit\0"
        )
        .as_bytes();
        let commits = parse_commits(raw).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec!["parent-one", "parent-two"]);
        assert_eq!(commits[0].summary, "Implement \u{1e}history");
        assert!(commits[1].parents.is_empty());
        assert_eq!(commits[1].summary, "Initial commit");

        let stashes = parse_stashes(
            b"stash@{0}\0On main: first line\nsecond line\0\nstash@{1}\0On topic: draft\0",
        )
        .unwrap();
        assert_eq!(stashes.len(), 2);
        assert_eq!(stashes[0].reference, "stash@{0}");
        assert_eq!(stashes[0].message, "On main: first line\nsecond line");
        assert_eq!(stashes[1].message, "On topic: draft");
    }

    #[test]
    fn lists_real_git_history_without_extra_format_records() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-history-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        let run_git = |arguments: &[&str]| {
            let status = Command::new("git")
                .args(["-C", repository.to_str().unwrap()])
                .args(arguments)
                .status()
                .unwrap();
            assert!(status.success());
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "history@example.invalid"]);
        run_git(&["config", "user.name", "History Test"]);
        fs::write(repository.join("history.txt"), "first\n").unwrap();
        run_git(&["add", "history.txt"]);
        run_git(&["commit", "-qm", "first commit"]);
        fs::write(repository.join("history.txt"), "second\n").unwrap();
        run_git(&["commit", "-am", "second commit"]);

        let commits = GitCore::new().commits(&repository, 100).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].summary, "second commit");
        assert_eq!(commits[1].summary, "first commit");
        assert!(commits.iter().all(|commit| {
            commit.sha.len() == 40 && commit.sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        }));
        GitCore::new()
            .commit_diff(&repository, &commits[1].sha)
            .expect("every listed SHA can be used to read its commit diff");

        let _ = fs::remove_dir_all(repository);
    }

    #[test]
    fn revision_validation_accepts_git_expressions_but_rejects_options() {
        assert!(validate_revision("HEAD~2", "引用").is_ok());
        assert!(validate_revision("stash@{0}", "引用").is_ok());
        assert!(validate_revision("-bad", "引用").is_err());
        assert!(validate_revision("HEAD\n--hard", "引用").is_err());
    }

    #[test]
    fn discard_path_validation_cannot_escape_the_worktree() {
        assert!(validate_worktree_relative_path("src/App.vue").is_ok());
        assert!(validate_worktree_relative_path("nested/file name.txt").is_ok());
        assert!(validate_worktree_relative_path("../outside.txt").is_err());
        assert!(validate_worktree_relative_path("./file.txt").is_err());
        assert!(validate_worktree_relative_path(".").is_err());
    }

    #[test]
    fn ui_paths_are_converted_to_literal_git_pathspecs() {
        assert_eq!(
            literal_pathspec(":(glob)**").to_string_lossy(),
            ":(literal):(glob)**"
        );
        assert_eq!(
            literal_pathspec("src/App.vue").to_string_lossy(),
            ":(literal)src/App.vue"
        );
        assert!(validate_paths(&[":(exclude)important.txt".into()]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn untracked_diff_reader_rejects_symlinked_components() {
        use std::os::unix::fs::symlink;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-symlink-test-{}-{suffix}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "broknowmygitdesktop-symlink-outside-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&repository).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        symlink(&outside, repository.join("linked")).unwrap();

        assert_eq!(
            read_untracked_file_safely(&repository, "linked/secret.txt")
                .unwrap_err()
                .code(),
            "untracked_diff_unavailable"
        );

        let _ = fs::remove_dir_all(repository);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn git_failure_never_echoes_token_like_diagnostic_content() {
        let error = git_error_from_diagnostic(
            "推送改动",
            "remote: authentication failed for https://secret-token@gitee.com/org/repo.git",
        );
        let returned = format!(
            "{} {}",
            error.message(),
            error.details().unwrap_or_default()
        );
        assert_eq!(error.code(), "git_authentication_failed");
        assert!(!returned.contains("secret-token"));
    }

    #[test]
    fn returned_remote_urls_run_through_domain_sanitizer() {
        assert_eq!(
            safe_remote_url("https://token-value@github.com/org/repo.git?access_token=hidden"),
            "https://github.com/org/repo.git"
        );
    }
}
