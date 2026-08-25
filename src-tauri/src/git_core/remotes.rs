use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;
use super::*;

const MAX_TAG_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_NETWORK_CONFIGURATION_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkConfigurationAssessment {
    pub unsafe_keys: Vec<String>,
    pub fingerprint: Option<String>,
    pub authorization_allowed: bool,
}

impl NetworkConfigurationAssessment {
    pub fn is_safe(&self) -> bool {
        self.fingerprint.is_none()
    }
}

impl GitCore {
    /// Builds a protected environment for operations intentionally delegated
    /// to the user's normal Git credential manager. Repository-local helpers
    /// are never trusted: read only system/global `credential.*` values, then
    /// reapply them at command scope after clearing lower-priority helpers.
    pub(crate) fn unbound_network_environment(&self) -> Result<GitEnvironment, AppError> {
        let mut trusted_credential_configuration = Vec::new();
        for scope in ["--system", "--global"] {
            let output = self.execute(
                None,
                &os_arguments(&[
                    "config",
                    scope,
                    "--includes",
                    "--null",
                    "--get-regexp",
                    "^credential\\..*$",
                ]),
                &GitEnvironment::default(),
            )?;
            if output.status.success() {
                trusted_credential_configuration
                    .extend(parse_trusted_credential_configuration(&output.stdout)?);
            } else if output.status.code() != Some(1) {
                return Err(git_command_error("读取系统凭据配置", &output));
            }
        }
        if trusted_credential_configuration.len() > 128 {
            return Err(AppError::new(
                "unsafe_git_config",
                "系统凭据配置数量过多，拒绝执行网络操作。",
            ));
        }
        let mut environment = GitEnvironment::new();
        environment.isolate_network_transport_and_disable_hooks_with_trusted_credentials(
            &trusted_credential_configuration,
        );
        Ok(environment)
    }

