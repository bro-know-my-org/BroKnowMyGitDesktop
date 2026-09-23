use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;

impl GitCore {
    /// Returns the checked-out local branch name. Publish recovery requires a
    /// symbolic branch so it can prove the exact commit reached the remote.
    pub(crate) fn current_branch_name(&self, repository: &Path) -> Result<String, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
            &GitEnvironment::default(),
        )?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Err(AppError::new("detached_head", "当前仓库未检出本地分支。"));
            }
            return Err(git_command_error("读取当前分支", &output));
        }
        let branch = output_text(&output.stdout).trim().to_owned();
        validate_ref_name(&branch, "当前分支名")?;
        Ok(branch)
    }

    /// Captures a branch name and HEAD commit that were observed together.
    /// External Git processes are not covered by the application lease, so
    /// retry a changing checkout instead of persisting an impossible pair.
    pub(crate) fn current_checkout_snapshot(
        &self,
        repository: &Path,
    ) -> Result<(String, String), AppError> {
        let mut environment = GitEnvironment::new();
        environment.isolate_network_credentials_and_disable_hooks();
        let read_snapshot = || -> Result<(String, String), AppError> {
            let output = self.run(
                Some(repository),
                &os_arguments(&[
                    "status",
                    "--porcelain=v2",
                    "--branch",
                    "--untracked-files=no",
                    "--",
                    ":(exclude)**",
                ]),
                &environment,
                "读取发布分支快照",
            )?;
            let output = output_text(&output.stdout);
            let branch = output
                .lines()
                .find_map(|line| line.strip_prefix("# branch.head "))
                .ok_or_else(|| AppError::new("git_parse_error", "无法读取发布分支快照。"))?;
            if branch == "(detached)" {
                return Err(AppError::new("detached_head", "当前仓库未检出本地分支。"));
            }
            validate_ref_name(branch, "当前分支名")?;
            let commit = output
                .lines()
                .find_map(|line| line.strip_prefix("# branch.oid "))
                .ok_or_else(|| AppError::new("git_parse_error", "无法读取发布分支快照。"))?;
            if !matches!(commit.len(), 40 | 64)
                || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(AppError::new(
                    "git_parse_error",
                    "无法读取发布分支提交标识。",
                ));
            }
            Ok((branch.to_owned(), commit.to_owned()))
        };
        let mut previous = read_snapshot()?;
        for _ in 0..3 {
            let current = read_snapshot()?;
            if current == previous {
                return Ok(current);
            }
            previous = current;
        }
        Err(AppError::new(
            "repository_changed",
            "当前分支或提交正在变化，请稍后重试。",
        ))
    }

    pub(crate) fn unique_local_branch_at_commit(
        &self,
        repository: &Path,
        commit: &str,
    ) -> Result<Option<String>, AppError> {
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取已发布提交标识。"));
        }
        let points_at = format!("--points-at={commit}");
        let output = self.run(
            Some(repository),
            &[
                OsString::from("for-each-ref"),
                OsString::from(points_at),
                OsString::from("--count=2"),
                OsString::from("--format=%(refname)"),
                OsString::from("refs/heads/"),
            ],
            &GitEnvironment::default(),
            "查找已发布本地分支",
        )?;
        let output = output_text(&output.stdout);
        let mut matching = output
            .lines()
            .filter(|line| !line.is_empty())
            .map(|reference| {
                let branch = reference
                    .strip_prefix("refs/heads/")
                    .ok_or_else(|| AppError::new("git_parse_error", "无法读取已发布本地分支。"))?;
                validate_ref_name(branch, "已发布本地分支名")?;
                Ok(branch.to_owned())
            });
        let first = matching.next().transpose()?;
        if matching.next().transpose()?.is_some() {
            return Ok(None);
        }
        Ok(first)
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
            validate_revision(start_point, "起点引用")?;
        }
        self.ensure_no_operation(repository)?;
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
        self.ensure_no_operation(repository)?;
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
        self.ensure_no_operation(repository)?;
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

        let remote_tracking_branch = remote_tracking_branch_name(remote_name, branch);
        if let Some(local_branch) = self
            .branches(repository)?
            .into_iter()
            .find(|local_branch| local_branch.name == branch)
        {
            if local_branch.upstream.as_deref() == Some(remote_tracking_branch.as_str())
                || self.branch_tracks_remote(repository, branch, remote_name, branch)?
            {
                if local_branch.is_current {
                    return Ok(());
                }
                return self.switch_branch(repository, branch);
            }
            return Err(AppError::new(
                "local_branch_remote_conflict",
                "同名本地分支没有跟踪所选远程分支；请先重命名、删除或切换到其他分支。",
            ));
        }
        // `git switch --track` accepts a short remote-tracking name such as
        // `origin/release`, rather than its fully-qualified
        // `refs/remotes/origin/release` form. Fetch requests the full branch
        // refspec positionally, so creating this branch never rewrites the
        // repository's persisted remote configuration.
        let arguments = os_arguments(&[
            "switch",
            "--track",
            "-c",
            branch,
            "--",
            &remote_tracking_branch,
        ]);
        let switch = self.execute(Some(repository), &arguments, &GitEnvironment::default())?;
        if switch.status.success() {
            return Ok(());
        }
        if !output_reports_uncovered_tracking_start_point(&switch) {
            return Err(self.git_command_error_for_repository(
                Some(repository),
                "切换远程分支",
                &switch,
            ));
        }

        // A single-branch clone (or any custom fetch refspec) leaves remote
        // refs under `refs/remotes/{remote}/` that `--track` refuses to wire
        // up: Git reports "starting point 'origin/x' is not a branch". The
        // remote-tracking ref itself was verified above, so write the same
        // upstream configuration `--track` would have produced, then create
        // the branch from the fully-qualified ref. Configuring first keeps a
        // failed switch from leaving a checked-out branch without upstream;
        // a failed config write instead leaves no new branch behind.
        let upstream_config = self.branch_upstream_config(repository, branch)?;
        let restore_error = |error: AppError| {
            if self
                .restore_branch_upstream_config(repository, branch, &upstream_config)
                .is_err()
            {
                error.with_cleanup_failure()
            } else {
                error
            }
        };
        if let Err(error) = self.configure_branch_upstream(repository, branch, remote_name, branch)
        {
            return Err(restore_error(error));
        }
        let expected_upstream = Some((
            remote_name.to_owned(),
            branch.to_owned(),
            remote_reference.clone(),
        ));
        let include_keys = self
            .execute(
                Some(repository),
                &os_arguments(&[
                    "config",
                    "--name-only",
                    "--get-regexp",
                    "^include\\.path$|^includeif\\..*\\.path$",
                ]),
                &GitEnvironment::default(),
            )
            .map_err(restore_error)?;
        let has_unconditional_include = if include_keys.status.code() == Some(1) {
            false
        } else if include_keys.status.success() {
            output_text(&include_keys.stdout).lines().any(|key| {
                let key = key.to_ascii_lowercase();
                key == "include.path" || !key.starts_with("includeif.onbranch:")
            })
        } else {
            return Err(restore_error(git_command_error(
                "检查 Git 配置包含项",
                &include_keys,
            )));
        };
        let effective_upstream = if has_unconditional_include {
            self.uncovered_branch_upstream(repository, branch)
        } else {
            self.uncovered_branch_upstream_without_includes(repository, branch)
        };
        match effective_upstream {
            Ok(upstream) if upstream == expected_upstream => {}
            Ok(_) => {
                return Err(restore_error(AppError::new(
                    "branch_upstream_config_conflict",
                    "分支上游配置与所选远程分支冲突。",
                )));
            }
            Err(error) => {
                return Err(restore_error(error));
            }
        }
        let arguments = os_arguments(&[
            "switch",
            "--no-track",
            "-c",
            branch,
            "--",
            &remote_reference,
        ]);
        let switch = match self.execute(Some(repository), &arguments, &GitEnvironment::default()) {
            Ok(output) => output,
            Err(error) => {
                let branch_reference = format!("refs/heads/{branch}");
                let branch_check = self.execute(
                    Some(repository),
                    &os_arguments(&["show-ref", "--verify", "--quiet", &branch_reference]),
                    &GitEnvironment::default(),
                );
                if matches!(error.code(), "git_operation_busy" | "git_unavailable")
                    || matches!(branch_check, Ok(ref output) if output.status.code() == Some(1))
                {
                    return Err(restore_error(error));
                }
                return Err(error);
            }
        };
        if !switch.status.success() {
            let error =
                self.git_command_error_for_repository(Some(repository), "切换远程分支", &switch);
            let branch_already_exists = String::from_utf8_lossy(&switch.stderr)
                .contains("a branch named")
                && String::from_utf8_lossy(&switch.stderr).contains("already exists");
            // A post-checkout hook can fail after Git created and checked out
            // the branch. Restore config only when Git confirms the branch is
            // absent; an inconclusive check must not strip a created branch's
            // upstream or hide the original checkout error.
            let branch_reference = format!("refs/heads/{branch}");
            let branch_check = self.execute(
                Some(repository),
                &os_arguments(&["show-ref", "--verify", "--quiet", &branch_reference]),
                &GitEnvironment::default(),
            );
            if branch_already_exists
                || matches!(branch_check, Ok(ref output) if output.status.code() == Some(1))
            {
                return Err(restore_error(error));
            }
            return Err(error);
        }
        if self.uncovered_branch_upstream(repository, branch)? != expected_upstream {
            return Err(AppError::new(
                "branch_upstream_config_conflict",
                "检出后的分支上游配置与所选远程分支冲突。",
            ));
        }
        Ok(())
    }

    pub(super) fn branch_upstream_config(
        &self,
        repository: &Path,
        branch: &str,
    ) -> Result<[Vec<String>; 2], AppError> {
        let mut values = [Vec::new(), Vec::new()];
        for (field, saved) in ["remote", "merge"].into_iter().zip(&mut values) {
            let key = format!("branch.{branch}.{field}");
            let output = self.execute(
                Some(repository),
                &os_arguments(&["config", "--local", "--null", "--get-all", &key]),
                &GitEnvironment::default(),
            )?;
            if output.status.code() == Some(1) {
                continue;
            }
            if !output.status.success() {
                return Err(git_command_error("读取分支上游", &output));
            }
            let records = output.stdout.strip_suffix(&[0]).unwrap_or(&output.stdout);
            for value in records.split(|byte| *byte == b'\0') {
                saved.push(
                    String::from_utf8(value.to_vec())
                        .map_err(|_| AppError::new("git_parse_error", "无法读取分支上游配置。"))?,
                );
            }
        }
        Ok(values)
    }

    /// Resolves a configured upstream when Git cannot infer one from the
    /// remote's fetch refspec, as in a single-branch clone.
    pub(super) fn uncovered_branch_upstream(
        &self,
        repository: &Path,
        branch: &str,
    ) -> Result<Option<(String, String, String)>, AppError> {
        self.uncovered_branch_upstream_with_includes(repository, branch, true)
    }

    fn uncovered_branch_upstream_without_includes(
        &self,
        repository: &Path,
        branch: &str,
    ) -> Result<Option<(String, String, String)>, AppError> {
        self.uncovered_branch_upstream_with_includes(repository, branch, false)
    }

    fn uncovered_branch_upstream_with_includes(
        &self,
        repository: &Path,
        branch: &str,
        includes: bool,
    ) -> Result<Option<(String, String, String)>, AppError> {
        let read_effective = |field: &str| -> Result<Vec<String>, AppError> {
            let key = format!("branch.{branch}.{field}");
            let mut arguments = os_arguments(&["config"]);
            if !includes {
                arguments.push("--no-includes".into());
            }
            arguments.extend(os_arguments(&["--null", "--get-all", &key]));
            let output = self.execute(Some(repository), &arguments, &GitEnvironment::default())?;
            if output.status.code() == Some(1) {
                return Ok(Vec::new());
            }
            if !output.status.success() {
                return Err(git_command_error("读取分支上游", &output));
            }
            let values = output.stdout.strip_suffix(&[0]).unwrap_or(&output.stdout);
            values
                .split(|byte| *byte == b'\0')
                .map(|value| {
                    String::from_utf8(value.to_vec())
                        .map_err(|_| AppError::new("git_parse_error", "无法读取分支上游配置。"))
                })
                .collect()
        };
        let remotes = read_effective("remote")?;
        let merges = read_effective("merge")?;
        let (Some(remote), [merge]) = (remotes.last(), merges.as_slice()) else {
            return Ok(None);
        };
        let Some(remote_branch) = merge.strip_prefix("refs/heads/") else {
            return Ok(None);
        };
        if validate_remote_name(&remote).is_err()
            || validate_ref_name(remote_branch, "远程分支名").is_err()
            || !self
                .remotes(repository)?
                .iter()
                .any(|item| item.name == *remote)
        {
            return Ok(None);
        }
        let reference = remote_tracking_reference(&remote, remote_branch);
        let mut fetch_arguments = os_arguments(&["config"]);
        if !includes {
            fetch_arguments.push("--no-includes".into());
        }
        fetch_arguments.extend(os_arguments(&[
            "--null",
            "--get-regexp",
            "^remote\\..*\\.fetch$",
        ]));
        let fetch_rules = self.execute(
            Some(repository),
            &fetch_arguments,
            &GitEnvironment::default(),
        )?;
        if !fetch_rules.status.success() && fetch_rules.status.code() != Some(1) {
            return Err(git_command_error("读取远程获取规则", &fetch_rules));
        }
        let mut rules = Vec::new();
        for record in fetch_rules
            .stdout
            .split(|byte| *byte == b'\0')
            .filter(|record| !record.is_empty())
        {
            let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
                return Err(AppError::new("git_parse_error", "无法读取远程获取规则。"));
            };
            let key = std::str::from_utf8(&record[..separator])
                .map_err(|_| AppError::new("git_parse_error", "无法读取远程获取规则。"))?;
            let owner = key
                .strip_prefix("remote.")
                .and_then(|key| key.strip_suffix(".fetch"))
                .ok_or_else(|| AppError::new("git_parse_error", "无法读取远程获取规则。"))?;
            let rule = std::str::from_utf8(&record[separator + 1..])
                .map_err(|_| AppError::new("git_parse_error", "无法读取远程获取规则。"))?;
            rules.push((
                owner.to_owned(),
                rule.strip_prefix('+').unwrap_or(rule).to_owned(),
            ));
        }
        for (owner, rule) in &rules {
            if let Some(exclusion) = rule.strip_prefix('^') {
                if owner == remote
                    && super::network::refspec_source_capture(exclusion, merge).is_some()
                {
                    return Ok(None);
                }
            }
        }
        for (owner, rule) in &rules {
            let Some((source, destination)) = rule.split_once(':') else {
                continue;
            };
            if owner == remote {
                if let Some(capture) = super::network::refspec_source_capture(source, merge) {
                    if super::network::expand_refspec_destination(destination, capture.as_deref())
                        .as_deref()
                        != Some(reference.as_str())
                    {
                        return Ok(None);
                    }
                }
            }
            if let Some(capture) = super::network::refspec_source_capture(destination, &reference) {
                let Some(mapped_source) =
                    super::network::expand_refspec_destination(source, capture.as_deref())
                else {
                    continue;
                };
                if rules.iter().any(|(negative_owner, negative)| {
                    negative_owner == owner
                        && negative.strip_prefix('^').is_some_and(|exclusion| {
                            super::network::refspec_source_capture(exclusion, &mapped_source)
                                .is_some()
                        })
                }) {
                    continue;
                }
                if owner != remote || mapped_source != *merge {
                    return Ok(None);
                }
            }
        }
        let exists = self.execute(
            Some(repository),
            &os_arguments(&["show-ref", "--verify", "--quiet", &reference]),
            &GitEnvironment::default(),
        )?;
        if !exists.status.success() {
            if exists.status.code() == Some(1) {
                return Ok(None);
            }
            return Err(git_command_error("检查远程分支", &exists));
        }
        Ok(Some((remote.clone(), remote_branch.to_owned(), reference)))
    }

    fn restore_branch_upstream_config(
        &self,
        repository: &Path,
        branch: &str,
        saved: &[Vec<String>; 2],
    ) -> Result<(), AppError> {
        for (field, values) in ["remote", "merge"].into_iter().zip(saved) {
            let key = format!("branch.{branch}.{field}");
            let output = self.execute(
                Some(repository),
                &os_arguments(&["config", "--local", "--unset-all", &key]),
                &GitEnvironment::default(),
            )?;
            if !output.status.success() && output.status.code() != Some(5) {
                return Err(git_command_error("恢复分支上游", &output));
            }
            for value in values {
                self.run(
                    Some(repository),
                    &os_arguments(&["config", "--local", "--add", &key, value]),
                    &GitEnvironment::default(),
                    "恢复分支上游",
                )?;
            }
        }
        Ok(())
    }

    /// Writes `branch.{name}.remote` and `branch.{name}.merge` for a remote
    /// branch that sits outside the configured fetch refspec. The keys are
    /// spelled unquoted (`branch.1.20.1.remote`): Git splits the final dot
    /// when addressing a key, so dotted branch names stay addressable, and
    /// `git config --get-regexp` reports the raw subsection that
    /// `branch_tracks_remote` compares against. A name the key grammar
    /// cannot express fails the write visibly instead of silently leaving
    /// the branch untracked.
    fn configure_branch_upstream(
        &self,
        repository: &Path,
        branch: &str,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<(), AppError> {
        for (key, value) in [
            (format!("branch.{branch}.remote"), remote_name.to_owned()),
            (
                format!("branch.{branch}.merge"),
                format!("refs/heads/{remote_branch}"),
            ),
        ] {
            self.run(
                Some(repository),
                &os_arguments(&["config", "--local", "--replace-all", &key, &value]),
                &GitEnvironment::default(),
                "写入分支上游",
            )?;
        }
        Ok(())
    }

    /// Reports whether the requested remote branch already resolves to the
    /// currently checked-out local tracking branch. Command handlers use this
    /// before stashing local changes, because switching to the current branch
    /// is a no-op and must not create an unexpected stash entry.
    pub(crate) fn is_current_remote_branch_target(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
    ) -> Result<bool, AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(branch, "远程分支名")?;
        let remote_tracking_branch = remote_tracking_branch_name(remote_name, branch);
        let Some(local_branch) = self
            .branches(repository)?
            .into_iter()
            .find(|local_branch| local_branch.name == branch && local_branch.is_current)
        else {
            return Ok(false);
        };
        Ok(
            local_branch.upstream.as_deref() == Some(remote_tracking_branch.as_str())
                || self.branch_tracks_remote(repository, branch, remote_name, branch)?,
        )
    }

    /// `%(upstream:short)` is empty when a valid upstream lies outside a
    /// single-branch clone's current fetch refspec. Read the branch's durable
    /// configuration as a fallback before deciding that an existing same-name
    /// branch conflicts with the requested remote branch.
    fn branch_tracks_remote(
        &self,
        repository: &Path,
        branch: &str,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<bool, AppError> {
        // Git's command-line config key grammar cannot quote a subsection;
        // asking for `branch."name".remote` therefore does not address a
        // branch whose name contains a dot or quote. Enumerate the two
        // branch fields as NUL-delimited records and compare the recovered
        // subsection exactly instead.
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "config",
                "--local",
                "--null",
                "--get-regexp",
                "^branch\\..*\\.(remote|merge)$",
            ]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        if !output.status.success() {
            return Err(git_command_error("读取本地分支上游", &output));
        }
        let mut configured_remote = None;
        let mut configured_merge = None;
        for record in output.stdout.split(|byte| *byte == b'\0') {
            let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
                continue;
            };
            let (key, value) = record.split_at(separator);
            let value = &value[1..];
            let key = std::str::from_utf8(key)
                .map_err(|_| AppError::new("git_parse_error", "无法读取本地分支上游。"))?;
            let value = std::str::from_utf8(value)
                .map_err(|_| AppError::new("git_parse_error", "无法读取本地分支上游。"))?;
            if let Some(configured_branch) = key
                .strip_prefix("branch.")
                .and_then(|key| key.strip_suffix(".remote"))
            {
                if configured_branch == branch {
                    configured_remote = Some(value.to_owned());
                }
            }
            if let Some(configured_branch) = key
                .strip_prefix("branch.")
                .and_then(|key| key.strip_suffix(".merge"))
            {
                if configured_branch == branch {
                    configured_merge = Some(value.to_owned());
                }
            }
        }
        Ok(configured_remote.as_deref() == Some(remote_name)
            && configured_merge.as_deref() == Some(&format!("refs/heads/{remote_branch}")))
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
        self.ensure_no_operation(repository)?;
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
        self.ensure_no_operation(repository)?;
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
            validate_revision(start_point, "起点引用")?;
        }
        self.ensure_no_operation(repository)?;
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
        self.ensure_no_operation(repository)?;
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
        if operation_marker_exists(&git_dir.join("rebase-merge"))?
            || operation_marker_exists(&git_dir.join("rebase-apply").join("rebasing"))?
        {
            return Ok(Some(GitOperationKind::Rebase));
        }
        if operation_marker_exists(&git_dir.join("CHERRY_PICK_HEAD"))? {
            return Ok(Some(GitOperationKind::CherryPick));
        }
        if operation_marker_exists(&git_dir.join("REVERT_HEAD"))? {
            return Ok(Some(GitOperationKind::Revert));
        }
        if operation_marker_exists(&git_dir.join("MERGE_HEAD"))? {
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
                noninteractive_git_editor_environment()?
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

/// `git switch --track` refuses a remote-tracking ref that the remote's
/// fetch refspec does not cover (single-branch clones and custom refspecs).
/// The environment pins `LC_ALL=C`, so this phrasing is stable.
fn output_reports_uncovered_tracking_start_point(output: &Output) -> bool {
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    diagnostic.contains("tracking information") && diagnostic.contains("is not a branch")
}

fn operation_marker_exists(path: &Path) -> Result<bool, AppError> {
    path.try_exists()
        .map_err(|_| AppError::new("git_operation_state_unavailable", "无法读取 Git 操作状态。"))
}
