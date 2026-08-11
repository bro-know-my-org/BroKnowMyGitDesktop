use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;

struct PushUpstream {
    local_branch: String,
    remote_branch: String,
    pushed_commit: String,
    tracking_reference: Option<String>,
    previous_tracking_commit: Option<String>,
}

struct BranchUpstream {
    remote: String,
    remote_reference: String,
    tracking_reference: Option<String>,
}

impl GitCore {
    /// Reads the current tip of one remote branch without updating local refs.
    /// Publish reconciliation uses this to prove that a successful-looking
    /// `gh repo create --push` really delivered the local commit before it
    /// clears its durable recovery intent.
    pub(crate) fn remote_branch_tip(
        &self,
        repository: &Path,
        remote_name: &str,
        branch: &str,
        environment: &GitEnvironment,
    ) -> Result<Option<String>, AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(branch, "远程分支名")?;
        // Publish reconciliation is a network operation too. In particular,
        // it must not let a repository-local credential helper or transport
        // setting run just because it is only reading a remote branch tip.
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = self.remote_url_for_operation(repository, remote_name, false)?;
        let remote_url = validate_git_remote_url(&remote_url)?;
        let reference = format!("refs/heads/{branch}");
        let arguments = network_execution_arguments(os_arguments(&[
            "ls-remote",
            "--heads",
            "--refs",
            &remote_url,
            &reference,
        ]));
        let output = self.run_network(
            Some(repository),
            &arguments,
            environment,
            "验证远程发布状态",
        )?;
        let output = output_text(&output.stdout);
        let Some(line) = output.lines().next() else {
            return Ok(None);
        };
        let Some((commit, returned_reference)) = line.split_once('\t') else {
            return Err(AppError::new("git_parse_error", "无法读取远程分支提交。"));
        };
        if returned_reference != reference
            || !(40..=64).contains(&commit.len())
            || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法读取远程分支提交。"));
        }
        Ok(Some(commit.to_owned()))
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
        self.ensure_configured_remote(repository, &options.remote)?;
        let refspecs = if options.refspecs.is_empty() {
            self.fetch_refspecs_for_remote(repository, &options.remote)?
        } else {
            options.refspecs.clone()
        };
        let remote_url = self.remote_url_for_operation(repository, &options.remote, false)?;
        self.fetch_with_environment_at_url(
            repository,
            &FetchOptions {
                remote: options.remote.clone(),
                refspecs,
                prune: options.prune,
            },
            &remote_url,
            environment,
        )
    }

    /// Captures the refspecs that a normal fetch would use before credentials
    /// are selected. The all-branches mapping replaces restrictive
    /// single-branch mappings, while non-branch namespaces such as Gerrit
    /// changes remain part of the operation.
    pub(crate) fn fetch_refspecs_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<Vec<String>, AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let mut refspecs = vec![all_remote_branches_fetch_refspec(remote_name)?];
        for refspec in self.repository_config_values(repository, &config_key)? {
            let refspec = refspec.trim();
            if refspec.is_empty()
                || (is_remote_branch_fetch_refspec(refspec) && !refspec.starts_with('^'))
            {
                continue;
            }
            validate_refspec(refspec)?;
            refspecs.push(refspec.to_owned());
        }
        Ok(refspecs)
    }

    /// Executes a fetch against the exact URL validated during credential
    /// resolution. This prevents a later rewrite of `remote.<name>.url` from
    /// changing where an app-selected credential is offered.
    pub(crate) fn fetch_with_environment_at_url(
        &self,
        repository: &Path,
        options: &FetchOptions,
        remote_url: &str,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = validate_git_remote_url(remote_url)?;
        let arguments = network_execution_arguments(fetch_arguments_for_url(options, &remote_url)?);
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
        self.ensure_configured_remote(repository, &options.remote)?;
        let branch = self.current_branch_name(repository)?;
        let refspec = match options.refspec.clone() {
            Some(refspec) => refspec,
            None => self.pull_refspec_for_remote(repository, &options.remote)?,
        };
        let remote_url = self.remote_url_for_operation(repository, &options.remote, false)?;
        if self.current_branch_name(repository)? != branch {
            return Err(AppError::new(
                "repository_changed",
                "当前分支已变化，请重新执行拉取。",
            ));
        }
        self.pull_with_environment_at_url(
            repository,
            &PullOptions {
                remote: options.remote.clone(),
                refspec: Some(refspec),
            },
            &remote_url,
            environment,
        )
    }

    /// Captures the current branch's configured upstream before a
    /// credential-bearing operation is prepared. Passing a raw URL to
    /// `git pull` bypasses `remote.<name>.fetch`, so use an explicit
    /// source:destination refspec: the source prevents Git from selecting the
    /// URL's advertised default branch, while the destination keeps the
    /// selected remote-tracking branch in sync just like a named-remote pull.
    pub(crate) fn pull_refspec_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<String, AppError> {
        let upstream = self.upstream_for_remote(repository, remote_name)?;
        let upstream_reference = upstream.remote_reference;
        let remote_branch = upstream_reference
            .strip_prefix("refs/heads/")
            .ok_or_else(|| {
                AppError::new("pull_upstream_not_a_branch", "当前分支上游不是远程分支。")
            })?;
        validate_ref_name(remote_branch, "远程分支名")?;
        let tracking_reference = upstream
            .tracking_reference
            .ok_or_else(pull_upstream_fetch_mapping_unavailable_error)?;
        let force = self.fetch_mapping_is_forced(
            repository,
            remote_name,
            &upstream_reference,
            &tracking_reference,
        )?;
        let refspec = format!(
            "{}{upstream_reference}:{tracking_reference}",
            if force { "+" } else { "" }
        );
        validate_refspec(&refspec)?;
        Ok(refspec)
    }

    fn upstream_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<BranchUpstream, AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let branch = self.current_branch_name(repository)?;
        let Some(upstream) = self.branch_upstream(repository, &branch)? else {
            return Err(pull_upstream_not_configured_error());
        };
        if upstream.remote != remote_name {
            return Err(AppError::new(
                "pull_upstream_remote_mismatch",
                "当前分支没有跟踪所选 remote。",
            ));
        }
        validate_refspec(&upstream.remote_reference)?;
        if let Some(tracking_reference) = &upstream.tracking_reference {
            validate_refspec(tracking_reference)?;
        }
        Ok(upstream)
    }

    fn fetch_mapping_is_forced(
        &self,
        repository: &Path,
        remote_name: &str,
        source_reference: &str,
        tracking_reference: &str,
    ) -> Result<bool, AppError> {
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let configured_refspecs = self.repository_config_values(repository, &config_key)?;
        if configured_refspecs.is_empty() {
            return Err(pull_upstream_fetch_mapping_unavailable_error());
        }

        let mut matching_force = None;
        for configured_refspec in configured_refspecs.iter().map(|refspec| refspec.trim()) {
            if configured_refspec.is_empty() {
                continue;
            }
            validate_refspec(configured_refspec)?;
            if let Some(exclusion) = configured_refspec.strip_prefix('^') {
                if refspec_source_capture(exclusion, source_reference).is_some() {
                    return Err(pull_upstream_fetch_mapping_unavailable_error());
                }
                continue;
            }
            let (forced, configured_refspec) = configured_refspec
                .strip_prefix('+')
                .map_or((false, configured_refspec), |refspec| (true, refspec));
            let Some((source_pattern, destination_pattern)) = configured_refspec.split_once(':')
            else {
                continue;
            };
            let Some(capture) = refspec_source_capture(source_pattern, source_reference) else {
                continue;
            };
            if expand_refspec_destination(destination_pattern, capture.as_deref()).as_deref()
                != Some(tracking_reference)
            {
                continue;
            }
            matching_force = Some(matching_force.unwrap_or(false) || forced);
        }
        matching_force.ok_or_else(pull_upstream_fetch_mapping_unavailable_error)
    }

    fn repository_config_values(
        &self,
        repository: &Path,
        config_key: &str,
    ) -> Result<Vec<String>, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--null", "--show-scope", "--get-all", config_key]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        if !output.status.success() {
            return Err(git_command_error("读取远程分支获取规则", &output));
        }
        parse_repository_config_values(&output.stdout)
    }

    /// Builds a deterministic current-branch push plan for the selected
    /// remote. Every path uses an explicit refspec, so user `push.default`
    /// configuration and a concurrent checkout cannot redirect the push.
    pub(crate) fn push_plan_for_remote(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<(Vec<String>, bool), AppError> {
        self.ensure_configured_remote(repository, remote_name)?;
        self.ensure_network_configuration_is_safe(repository)?;
        let branch = self.current_branch_name(repository)?;
        let local_reference = format!("refs/heads/{branch}");
        let (destination, set_upstream) = match self.branch_upstream(repository, &branch)? {
            Some(upstream)
                if upstream.remote == remote_name
                    && upstream.remote_reference.starts_with("refs/heads/") =>
            {
                (upstream.remote_reference, false)
            }
            Some(upstream) if upstream.remote == remote_name => {
                return Err(AppError::new(
                    "push_upstream_not_a_branch",
                    "当前分支上游不是远程分支。",
                ));
            }
            Some(_) => (local_reference.clone(), false),
            None => (local_reference.clone(), true),
        };
        let refspec = format!("{local_reference}:{destination}");
        validate_refspec(&refspec)?;
        Ok((vec![refspec], set_upstream))
    }

    fn branch_upstream(
        &self,
        repository: &Path,
        branch: &str,
    ) -> Result<Option<BranchUpstream>, AppError> {
        let branch_reference = format!("refs/heads/{branch}");
        let output = self.execute(
            Some(repository),
            &[
                OsString::from("for-each-ref"),
                OsString::from(
                    "--format=%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
                ),
                OsString::from(&branch_reference),
            ],
            &GitEnvironment::default(),
        )?;
        if !output.status.success() {
            return Err(git_command_error("读取当前分支上游", &output));
        }

        let mut matching_record = None;
        for record in output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
        {
            let Some(separator) = record.iter().position(|byte| *byte == b'\0') else {
                return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
            };
            if record[..separator] != *branch_reference.as_bytes() {
                continue;
            }
            if matching_record.replace(&record[separator + 1..]).is_some() {
                return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
            }
        }
        let record = match matching_record {
            Some(record) => record,
            None if self.current_branch_name(repository).ok().as_deref() == Some(branch)
                && self.has_unborn_head(repository)? =>
            {
                return Ok(None);
            }
            None => {
                return Err(AppError::new(
                    "repository_changed",
                    "仓库引用正在变化，请稍后重试。",
                ));
            }
        };
        let fields = record.split(|byte| *byte == b'\0').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(AppError::new("git_parse_error", "无法读取当前分支上游。"));
        }
        let configured_remote = std::str::from_utf8(fields[0])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        let remote_reference = std::str::from_utf8(fields[1])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        let tracking_reference = std::str::from_utf8(fields[2])
            .map_err(|_| AppError::new("git_parse_error", "无法读取当前分支上游。"))?;
        if configured_remote.is_empty() || remote_reference.is_empty() {
            return Ok(None);
        }
        validate_remote_name(configured_remote)?;
        validate_refspec(remote_reference)?;
        if !tracking_reference.is_empty() {
            validate_refspec(tracking_reference)?;
        }
        Ok(Some(BranchUpstream {
            remote: configured_remote.to_owned(),
            remote_reference: remote_reference.to_owned(),
            tracking_reference: (!tracking_reference.is_empty())
                .then(|| tracking_reference.to_owned()),
        }))
    }

    /// Pulls from an immutable, previously validated operation URL.
    pub(crate) fn pull_with_environment_at_url(
        &self,
        repository: &Path,
        options: &PullOptions,
        remote_url: &str,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let remote_url = validate_git_remote_url(remote_url)?;
        let arguments = network_execution_arguments(pull_arguments_for_url(options, &remote_url)?);
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
        self.ensure_configured_remote(repository, &options.remote)?;
        let remote_urls = self.remote_urls_for_operation(repository, &options.remote, true)?;
        self.push_with_environment_at_urls(repository, options, &remote_urls, environment)
    }

    /// Pushes to every URL captured before credential resolution. Git treats
    /// multiple push URLs as mirrors; running them one at a time preserves
    /// that behavior while avoiding a mutable remote-name lookup after the
    /// selected credential environment exists.
    pub(crate) fn push_with_environment_at_urls(
        &self,
        repository: &Path,
        options: &PushOptions,
        remote_urls: &[String],
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        if remote_urls.is_empty() {
            return Err(AppError::new("remote_not_found", "找不到该 remote。"));
        }
        for refspec in &options.refspecs {
            validate_refspec(refspec)?;
        }
        let (refspecs, upstream) = self.push_refspecs_for_url(repository, options)?;
        for remote_url in remote_urls {
            let remote_url = validate_git_remote_url(remote_url)?;
            let mut arguments = vec![
                OsString::from("push"),
                OsString::from("--recurse-submodules=no"),
            ];
            if options.force_with_lease {
                arguments.push(OsString::from("--force-with-lease"));
            }
            if options.follow_tags {
                arguments.push(OsString::from("--follow-tags"));
            }
            arguments.push(OsString::from(remote_url.as_str()));
            arguments.extend(refspecs.iter().map(OsString::from));
            let arguments = network_execution_arguments(arguments);
            self.run_network(Some(repository), &arguments, environment, "推送改动")?;
        }
        if let Some(upstream) = upstream {
            self.configure_push_upstream(repository, &options.remote, &upstream)?;
        }
        Ok(())
    }

    /// Passing a URL in place of a remote name preserves the credential
    /// binding, but Git cannot associate that URL with `branch.*.remote`.
    /// `push.default=upstream` therefore rejects an otherwise valid push.
    /// Capture the configured upstream and express it as an explicit refspec
    /// before using the immutable URL. A `--set-upstream` request is also
    /// converted to an explicit immutable-commit refspec: the network child
    /// holds the config lock, then this process establishes the named remote's
    /// official tracking relationship only after the push has completed.
    fn push_refspecs_for_url(
        &self,
        repository: &Path,
        options: &PushOptions,
    ) -> Result<(Vec<String>, Option<PushUpstream>), AppError> {
        if options.set_upstream {
            let mut upstream = self.push_upstream_for_url(repository, options)?;
            if let Some(tracking_reference) = self.configured_tracking_reference(
                repository,
                &options.remote,
                &upstream.remote_branch,
            )? {
                validate_ref_name(&tracking_reference, "远程跟踪引用")
                    .map_err(|_| push_upstream_fetch_mapping_unavailable_error())?;
                upstream.previous_tracking_commit =
                    self.reference_commit(repository, &tracking_reference)?;
                upstream.tracking_reference = Some(tracking_reference);
            }
            let refspec = format!(
                "{}:refs/heads/{}",
                upstream.pushed_commit, upstream.remote_branch
            );
            validate_refspec(&refspec)?;
            return Ok((vec![refspec], Some(upstream)));
        }
        if !options.refspecs.is_empty() || !self.push_default_is_upstream(repository)? {
            return Ok((options.refspecs.clone(), None));
        }
        let local_branch = self.current_branch_name(repository)?;
        let upstream_branch = self
            .upstream_for_remote(repository, &options.remote)?
            .remote_reference;
        let refspec = format!("refs/heads/{local_branch}:{upstream_branch}");
        validate_refspec(&refspec)?;
        Ok((vec![refspec], None))
    }

    fn push_upstream_for_url(
        &self,
        repository: &Path,
        options: &PushOptions,
    ) -> Result<PushUpstream, AppError> {
        match options.refspecs.as_slice() {
            [] => {
                let local_branch = self.current_branch_name(repository)?;
                let pushed_commit = self.commit_for_local_branch(repository, &local_branch)?;
                Ok(PushUpstream {
                    remote_branch: local_branch.clone(),
                    local_branch,
                    pushed_commit,
                    tracking_reference: None,
                    previous_tracking_commit: None,
                })
            }
            [refspec] => {
                let (local_branch, remote_branch) = push_branches_for_refspec(refspec)?;
                let local_reference = format!("refs/heads/{local_branch}^{{commit}}");
                let commit = self.execute(
                    Some(repository),
                    &[
                        OsString::from("rev-parse"),
                        OsString::from("--verify"),
                        OsString::from("--quiet"),
                        OsString::from(local_reference),
                    ],
                    &GitEnvironment::default(),
                )?;
                if !commit.status.success() {
                    return Err(AppError::new(
                        "set_upstream_requires_local_branch",
                        "设置上游必须指定一个存在的本地分支。",
                    ));
                }
                Ok(PushUpstream {
                    local_branch,
                    remote_branch,
                    pushed_commit: parse_commit_id(
                        &commit.stdout,
                        "无法读取首次推送的本地提交标识。",
                    )?,
                    tracking_reference: None,
                    previous_tracking_commit: None,
                })
            }
            _ => Err(AppError::new(
                "set_upstream_requires_single_branch",
                "设置上游只能推送当前分支到一个远程分支。",
            )),
        }
    }

    fn configure_push_upstream(
        &self,
        repository: &Path,
        remote_name: &str,
        upstream: &PushUpstream,
    ) -> Result<(), AppError> {
        validate_remote_name(remote_name)?;
        validate_ref_name(&upstream.local_branch, "本地分支名")?;
        validate_ref_name(&upstream.remote_branch, "远程分支名")?;
        let created_tracking_reference;
        let tracking_reference = if let Some(reference) = upstream.tracking_reference.as_deref() {
            reference
        } else {
            created_tracking_reference = self.ensure_push_remote_tracking_mapping(
                repository,
                remote_name,
                &upstream.remote_branch,
            )?;
            created_tracking_reference.as_str()
        };
        validate_ref_name(tracking_reference, "远程跟踪引用")
            .map_err(|_| push_tracking_setup_incomplete_error())?;

        self.update_tracking_reference_after_push(
            repository,
            tracking_reference,
            &upstream.pushed_commit,
            upstream.previous_tracking_commit.as_deref(),
        )?;

        let set_upstream_argument = format!("--set-upstream-to={tracking_reference}");
        self.run(
            Some(repository),
            &[
                OsString::from("branch"),
                OsString::from(set_upstream_argument),
                OsString::from(&upstream.local_branch),
            ],
            &GitEnvironment::default(),
            "设置当前分支上游",
        )?;
        let configured = self
            .branch_upstream(repository, &upstream.local_branch)
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        let expected_remote_reference = format!("refs/heads/{}", upstream.remote_branch);
        if !configured.is_some_and(|configured| {
            configured.remote == remote_name
                && configured.remote_reference == expected_remote_reference
                && configured.tracking_reference.as_deref() == Some(tracking_reference)
        }) {
            return Err(push_tracking_setup_incomplete_error());
        }
        Ok(())
    }

    pub(super) fn update_tracking_reference_after_push(
        &self,
        repository: &Path,
        tracking_reference: &str,
        pushed_commit: &str,
        previous_tracking_commit: Option<&str>,
    ) -> Result<(), AppError> {
        validate_ref_name(tracking_reference, "远程跟踪引用")
            .map_err(|_| push_tracking_setup_incomplete_error())?;
        parse_commit_id(pushed_commit.as_bytes(), "无法读取首次推送的本地提交标识。")?;
        let expected = previous_tracking_commit.unwrap_or("");
        let output = self.execute(
            Some(repository),
            &os_arguments(&[
                "update-ref",
                "--no-deref",
                "--create-reflog",
                "-m",
                "update by push",
                tracking_reference,
                pushed_commit,
                expected,
            ]),
            &GitEnvironment::default(),
        )?;
        if output.status.success() {
            return Ok(());
        }
        let verification = self.execute(
            Some(repository),
            &os_arguments(&[
                "update-ref",
                "--no-deref",
                tracking_reference,
                pushed_commit,
                pushed_commit,
            ]),
            &GitEnvironment::default(),
        )?;
        if verification.status.success() {
            Ok(())
        } else {
            Err(push_tracking_setup_incomplete_error())
        }
    }

    fn commit_for_local_branch(&self, repository: &Path, branch: &str) -> Result<String, AppError> {
        validate_ref_name(branch, "本地分支名")?;
        let revision = format!("refs/heads/{branch}^{{commit}}");
        let output = self.run(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", &revision]),
            &GitEnvironment::default(),
            "读取首次推送的本地提交",
        )?;
        parse_commit_id(&output.stdout, "无法读取首次推送的本地提交标识。")
    }

    fn reference_commit(
        &self,
        repository: &Path,
        reference: &str,
    ) -> Result<Option<String>, AppError> {
        validate_refspec(reference)?;
        let revision = format!("{reference}^{{commit}}");
        let output = self.execute(
            Some(repository),
            &os_arguments(&["rev-parse", "--verify", "--quiet", &revision]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(git_command_error("读取远程跟踪分支", &output));
        }
        parse_commit_id(&output.stdout, "无法读取远程跟踪分支提交标识。").map(Some)
    }

    /// Ensure Git can reverse-map the pushed remote branch to a local
    /// remote-tracking ref. `git remote set-branches --add` is the porcelain
    /// operation Git provides for extending a restrictive single-branch
    /// clone without replacing the user's existing tracked branch set.
    fn ensure_push_remote_tracking_mapping(
        &self,
        repository: &Path,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<String, AppError> {
        if let Some(reference) =
            self.configured_tracking_reference(repository, remote_name, remote_branch)?
        {
            return Ok(reference);
        }
        self.run(
            Some(repository),
            &[
                OsString::from("remote"),
                OsString::from("set-branches"),
                OsString::from("--add"),
                OsString::from(remote_name),
                OsString::from(remote_branch),
            ],
            &GitEnvironment::default(),
            "扩展远程分支跟踪范围",
        )?;
        self.configured_tracking_reference(repository, remote_name, remote_branch)?
            .ok_or_else(push_upstream_fetch_mapping_unavailable_error)
    }

    pub(super) fn configured_tracking_reference(
        &self,
        repository: &Path,
        remote_name: &str,
        remote_branch: &str,
    ) -> Result<Option<String>, AppError> {
        let config_key = remote_fetch_refspec_config_key(remote_name)?;
        let source_reference = format!("refs/heads/{remote_branch}");
        let mut tracking_reference = None;
        let mut excluded = false;
        for configured_refspec in self
            .repository_config_values(repository, &config_key)?
            .iter()
            .map(|refspec| refspec.trim())
        {
            if configured_refspec.is_empty() {
                continue;
            }
            validate_refspec(configured_refspec)?;
            if let Some(exclusion) = configured_refspec.strip_prefix('^') {
                excluded |= refspec_source_capture(exclusion, &source_reference).is_some();
                continue;
            }
            let configured_refspec = configured_refspec
                .strip_prefix('+')
                .unwrap_or(configured_refspec);
            let Some((source_pattern, destination_pattern)) = configured_refspec.split_once(':')
            else {
                continue;
            };
            let Some(capture) = refspec_source_capture(source_pattern, &source_reference) else {
                continue;
            };
            let Some(destination) =
                expand_refspec_destination(destination_pattern, capture.as_deref())
            else {
                continue;
            };
            validate_ref_name(&destination, "远程跟踪引用")
                .map_err(|_| push_upstream_fetch_mapping_unavailable_error())?;
            let expected_prefix = format!("refs/remotes/{remote_name}/");
            if !destination.starts_with(&expected_prefix) {
                return Err(push_upstream_fetch_mapping_unavailable_error());
            }
            match tracking_reference.as_deref() {
                Some(existing) if existing != destination => {
                    return Err(push_upstream_fetch_mapping_unavailable_error());
                }
                Some(_) => {}
                None => tracking_reference = Some(destination),
            }
        }
        if excluded {
            return Err(push_upstream_fetch_mapping_unavailable_error());
        }
        Ok(tracking_reference)
    }

    fn push_default_is_upstream(&self, repository: &Path) -> Result<bool, AppError> {
        let output = self.execute(
            Some(repository),
            &os_arguments(&["config", "--get", "push.default"]),
            &GitEnvironment::default(),
        )?;
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        if !output.status.success() {
            return Err(git_command_error("读取推送默认规则", &output));
        }
        Ok(output_text(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("upstream"))
    }

    /// Git accepts a URL in the positional repository slot for fetch, pull,
    /// and push. The desktop API accepts a remote *name* only, because its
    /// credential resolution is bound to one persisted remote configuration.
    fn ensure_configured_remote(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        if self
            .remotes(repository)?
            .iter()
            .any(|remote| remote.name == name)
        {
            // The name came from a configured remote, but it will later be
            // interpolated into refspecs. Reject names that Git can store yet
            // cannot safely use in that generated namespace.
            validate_remote_name(name)?;
            Ok(())
        } else {
            Err(AppError::new("remote_not_found", "找不到该 remote。"))
        }
    }
}

fn pull_upstream_fetch_mapping_unavailable_error() -> AppError {
    AppError::new(
        "pull_upstream_fetch_mapping_unavailable",
        "当前分支上游没有可用的远程跟踪引用映射。",
    )
}

fn push_upstream_fetch_mapping_unavailable_error() -> AppError {
    AppError::new(
        "push_upstream_fetch_mapping_unavailable",
        "无法为首次推送的远程分支建立唯一的远程跟踪引用映射。",
    )
}

fn push_tracking_setup_incomplete_error() -> AppError {
    AppError::new(
        "push_tracking_setup_incomplete",
        "远程推送已成功，但本地远程跟踪引用同时发生变化，上游设置未能完成。",
    )
}

fn parse_commit_id(output: &[u8], message: &'static str) -> Result<String, AppError> {
    let output = output_text(output);
    let commit_id = output.trim();
    if !(40..=64).contains(&commit_id.len())
        || !commit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::new("git_parse_error", message));
    }
    Ok(commit_id.to_owned())
}

