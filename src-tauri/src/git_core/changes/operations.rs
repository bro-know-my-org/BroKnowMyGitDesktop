impl GitCore {
    /// Opens the repository directory in the application's configured
    /// external editor without involving a shell or repository-local config.
    pub fn open_repository_in_external_editor(
        &self,
        repository: &Path,
        editor: &ExternalEditorConfig,
    ) -> Result<(), AppError> {
        let editor = normalize_external_editor_config(editor.clone())?;
        let repository_target = fs::canonicalize(repository).map_err(|_| {
            AppError::new(
                "external_editor_repository_unavailable",
                "无法访问当前仓库，未启动外部编辑器。",
            )
        })?;
        launch_repository_editor(&repository_target, &editor, std::slice::from_ref(&repository_target))
    }

    /// Returns index paths for watcher planning. Git ignore rules can match a
    /// path after it has already been committed; those tracked files must
    /// remain observable even when an ignore-aware directory walk skips the
    /// surrounding untracked content.
    pub(crate) fn tracked_worktree_paths(
        &self,
        repository: &Path,
    ) -> Result<Vec<PathBuf>, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["ls-files", "--cached", "-z"]),
            &GitEnvironment::default(),
            "读取已跟踪路径",
        )?;
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty())
            .map(|field| {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Ok(PathBuf::from(OsString::from_vec(field.to_vec())))
                }
                #[cfg(not(unix))]
                {
                    String::from_utf8(field.to_vec())
                        .map(PathBuf::from)
                        .map_err(|_| {
                            AppError::new("git_parse_error", "无法解析用于仓库监听的已跟踪路径。")
                        })
                }
            })
            .collect()
    }

    /// Resolves the effective `core.excludesFile` with Git's own path
    /// expansion. Repository-local configuration may override the user's
    /// global ignore file, so watcher planning cannot infer this path from the
    /// process environment alone.
    pub(crate) fn worktree_excludes_file(
        &self,
        repository: &Path,
    ) -> Result<Option<WorktreeExcludesFile>, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "config",
                "--null",
                "--show-scope",
                "--path",
                "--get",
                "core.excludesFile",
            ]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            let mut fields = output.stdout.split(|byte| *byte == 0);
            let scope = fields.next().unwrap_or_default();
            let path = fields.next().unwrap_or_default();
            if scope.is_empty() || path.is_empty() || fields.any(|field| !field.is_empty()) {
                return Err(AppError::new(
                    "git_parse_error",
                    "无法读取 Git 全局忽略文件位置。",
                ));
            }
            let path = git_path_from_output(path, "无法读取 Git 全局忽略文件位置。")?;
            let path = if path.is_absolute() {
                path
            } else {
                repository.join(path)
            };
            if matches!(scope, b"local" | b"worktree") {
                let repository_anchor = fs::canonicalize(repository).map_err(|_| {
                    AppError::new("repository_not_found", "仓库目录不存在或无法访问。")
                })?;
                let git_directory_anchor = fs::canonicalize(self.git_directory(repository)?)
                    .map_err(|_| {
                        AppError::new("repository_invalid", "Git 元数据目录不存在或无法访问。")
                    })?;
                let common_directory_anchor =
                    fs::canonicalize(self.git_common_directory(repository)?).map_err(|_| {
                        AppError::new("repository_invalid", "Git 共享元数据目录不存在或无法访问。")
                    })?;
                let mut existing_path = path.as_path();
                while !existing_path.exists() {
                    let Some(parent) = existing_path.parent() else {
                        return Ok(None);
                    };
                    existing_path = parent;
                }
                let unresolved_suffix = path.strip_prefix(existing_path).map_err(|_| {
                    AppError::new(
                        "repository_watch_failed_transient",
                        "无法验证 Git 忽略文件位置。",
                    )
                })?;
                if unresolved_suffix
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Ok(None);
                }
                let existing_anchor = fs::canonicalize(existing_path).map_err(|_| {
                    AppError::new(
                        "repository_watch_failed_transient",
                        "无法验证 Git 忽略文件位置。",
                    )
                })?;
                if !existing_anchor.starts_with(repository_anchor)
                    && !existing_anchor.starts_with(git_directory_anchor)
                    && !existing_anchor.starts_with(common_directory_anchor)
                {
                    // Repository-local config is controlled by the opened
                    // repository. Never let it expand watcher scope to an
                    // arbitrary external directory.
                    return Ok(None);
                }
                let fully_resolved = unresolved_suffix.as_os_str().is_empty();
                let load_rules = fully_resolved && existing_anchor.is_file();
                let validated_path = if fully_resolved {
                    existing_anchor
                } else {
                    existing_anchor.join(unresolved_suffix)
                };
                return Ok(Some(WorktreeExcludesFile {
                    path: validated_path,
                    watch_externally: false,
                    load_rules,
                }));
            }
            return Ok(Some(WorktreeExcludesFile {
                path,
                watch_externally: true,
                load_rules: true,
            }));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Err(git_command_error("读取 Git 全局忽略文件位置", &output))
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
            // Status refreshes are observational. Git otherwise takes an
            // optional index lock to refresh cached stat information, and
            // the repository watcher interprets that temporary lock as an
            // external change, creating a self-sustaining refresh loop.
            &GitEnvironment::new().with_variable("GIT_OPTIONAL_LOCKS", "0"),
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
        launch_external_editor(repository, &editor, &files)?;
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
            if self.is_binary_resource(repository, path)? {
                return Ok("Binary files /dev/null and working tree differ\n".into());
            }
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

    fn discard_index_entries(
        &self,
        repository: &Path,
        tracked_paths: &[String],
    ) -> Result<Vec<u8>, AppError> {
        if tracked_paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut arguments = os_arguments(&["ls-files", "--stage", "-z", "--"]);
        arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
        Ok(self
            .run(
                Some(repository),
                &arguments,
                &GitEnvironment::default(),
                "读取暂存区状态",
            )?
            .stdout)
    }

    fn discard_head_reference(&self, repository: &Path) -> Result<Option<String>, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["symbolic-ref", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            let reference = output_text(&output.stdout).trim().to_owned();
            if reference.starts_with("refs/") && !reference.contains(['\r', '\n', '\0']) {
                return Ok(Some(reference));
            }
            return Err(AppError::new("git_parse_error", "无法读取当前分支引用。"));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Err(git_command_error("读取当前分支引用", &output))
    }

    #[cfg(target_os = "linux")]
    fn discard_confirmed_paths_linux(
        &self,
        repository: &Path,
        tracked_paths: &[String],
        removal_fingerprints: &[CleanUntrackedPathFingerprint],
        confirmation: &DiscardChangesFingerprint,
    ) -> Result<(), AppError> {
        let mut transaction = DiscardTransaction::create(repository)?;
        let desired_name = CString::new("desired").expect("fixed directory name has no NUL");
        transaction.create_anchored_directory(&desired_name)?;
        let desired_root =
            DiscardTransaction::anchored_path(&transaction.directory_file, "desired");
        let mut index_guard_paths = tracked_paths.to_vec();
        index_guard_paths.extend(
            removal_fingerprints
                .iter()
                .map(|fingerprint| fingerprint.path.clone()),
        );
        index_guard_paths.sort();
        index_guard_paths.dedup();
        let mut index_lock = if index_guard_paths.is_empty() {
            None
        } else {
            Some(DiscardIndexLock::acquire(self, repository)?)
        };
        let _reference_locks = if tracked_paths.is_empty() {
            None
        } else {
            Some(DiscardReferenceLocks::acquire(
                self,
                repository,
                confirmation.head_reference.as_deref(),
            )?)
        };
        if index_lock.is_some() {
            // Do not run `git status` while holding the real index.lock: it
            // may refresh the index and contend with our own guard. The locks
            // freeze conventional HEAD/index writers; compare those exact
            // states directly, while filesystem identities are checked when
            // each worktree entry is atomically claimed below.
            let head_changed = match confirmation.head_commit.as_deref() {
                Some(expected) => self.head_commit_id(repository)? != expected,
                None => self.has_head(repository)?,
            };
            if head_changed
                || self.discard_head_reference(repository)? != confirmation.head_reference
                || self.discard_index_entries(repository, &index_guard_paths)?
                    != confirmation.index_entries
            {
                return Err(discard_confirmation_changed_error());
            }
        }
        if discard_index_contains_gitlink(&confirmation.index_entries)? {
            return Err(AppError::new(
                "discard_protected_unsupported",
                "暂不支持安全丢弃子模块改动。",
            ));
        }

        let captured_head = confirmation.head_commit.as_deref();
        let next_index = index_lock
            .as_ref()
            .map(|lock| lock.prepared_index_path().to_path_buf())
            .unwrap_or_else(|| {
                DiscardTransaction::anchored_path(&transaction.directory_file, "next-index")
            });
        if !tracked_paths.is_empty() {
            let lock = index_lock.as_ref().ok_or_else(discard_filesystem_error)?;
            lock.copy_current_to_prepared()?;
            let environment = GitEnvironment::new().with_variable("GIT_INDEX_FILE", &next_index);
            if !next_index.exists() {
                self.run(
                    Some(repository),
                    &os_arguments(&["read-tree", "--empty"]),
                    &environment,
                    "准备暂存区回滚",
                )?;
            }
            if let Some(head_commit) = captured_head {
                let source = format!("--source={head_commit}");
                let mut arguments = os_arguments(&["restore"]);
                arguments.push(OsString::from(source));
                arguments.extend(os_arguments(&["--staged", "--"]));
                arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
                self.run(Some(repository), &arguments, &environment, "准备暂存区回滚")?;
            } else {
                let mut arguments = os_arguments(&["rm", "--cached", "--ignore-unmatch", "--"]);
                arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
                self.run(Some(repository), &arguments, &environment, "准备暂存区回滚")?;
            }
        }

        let mut materialized_paths = HashSet::new();
        let mut gitlink_paths = HashSet::new();
        if let Some(head_commit) = captured_head.filter(|_| !tracked_paths.is_empty()) {
            // Scope the inheritable duplicate to only the Git processes that
            // must resolve transaction-local proc-fd paths. All later Git
            // commands see only the normal CLOEXEC descriptor.
            let git_directory_anchor = transaction.inheritable_directory_anchor()?;
            let temporary_index =
                DiscardTransaction::anchored_path(&git_directory_anchor, "head-index");
            let git_desired_root =
                DiscardTransaction::anchored_path(&git_directory_anchor, "desired");
            let environment =
                GitEnvironment::new().with_variable("GIT_INDEX_FILE", &temporary_index);
            self.run(
                Some(repository),
                &os_arguments(&["read-tree", head_commit]),
                &environment,
                "准备丢弃改动",
            )?;

            let mut list_arguments = os_arguments(&["ls-files", "--stage", "-z", "--"]);
            list_arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
            let listed = self.run(
                Some(repository),
                &list_arguments,
                &environment,
                "准备丢弃改动",
            )?;
            for record in listed
                .stdout
                .split(|byte| *byte == 0)
                .filter(|record| !record.is_empty())
            {
                let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                    return Err(AppError::new("git_parse_error", "无法读取待恢复文件状态。"));
                };
                let metadata = std::str::from_utf8(&record[..tab])
                    .map_err(|_| AppError::new("git_parse_error", "无法读取待恢复文件状态。"))?;
                let path = std::str::from_utf8(&record[tab + 1..])
                    .map_err(|_| AppError::new("git_parse_error", "无法读取待恢复文件路径。"))?
                    .to_owned();
                if metadata.starts_with("160000 ") {
                    gitlink_paths.insert(path);
                } else {
                    materialized_paths.insert(path);
                }
            }

            if !gitlink_paths.is_empty() {
                return Err(AppError::new(
                    "discard_protected_unsupported",
                    "暂不支持安全丢弃子模块改动。",
                ));
            }

            if !materialized_paths.is_empty() {
                let mut prefix = OsString::from("--prefix=");
                prefix.push(&git_desired_root);
                prefix.push(std::path::MAIN_SEPARATOR.to_string());
                let mut checkout_arguments = os_arguments(&["checkout-index", "--force"]);
                checkout_arguments.push(prefix);
                checkout_arguments.push(OsString::from("--"));
                // checkout-index treats arguments after `--` as exact cache
                // entry names rather than pathspecs, so Git pathspec magic
                // would become part of the filename here.
                checkout_arguments.extend(materialized_paths.iter().map(OsString::from));
                self.run(
                    Some(repository),
                    &checkout_arguments,
                    &environment,
                    "准备丢弃改动",
                )?;
            }
        }

        let mut applied = Vec::new();
        let mut created_directories = Vec::new();
        let worktree_fingerprints = confirmation
            .worktree_paths
            .iter()
            .map(|fingerprint| match fingerprint {
                DiscardWorktreePathFingerprint::Missing(path) => (path.as_str(), fingerprint),
                DiscardWorktreePathFingerprint::Present(path) => (path.path.as_str(), fingerprint),
            })
            .collect::<HashMap<_, _>>();
        let tracked_path_set = tracked_paths
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let worktree_result = (|| -> Result<(), AppError> {
            for path in tracked_paths {
                if gitlink_paths.contains(path) {
                    continue;
                }
                let expected = *worktree_fingerprints
                    .get(path.as_str())
                    .ok_or_else(discard_confirmation_changed_error)?;
                if matches!(
                    expected,
                    DiscardWorktreePathFingerprint::Present(fingerprint)
                        if matches!(
                            fingerprint.identity.kind,
                            CleanUntrackedPathKind::Directory | CleanUntrackedPathKind::Other
                        )
                ) {
                    return Err(discard_confirmation_changed_error());
                }

                let desired_exists = materialized_paths.contains(path);
                if matches!(expected, DiscardWorktreePathFingerprint::Missing(_)) && !desired_exists
                {
                    continue;
                }
                let (target_parent, target_leaf) =
                    open_or_create_discard_parent(repository, path, &mut created_directories)?;
                let mut entry = AppliedDiscardPath {
                    target_parent,
                    target_leaf,
                    backup_name: None,
                    desired_parent: None,
                    desired_leaf: None,
                    desired_fingerprint: None,
                    desired_installed: false,
                };

                if let DiscardWorktreePathFingerprint::Present(fingerprint) = expected {
                    let backup_name = CString::new(format!("backup-{}", Uuid::new_v4()))
                        .map_err(|_| discard_filesystem_error())?;
                    rename_at_no_replace(
                        entry.target_parent.as_raw_fd(),
                        &entry.target_leaf,
                        transaction.directory_file.as_raw_fd(),
                        &backup_name,
                    )
                    .map_err(|_| discard_confirmation_changed_error())?;
                    transaction.mark_claimed();
                    entry.backup_name = Some(backup_name);
                    applied.push(entry);
                    let claimed = applied.last().expect("the claimed path is retained");
                    let identity_matches = clean_untracked_fingerprint_matches_at(
                        transaction.directory_file.as_raw_fd(),
                        claimed
                            .backup_name
                            .as_ref()
                            .expect("the backup name is retained"),
                        fingerprint,
                    )?;
                    if !identity_matches {
                        return Err(discard_confirmation_changed_error());
                    }
                } else {
                    applied.push(entry);
                }

                if desired_exists {
                    let desired_fingerprint = clean_untracked_path_fingerprint(&desired_root, path)
                        .map_err(|_| discard_filesystem_error())?;
                    let (desired_parent, desired_leaf) =
                        open_clean_untracked_parent(&desired_root, path)
                            .map_err(|_| discard_filesystem_error())?;
                    let current = applied.last_mut().expect("the pending path is retained");
                    rename_at_no_replace(
                        desired_parent.as_raw_fd(),
                        &desired_leaf,
                        current.target_parent.as_raw_fd(),
                        &current.target_leaf,
                    )
                    .map_err(|_| discard_confirmation_changed_error())?;
                    current.desired_parent = Some(desired_parent);
                    current.desired_leaf = Some(desired_leaf);
                    current.desired_fingerprint = Some(desired_fingerprint);
                    current.desired_installed = true;
                }
            }

            for fingerprint in removal_fingerprints {
                if tracked_path_set.contains(fingerprint.path.as_str()) {
                    continue;
                }
                let (target_parent, target_leaf) =
                    open_clean_untracked_parent(repository, &fingerprint.path)
                        .map_err(|_| discard_confirmation_changed_error())?;
                let backup_name = CString::new(format!("backup-{}", Uuid::new_v4()))
                    .map_err(|_| discard_filesystem_error())?;
                rename_at_no_replace(
                    target_parent.as_raw_fd(),
                    &target_leaf,
                    transaction.directory_file.as_raw_fd(),
                    &backup_name,
                )
                .map_err(|_| discard_confirmation_changed_error())?;
                transaction.mark_claimed();
                applied.push(AppliedDiscardPath {
                    target_parent,
                    target_leaf,
                    backup_name: Some(backup_name),
                    desired_parent: None,
                    desired_leaf: None,
                    desired_fingerprint: None,
                    desired_installed: false,
                });
                let claimed = applied.last().expect("the claimed path is retained");
                if !clean_untracked_fingerprint_matches_at(
                    transaction.directory_file.as_raw_fd(),
                    claimed
                        .backup_name
                        .as_ref()
                        .expect("the backup name is retained"),
                    fingerprint,
                )? {
                    return Err(discard_confirmation_changed_error());
                }
            }
            Ok(())
        })();
        if let Err(error) = worktree_result {
            if !rollback_applied_discard_paths(&transaction, &applied) {
                return Err(discard_recovery_required_error(&transaction));
            }
            remove_empty_clean_untracked_parent_directories(repository, &created_directories);
            transaction.finish()?;
            return Err(error);
        }

        let state_still_matches = (|| -> Result<bool, AppError> {
            if let Some(head_commit) = captured_head {
                if self.head_commit_id(repository)?.as_str() != head_commit {
                    return Ok(false);
                }
            }
            Ok(self.discard_index_entries(repository, &index_guard_paths)?
                == confirmation.index_entries)
        })();
        let state_still_matches = match state_still_matches {
            Ok(matches) => matches,
            Err(error) => {
                if !rollback_applied_discard_paths(&transaction, &applied) {
                    return Err(discard_recovery_required_error(&transaction));
                }
                remove_empty_clean_untracked_parent_directories(repository, &created_directories);
                transaction.finish()?;
                return Err(error);
            }
        };
        if !state_still_matches {
            if !rollback_applied_discard_paths(&transaction, &applied) {
                return Err(discard_recovery_required_error(&transaction));
            }
            remove_empty_clean_untracked_parent_directories(repository, &created_directories);
            transaction.finish()?;
            return Err(discard_confirmation_changed_error());
        }
        if !tracked_paths.is_empty() {
            let lock = index_lock.take().ok_or_else(discard_filesystem_error)?;
            if let Err(error) = lock.commit() {
                if !rollback_applied_discard_paths(&transaction, &applied) {
                    return Err(discard_recovery_required_error(&transaction));
                }
                remove_empty_clean_untracked_parent_directories(repository, &created_directories);
                transaction.finish()?;
                return Err(error);
            }
        }

        transaction.finish()
    }

    /// Captures the exact state that a later destructive discard is allowed
    /// to consume. Unrelated repository changes are intentionally excluded,
    /// while HEAD, selected status records, index entries, rename sources and
    /// filesystem identities are all bound to the confirmation.
    pub(crate) fn capture_discard_changes_fingerprint(
        &self,
        repository: &Path,
        paths: &[String],
    ) -> Result<DiscardChangesFingerprint, AppError> {
        validate_paths(paths)?;
        for path in paths {
            validate_worktree_relative_path(path)?;
        }
        self.ensure_no_operation(repository)?;

        let mut paths = paths.to_vec();
        paths.sort();
        paths.dedup();
        let status = self.status(repository)?;
        let entries_by_path = status
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect::<HashMap<_, _>>();
        let mut entries = Vec::with_capacity(paths.len());
        let mut observed_paths = HashSet::new();
        let mut tracked_seen = HashSet::new();
        let mut tracked_paths = Vec::new();
        let mut untracked_paths = Vec::new();
        for path in &paths {
            let entry = entries_by_path.get(path.as_str()).copied().ok_or_else(|| {
                AppError::new("nothing_to_discard", "所选文件已经没有可丢弃的改动")
            })?;
            entries.push(entry.clone());
            observed_paths.insert(entry.path.clone());
            if entry.index_status.is_empty() && entry.worktree_status == "?" {
                untracked_paths.push(entry.path.clone());
            } else {
                let related_path = (entry.index_status == "R" || entry.worktree_status == "R")
                    .then_some(entry.original_path.as_deref())
                    .flatten();
                for tracked_path in std::iter::once(entry.path.as_str()).chain(related_path) {
                    validate_worktree_relative_path(tracked_path)?;
                    observed_paths.insert(tracked_path.to_owned());
                    if tracked_seen.insert(tracked_path) {
                        tracked_paths.push(tracked_path.to_owned());
                    }
                }
            }
        }

        tracked_paths.sort();
        let mut index_guard_paths = tracked_paths.clone();
        index_guard_paths.extend(untracked_paths.iter().cloned());
        index_guard_paths.sort();
        index_guard_paths.dedup();
        let index_entries = self.discard_index_entries(repository, &index_guard_paths)?;

        let mut observed_paths = observed_paths.into_iter().collect::<Vec<_>>();
        observed_paths.sort();
        let worktree_paths = observed_paths
            .iter()
            .map(|path| discard_worktree_path_fingerprint(repository, path))
            .collect::<Result<Vec<_>, _>>()?;
        let untracked_path_set = untracked_paths
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let tracked_path_set = tracked_paths
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let removal_fingerprints = worktree_paths
            .iter()
            .filter_map(|fingerprint| match fingerprint {
                DiscardWorktreePathFingerprint::Present(fingerprint)
                    if untracked_path_set.contains(fingerprint.path.as_str())
                        || status.head_commit.is_none()
                            && tracked_path_set.contains(fingerprint.path.as_str()) =>
                {
                    Some(fingerprint.clone())
                }
                _ => None,
            })
            .collect();

        Ok(DiscardChangesFingerprint {
            paths,
            head_commit: status.head_commit,
            head_reference: self.discard_head_reference(repository)?,
            entries,
            index_entries,
            worktree_paths,
            removal_fingerprints,
            untracked_count: untracked_paths.len(),
        })
    }

    pub(crate) fn validate_discard_changes_fingerprint(
        &self,
        repository: &Path,
        fingerprint: &DiscardChangesFingerprint,
    ) -> Result<(), AppError> {
        let current = self
            .capture_discard_changes_fingerprint(repository, &fingerprint.paths)
            .map_err(|error| {
                if error.code() == "nothing_to_discard" {
                    discard_confirmation_changed_error()
                } else {
                    error
                }
            })?;
        if current != *fingerprint {
            return Err(discard_confirmation_changed_error());
        }
        Ok(())
    }

    pub(crate) fn discard_paths_with_fingerprint(
        &self,
        repository: &Path,
        fingerprint: &DiscardChangesFingerprint,
    ) -> Result<(), AppError> {
        self.discard_paths_internal(repository, &fingerprint.paths, Some(fingerprint))
    }

    /// Discards all indexed and working-tree changes for the selected status
    /// entries. Purely untracked entries are removed directly, while tracked
    /// entries go through Git so renames, the index, and file modes remain
    /// consistent. IPC callers must use `discard_paths_with_fingerprint`.
    pub fn discard_paths(&self, repository: &Path, paths: &[String]) -> Result<(), AppError> {
        self.discard_paths_internal(repository, paths, None)
    }

    fn discard_paths_internal(
        &self,
        repository: &Path,
        paths: &[String],
        confirmation: Option<&DiscardChangesFingerprint>,
    ) -> Result<(), AppError> {
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

        // Revalidate after resolving the current status and immediately
        // before the first destructive mutation. This minimizes the window in
        // which an external editor or Git client can invalidate the preview.
        if let Some(fingerprint) = confirmation {
            self.validate_discard_changes_fingerprint(repository, fingerprint)?;
        }
        let paths_to_remove_set = paths_to_remove.iter().cloned().collect::<HashSet<_>>();

        #[cfg(target_os = "linux")]
        if let Some(fingerprint) = confirmation {
            self.ensure_no_operation(repository)?;
            let removal_fingerprints = fingerprint
                .removal_fingerprints
                .iter()
                .filter(|fingerprint| paths_to_remove_set.contains(&fingerprint.path))
                .cloned()
                .collect::<Vec<_>>();
            return self.discard_confirmed_paths_linux(
                repository,
                &tracked_paths,
                &removal_fingerprints,
                fingerprint,
            );
        }

        if !tracked_paths.is_empty() {
            // Another Git client can begin a merge, rebase, or cherry-pick
            // after the earlier status read. Check immediately before each
            // destructive Git mutation so we never knowingly erase conflict
            // stages from a newly paused operation.
            self.ensure_no_operation(repository)?;
            #[cfg(target_os = "linux")]
            {
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
                    let mut arguments = os_arguments(&["rm", "--cached", "--ignore-unmatch", "--"]);
                    arguments.extend(tracked_paths.iter().map(|path| literal_pathspec(path)));
                    self.run(
                        Some(repository),
                        &arguments,
                        &GitEnvironment::default(),
                        "丢弃改动",
                    )?;
                    paths_to_remove.extend(tracked_paths.clone());
                }
            }

            #[cfg(not(target_os = "linux"))]
            if confirmation.is_some() {
                return Err(AppError::new(
                    "discard_protected_unsupported",
                    "当前平台暂不支持安全丢弃已跟踪改动。",
                ));
            } else if self.has_head(repository)? {
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
            if let Some(fingerprint) = confirmation {
                let fingerprints = fingerprint
                    .removal_fingerprints
                    .iter()
                    .filter(|fingerprint| paths_to_remove_set.contains(&fingerprint.path))
                    .cloned()
                    .collect::<Vec<_>>();
                if !fingerprints.is_empty() {
                    self.clean_untracked_paths_with_fingerprints(repository, &fingerprints)?;
                }
            } else {
                // Keep this direct core API for internal callers. The Tauri
                // command always supplies confirmed fingerprints and uses the
                // descriptor-safe deletion path where the platform supports it.
                let mut arguments = os_arguments(&["clean", "-f", "--"]);
                arguments.extend(paths_to_remove.iter().map(|path| literal_pathspec(path)));
                self.run(
                    Some(repository),
                    &arguments,
                    &GitEnvironment::default(),
                    "丢弃未跟踪改动",
                )?;
            }
        }
        Ok(())
    }
}

