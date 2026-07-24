use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;
use std::io::Write;
use uuid::Uuid;

/// Owns the message file passed to Git's normal commit hooks during a
/// compare-and-swap squash. `commit-tree` does not run hooks itself, so this
/// private file lets prepare/commit-msg hooks validate or amend the same
/// message without sharing `.git/COMMIT_EDITMSG` with another Git process.
struct SquashCommitMessageFile {
    path: PathBuf,
}

impl SquashCommitMessageFile {
    fn create(git: &GitCore, repository: &Path, message: &str) -> Result<Self, AppError> {
        let git_directory = git.git_directory(repository)?;
        for _ in 0..16 {
            let path = git_directory.join(format!("bkm-squash-message-{}", Uuid::new_v4()));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(message.as_bytes()) {
                        let _ = fs::remove_file(&path);
                        return Err(AppError::io("写入压缩提交信息", error));
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AppError::io("创建压缩提交信息", error)),
            }
        }
        Err(AppError::new(
            "git_operation_failed",
            "无法创建压缩提交信息文件。",
        ))
    }

    fn read(&self) -> Result<String, AppError> {
        fs::read_to_string(&self.path).map_err(|error| AppError::io("读取压缩提交信息", error))
    }
}

impl Drop for SquashCommitMessageFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl GitCore {
    /// Lists stashes in Git's user-visible order, newest first.
    pub fn stashes(&self, repository: &Path) -> Result<Vec<StashEntry>, AppError> {
        let arguments = os_arguments(&["stash", "list", "--format=format:%gd%x00%H%x00%gs%x00"]);
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
            validate_stash_message(message)?;
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

    /// Applies one immutable stash commit. Removing an entry by reflog index
    /// is only safe when it is the sole stash: another Git client can shift a
    /// `stash@{n}` selector between separate apply/drop processes. In a
    /// multi-entry stack this deliberately preserves the applied entry rather
    /// than deleting a different user's stash.
    pub fn stash_pop(
        &self,
        repository: &Path,
        reference: &str,
        expected_commit_id: &str,
    ) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
        self.ensure_stash_reference_matches(repository, reference, expected_commit_id)?;
        let arguments = os_arguments(&["stash", "apply", expected_commit_id]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "恢复暂存栈",
        )?;
        if self.stash_stack_is_single_matching(repository, expected_commit_id)? {
            self.drop_only_stash_if_still_matching(repository, expected_commit_id)?;
        }
        Ok(())
    }

