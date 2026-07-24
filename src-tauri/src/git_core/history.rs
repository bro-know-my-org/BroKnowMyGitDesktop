use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

const MAX_COMMIT_HISTORY_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_COMMIT_DIFF_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

impl GitCore {
    /// Creates a commit while preserving the user's normal Git hooks and
    /// credential/configuration behavior.
    pub fn commit(
        &self,
        repository: &Path,
        message: &str,
        amend: bool,
    ) -> Result<CommitResult, AppError> {
        validate_commit_message(message)?;
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
            // Repository/user configuration can otherwise append GPG
            // signature presentation text to this machine-readable stream.
            OsString::from("--no-show-signature"),
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
        if head.status.code() == Some(1) && self.has_unborn_head(repository)? {
            return Ok(Vec::new());
        }
        if !head.status.success() {
            return Err(git_command_error("读取提交历史", &head));
        }
        let output = self.run_with_output_limit(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            MAX_COMMIT_HISTORY_OUTPUT_BYTES,
            "读取提交历史",
        )?;
        parse_commits(&output.stdout)
    }

    /// Reads a complete patch for one existing commit without exposing Git's
    /// raw diagnostic output on failure.
    pub fn commit_diff(&self, repository: &Path, revision: &str) -> Result<String, AppError> {
        validate_revision(revision, "提交引用")?;
        let commit_expression = format!("{revision}^{{commit}}");
        let resolved = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", &commit_expression]),
            &GitEnvironment::default(),
            "验证提交引用",
        )?;
        let commit = output_text(&resolved.stdout).trim().to_owned();
        if !(40..=64).contains(&commit.len())
            || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取提交标识。"));
        }
        let arguments = vec![
            OsString::from("show"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--format="),
            OsString::from("--binary"),
            OsString::from("--find-renames"),
            OsString::from("--unified=3"),
            OsString::from(commit),
            OsString::from("--"),
        ];
        let output = self.run_with_output_limit(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            MAX_COMMIT_DIFF_OUTPUT_BYTES,
            "读取提交改动",
        )?;
        commit_diff_output_text(output.stdout)
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

/// Commit diffs are display-only on this IPC surface. Git can emit raw bytes
/// from a text blob even when every path is valid UTF-8, so decode lossy text
/// rather than misreporting such blob content as an unsupported filename.
pub(super) fn commit_diff_output_text(bytes: Vec<u8>) -> Result<String, AppError> {
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
