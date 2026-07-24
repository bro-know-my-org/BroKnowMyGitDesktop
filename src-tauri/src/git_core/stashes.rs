use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

impl GitCore {
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
}
