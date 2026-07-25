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
        self.commits_with_consistency_retry(repository, limit, 2)
    }

    fn commits_with_consistency_retry(
        &self,
        repository: &Path,
        limit: usize,
        retries_remaining: usize,
    ) -> Result<Vec<CommitSummary>, AppError> {
        if limit == 0 || limit > 1_000 {
            return Err(AppError::new(
                "invalid_history_limit",
                "提交历史读取数量必须在 1 到 1000 之间",
            ));
        }
        let format = "%H%x00%h%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00";
        let mut arguments = vec![
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
        // Capture the symbolic branch before resolving its tip. Using that
        // immutable branch ref for both HEAD and upstream lookup keeps the
        // pair coherent even if another Git client checks out a branch while
        // this history request is running. Detached HEAD has no upstream.
        let symbolic_head = self.execute(
            Some(repository),
            &os_arguments(&["symbolic-ref", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        let branch_reference = if symbolic_head.status.success() {
            let reference = output_text(&symbolic_head.stdout).trim().to_owned();
            if reference.is_empty() {
                return Err(AppError::new("git_parse_error", "无法读取当前分支。"));
            }
            Some(reference)
        } else if symbolic_head.status.code() == Some(1) {
            None
        } else {
            return Err(git_command_error("读取当前分支", &symbolic_head));
        };
        let history_tip = branch_reference.as_deref().unwrap_or("HEAD");
        let head = self.execute(
            Some(repository),
            &[
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from(history_tip),
            ],
            &GitEnvironment::default(),
        )?;
        if head.status.code() == Some(1) {
            let current_symbolic_head = self.execute(
                Some(repository),
                &os_arguments(&["symbolic-ref", "--quiet", "HEAD"]),
                &GitEnvironment::default(),
            )?;
            let still_on_captured_branch = current_symbolic_head.status.success()
                && output_text(&current_symbolic_head.stdout).trim() == history_tip;
            if still_on_captured_branch && self.has_unborn_head(repository)? {
                return Ok(Vec::new());
            }
            if branch_reference.is_some() && retries_remaining > 0 {
                return self.commits_with_consistency_retry(
                    repository,
                    limit,
                    retries_remaining - 1,
                );
            }
            if branch_reference.is_some() {
                return Err(AppError::new(
                    "repository_changed",
                    "仓库引用正在变化，请稍后重试。",
                ));
            }
        }
        if !head.status.success() {
            return Err(git_command_error("读取提交历史", &head));
        }
        let head_commit = output_text(&head.stdout).trim().to_owned();
        if head_commit.is_empty() {
            return Err(AppError::new("git_parse_error", "无法读取当前提交。"));
        }
        let upstream_commit = match self.upstream_commit_id(repository, branch_reference.as_deref())
        {
            Ok(commit) => commit,
            Err(error) if error.code() == "repository_changed" && retries_remaining > 0 => {
                return self.commits_with_consistency_retry(
                    repository,
                    limit,
                    retries_remaining - 1,
                );
            }
            Err(error) => return Err(error),
        };
        arguments.push(OsString::from(&head_commit));
        let output = self.run_with_output_limit(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            MAX_COMMIT_HISTORY_OUTPUT_BYTES,
            "读取提交历史",
        )?;
        let mut commits = parse_commits(&output.stdout)?;
        // Decorations are intentionally not embedded in `git log`: a single
        // commit can have enough refs to exhaust the history output budget.
        // Enumerate tags once with its own bound, then join by immutable
        // object ID. If an unusually large tag namespace exceeds that bound,
        // keep history usable while explicitly marking tag data incomplete.
        let tags_snapshot = match self.tags(repository) {
            Ok(tags) => {
                let mut tags_by_target = std::collections::HashMap::<String, Vec<String>>::new();
                for tag in &tags {
                    tags_by_target
                        .entry(tag.target.clone())
                        .or_default()
                        .push(tag.name.clone());
                }
                for commit in &mut commits {
                    commit.tags = tags_by_target.remove(&commit.sha).unwrap_or_default();
                }
                Some(tags)
            }
            Err(error) if error.code() == "git_output_too_large" => {
                for commit in &mut commits {
                    commit.tags_truncated = true;
                }
                None
            }
            Err(error) => return Err(error),
        };
        let local_only_commits = self.local_only_commit_ids(
            repository,
            limit,
            &head_commit,
            upstream_commit.as_deref(),
        )?;
        for commit in &mut commits {
            commit.pushed = local_only_commits
                .as_ref()
                .map(|local_only| !local_only.contains(&commit.sha));
        }
        let verified_head = self.execute(
            Some(repository),
            &[
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from(history_tip),
            ],
            &GitEnvironment::default(),
        )?;
        let verified_head_commit = if verified_head.status.success() {
            output_text(&verified_head.stdout).trim().to_owned()
        } else if verified_head.status.code() == Some(1) {
            String::new()
        } else {
            return Err(git_command_error("读取提交历史", &verified_head));
        };
        let verified_symbolic_head = self.execute(
            Some(repository),
            &os_arguments(&["symbolic-ref", "--quiet", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        let verified_branch_reference = if verified_symbolic_head.status.success() {
            Some(
                output_text(&verified_symbolic_head.stdout)
                    .trim()
                    .to_owned(),
            )
        } else if verified_symbolic_head.status.code() == Some(1) {
            None
        } else {
            return Err(git_command_error("读取当前分支", &verified_symbolic_head));
        };
        let verified_upstream =
            match self.upstream_commit_id(repository, branch_reference.as_deref()) {
                Ok(commit) => commit,
                Err(error) if error.code() == "repository_changed" && retries_remaining > 0 => {
                    return self.commits_with_consistency_retry(
                        repository,
                        limit,
                        retries_remaining - 1,
                    );
                }
                Err(error) => return Err(error),
            };
        let verified_tags = match self.tags(repository) {
            Ok(tags) => Some(tags),
            Err(error) if error.code() == "git_output_too_large" => None,
            Err(error) => return Err(error),
        };
        if verified_branch_reference == branch_reference
            && verified_head_commit == head_commit
            && verified_upstream == upstream_commit
            && verified_tags == tags_snapshot
        {
            return Ok(commits);
        }
        if retries_remaining > 0 {
            return self.commits_with_consistency_retry(repository, limit, retries_remaining - 1);
        }
        Err(AppError::new(
            "repository_changed",
            "仓库引用正在变化，请稍后重试。",
        ))
    }

    /// Returns commits reachable from the current branch but not its upstream.
    /// A missing upstream is represented separately so the UI does not claim
    /// that every commit is unpushed when Git has no comparison target.
    fn upstream_commit_id(
        &self,
        repository: &Path,
        branch_reference: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        let Some(branch_reference) = branch_reference else {
            return Ok(None);
        };
        let configured_upstream = self.execute(
            Some(repository),
            &[
                OsString::from("for-each-ref"),
                OsString::from("--format=%(refname)%00%(upstream)"),
                OsString::from(branch_reference),
            ],
            &GitEnvironment::default(),
        )?;
        if !configured_upstream.status.success() {
            return Err(git_command_error("读取上游配置", &configured_upstream));
        }
        let mut upstream_reference = None;
        for record in configured_upstream
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
        {
            let Some(separator) = record.iter().position(|byte| *byte == b'\0') else {
                return Err(AppError::new("git_parse_error", "无法读取上游配置。"));
            };
            let refname = std::str::from_utf8(&record[..separator])
                .map_err(|_| AppError::new("git_parse_error", "无法读取上游配置。"))?;
            if refname != branch_reference {
                continue;
            }
            if upstream_reference.is_some() {
                return Err(AppError::new("git_parse_error", "无法读取上游配置。"));
            }
            upstream_reference = Some(
                std::str::from_utf8(&record[separator + 1..])
                    .map_err(|_| AppError::new("git_parse_error", "无法读取上游配置。"))?
                    .to_owned(),
            );
        }
        let upstream_reference = upstream_reference
            .ok_or_else(|| AppError::new("repository_changed", "仓库引用正在变化，请稍后重试。"))?;
        if upstream_reference.is_empty() {
            return Ok(None);
        }
        let upstream_expression = format!("{upstream_reference}^{{commit}}");
        let upstream = self.execute(
            Some(repository),
            &[
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from(upstream_expression),
            ],
            &GitEnvironment::default(),
        )?;
        if upstream.status.code() == Some(1) {
            return Ok(None);
        }
        if !upstream.status.success() {
            return Err(git_command_error("读取上游分支", &upstream));
        }
        let commit = output_text(&upstream.stdout).trim().to_owned();
        if commit.is_empty() {
            return Err(AppError::new("git_parse_error", "无法读取上游提交。"));
        }
        Ok(Some(commit))
    }

    fn local_only_commit_ids(
        &self,
        repository: &Path,
        limit: usize,
        head_commit: &str,
        upstream_commit: Option<&str>,
    ) -> Result<Option<HashSet<String>>, AppError> {
        let Some(upstream_commit) = upstream_commit else {
            return Ok(None);
        };

        let output = self.run_with_output_limit(
            Some(repository),
            &[
                OsString::from("rev-list"),
                OsString::from(format!("--max-count={limit}")),
                OsString::from(head_commit),
                OsString::from("--not"),
                OsString::from(upstream_commit),
            ],
            &GitEnvironment::default(),
            MAX_COMMIT_HISTORY_OUTPUT_BYTES,
            "读取未推送提交",
        )?;
        Ok(Some(
            output_text(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        ))
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