fn launch_external_editor(
    repository: &Path,
    editor: &ExternalEditorConfig,
    targets: &[PathBuf],
) -> Result<(), AppError> {
    validate_external_editor_command_size(editor, targets)?;

    // Reserve the reaper before starting a process so repeated calls cannot
    // create an unbounded collection of blocked `Child::wait` threads for
    // editors left open indefinitely.
    let reaper_permit = ExternalEditorReaperPermit::acquire()?;
    let mut child = external_editor_command(repository, editor, targets)
        .spawn()
        .map_err(|_| {
        AppError::new(
            "external_editor_launch_failed",
            "无法启动外部编辑器；请检查可执行文件路径和参数。",
        )
    })?;
    // A GUI editor may remain open for a long time. Reap it in a detached
    // worker so the desktop process never accumulates zombie children.
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
    Ok(())
}

type RepositoryEditorReaperJob = (std::process::Child, RepositoryEditorReaperPermit);
static REPOSITORY_EDITOR_REAPER_SENDER: OnceLock<Mutex<Option<mpsc::Sender<RepositoryEditorReaperJob>>>> =
    OnceLock::new();

fn enqueue_repository_editor_reaper_job(
    job: RepositoryEditorReaperJob,
) -> Result<(), RepositoryEditorReaperJob> {
    let sender_slot = REPOSITORY_EDITOR_REAPER_SENDER.get_or_init(|| Mutex::new(None));
    let Ok(mut sender_slot) = sender_slot.lock() else {
        return Err(job);
    };
    if sender_slot.is_none() {
        let (sender, receiver) = mpsc::channel::<RepositoryEditorReaperJob>();
        if thread::Builder::new()
            .name("bkm-repository-editor-reaper".into())
            .spawn(move || {
                let mut children = Vec::<RepositoryEditorReaperJob>::new();
                let mut disconnected = false;
                loop {
                    if disconnected {
                        if children.is_empty() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    } else if children.is_empty() {
                        match receiver.recv() {
                            Ok(child) => children.push(child),
                            Err(_) => disconnected = true,
                        }
                    } else {
                        match receiver.recv_timeout(Duration::from_millis(100)) {
                            Ok(child) => children.push(child),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
                        }
                    }
                    if !disconnected {
                        while let Ok(child) = receiver.try_recv() {
                            children.push(child);
                        }
                    }
                    children.retain_mut(|(child, _permit)| match child.try_wait() {
                        Ok(None) => true,
                        Ok(Some(_)) => false,
                        Err(_) => true,
                    });
                }
            })
            .is_err()
        {
            return Err(job);
        }
        *sender_slot = Some(sender);
    }
    let result = sender_slot.as_ref().expect("sender initialized").send(job);
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            *sender_slot = None;
            Err(error.0)
        }
    }
}