    /// Permanently removes one stash entry after the UI has confirmed it.
    pub fn stash_drop(
        &self,
        repository: &Path,
        reference: &str,
        expected_commit_id: &str,
    ) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
        self.ensure_stash_reference_matches(repository, reference, expected_commit_id)?;
        if !self.stash_stack_is_single_matching(repository, expected_commit_id)? {
            return Err(AppError::new(
                "stash_drop_requires_manual_git",
                "为避免并发 Git 操作删除其他暂存项，多条暂存记录请在系统 Git 中手动删除。",
            ));
        }
        self.drop_only_stash_if_still_matching(repository, expected_commit_id)?;
        Ok(())
    }

    fn stash_stack_is_single_matching(
        &self,
        repository: &Path,
        expected_commit_id: &str,
    ) -> Result<bool, AppError> {
        let stashes = self.stashes(repository)?;
        Ok(stashes.len() == 1
            && stashes[0]
                .commit_id
                .eq_ignore_ascii_case(expected_commit_id))
    }

    /// `update-ref -d <ref> <old>` is a single compare-and-swap operation.
    /// It can safely clear the sole `refs/stash` entry only when that exact
    /// commit is still current; a concurrent push/drop makes it fail without
    /// touching the other client's stash.
    fn drop_only_stash_if_still_matching(
        &self,
        repository: &Path,
        expected_commit_id: &str,
    ) -> Result<(), AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["update-ref", "-d", "refs/stash", expected_commit_id]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(AppError::new(
                "stash_changed",
                "暂存栈已被其他 Git 操作修改，请刷新列表后重试。",
            ))
        }
    }

    /// A reflog selector such as `stash@{0}` is only a position. Another Git
    /// client can push or drop a stash after the UI has displayed it, so never
    /// perform a destructive action until that selector still resolves to the
    /// immutable commit ID returned with the list entry.
    fn ensure_stash_reference_matches(
        &self,
        repository: &Path,
        reference: &str,
        expected_commit_id: &str,
    ) -> Result<(), AppError> {
        validate_stash_reference(reference)?;
        if !(40..=64).contains(&expected_commit_id.len())
            || !expected_commit_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("invalid_stash_reference", "暂存栈引用无效。"));
        }
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{reference}^{{commit}}"),
            ]),
            &GitEnvironment::default(),
        )?;
        let current = output_text(&output.stdout);
        let current = current.trim();
        if !output.status.success() || !current.eq_ignore_ascii_case(expected_commit_id) {
            return Err(AppError::new(
                "stash_changed",
                "暂存栈已被其他 Git 操作修改，请刷新列表后重试。",
            ));
        }
        Ok(())
    }

    /// Resets the current branch to a selected revision. The reset mode is
    /// explicit so the frontend can present a proportionate confirmation.
    pub fn reset(&self, repository: &Path, target: &str, mode: ResetMode) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
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
        validate_commit_message(message)?;
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
        // Resolve the base from the immutable original commit rather than a
        // symbolic `HEAD~N`. Another Git client may move HEAD while this
        // command is preparing the squash; the selected history must never
        // silently change to that newer tip.
        let original_head = self.head_commit_id(repository)?;
        let expected_index_tree = self.index_tree_id(repository)?;
        let original_head_tree = self.commit_tree_id(repository, &original_head)?;
        if expected_index_tree != original_head_tree {
            return Err(AppError::new(
                "git_operation_concurrent_change",
                "压缩提交前暂存区已被其他 Git 操作修改，请刷新后重试。",
            ));
        }
        let base_expression = format!("{original_head}~{count}");
        let base = self.execute(
            Some(repository),
            &os_arguments(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{base_expression}^{{commit}}"),
            ]),
            &GitEnvironment::default(),
        )?;
        if !base.status.success() {
            return Err(AppError::new(
                "invalid_squash_count",
                "压缩提交后必须保留一个更早的基线提交",
            ));
        }
        let base = output_text(&base.stdout).trim().to_owned();
        if !(40..=64).contains(&base.len()) || !base.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::new(
                "invalid_squash_count",
                "压缩提交后必须保留一个更早的基线提交",
            ));
        }
        // Allocate the hook message file before moving HEAD. A filesystem
        // failure here must never leave a partial soft-reset behind.
        let message_file = SquashCommitMessageFile::create(self, repository, message)?;
        // `update-ref` compares the current HEAD/ref target atomically. This
        // is the soft-reset equivalent: it changes only the current history
        // pointer and intentionally leaves the original tree staged for the
        // replacement commit.
        self.ensure_no_operation(repository)?;
        let rewrite = self.execute(
            Some(repository),
            &os_arguments(&["update-ref", "HEAD", &base, &original_head]),
            &GitEnvironment::default(),
        )?;
        if !rewrite.status.success() {
            if self.head_commit_id(repository).ok().as_deref() != Some(original_head.as_str()) {
                return Err(AppError::new(
                    "git_operation_concurrent_change",
                    "压缩提交前仓库状态已变化，请刷新后重试。",
                ));
            }
            return Err(git_command_error("准备压缩提交", &rewrite));
        }
        // The successful CAS above establishes `base` as the only expected
        // interim target. Keep that immutable ID rather than issuing another
        // fallible read that could leave recovery ambiguous.
        let rewrite_head = base.clone();
        // `update-ref` deliberately leaves the original tree staged. If an
        // external Git client changed the shared index while the history was
        // being prepared, restoring the old ref is safe but committing would
        // otherwise absorb those unrelated staged changes into the squash.
        let current_index_tree = match self.index_tree_id(repository) {
            Ok(tree) => tree,
            Err(error) => {
                if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(error);
            }
        };
        if current_index_tree != expected_index_tree {
            if self.head_commit_id(repository).ok().as_deref() == Some(rewrite_head.as_str()) {
                let _ = self.execute(
                    Some(repository),
                    &os_arguments(&["update-ref", "HEAD", &original_head, &rewrite_head]),
                    &GitEnvironment::default(),
                );
            }
            return Err(AppError::new(
                "git_operation_concurrent_change",
                "压缩提交期间暂存区已被其他 Git 操作修改，请刷新后重试。",
            ));
        }
        if let Err(error) = self.run_squash_commit_hooks(repository, &message_file.path) {
            if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                return Err(AppError::new(
                    "squash_recovery_failed",
                    "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                ));
            }
            return Err(error);
        }
        // Hooks may deliberately stage formatting output. Capture the tree
        // they produced immediately, so later changes to the shared index
        // cannot be folded into this squash commit.
        let hook_index_tree = match self.index_tree_id(repository) {
            Ok(tree) => tree,
            Err(error) => {
                if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(error);
            }
        };
        let message = match message_file.read().and_then(|message| {
            validate_commit_message(&message)?;
            Ok(message)
        }) {
            Ok(message) => message,
            Err(error) => {
                if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(error);
            }
        };
        if let Err(error) = self.ensure_no_operation(repository) {
            if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                return Err(AppError::new(
                    "squash_recovery_failed",
                    "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                ));
            }
            return Err(error);
        }
        // `git commit` updates HEAD without a caller-supplied old value. Build
        // the replacement object from the already verified index tree, then
        // publish it through update-ref's compare-and-swap so a concurrent
        // branch movement cannot become this squash commit's parent.
        let replacement = match self.run_with_input(
            Some(repository),
            &os_arguments(&["commit-tree", &hook_index_tree, "-p", &rewrite_head]),
            message.as_bytes(),
            &GitEnvironment::default(),
            "创建压缩提交",
        ) {
            Ok(output) => {
                let commit = output_text(&output.stdout).trim().to_owned();
                if !(40..=64).contains(&commit.len())
                    || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(AppError::new("git_parse_error", "无法读取压缩提交标识。"));
                }
                commit
            }
            Err(error) => {
                if !self.restore_squash_head(repository, &original_head, &rewrite_head) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(error);
            }
        };
        let publish = self.execute(
            Some(repository),
            &os_arguments(&["update-ref", "HEAD", &replacement, &rewrite_head]),
            &GitEnvironment::default(),
        );
        match publish {
            Ok(output) if output.status.success() => {
                // Git's post-commit hook observes a completed commit and may
                // not veto it. Match that behavior: a hook failure never
                // rewinds a successfully published CAS update.
                let _ = self.run(
                    Some(repository),
                    &os_arguments(&["hook", "run", "--ignore-missing", "post-commit"]),
                    &GitEnvironment::default(),
                    "执行压缩提交后钩子",
                );
            }
            Ok(output) => {
                if self.head_commit_id(repository).ok().as_deref() != Some(rewrite_head.as_str()) {
                    return Err(AppError::new(
                        "git_operation_concurrent_change",
                        "压缩提交期间仓库历史已被其他 Git 操作修改，请刷新后重试。",
                    ));
                }
                let restored = self.execute(
                    Some(repository),
                    &os_arguments(&["update-ref", "HEAD", &original_head, &rewrite_head]),
                    &GitEnvironment::default(),
                );
                if !matches!(restored, Ok(output) if output.status.success()) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(git_command_error("完成压缩提交", &output));
            }
            Err(error) => {
                if self.head_commit_id(repository).ok().as_deref() != Some(rewrite_head.as_str()) {
                    return Err(AppError::new(
                        "git_operation_concurrent_change",
                        "压缩提交期间仓库历史已被其他 Git 操作修改，请刷新后重试。",
                    ));
                }
                let restored = self.execute(
                    Some(repository),
                    &os_arguments(&["update-ref", "HEAD", &original_head, &rewrite_head]),
                    &GitEnvironment::default(),
                );
                if !matches!(restored, Ok(output) if output.status.success()) {
                    return Err(AppError::new(
                        "squash_recovery_failed",
                        "压缩提交失败且无法自动恢复原分支状态；请检查 Git 历史和暂存区。",
                    ));
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn run_squash_commit_hooks(
        &self,
        repository: &Path,
        message_path: &Path,
    ) -> Result<(), AppError> {
        self.run(
            Some(repository),
            &os_arguments(&["hook", "run", "--ignore-missing", "pre-commit"]),
            &GitEnvironment::default(),
            "执行压缩提交前钩子",
        )?;
        let mut prepare_message_arguments = os_arguments(&[
            "hook",
            "run",
            "--ignore-missing",
            "prepare-commit-msg",
            "--",
        ]);
        prepare_message_arguments.push(message_path.as_os_str().to_os_string());
        prepare_message_arguments.push(OsString::from("message"));
        self.run(
            Some(repository),
            &prepare_message_arguments,
            &GitEnvironment::default(),
            "执行压缩提交信息钩子",
        )?;
        let mut commit_message_arguments =
            os_arguments(&["hook", "run", "--ignore-missing", "commit-msg", "--"]);
        commit_message_arguments.push(message_path.as_os_str().to_os_string());
        self.run(
            Some(repository),
            &commit_message_arguments,
            &GitEnvironment::default(),
            "执行压缩提交校验钩子",
        )
        .map(|_| ())
    }

    fn restore_squash_head(
        &self,
        repository: &Path,
        original_head: &str,
        rewrite_head: &str,
    ) -> bool {
        matches!(
            self.execute(
                Some(repository),
                &os_arguments(&["update-ref", "HEAD", original_head, rewrite_head]),
                &GitEnvironment::default(),
            ),
            Ok(output) if output.status.success()
        )
    }

    /// Returns the immutable tree recorded by a commit. Comparing it with the
    /// shared index prevents a concurrent branch move from combining one
    /// commit's history with another commit's staged contents.
    fn commit_tree_id(&self, repository: &Path, commit: &str) -> Result<String, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{commit}^{{tree}}"),
            ]),
            &GitEnvironment::default(),
        )?;
        if !output.status.success() {
            return Err(git_command_error("读取提交树", &output));
        }
        let tree = output_text(&output.stdout).trim().to_owned();
        if !(40..=64).contains(&tree.len()) || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::new("git_parse_error", "无法读取提交树对象标识。"));
        }
        Ok(tree)
    }

    /// Returns the current index tree object. Unlike porcelain status, this
    /// captures staged content exactly and is therefore suitable for guarding
    /// a history rewrite that intentionally reuses the shared index.
    fn index_tree_id(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["write-tree"]),
            &GitEnvironment::default(),
        )?;
        if !output.status.success() {
            return Err(git_command_error("读取暂存区树", &output));
        }
        let tree = output_text(&output.stdout).trim().to_owned();
        if !(40..=64).contains(&tree.len()) || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::new(
                "git_parse_error",
                "无法读取暂存区树对象标识。",
            ));
        }
        Ok(tree)
    }

    /// Creates a new revert commit for a selected commit. Git's normal hooks
    /// and conflict behavior are preserved.
    pub fn revert_commit(&self, repository: &Path, revision: &str) -> Result<(), AppError> {
        self.ensure_no_operation(repository)?;
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
}
