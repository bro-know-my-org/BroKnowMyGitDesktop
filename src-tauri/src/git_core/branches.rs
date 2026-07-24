use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;

impl GitCore {
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

    /// Switches to a remote-tracking branch. If a local branch with the same
    /// name already exists, Git switches to that branch; otherwise this
    /// creates a conventional local tracking branch from the selected remote
    /// ref. The remote ref is verified first so a stale UI row cannot create
    /// an unrelated branch from an ambiguous revision.
    pub fn switch_remote_branch(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
    ) -> Result<(), AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(branch, "远程分支名")?;
        if !self
            .remotes(repository)?
            .iter()
            .any(|remote| remote.name == remote_name)
        {
            return Err(AppError::new("remote_not_found", "找不到该 remote。"));
        }

        let remote_reference = remote_tracking_reference(remote_name, branch);
        let reference_exists = self.execute(
            Some(repository),
            &os_arguments(&["show-ref", "--verify", "--quiet", &remote_reference]),
            &GitEnvironment::default(),
        )?;
        if !reference_exists.status.success() {
            if reference_exists.status.code() == Some(1) {
                return Err(AppError::new(
                    "remote_branch_not_found",
                    "找不到该远程分支；请先获取远程更新后重试。",
                ));
            }
            return Err(git_command_error("检查远程分支", &reference_exists));
        }

        if self
            .branches(repository)?
            .iter()
            .any(|local_branch| local_branch.name == branch)
        {
            return self.switch_branch(repository, branch);
        }

        // `git switch --track` accepts a short remote-tracking name such as
        // `origin/release`, rather than its fully-qualified
        // `refs/remotes/origin/release` form. A single-branch clone also
        // needs the matching fetch refspec before Git will allow the upstream
        // to be configured.
        self.ensure_remote_fetches_all_branches(repository, remote_name)?;
        let remote_tracking_branch = remote_tracking_branch_name(remote_name, branch);
        let arguments = os_arguments(&[
            "switch",
            "--track",
            "-c",
            branch,
            "--",
            &remote_tracking_branch,
        ]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "切换远程分支",
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
}
