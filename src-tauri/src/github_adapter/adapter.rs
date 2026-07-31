impl GitHubAdapter {
    fn repository_slug(context: &AdapterContext) -> Result<String, AppError> {
        let parsed = &context.remote().parsed;
        let owner = parsed
            .namespace
            .as_deref()
            .filter(|value| is_github_route_segment(value));
        let repository = parsed
            .repository
            .as_deref()
            .filter(|value| is_github_route_segment(value));
        match (owner, repository) {
            (Some(owner), Some(repository)) => Ok(format!("{owner}/{repository}")),
            _ => Err(AppError::new(
                "repository_slug_unavailable",
                "该 remote URL 不包含可识别的 GitHub 仓库路径。",
            )),
        }
    }

    fn gh_api(&self, endpoint: &CodeHostEndpoint, path: &str) -> Result<String, AppError> {
        self.gh_api_until(endpoint, path, Instant::now() + GH_COMMAND_TIMEOUT)
    }

    /// Executes one GitHub API request within a caller-owned deadline. Pull
    /// request pagination shares one deadline across every page so a single
    /// desktop command cannot multiply the network timeout by its page cap.
    fn gh_api_until(
        &self,
        endpoint: &CodeHostEndpoint,
        path: &str,
        deadline: Instant,
    ) -> Result<String, AppError> {
        self.gh_api_until_with_identity(endpoint, path, deadline, None)
    }

    fn gh_api_until_with_identity(
        &self,
        endpoint: &CodeHostEndpoint,
        path: &str,
        deadline: Instant,
        identity: Option<&PinnedGhIdentity>,
    ) -> Result<String, AppError> {
        let hostname = gh_hostname(endpoint)?;
        let mut command = Command::new("gh");
        command.args(["api", path, "--hostname", hostname.as_str()]);
        if let Some(identity) = identity {
            apply_pinned_gh_identity(&mut command, endpoint, identity)?;
        }
        let output = run_gh_with_timeout(command, None, deadline)?;

        if !output.status.success() {
            return Err(AppError::with_safe_details(
                "github_api_failed",
                "GitHub 请求失败，请检查 gh 登录状态和仓库访问权限。",
                safe_command_error(&output.stderr),
            ));
        }

        String::from_utf8(output.stdout).map_err(|_| {
            AppError::new(
                "github_api_invalid_response",
                "GitHub 返回了无法读取的数据。",
            )
        })
    }

    /// Sends JSON on stdin so user-authored title/body strings never become
    /// shell text, URL fragments, or CLI field syntax.
    fn gh_api_post_json<T: Serialize>(
        &self,
        endpoint: &CodeHostEndpoint,
        path: &str,
        payload: &T,
    ) -> Result<String, AppError> {
        self.gh_api_post_json_until(endpoint, path, payload, Instant::now() + GH_COMMAND_TIMEOUT)
    }

    fn gh_api_post_json_until<T: Serialize>(
        &self,
        endpoint: &CodeHostEndpoint,
        path: &str,
        payload: &T,
        deadline: Instant,
    ) -> Result<String, AppError> {
        self.gh_api_post_json_until_with_identity(endpoint, path, payload, deadline, None)
    }

    fn gh_api_post_json_until_with_identity<T: Serialize>(
        &self,
        endpoint: &CodeHostEndpoint,
        path: &str,
        payload: &T,
        deadline: Instant,
        identity: Option<&PinnedGhIdentity>,
    ) -> Result<String, AppError> {
        let serialized = serde_json::to_vec(payload)
            .map_err(|_| AppError::new("github_api_invalid_request", "无法准备 GitHub 请求。"))?;
        let hostname = gh_hostname(endpoint)?;
        let mut command = Command::new("gh");
        command.args([
            "api",
            path,
            "--hostname",
            hostname.as_str(),
            "--method",
            "POST",
            "--input",
            "-",
        ]);
        if let Some(identity) = identity {
            apply_pinned_gh_identity(&mut command, endpoint, identity)?;
        }
        let output = run_gh_with_timeout(command, Some(&serialized), deadline)?;
        if !output.status.success() {
            return Err(AppError::with_safe_details(
                "github_api_failed",
                "GitHub 请求失败，请检查 gh 登录状态和仓库访问权限。",
                safe_command_error(&output.stderr),
            ));
        }
        String::from_utf8(output.stdout).map_err(|_| {
            AppError::new(
                "github_api_invalid_response",
                "GitHub 返回了无法读取的数据。",
            )
        })
    }

    pub(crate) fn operation_deadline() -> Instant {
        Instant::now() + GH_COMMAND_TIMEOUT
    }

    pub(crate) fn list_publish_owners_until(
        &self,
        endpoint: &CodeHostEndpoint,
        expected_login: &str,
        deadline: Instant,
    ) -> Result<Vec<GitHubPublishOwner>, AppError> {
        let identity = pin_gh_identity_until(endpoint, expected_login, deadline)?;
        self.list_publish_owners_with_identity_until(endpoint, expected_login, &identity, deadline)
    }

    fn list_publish_owners_with_identity_until(
        &self,
        endpoint: &CodeHostEndpoint,
        expected_login: &str,
        identity: &PinnedGhIdentity,
        deadline: Instant,
    ) -> Result<Vec<GitHubPublishOwner>, AppError> {
        if endpoint.kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "GitHub adapter 不适用于该 Endpoint。",
            ));
        }
        let mut owners = Vec::new();
        let mut cursor = None;
        for _ in 0..MAX_PUBLISH_OWNER_PAGES {
            let response = self.gh_api_post_json_until_with_identity(
                endpoint,
                "graphql",
                &GitHubPublishOwnersRequest {
                    query: GITHUB_PUBLISH_OWNERS_QUERY,
                    variables: GitHubPublishOwnersVariables {
                        cursor: cursor.as_deref(),
                    },
                },
                deadline,
                Some(identity),
            )?;
            let page = parse_publish_owners_page(&response, expected_login)?;
            if owners.is_empty() {
                owners.push(GitHubPublishOwner {
                    login: page.viewer_login.clone(),
                    name: page.viewer_name,
                    kind: GitHubPublishOwnerKind::User,
                    can_create_repositories: true,
                });
            }
            owners.extend(
                page.organizations
                    .into_iter()
                    .map(|organization| GitHubPublishOwner {
                        login: organization.login,
                        name: organization.name,
                        kind: GitHubPublishOwnerKind::Organization,
                        can_create_repositories: organization.viewer_can_create_repositories,
                    }),
            );
            if !page.has_next_page {
                owners[1..].sort_by(|left, right| {
                    left.login
                        .to_ascii_lowercase()
                        .cmp(&right.login.to_ascii_lowercase())
                });
                return Ok(owners);
            }
            let next_cursor = page.end_cursor.ok_or_else(|| {
                AppError::new(
                    "github_api_invalid_response",
                    "GitHub 组织列表缺少分页游标。",
                )
            })?;
            if cursor.as_deref() == Some(next_cursor.as_str()) {
                return Err(AppError::new(
                    "github_api_invalid_response",
                    "GitHub 组织列表分页游标没有推进。",
                ));
            }
            cursor = Some(next_cursor);
        }
        Err(AppError::new(
            "github_publish_owners_page_limit",
            "GitHub 组织数量超过当前安全读取上限。",
        ))
    }

    pub(crate) fn validate_publish_owner_until(
        &self,
        endpoint: &CodeHostEndpoint,
        expected_login: &str,
        requested_owner: &str,
        deadline: Instant,
    ) -> Result<ValidatedGitHubPublishOwner, AppError> {
        validate_repository_segment(requested_owner)?;
        let identity = pin_gh_identity_until(endpoint, expected_login, deadline)?;
        let login = if requested_owner.eq_ignore_ascii_case(expected_login) {
            let response =
                self.gh_api_until_with_identity(endpoint, "user", deadline, Some(&identity))?;
            let viewer: GitHubUser = serde_json::from_str(&response).map_err(|_| {
                AppError::new(
                    "github_api_invalid_response",
                    "无法解析当前 GitHub 账号信息。",
                )
            })?;
            if !viewer.login.eq_ignore_ascii_case(expected_login) {
                return Err(AppError::new(
                    "cli_auth_unavailable",
                    "当前 gh 登录与所选发布账号不匹配或已失效。",
                ));
            }
            viewer.login
        } else {
            self.list_publish_owners_with_identity_until(
                endpoint,
                expected_login,
                &identity,
                deadline,
            )?
            .into_iter()
            .find(|owner| {
                owner.kind == GitHubPublishOwnerKind::Organization
                    && owner.login.eq_ignore_ascii_case(requested_owner)
                    && owner.can_create_repositories
            })
            .map(|owner| owner.login)
            .ok_or_else(|| {
                AppError::new(
                    "github_organization_publish_forbidden",
                    "所选组织不允许当前账号创建仓库，请检查组织成员权限和 GitHub 授权。",
                )
            })?
        };
        Ok(ValidatedGitHubPublishOwner {
            endpoint_id: endpoint.id.clone(),
            login,
            identity,
        })
    }

    /// Creates a GitHub repository from an existing local worktree, configures
    /// origin, and pushes through the currently active matching gh identity.
    /// The caller validates account identity and confirms the external write.
    pub(crate) fn publish_repository(
        &self,
        endpoint: &CodeHostEndpoint,
        repository: &Path,
        name: &str,
        owner: ValidatedGitHubPublishOwner,
        private: bool,
        deadline: Instant,
    ) -> Result<(), AppError> {
        if endpoint.kind != CodeHostKind::GitHub || endpoint.id != owner.endpoint_id {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "GitHub adapter 不适用于该 Endpoint。",
            ));
        }
        validate_repository_segment(name)?;
        let repository_slug = format!("{}/{}", owner.login, name.trim());
        let mut command = Command::new("gh");
        command
            .arg("repo")
            .arg("create")
            .arg(repository_slug)
            .arg("--hostname")
            .arg(owner.identity.hostname.as_str())
            .arg("--source")
            .arg(repository)
            .arg("--remote")
            .arg("origin")
            .arg("--push")
            .arg(if private { "--private" } else { "--public" });
        apply_pinned_gh_identity(&mut command, endpoint, &owner.identity)?;
        isolate_pinned_publish_git_environment(&mut command);
        let output = run_gh_with_timeout(command, None, deadline)?;
        if !output.status.success() {
            return Err(github_publish_command_error(&output.stderr));
        }
        Ok(())
    }
}

