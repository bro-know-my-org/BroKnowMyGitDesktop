use super::environment::*;
use super::error::*;
use super::models::*;
use super::validation::*;
use super::*;

impl GitCore {
    /// Fetches exactly one remote.  The caller must resolve its credentials
    /// before calling this method.
    pub fn fetch(&self, repository: &Path, options: &FetchOptions) -> Result<(), AppError> {
        self.fetch_with_environment(repository, options, &GitEnvironment::default())
    }

    /// Makes a remote's normal fetch operation include every branch it
    /// advertises. Git creates a restrictive refspec for `--single-branch`
    /// clones (and implicitly for shallow clones), which otherwise leaves
    /// branches absent from both the remote-branch list and `--track`
    /// checkout.
    ///
    /// Branch-tracking refspecs are replaced rather than supplemented:
    /// retaining an exact single-branch source makes every later `git fetch`
    /// fail once that branch is deleted on the server, even if a wildcard
    /// refspec was added alongside it. Non-branch custom fetch rules, such as
    /// Gerrit change refs, are kept intact.
    pub fn ensure_remote_fetches_all_branches(
        &self,
        repository: &Path,
        remote_name: &str,
    ) -> Result<(), AppError> {
        validate_remote_name(remote_name)?;
        if !self
            .remotes(repository)?
            .iter()
            .any(|remote| remote.name == remote_name)
        {
            return Err(AppError::new("remote_not_found", "找不到该 remote。"));
        }

        let refspec = all_remote_branches_fetch_refspec(remote_name)?;
        let config_key = remote_fetch_refspec_config_key(remote_name);
        let existing = self.execute(
            Some(repository),
            &os_arguments(&["config", "--local", "--get-all", &config_key]),
            &GitEnvironment::default(),
        )?;
        if existing.status.success() {
            let configured = output_text(&existing.stdout);
            if configured.lines().eq([refspec.as_str()]) {
                return Ok(());
            }
            if configured.lines().any(is_remote_branch_fetch_refspec) {
                self.run(
                    Some(repository),
                    &os_arguments(&[
                        "config",
                        "--local",
                        "--unset-all",
                        &config_key,
                        REMOTE_BRANCH_FETCH_REFSPEC_PATTERN,
                    ]),
                    &GitEnvironment::default(),
                    "迁移远程分支获取规则",
                )?;
            }
        } else if existing.status.code() != Some(1) {
            return Err(git_command_error("读取远程分支获取规则", &existing));
        }

        self.run(
            Some(repository),
            &os_arguments(&["config", "--local", "--add", &config_key, &refspec]),
            &GitEnvironment::default(),
            "启用全部远程分支获取",
        )?;
        Ok(())
    }

    /// Backend-only equivalent of [`Self::fetch`] with a controlled process
    /// environment, for example an askpass helper selected by CredentialBroker.
    pub fn fetch_with_environment(
        &self,
        repository: &Path,
        options: &FetchOptions,
        environment: &GitEnvironment,
    ) -> Result<(), AppError> {
        self.ensure_network_configuration_is_safe(repository)?;
        let arguments = fetch_arguments(options)?;
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
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        if let Some(refspec) = options.refspec.as_deref() {
            validate_refspec(refspec)?;
        }
        let mut arguments = os_arguments(&["pull", &options.remote]);
        if let Some(refspec) = options.refspec.as_deref() {
            arguments.push(OsString::from(refspec));
        }
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
        self.ensure_network_configuration_is_safe(repository)?;
        validate_remote_name(&options.remote)?;
        for refspec in &options.refspecs {
            validate_refspec(refspec)?;
        }
        let mut arguments = vec![OsString::from("push")];
        if options.set_upstream {
            arguments.push(OsString::from("--set-upstream"));
        }
        if options.force_with_lease {
            arguments.push(OsString::from("--force-with-lease"));
        }
        if options.follow_tags {
            arguments.push(OsString::from("--follow-tags"));
        }
        arguments.push(OsString::from(&options.remote));
        arguments.extend(options.refspecs.iter().map(OsString::from));
        self.run_network(Some(repository), &arguments, environment, "推送改动")?;
        Ok(())
    }
}