fn external_editor_command(
    repository: &Path,
    editor: &ExternalEditorConfig,
    targets: &[PathBuf],
) -> Command {
    let mut command = Command::new(&editor.executable);
    command
        .args(&editor.arguments)
        .args(targets)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn validate_external_editor_command_size(
    editor: &ExternalEditorConfig,
    targets: &[PathBuf],
) -> Result<(), AppError> {
    let command_units = external_editor_command_units(
        std::iter::once(editor.executable.as_str())
            .chain(editor.arguments.iter().map(String::as_str)),
    );
    let target_units = targets.iter().map(|target| {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            2 + target.as_os_str().as_bytes().len().saturating_mul(2)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            2 + target.as_os_str().encode_wide().count().saturating_mul(2)
        }
        #[cfg(not(any(unix, windows)))]
        {
            2 + target.to_string_lossy().encode_utf16().count().saturating_mul(2)
        }
    });
    if command_units.saturating_add(target_units.sum::<usize>())
        > MAX_EXTERNAL_EDITOR_LAUNCH_COMMAND_UNITS
    {
        return Err(AppError::new(
            "external_editor_arguments_invalid",
            "要在外部编辑器中打开的路径过长。",
        ));
    }
    Ok(())
}

fn launch_repository_editor(
    repository: &Path,
    editor: &ExternalEditorConfig,
    targets: &[PathBuf],
) -> Result<(), AppError> {
    validate_external_editor_command_size(editor, targets)?;
    let reaper_permit = RepositoryEditorReaperPermit::acquire()?;
    let mut child = external_editor_command(repository, editor, targets)
        .spawn()
        .map_err(|_| {
            AppError::new(
                "external_editor_launch_failed",
                "无法启动外部编辑器；请检查可执行文件路径和参数。",
            )
        })?;
    if let Err((returned_child, _permit)) =
        enqueue_repository_editor_reaper_job((child, reaper_permit))
    {
        child = returned_child;
        if child.kill().is_ok() {
            let _ = child.wait();
        }
        return Err(AppError::new(
            "external_editor_reaper_failed",
            "无法监控外部编辑器进程，请稍后重试。",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod repository_editor_command_tests {
    use super::*;

    #[test]
    fn repository_target_is_one_literal_argument_and_sets_exact_working_directory() {
        let repository = PathBuf::from("/tmp/repository -with spaces");
        let target = repository.join("--option-like target");
        let editor = ExternalEditorConfig {
            executable: "/usr/bin/editor".into(),
            arguments: vec!["--reuse-window".into()],
        };
        let command = external_editor_command(&repository, &editor, std::slice::from_ref(&target));

        assert_eq!(command.get_current_dir(), Some(repository.as_path()));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("--reuse-window"), target.as_os_str()]
        );
    }
}
