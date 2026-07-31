impl CodeHostAdapter for GitHubAdapter {
    fn kind(&self) -> CodeHostKind {
        CodeHostKind::GitHub
    }

    fn capabilities(&self, endpoint: &CodeHostEndpoint) -> CodeHostCapabilities {
        CodeHostCapabilities::for_endpoint(endpoint)
    }

    fn repository_info(
        &self,
        context: &AdapterContext,
    ) -> Result<CodeHostRepositoryInfo, AppError> {
        if context.endpoint().kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "GitHub adapter 不适用于该 Endpoint。",
            ));
        }
        let slug = Self::repository_slug(context)?;
        let response = self.gh_api(context.endpoint(), &format!("repos/{slug}"))?;
        let repository: GitHubRepositoryResponse =
            serde_json::from_str(&response).map_err(|_| {
                AppError::new("github_api_invalid_response", "无法解析 GitHub 仓库信息。")
            })?;
        Ok(CodeHostRepositoryInfo {
            full_name: repository.full_name,
            description: repository.description,
            default_branch: repository.default_branch,
            web_url: repository.html_url,
            is_private: Some(repository.private),
        })
    }

    fn list_pull_requests(
        &self,
        context: &AdapterContext,
    ) -> Result<Vec<PullRequestSummary>, AppError> {
        if context.endpoint().kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "GitHub adapter 不适用于该 Endpoint。",
            ));
        }
        let slug = Self::repository_slug(context)?;
        let mut summaries = Vec::new();
        let deadline = Instant::now() + GH_COMMAND_TIMEOUT;
        for page in 1..=MAX_PULL_REQUEST_PAGES {
            let response = self.gh_api_until(
                context.endpoint(),
                &format!("repos/{slug}/pulls?state=open&per_page=100&page={page}"),
                deadline,
            )?;
            let pull_requests: Vec<GitHubPullRequestResponse> = serde_json::from_str(&response)
                .map_err(|_| {
                    AppError::new(
                        "github_api_invalid_response",
                        "无法解析 GitHub Pull Request 列表。",
                    )
                })?;
            let page_is_final = pull_requests.len() < 100;
            summaries.extend(
                pull_requests
                    .into_iter()
                    .map(|pull_request| PullRequestSummary {
                        number: pull_request.number,
                        title: pull_request.title,
                        author: pull_request.user.login,
                        state: pull_request.state,
                        web_url: pull_request.html_url,
                        updated_at: pull_request.updated_at,
                    }),
            );
            if page_is_final {
                return Ok(summaries);
            }
        }
        // A full final page can still be the complete result. Probe one extra
        // entry before reporting a limit instead of rejecting exactly 1,000
        // open pull requests as if the list were known to be truncated.
        let response = self.gh_api_until(
            context.endpoint(),
            &format!(
                "repos/{slug}/pulls?state=open&per_page=100&page={}",
                MAX_PULL_REQUEST_PAGES + 1
            ),
            deadline,
        )?;
        let extra: Vec<GitHubPullRequestResponse> =
            serde_json::from_str(&response).map_err(|_| {
                AppError::new(
                    "github_api_invalid_response",
                    "无法解析 GitHub Pull Request 列表。",
                )
            })?;
        if extra.is_empty() {
            return Ok(summaries);
        }
        Err(AppError::new(
            "github_api_page_limit",
            "开放 Pull Request 数量超过当前安全读取上限，请在 GitHub 中缩小范围后重试。",
        ))
    }

    fn create_pull_request(
        &self,
        context: &AdapterContext,
        input: &CreatePullRequestInput,
    ) -> Result<PullRequestSummary, AppError> {
        if context.endpoint().kind != CodeHostKind::GitHub {
            return Err(AppError::new(
                "adapter_endpoint_mismatch",
                "GitHub adapter 不适用于该 Endpoint。",
            ));
        }
        validate_pull_request_input(input)?;
        let slug = Self::repository_slug(context)?;
        let response = self.gh_api_post_json(
            context.endpoint(),
            &format!("repos/{slug}/pulls"),
            &GitHubCreatePullRequestRequest {
                title: input.title.trim(),
                body: &input.body,
                head: input.head.trim(),
                base: input.base.trim(),
                draft: input.draft,
            },
        )?;
        let pull_request: GitHubPullRequestResponse =
            serde_json::from_str(&response).map_err(|_| {
                AppError::new(
                    "github_api_invalid_response",
                    "无法解析新建的 GitHub Pull Request。",
                )
            })?;
        Ok(PullRequestSummary {
            number: pull_request.number,
            title: pull_request.title,
            author: pull_request.user.login,
            state: pull_request.state,
            web_url: pull_request.html_url,
            updated_at: pull_request.updated_at,
        })
    }
}