pub(super) fn parse_repository_config_values(output: &[u8]) -> Result<Vec<String>, AppError> {
    let mut fields = output.split(|byte| *byte == b'\0').collect::<Vec<_>>();
    if fields.last() == Some(&&[][..]) {
        fields.pop();
    }
    if fields.len() % 2 != 0 {
        return Err(AppError::new(
            "git_parse_error",
            "无法读取远程分支获取规则。",
        ));
    }

    fields
        .chunks_exact(2)
        .filter(|fields| fields[0] == b"local" || fields[0] == b"worktree")
        .map(|fields| {
            std::str::from_utf8(fields[1])
                .map(str::to_owned)
                .map_err(|_| AppError::new("git_parse_error", "无法读取远程分支获取规则。"))
        })
        .collect()
}

fn refspec_source_capture(pattern: &str, reference: &str) -> Option<Option<String>> {
    let Some(wildcard_index) = pattern.find('*') else {
        return (pattern == reference).then_some(None);
    };
    if pattern[wildcard_index + 1..].contains('*') {
        return None;
    }
    let prefix = &pattern[..wildcard_index];
    let suffix = &pattern[wildcard_index + 1..];
    if !reference.starts_with(prefix)
        || !reference.ends_with(suffix)
        || reference.len() < prefix.len() + suffix.len()
    {
        return None;
    }
    Some(Some(
        reference[prefix.len()..reference.len() - suffix.len()].to_owned(),
    ))
}

