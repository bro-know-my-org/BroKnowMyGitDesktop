use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

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
        parse_status_porcelain_v2(&output.stdout)
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
        let _ = thread::Builder::new()
            .name("bkm-external-editor-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
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

    /// Previews files and directories that a normal clean operation would
    /// delete. Ignored files are deliberately excluded; cleaning them needs a
    /// separate, more explicit workflow.
    pub fn preview_clean_untracked(&self, repository: &Path) -> Result<String, AppError> {
        self.ensure_no_operation(repository)?;
        let environment = GitEnvironment::new()
            .with_variable("LC_ALL", "C")
            .with_variable("LANG", "C");
        let output = self.run(
            Some(repository),
            &os_arguments(&["clean", "-nd"]),
            &environment,
            "预览清理未跟踪文件",
        )?;
        Ok(output_text(&output.stdout))
    }

    /// Deletes exactly the previewed, untracked paths. Supplying individual
    /// literal file pathspecs prevents a later broad clean from removing a
    /// file created after the user reviewed the preview.
    pub fn clean_untracked_paths(
        &self,
        repository: &Path,
        paths: &[String],
    ) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
        if paths.is_empty() {
            return Ok(());
        }
        validate_paths(paths)?;
        let mut arguments = os_arguments(&["clean", "-f", "--"]);
        arguments.extend(paths.iter().map(|path| literal_pathspec(path)));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "清理已预览的未跟踪文件",
        )?;
        remove_empty_untracked_parent_directories(repository, paths);
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
        Ok(render_untracked_file_diff(path, &content))
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
}

/// `git clean -- <file>` intentionally leaves an empty parent directory.
/// Remove only parents of the exact deleted files, deepest first, and only
/// when they are still empty. A concurrently created file therefore makes the
/// removal fail harmlessly instead of being deleted without confirmation.
fn remove_empty_untracked_parent_directories(repository: &Path, paths: &[String]) {
    let mut directories = HashSet::new();
    for path in paths {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(directory.to_path_buf());
            parent = directory.parent();
        }
    }
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
    for directory in directories {
        let _ = fs::remove_dir(repository.join(directory));
    }
}