/// GitHub REST routes use exactly one owner and one repository segment. The
/// generic Git remote parser intentionally supports nested namespaces, so API
/// callers must narrow that data before interpolating it into a route.
fn is_github_route_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !matches!(value, "." | "..")
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

#[derive(Debug, Serialize)]
struct GitHubCreatePullRequestRequest<'a> {
    title: &'a str,
    body: &'a str,
    head: &'a str,
    base: &'a str,
    draft: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubRepositoryResponse {
    full_name: String,
    description: Option<String>,
    default_branch: Option<String>,
    html_url: String,
    private: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestResponse {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    updated_at: Option<String>,
    user: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

fn validate_pull_request_input(input: &CreatePullRequestInput) -> Result<(), AppError> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 256 || title.contains(['\0', '\r', '\n']) {
        return Err(AppError::new(
            "invalid_pull_request_title",
            "Pull Request 标题不能为空，且不能包含换行。",
        ));
    }
    if input.body.len() > 65_536 || input.body.contains('\0') {
        return Err(AppError::new(
            "invalid_pull_request_body",
            "Pull Request 描述过长或包含无效字符。",
        ));
    }
    for reference in [&input.head, &input.base] {
        let reference = reference.trim();
        if reference.is_empty()
            || reference.starts_with('-')
            || reference.len() > 512
            || reference.contains(['\0', '\r', '\n'])
        {
            return Err(AppError::new(
                "invalid_pull_request_branch",
                "Pull Request 的源分支和目标分支必须是有效引用。",
            ));
        }
    }
    if input.head.trim() == input.base.trim() {
        return Err(AppError::new(
            "invalid_pull_request_branch",
            "Pull Request 的源分支和目标分支不能相同。",
        ));
    }
    Ok(())
}

fn validate_repository_name(value: &str) -> Result<(), AppError> {
    let value = value.trim();
    let segments: Vec<_> = value.split('/').collect();
    if value.is_empty()
        || segments.len() > 2
        || segments.iter().any(|segment| {
            segment.is_empty()
                || segment.len() > 100
                || segment.starts_with('-')
                || matches!(*segment, "." | "..")
                || !segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                })
        })
    {
        return Err(AppError::new(
            "invalid_repository_name",
            "GitHub 仓库名称只能包含字母、数字、点、下划线、连字符和可选的 owner/ 前缀。",
        ));
    }
    Ok(())
}

fn validate_repository_segment(value: &str) -> Result<(), AppError> {
    if value.trim().contains('/') {
        return Err(AppError::new(
            "invalid_repository_name",
            "GitHub 仓库所有者和名称必须分别填写。",
        ));
    }
    validate_repository_name(value)
}

fn safe_command_error(stderr: &[u8]) -> &'static str {
    // Never forward a CLI diagnostic verbatim: HTTP tooling can echo response
    // bodies or credential-looking URLs. Keep only a small, token-free
    // classification that is useful to the user.
    let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if diagnostic.contains("not logged") || diagnostic.contains("authentication") {
        "gh 未登录或当前账号已失效。"
    } else if diagnostic.contains("404") || diagnostic.contains("not found") {
        "找不到仓库，或当前账号没有访问权限。"
    } else if diagnostic.contains("rate limit") {
        "GitHub API 速率受限，请稍后重试。"
    } else {
        "GitHub CLI 请求未完成，请检查 gh 登录状态和网络连接。"
    }
}

fn github_publish_command_error(stderr: &[u8]) -> AppError {
    let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if diagnostic.contains("already exists") {
        AppError::new(
            "github_repository_exists",
            "所选所有者下已存在同名 GitHub 仓库。",
        )
    } else if diagnostic.contains("saml")
        || diagnostic.contains("single sign-on")
        || diagnostic.contains("sso")
    {
        AppError::new(
            "github_sso_required",
            "组织要求完成 SSO 授权后才能创建仓库，请先在 GitHub 或 gh 中授权当前身份。",
        )
    } else if diagnostic.contains("resource not accessible")
        || diagnostic.contains("not permitted")
        || diagnostic.contains("not allowed")
        || diagnostic.contains("permission")
        || diagnostic.contains("organization owner")
    {
        AppError::new(
            "github_organization_publish_forbidden",
            "所选所有者不允许当前账号创建该仓库，请检查组织成员权限、仓库可见性和 GitHub 应用授权。",
        )
    } else {
        AppError::with_safe_details(
            "github_publish_failed",
            "GitHub 仓库发布失败，请检查 gh 登录、仓库名称和网络连接。",
            safe_command_error(stderr),
        )
    }
}