fn pin_gh_identity_until(
    endpoint: &CodeHostEndpoint,
    expected_login: &str,
    deadline: Instant,
) -> Result<PinnedGhIdentity, AppError> {
    validate_repository_segment(expected_login)?;
    let hostname = gh_hostname(endpoint)?;
    let mut command = Command::new("gh");
    command.args([
        "auth",
        "token",
        "--hostname",
        hostname.as_str(),
        "--user",
        expected_login,
    ]);
    remove_inherited_gh_token_environment(&mut command);
    let output = run_gh_with_timeout(command, None, deadline)?;
    if !output.status.success() {
        return Err(AppError::with_safe_details(
            "cli_auth_unavailable",
            "无法固定所选 gh 账号的认证身份。",
            safe_command_error(&output.stderr),
        ));
    }
    let token =
        Zeroizing::new(String::from_utf8(output.stdout).map_err(|_| {
            AppError::new("cli_token_unavailable", "gh 返回了无法读取的认证信息。")
        })?);
    let token = token.trim();
    if token.is_empty()
        || token.len() > 16 * 1024
        || token
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(AppError::new(
            "cli_token_unavailable",
            "gh 没有返回可用的认证信息。",
        ));
    }
    Ok(PinnedGhIdentity {
        hostname,
        token: Zeroizing::new(token.to_owned()),
        config_directory: PinnedGhConfigDirectory::create()?,
    })
}

