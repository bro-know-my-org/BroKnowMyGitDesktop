use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

impl GitCore {
    /// Lists tags with their object IDs, newest first according to Git's
    /// creator date ordering.
    pub fn tags(&self, repository: &Path) -> Result<Vec<TagInfo>, AppError> {
        let arguments = os_arguments(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname:short)%00%(objectname)",
            "refs/tags",
        ]);
        let output = self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "读取标签",
        )?;
        parse_tags(&output.stdout)
    }

    /// Creates a lightweight tag, or an annotated tag when `message` is set.
    pub fn create_tag(
        &self,
        repository: &Path,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), AppError> {
        validate_ref_name(name, "标签名")?;
        if let Some(target) = target {
            validate_ref_name(target, "标签目标")?;
        }
        if let Some(message) = message {
            if message.trim().is_empty() {
                return Err(AppError::new("invalid_tag_message", "注释标签信息不能为空"));
            }
        }

        let mut arguments = vec![OsString::from("tag")];
        if let Some(message) = message {
            arguments.push(OsString::from("-a"));
            arguments.push(OsString::from(name));
            arguments.push(OsString::from("-m"));
            arguments.push(OsString::from(message));
        } else {
            arguments.push(OsString::from(name));
        }
        if let Some(target) = target {
            arguments.push(OsString::from(target));
        }
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "创建标签",
        )?;
        Ok(())
    }

    /// Deletes a local tag only; it does not delete the corresponding remote
    /// tag.
    pub fn delete_tag(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        validate_ref_name(name, "标签名")?;
        let arguments = os_arguments(&["tag", "-d", name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "删除标签",
        )?;
        Ok(())
    }

    /// Lists all configured remotes with credential-safe display URLs and
    /// built-in endpoint recognition.
    pub fn remotes(&self, repository: &Path) -> Result<Vec<RemoteProfile>, AppError> {
        let output = self.run(
            Some(repository),
            &os_arguments(&["remote"]),
            &GitEnvironment::default(),
            "读取远程仓库",
        )?;

        let endpoints = CodeHostEndpoint::builtin();
        let mut remotes = Vec::new();
        for name in output_text(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let fetch_url = self.remote_url(repository, name, false)?;
            let push_url = self.remote_url(repository, name, true)?;
            let parsed = parse_remote_url(&fetch_url);
            let endpoint = parsed
                .authority()
                .as_deref()
                .and_then(|authority| endpoint_for_host(authority, &endpoints));
            remotes.push(RemoteProfile {
                name: name.to_owned(),
                fetch_url: safe_remote_url(&fetch_url),
                push_url: safe_remote_url(&push_url),
                parsed,
                endpoint_id: endpoint.map(|endpoint| endpoint.id.clone()),
                code_host_kind: endpoint
                    .map(|endpoint| endpoint.kind.clone())
                    .unwrap_or(CodeHostKind::Unknown),
            });
        }
        remotes.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(remotes)
    }

    /// Returns the configured URL for backend credential resolution only.
    ///
    /// Unlike [`Self::remotes`], this can contain userinfo from an unsafe
    /// pre-existing Git configuration, so it must never be serialized, logged,
    /// or returned through a Tauri command. CredentialBroker validates and
    /// rejects unsafe HTTP(S) values before a network operation proceeds.
    pub fn remote_url_for_operation(
        &self,
        repository: &Path,
        name: &str,
        push: bool,
    ) -> Result<String, AppError> {
        self.remote_url(repository, name, push)
    }

    /// Adds a remote.  URLs containing an HTTP(S) username, password, query,
    /// or fragment are rejected to ensure a PAT cannot become Git config.
    pub fn add_remote(&self, repository: &Path, name: &str, url: &str) -> Result<(), AppError> {
        validate_remote_name(name)?;
        validate_git_remote_url(url)?;
        let arguments = os_arguments(&["remote", "add", name, url]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "添加远程仓库",
        )?;
        Ok(())
    }

    /// Replaces either the fetch URL or push URL of an existing remote.
    pub fn set_remote_url(
        &self,
        repository: &Path,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
    ) -> Result<(), AppError> {
        validate_remote_name(name)?;
        validate_git_remote_url(url)?;
        let mut arguments = os_arguments(&["remote", "set-url"]);
        if kind == RemoteUrlKind::Push {
            arguments.push(OsString::from("--push"));
        }
        arguments.push(OsString::from(name));
        arguments.push(OsString::from(url));
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "更新远程地址",
        )?;
        Ok(())
    }

    /// Reject repository-local configuration that can execute a credential
    /// helper or inject transport credentials. Local `.git/config` belongs to
    /// the opened repository, so it is not a trusted source for an
    /// application-selected account. Global/system helpers remain available
    /// only for an unscoped URL-based fallback, never for a selected profile.
    pub fn ensure_network_configuration_is_safe(&self, repository: &Path) -> Result<(), AppError> {
        let arguments = os_arguments(&[
            "config",
            "--local",
            "--null",
            "--name-only",
            "--get-regexp",
            ".",
        ]);
        let output = self.execute(Some(repository), &arguments, &GitEnvironment::default())?;
        if output.status.success() {
            let unsafe_key = output
                .stdout
                .split(|byte| *byte == b'\0')
                .filter_map(|key| std::str::from_utf8(key).ok())
                .any(is_unsafe_network_config_key);
            return if unsafe_key {
                Err(AppError::new(
                    "unsafe_git_config",
                    "仓库本地 Git 配置包含不安全的凭据或网络设置；请移除后再执行网络操作。",
                ))
            } else {
                Ok(())
            };
        }

        // `git config --get-regexp` exits with 1 when there are no matching
        // keys. That is a safe, ordinary empty-configuration result.
        if output.status.code() == Some(1) {
            Ok(())
        } else {
            Err(git_command_error("检查仓库网络配置", &output))
        }
    }

    /// Renames a local remote configuration entry.
    pub fn rename_remote(
        &self,
        repository: &Path,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), AppError> {
        validate_remote_name(old_name)?;
        validate_remote_name(new_name)?;
        let arguments = os_arguments(&["remote", "rename", old_name, new_name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "重命名远程仓库",
        )?;
        Ok(())
    }

    /// Removes a local remote configuration entry only.
    pub fn remove_remote(&self, repository: &Path, name: &str) -> Result<(), AppError> {
        validate_remote_name(name)?;
        let arguments = os_arguments(&["remote", "remove", name]);
        self.run(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            "移除远程仓库",
        )?;
        Ok(())
    }

    /// Loads the fields most repository pages need in one backend call.
    pub fn snapshot(&self, repository: &Path) -> Result<RepositorySnapshot, AppError> {
        let remotes = self.remotes(repository)?;
        let remote_names = remotes
            .iter()
            .map(|remote| remote.name.clone())
            .collect::<Vec<_>>();
        Ok(RepositorySnapshot {
            repository: self.repository_record(repository)?,
            status: self.status(repository)?,
            branches: self.branches(repository)?,
            remote_branches: self.remote_branches_for_names(repository, &remote_names)?,
            remotes,
            tags: self.tags(repository)?,
        })
    }
}
