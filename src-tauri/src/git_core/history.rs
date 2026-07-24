use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

impl GitCore {
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
        if head.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        if !head.status.success() {
            return Err(git_command_error("读取提交历史", &head));
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

    /// Lists the locally stored remote-tracking branches for configured
    /// remotes. Symbolic refs such as `origin/HEAD` and refs left behind for a
    /// removed remote are intentionally omitted: neither can be checked out
    /// as a user-selectable remote branch.
    pub fn remote_branches(&self, repository: &Path) -> Result<Vec<RemoteBranchInfo>, AppError> {
        let remote_names = self
            .remotes(repository)?
            .into_iter()
            .map(|remote| remote.name)
            .collect::<Vec<_>>();
        self.remote_branches_for_names(repository, &remote_names)
    }

    pub(super) fn remote_branches_for_names(
        &self,
        repository: &Path,
        remote_names: &[String],
    ) -> Result<Vec<RemoteBranchInfo>, AppError> {
        let arguments = os_arguments(&[
            "for-each-ref",
            "--format=%(refname)%00%(symref)",
            "refs/remotes",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取远程分支",
        )?;
        parse_remote_branches(&output.stdout, remote_names)
    }
}