fn expand_refspec_destination(pattern: &str, capture: Option<&str>) -> Option<String> {
    match (pattern.find('*'), capture) {
        (None, None) => Some(pattern.to_owned()),
        (Some(wildcard_index), Some(capture)) if !pattern[wildcard_index + 1..].contains('*') => {
            Some(format!(
                "{}{capture}{}",
                &pattern[..wildcard_index],
                &pattern[wildcard_index + 1..]
            ))
        }
        _ => None,
    }
}

fn push_branches_for_refspec(refspec: &str) -> Result<(String, String), AppError> {
    let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
    let (source, destination) = refspec
        .split_once(':')
        .map_or((refspec, None), |(source, destination)| {
            (source, Some(destination))
        });
    let local_branch = source.strip_prefix("refs/heads/").unwrap_or(source);
    if source == "HEAD"
        || source.is_empty()
        || source.starts_with("refs/") && !source.starts_with("refs/heads/")
    {
        return Err(AppError::new(
            "set_upstream_requires_current_branch",
            "设置上游必须明确指定一个本地分支。",
        ));
    }
    validate_ref_name(local_branch, "本地分支名")?;
    let destination = destination.unwrap_or(local_branch);
    if destination.starts_with("refs/") && !destination.starts_with("refs/heads/") {
        return Err(AppError::new(
            "set_upstream_requires_remote_branch",
            "设置上游必须指定一个远程分支。",
        ));
    }
    let remote_branch = destination
        .strip_prefix("refs/heads/")
        .unwrap_or(destination);
    validate_ref_name(remote_branch, "远程分支名")?;
    Ok((local_branch.to_owned(), remote_branch.to_owned()))
}