fn apply_pinned_gh_identity(
    command: &mut Command,
    endpoint: &CodeHostEndpoint,
    identity: &PinnedGhIdentity,
) -> Result<(), AppError> {
    let hostname = gh_hostname(endpoint)?;
    if hostname != identity.hostname {
        return Err(AppError::new(
            "adapter_context_mismatch",
            "固定的 gh 身份与 GitHub Endpoint 不匹配。",
        ));
    }
    remove_inherited_gh_token_environment(command);
    if hostname.eq_ignore_ascii_case("github.com") {
        command.env("GH_TOKEN", identity.token.as_str());
    } else {
        command.env("GH_ENTERPRISE_TOKEN", identity.token.as_str());
    }
    command
        .env("GH_CONFIG_DIR", &identity.config_directory.path)
        .env("GH_PROMPT_DISABLED", "1");
    Ok(())
}

fn remove_inherited_gh_token_environment(command: &mut Command) {
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(name);
    }
}

#[cfg(windows)]
const DISABLED_PUBLISH_GIT_PATH: &str = "NUL";
#[cfg(not(windows))]
const DISABLED_PUBLISH_GIT_PATH: &str = "/dev/null";

/// A pinned gh bearer token is inherited by gh's Git child. Prevent hooks,
/// fsmonitor, custom transports, and repository credential helpers from
/// receiving or redirecting that authorization during the initial push.
fn isolate_pinned_publish_git_environment(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_CONFIG_")
        {
            command.env_remove(name);
        }
    }
    for name in [
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_EXEC_PATH",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_GRAFT_FILE",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
    ] {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("BKM_GIT_ASKPASS_")
        {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", DISABLED_PUBLISH_GIT_PATH)
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_TERMINAL_PROMPT", "0");
    // `gh repo create --push` invokes Git through gh's AuthenticatedCommand,
    // which adds a command-level empty helper followed by `gh auth
    // git-credential`. The pinned GH_TOKEN is therefore the only credential
    // source; do not add another environment-level helper here.
    let configuration = [
        ("http.extraheader", ""),
        ("http.proxy", ""),
        ("http.sslverify", "true"),
        ("http.cookiefile", ""),
        ("http.savecookies", "false"),
        ("core.hooksPath", DISABLED_PUBLISH_GIT_PATH),
        ("core.fsmonitor", "false"),
        ("core.gitProxy", ""),
        ("core.attributesFile", DISABLED_PUBLISH_GIT_PATH),
        ("protocol.allow", "never"),
        ("protocol.https.allow", "always"),
        ("protocol.ext.allow", "never"),
    ];
    command.env("GIT_CONFIG_COUNT", configuration.len().to_string());
    for (index, (key, value)) in configuration.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
}

const GITHUB_PUBLISH_OWNERS_QUERY: &str = r#"
query PublishOwners($cursor: String) {
  viewer {
    login
    name
    organizations(first: 100, after: $cursor) {
      nodes {
        login
        name
        viewerCanCreateRepositories
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#;