    /// Lists tags with their object IDs, newest first according to Git's
    /// creator date ordering.
    pub fn tags(&self, repository: &Path) -> Result<Vec<TagInfo>, AppError> {
        let arguments = os_arguments(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname:short)%00%(*objectname)%00%(objectname)",
            "refs/tags",
        ]);
        let output = self.run_with_output_limit(
            Some(repository),
            &arguments,
            &GitEnvironment::default(),
            MAX_TAG_OUTPUT_BYTES,
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
            validate_revision(target, "标签目标")?;
        }
        if let Some(message) = message {
            validate_tag_message(message)?;
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
        let names = std::str::from_utf8(&output.stdout)
            .map_err(|_| AppError::new("git_parse_error", "无法解析远程仓库名称"))?;
        for name in names.lines().map(str::trim).filter(|name| !name.is_empty()) {
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

    /// Returns every effective URL Git would use for one remote operation.
    /// A push remote may deliberately mirror to several destinations; callers
    /// that authorize a credential must validate each of them rather than
    /// silently checking only the first URL.
    pub fn remote_urls_for_operation(
        &self,
        repository: &Path,
        name: &str,
        push: bool,
    ) -> Result<Vec<String>, AppError> {
        self.remote_urls(repository, name, push)
    }

    /// Adds a remote.  URLs containing an HTTP(S) username, password, query,
    /// or fragment are rejected to ensure a PAT cannot become Git config.
    pub fn add_remote(&self, repository: &Path, name: &str, url: &str) -> Result<(), AppError> {
        validate_remote_name(name)?;
        let url = validate_git_remote_url(url)?;
        let arguments = os_arguments(&["remote", "add", name, &url]);
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
        let url = validate_git_remote_url(url)?;
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

    /// Reject repository-scoped configuration that can execute a credential
    /// helper or inject transport credentials. This reads the effective local
    /// and worktree scopes with global/system configuration disabled, so a
    /// linked worktree's `config.worktree` cannot bypass the same boundary.
    pub(crate) fn network_configuration_assessment(
        &self,
        repository: &Path,
    ) -> Result<NetworkConfigurationAssessment, AppError> {
        let repository_root = self.repository_root(repository)?;
        if !paths_match(&repository_root, repository) {
            return Err(AppError::new(
                "repository_root_required",
                "网络操作必须从仓库根目录执行。",
            ));
        }
        let arguments = os_arguments(&["config", "--includes", "--null", "--list"]);
        let environment = GitEnvironment::new()
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH);
        let output = self.run_with_output_limit(
            Some(repository),
            &arguments,
            &environment,
            MAX_NETWORK_CONFIGURATION_BYTES,
            "检查仓库网络配置",
        )?;
        {
            let mut unsafe_entries = Vec::new();
            let mut unsafe_keys = Vec::new();
            let mut authorization_allowed = true;
            for record in output.stdout.split(|byte| *byte == b'\0') {
                if record.is_empty() {
                    continue;
                }
                let separator = record
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .ok_or_else(|| {
                        AppError::new(
                            "unsafe_git_config",
                            "仓库 Git 配置格式无效，拒绝执行网络操作。",
                        )
                    })?;
                let key = std::str::from_utf8(&record[..separator]).map_err(|_| {
                    AppError::new(
                        "unsafe_git_config",
                        "仓库 Git 配置包含无法安全确认的键，拒绝执行网络操作。",
                    )
                })?;
                if is_unsafe_network_config_key(key) {
                    let classification_key = key.trim().to_ascii_lowercase();
                    authorization_allowed &=
                        !is_unapprovable_network_config_key(&classification_key);
                    unsafe_keys.push(safe_unsafe_network_config_key_label(&classification_key));
                    unsafe_entries.push((
                        canonicalize_git_config_key_for_fingerprint(key)?,
                        record[separator + 1..].to_vec(),
                    ));
                }
            }
            unsafe_keys.sort_unstable();
            unsafe_keys.dedup();
            let fingerprint = if unsafe_entries.is_empty() {
                None
            } else {
                let mut digest = Sha256::new();
                digest.update(b"broknowmygitdesktop:network-config:v1\0");
                for (key, value) in &unsafe_entries {
                    digest.update((key.len() as u64).to_be_bytes());
                    digest.update(key.as_bytes());
                    digest.update((value.len() as u64).to_be_bytes());
                    digest.update(value);
                }
                Some(format!("{:x}", digest.finalize()))
            };
            Ok(NetworkConfigurationAssessment {
                unsafe_keys,
                fingerprint,
                authorization_allowed,
            })
        }
    }

    /// Rejects executable or credential-affecting repository configuration
    /// unless the application has approved this exact current fingerprint.
    pub fn ensure_network_configuration_is_safe(&self, repository: &Path) -> Result<(), AppError> {
        let repository_root = self.repository_root(repository)?;
        let assessment = self.network_configuration_assessment(&repository_root)?;
        if assessment.is_safe() {
            return Ok(());
        }
        if !assessment.authorization_allowed {
            return Err(AppError::with_safe_details(
                "unsafe_git_config_unapprovable",
                "该仓库的本地 Git 配置无法安全授权。",
                "仓库使用了 include.path 或 includeIf.*.path。外部包含文件可在检查后变化，因此网络操作始终拒绝这类仓库级配置。",
            ));
        }
        if self
            .trusted_network_configurations
            .get(&repository_root)
            .is_some_and(|fingerprint| assessment.fingerprint.as_deref() == Some(fingerprint))
        {
            return Ok(());
        }
        Err(AppError::with_safe_details(
            "unsafe_git_config",
            "应用的网络安全检查拒绝了该仓库的本地 Git 配置。",
            "仓库包含能够执行命令或改变网络认证边界的 Git 配置。仅在确认来源可信后，允许当前配置指纹并重试。",
        ))
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

fn canonicalize_git_config_key_for_fingerprint(key: &str) -> Result<String, AppError> {
    let first_separator = key.find('.').ok_or_else(|| {
        AppError::new(
            "unsafe_git_config",
            "仓库 Git 配置包含无法安全确认的键，拒绝执行网络操作。",
        )
    })?;
    let last_separator = key.rfind('.').unwrap_or(first_separator);
    if first_separator == 0 || last_separator + 1 >= key.len() {
        return Err(AppError::new(
            "unsafe_git_config",
            "仓库 Git 配置包含无法安全确认的键，拒绝执行网络操作。",
        ));
    }
    let section = key[..first_separator].to_ascii_lowercase();
    let variable = key[last_separator + 1..].to_ascii_lowercase();
    if first_separator == last_separator {
        Ok(format!("{section}.{variable}"))
    } else {
        Ok(format!(
            "{section}.{}.{variable}",
            &key[first_separator + 1..last_separator]
        ))
    }
}

fn is_unapprovable_network_config_key(key: &str) -> bool {
    key == "include.path" || key.starts_with("includeif.") && key.ends_with(".path")
}

fn safe_unsafe_network_config_key_label(key: &str) -> String {
    if key.starts_with("credential.") && key != "credential.helper" {
        return "credential.*".into();
    }
    if key.starts_with("http.") || key.starts_with("remote.") {
        let mut components = key.split('.');
        let section = components.next().unwrap_or_default();
        let remainder = components.collect::<Vec<_>>();
        if remainder.len() == 1 {
            return key.to_owned();
        }
        let suffix = remainder.last().copied().unwrap_or("*");
        return format!("{section}.*.{suffix}");
    }
    if key.starts_with("submodule.") {
        return "submodule.*.update".into();
    }
    if key.starts_with("filter.") {
        let suffix = key.rsplit('.').next().unwrap_or("*");
        return format!("filter.*.{suffix}");
    }
    if key.starts_with("protocol.") {
        return "protocol.*".into();
    }
    if key.starts_with("merge.") {
        return "merge.*.driver".into();
    }
    if key.starts_with("url.") {
        let suffix = key.rsplit('.').next().unwrap_or("*");
        return format!("url.*.{suffix}");
    }
    if key.starts_with("includeif.") {
        return "includeIf.*.path".into();
    }
    key.to_owned()
}

fn parse_trusted_credential_configuration(raw: &[u8]) -> Result<Vec<(String, String)>, AppError> {
    let mut configuration = Vec::new();
    for record in raw
        .split(|byte| *byte == b'\0')
        .filter(|record| !record.is_empty())
    {
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err(AppError::new(
                "unsafe_git_config",
                "系统凭据配置格式无效，拒绝执行网络操作。",
            ));
        };
        let key = std::str::from_utf8(&record[..separator]).map_err(|_| {
            AppError::new(
                "unsafe_git_config",
                "系统凭据配置格式无效，拒绝执行网络操作。",
            )
        })?;
        let value = std::str::from_utf8(&record[separator + 1..]).map_err(|_| {
            AppError::new(
                "unsafe_git_config",
                "系统凭据配置格式无效，拒绝执行网络操作。",
            )
        })?;
        if !key.to_ascii_lowercase().starts_with("credential.") || key.contains(['\0', '\r', '\n'])
        {
            return Err(AppError::new(
                "unsafe_git_config",
                "系统凭据配置格式无效，拒绝执行网络操作。",
            ));
        }
        configuration.push((key.to_owned(), value.to_owned()));
    }
    Ok(configuration)
}

#[cfg(test)]
mod network_configuration_label_tests {
    use super::safe_unsafe_network_config_key_label;

    #[test]
    fn display_labels_hide_url_scopes_and_user_defined_subsections() {
        assert_eq!(
            safe_unsafe_network_config_key_label(
                "credential.https://token@example.test/repository.helper"
            ),
            "credential.*"
        );
        assert_eq!(
            safe_unsafe_network_config_key_label(
                "url.https://token@example.test/repository.insteadof"
            ),
            "url.*.insteadof"
        );
        assert_eq!(
            safe_unsafe_network_config_key_label("merge.customer-secret.driver"),
            "merge.*.driver"
        );
    }
}