/// Command-scope overrides complement the preflight: a repository config may
/// still change after it has been inspected. The remote URL itself is passed
/// positionally, while these transport and hook settings prevent that raced
/// config from selecting a proxy, SSH command, protocol, or hook helper.
fn network_execution_arguments(mut arguments: Vec<OsString>) -> Vec<OsString> {
    let mut protected = Vec::with_capacity(arguments.len() + 30);
    for (key, value) in [
        ("http.extraheader", ""),
        ("http.proxy", ""),
        ("http.sslverify", "true"),
        ("core.hooksPath", DISABLED_GIT_HOOKS_PATH),
        ("core.gitProxy", ""),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
        ("protocol.http.allow", "always"),
        ("protocol.https.allow", "always"),
        ("protocol.ssh.allow", "always"),
        ("protocol.git.allow", "always"),
        ("protocol.file.allow", "always"),
        ("protocol.ext.allow", "never"),
    ] {
        protected.push(OsString::from("-c"));
        protected.push(OsString::from(format!("{key}={value}")));
    }
    protected.append(&mut arguments);
    protected
}

pub(super) fn fetch_arguments_for_url(
    options: &FetchOptions,
    remote_url: &str,
) -> Result<Vec<OsString>, AppError> {
    validate_remote_name(&options.remote)?;
    let remote_url = validate_git_remote_url(remote_url)?;
    for refspec in &options.refspecs {
        validate_refspec(refspec)?;
    }
    let mut arguments = vec![
        OsString::from("fetch"),
        OsString::from("--no-recurse-submodules"),
    ];
    if options.prune {
        arguments.push(OsString::from("--prune"));
    }
    arguments.push(OsString::from(remote_url));
    if options.refspecs.is_empty() {
        return Err(AppError::new(
            "fetch_refspec_required",
            "获取远程更新需要已确认的引用规格。",
        ));
    }
    arguments.extend(options.refspecs.iter().map(OsString::from));
    Ok(arguments)
}

fn remote_fetch_refspec_config_key(remote_name: &str) -> Result<String, AppError> {
    validate_remote_name(remote_name)?;
    Ok(format!("remote.{remote_name}.fetch"))
}

fn is_remote_branch_fetch_refspec(refspec: &str) -> bool {
    let source = refspec.strip_prefix('+').unwrap_or(refspec);
    source.starts_with("refs/heads/") || source.starts_with("^refs/heads/")
}

pub(super) fn pull_arguments_for_url(
    options: &PullOptions,
    remote_url: &str,
) -> Result<Vec<OsString>, AppError> {
    validate_remote_name(&options.remote)?;
    let remote_url = validate_git_remote_url(remote_url)?;
    let refspec = options
        .refspec
        .as_deref()
        .ok_or_else(pull_upstream_not_configured_error)?;
    validate_refspec(refspec)?;
    Ok(os_arguments(&[
        "pull",
        "--no-edit",
        "--no-recurse-submodules",
        &remote_url,
        refspec,
    ]))
}

fn pull_upstream_not_configured_error() -> AppError {
    AppError::new(
        "pull_upstream_not_configured",
        "当前分支未配置可拉取的上游。",
    )
}
