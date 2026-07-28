use crate::app_error::AppError;
use crate::codehost::{
    AdapterContext, CodeHostAdapter, CodeHostRepositoryInfo, CreatePullRequestInput,
    GitHubPublishOwner, GitHubPublishOwnerKind, PullRequestSummary,
};
use crate::domain::{
    git_authority_has_explicit_port, github_endpoint_has_gh_compatible_api,
    hostname_from_git_authority, CodeHostCapabilities, CodeHostEndpoint, CodeHostKind,
};
#[cfg(windows)]
use crate::process_tree::terminate_process_tree;
use crate::process_tree::{attach_process_tree, ProcessTreeHandle};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const GH_COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const GH_PIPE_COLLECTION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_GH_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_GH_DIAGNOSTIC_BYTES: usize = 512 * 1024;
const MAX_PULL_REQUEST_PAGES: usize = 10;
const MAX_PUBLISH_OWNER_PAGES: usize = 10;
const MAX_GH_PIPE_OPERATIONS: usize = 4;
const GH_DETACHED_PIPE_RELEASE_GRACE: Duration = Duration::from_secs(10);

static ACTIVE_GH_PIPE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

/// One permit remains held while a gh operation's reader/writer workers drain
/// their pipes. An escaped descendant can retain a pipe after its parent is
/// killed, so abandoned workers eventually release their concurrency slot.
struct GhPipeOperationPermit {
    released: AtomicBool,
}

impl GhPipeOperationPermit {
    fn acquire() -> Result<Arc<Self>, AppError> {
        let mut active = ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire);
        loop {
            if active >= MAX_GH_PIPE_OPERATIONS {
                return Err(AppError::new(
                    "github_api_busy",
                    "GitHub 请求正在清理先前的网络任务，请稍后重试。",
                ));
            }
            match ACTIVE_GH_PIPE_OPERATIONS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Arc::new(Self {
                        released: AtomicBool::new(false),
                    }));
                }
                Err(observed) => active = observed,
            }
        }
    }

    /// Blocking reads and writes cannot be safely cancelled from another Rust
    /// thread. Keep backpressure while the process tree is being terminated,
    /// then free the slot so a stale descriptor cannot make GitHub actions
    /// permanently report `github_api_busy`.
    fn release_after_cleanup_grace(self: &Arc<Self>) {
        let permit = Arc::downgrade(self);
        thread::spawn(move || {
            thread::sleep(GH_DETACHED_PIPE_RELEASE_GRACE);
            if let Some(permit) = permit.upgrade() {
                permit.release();
            }
        });
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            ACTIVE_GH_PIPE_OPERATIONS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for GhPipeOperationPermit {
    fn drop(&mut self) {
        self.release();
    }
}

/// GitHub adapter backed by the user's existing gh authentication. It never
/// changes the active gh account or asks gh to persist Git configuration.
pub struct GitHubAdapter;

pub(crate) struct ValidatedGitHubPublishOwner {
    endpoint_id: String,
    login: String,
    identity: PinnedGhIdentity,
}

impl ValidatedGitHubPublishOwner {
    pub(crate) fn login(&self) -> &str {
        &self.login
    }
}

struct PinnedGhIdentity {
    hostname: String,
    token: Zeroizing<String>,
    config_directory: PinnedGhConfigDirectory,
}

struct PinnedGhConfigDirectory {
    path: PathBuf,
}

impl PinnedGhConfigDirectory {
    fn create() -> Result<Self, AppError> {
        let parent = std::env::temp_dir();
        for _ in 0..16 {
            let path = parent.join(format!("broknowmygitdesktop-gh-{}", uuid::Uuid::new_v4()));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AppError::io("创建临时 gh 配置目录", error)),
            }
        }
        Err(AppError::new("io_error", "无法创建临时 gh 配置目录。"))
    }
}

impl Drop for PinnedGhConfigDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

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

#[derive(Debug, Serialize)]
struct GitHubPublishOwnersRequest<'a> {
    query: &'static str,
    variables: GitHubPublishOwnersVariables<'a>,
}

#[derive(Debug, Serialize)]
struct GitHubPublishOwnersVariables<'a> {
    cursor: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersResponse {
    data: Option<GitHubPublishOwnersData>,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersData {
    viewer: GitHubPublishOwnersViewer,
}

#[derive(Debug, Deserialize)]
struct GitHubPublishOwnersViewer {
    login: String,
    name: Option<String>,
    organizations: GitHubPublishOrganizationsConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPublishOrganizationsConnection {
    nodes: Vec<GitHubPublishOrganization>,
    page_info: GitHubPageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPublishOrganization {
    login: String,
    name: Option<String>,
    viewer_can_create_repositories: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

struct GitHubPublishOwnersPage {
    viewer_login: String,
    viewer_name: Option<String>,
    organizations: Vec<GitHubPublishOrganization>,
    has_next_page: bool,
    end_cursor: Option<String>,
}

fn parse_publish_owners_page(
    response: &str,
    expected_login: &str,
) -> Result<GitHubPublishOwnersPage, AppError> {
    let response: GitHubPublishOwnersResponse = serde_json::from_str(response).map_err(|_| {
        AppError::new(
            "github_api_invalid_response",
            "无法解析 GitHub 发布所有者列表。",
        )
    })?;
    if !response.errors.is_empty() {
        return Err(AppError::new(
            "github_publish_owners_unavailable",
            "GitHub 无法读取组织发布权限，请检查 gh 的组织访问授权。",
        ));
    }
    let viewer = response
        .data
        .ok_or_else(|| {
            AppError::new(
                "github_api_invalid_response",
                "GitHub 发布所有者响应缺少账号信息。",
            )
        })?
        .viewer;
    if !viewer.login.eq_ignore_ascii_case(expected_login) {
        return Err(AppError::new(
            "cli_auth_unavailable",
            "当前 gh 登录与所选发布账号不匹配或已失效。",
        ));
    }
    Ok(GitHubPublishOwnersPage {
        viewer_login: viewer.login,
        viewer_name: viewer.name,
        organizations: viewer.organizations.nodes,
        has_next_page: viewer.organizations.page_info.has_next_page,
        end_cursor: viewer.organizations.page_info.end_cursor,
    })
}

/// `gh` and Git askpass identify an authenticated host by hostname, whereas
/// Endpoint routing needs the full Git authority. Keep the port out of this
/// CLI argument so it remains subject to the same hostname validation as the
/// credential broker.
fn gh_hostname(endpoint: &CodeHostEndpoint) -> Result<String, AppError> {
    if git_authority_has_explicit_port(&endpoint.git_host) {
        return Err(AppError::new(
            "github_ported_endpoint_unsupported",
            "GitHub 平台功能暂不支持带端口的 Endpoint；请使用 PAT、SSH 或系统 Git 凭据执行 Git 操作。",
        ));
    }
    if !github_endpoint_has_gh_compatible_api(endpoint) {
        return Err(AppError::new(
            "github_api_endpoint_unsupported",
            "GitHub Endpoint 的 API 地址与 gh 的 host 路由不兼容；请使用 PAT、SSH 或系统 Git 凭据执行 Git 操作。",
        ));
    }
    hostname_from_git_authority(&endpoint.git_host).ok_or_else(|| {
        AppError::new(
            "adapter_endpoint_mismatch",
            "GitHub Endpoint 的 Git host 无效。",
        )
    })
}

/// Run `gh` with bounded output and a hard deadline. Network and
/// credential-helper stalls must not leave a desktop command pending forever.
fn run_gh_with_timeout(
    mut command: Command,
    input: Option<&[u8]>,
    deadline: Instant,
) -> Result<Output, AppError> {
    if Instant::now() >= deadline {
        return Err(github_api_timeout());
    }
    let operation_permit = GhPipeOperationPermit::acquire()?;
    configure_gh_process_group(&mut command);
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new(
                "gh_unavailable",
                "未找到 gh；安装并登录 GitHub CLI 后可使用 GitHub 平台功能。",
            )
        } else {
            AppError::io("启动 gh", error)
        }
    })?;
    let process_id = child.id();
    let process_tree = match attach_process_tree(&child) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            terminate_gh_child(&mut child, process_id);
            return Err(AppError::io("配置 gh 子进程树", error));
        }
    };

    let stdin_writer = if let Some(input) = input {
        let Some(stdin) = child.stdin.take() else {
            terminate_gh_child(&mut child, process_id);
            return Err(AppError::new(
                "github_api_failed",
                "无法向 GitHub CLI 写入请求。",
            ));
        };
        let input = input.to_vec();
        Some(spawn_gh_stdin_writer(
            stdin,
            input,
            Arc::clone(&operation_permit),
        ))
    } else {
        None
    };

    let Some(stdout) = child.stdout.take() else {
        terminate_gh_child(&mut child, process_id);
        operation_permit.release_after_cleanup_grace();
        return Err(AppError::new(
            "github_api_failed",
            "无法读取 GitHub CLI 输出。",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_gh_child(&mut child, process_id);
        operation_permit.release_after_cleanup_grace();
        return Err(AppError::new(
            "github_api_failed",
            "无法读取 GitHub CLI 诊断。",
        ));
    };
    let stdout_reader =
        spawn_gh_pipe_reader(stdout, MAX_GH_RESPONSE_BYTES, Arc::clone(&operation_permit));
    let stderr_reader = spawn_gh_pipe_reader(
        stderr,
        MAX_GH_DIAGNOSTIC_BYTES,
        Arc::clone(&operation_permit),
    );

    let mut stdout_ready = None;
    let mut stderr_ready = None;
    let status = loop {
        if let Err(error) = poll_gh_pipe(&stdout_reader, &mut stdout_ready) {
            terminate_gh_child(&mut child, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
        if let Err(error) = poll_gh_pipe(&stderr_reader, &mut stderr_ready) {
            terminate_gh_child(&mut child, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                terminate_gh_child(&mut child, process_id);
                // Do not join helper threads here. A descendant process can
                // retain a pipe even after `gh` exits; detached readers
                // cannot keep this UI command pending.
                operation_permit.release_after_cleanup_grace();
                return Err(github_api_timeout());
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_gh_child(&mut child, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(AppError::io("等待 gh", error));
            }
        }
    };

    let stdout = match collect_gh_pipe(stdout_reader, stdout_ready, deadline) {
        Ok(stdout) => stdout,
        Err(error) => {
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    let stderr = match collect_gh_pipe(stderr_reader, stderr_ready, deadline) {
        Ok(stderr) => stderr,
        Err(error) => {
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
            return Err(error);
        }
    };
    if let Some(writer) = stdin_writer {
        if status.success() {
            if let Err(error) = collect_gh_stdin_writer(writer, deadline) {
                terminate_remaining_gh_process_tree(&process_tree, process_id);
                operation_permit.release_after_cleanup_grace();
                return Err(error);
            }
        } else {
            // A failed gh process can leave a descendant holding stdin even
            // after both output readers complete. Do not let it pin this
            // operation's permit forever.
            terminate_remaining_gh_process_tree(&process_tree, process_id);
            operation_permit.release_after_cleanup_grace();
        }
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn terminate_gh_child(child: &mut Child, process_id: u32) {
    terminate_gh_process_group(process_id);
    let _ = child.kill();
    let _ = child.wait();
}

/// The direct child has already exited, so its numeric PID must not be used
/// as a Windows tree identity. Unix process groups retain their kernel-backed
/// PGID while descendants exist; Windows uses the Job Object attached at
/// spawn time instead.
fn terminate_remaining_gh_process_tree(_process_tree: &ProcessTreeHandle, _process_id: u32) {
    #[cfg(unix)]
    terminate_gh_process_group(_process_id);
    #[cfg(windows)]
    terminate_process_tree(_process_tree);
}

#[derive(Debug)]
enum GhPipeError {
    TooLarge,
    Io(std::io::Error),
}

fn spawn_gh_pipe_reader(
    pipe: impl Read + Send + 'static,
    maximum_bytes: usize,
    operation_permit: Arc<GhPipeOperationPermit>,
) -> Receiver<Result<Vec<u8>, GhPipeError>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _permit = operation_permit;
        let _ = sender.send(read_gh_pipe(pipe, maximum_bytes));
    });
    receiver
}

fn spawn_gh_stdin_writer(
    mut stdin: std::process::ChildStdin,
    input: Vec<u8>,
    operation_permit: Arc<GhPipeOperationPermit>,
) -> Receiver<std::io::Result<()>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _permit = operation_permit;
        let _ = sender.send(stdin.write_all(&input));
    });
    receiver
}

fn read_gh_pipe(mut pipe: impl Read, maximum_bytes: usize) -> Result<Vec<u8>, GhPipeError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(GhPipeError::Io)?;
        if count == 0 {
            break;
        }
        let remaining = maximum_bytes.saturating_sub(bytes.len());
        if count > remaining {
            // Notify the parent immediately. Continuing to drain a hostile
            // pipe would turn this memory limit into a 45-second wait.
            return Err(GhPipeError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn poll_gh_pipe(
    reader: &Receiver<Result<Vec<u8>, GhPipeError>>,
    ready: &mut Option<Result<Vec<u8>, GhPipeError>>,
) -> Result<(), AppError> {
    if ready.is_some() {
        return Ok(());
    }
    match reader.try_recv() {
        Ok(Ok(bytes)) => {
            *ready = Some(Ok(bytes));
            Ok(())
        }
        Ok(Err(error)) => Err(gh_pipe_error(error)),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AppError::new(
            "github_api_failed",
            "GitHub CLI 输出任务未能完成。",
        )),
    }
}

fn collect_gh_pipe(
    reader: Receiver<Result<Vec<u8>, GhPipeError>>,
    ready: Option<Result<Vec<u8>, GhPipeError>>,
    deadline: Instant,
) -> Result<Vec<u8>, AppError> {
    let result = match ready {
        Some(result) => result,
        None => match reader.recv_timeout(remaining_gh_collection_timeout(deadline)?) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                return Err(AppError::new(
                    "github_api_pipe_timeout",
                    "GitHub CLI 输出未能及时关闭。",
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AppError::new(
                    "github_api_failed",
                    "GitHub CLI 输出任务未能完成。",
                ));
            }
        },
    };
    match result {
        Ok(bytes) => Ok(bytes),
        Err(error) => Err(gh_pipe_error(error)),
    }
}

fn gh_pipe_error(error: GhPipeError) -> AppError {
    match error {
        GhPipeError::TooLarge => AppError::new(
            "github_api_response_too_large",
            "GitHub CLI 返回的数据过大，已停止处理。",
        ),
        GhPipeError::Io(error) => AppError::io("读取 gh 输出", error),
    }
}

fn collect_gh_stdin_writer(
    writer: Receiver<std::io::Result<()>>,
    deadline: Instant,
) -> Result<(), AppError> {
    match writer.recv_timeout(remaining_gh_collection_timeout(deadline)?) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(AppError::new("github_api_failed", "GitHub 请求未能发送。")),
        Err(RecvTimeoutError::Timeout) => Err(AppError::new(
            "github_api_pipe_timeout",
            "GitHub CLI 请求输入未能及时关闭。",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
            "github_api_failed",
            "GitHub 请求发送任务未能完成。",
        )),
    }
}

fn remaining_gh_collection_timeout(deadline: Instant) -> Result<Duration, AppError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(github_api_timeout());
    }
    Ok(remaining.min(GH_PIPE_COLLECTION_TIMEOUT))
}

fn github_api_timeout() -> AppError {
    AppError::new(
        "github_api_timeout",
        "GitHub 请求超时，请检查网络和 gh 登录状态后重试。",
    )
}

#[cfg(unix)]
fn configure_gh_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Delegate the post-fork setup to the standard library rather than
    // invoking Rust code from a `pre_exec` closure in this multithreaded app.
    command.process_group(0);
}

#[cfg(windows)]
fn configure_gh_process_group(_command: &mut Command) {}

#[cfg(all(not(unix), not(windows)))]
fn configure_gh_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_gh_process_group(process_id: u32) {
    // SAFETY: a negative PID targets the process group created by
    // configure_gh_process_group; failing to find an already-exited group is
    // harmless and the direct child is also killed by the caller when present.
    unsafe {
        let _ = libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_gh_process_group(process_id: u32) {
    // `taskkill /T` is the Windows process-tree primitive available on every
    // supported desktop installation. Wait for it before the caller reaps
    // the direct child so `/T` can still locate and terminate descendants.
    let process_id = process_id.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", process_id.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_gh_process_group(_process_id: u32) {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::io::Cursor;

    #[test]
    fn gh_pipe_reader_reports_limit_without_draining_remaining_output() {
        let error = read_gh_pipe(Cursor::new(vec![b'x'; 16]), 8)
            .expect_err("oversized output must stop the reader immediately");
        assert!(matches!(error, GhPipeError::TooLarge));
    }

    #[test]
    fn command_error_never_forwards_cli_output() {
        assert_eq!(
            safe_command_error(b"remote: token=secret-value\nsecond line"),
            "GitHub CLI 请求未完成，请检查 gh 登录状态和网络连接。"
        );
    }

    #[test]
    fn shared_request_deadline_caps_pipe_collection_time() {
        let error = remaining_gh_collection_timeout(Instant::now() - Duration::from_millis(1))
            .expect_err("an expired operation deadline must not wait for another pipe timeout");
        assert_eq!(error.code(), "github_api_timeout");

        let wait = remaining_gh_collection_timeout(Instant::now() + Duration::from_secs(10))
            .expect("future deadline has a collection window");
        assert_eq!(wait, GH_PIPE_COLLECTION_TIMEOUT);
    }

    #[test]
    fn gh_pipe_operation_permit_can_release_before_detached_workers_finish() {
        let before = ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire);
        let permit = GhPipeOperationPermit::acquire().expect("a test permit is available");
        let worker = Arc::clone(&permit);
        assert_eq!(
            ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );

        drop(permit);
        assert_eq!(
            ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );
        worker.release();
        assert_eq!(ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire), before);
        drop(worker);
        assert_eq!(ACTIVE_GH_PIPE_OPERATIONS.load(Ordering::Acquire), before);
    }

    #[test]
    fn pull_request_fields_reject_control_characters_and_options() {
        let valid = CreatePullRequestInput {
            title: "Add worktree controls".into(),
            body: "Description".into(),
            head: "feature/worktrees".into(),
            base: "main".into(),
            draft: false,
        };
        assert!(validate_pull_request_input(&valid).is_ok());

        let mut invalid = valid.clone();
        invalid.head = "-unsafe".into();
        assert!(validate_pull_request_input(&invalid).is_err());
        invalid = valid.clone();
        invalid.title = "unsafe\nheader".into();
        assert!(validate_pull_request_input(&invalid).is_err());

        let mut localized = valid;
        localized.title = "你".repeat(256);
        assert!(validate_pull_request_input(&localized).is_ok());
        localized.title.push('你');
        assert!(validate_pull_request_input(&localized).is_err());
    }

    #[test]
    fn repository_name_validation_accepts_owner_slug_without_shell_metacharacters() {
        assert!(validate_repository_name("desktop-client").is_ok());
        assert!(validate_repository_name("octo-org/desktop-client").is_ok());
        assert!(validate_repository_name("bad name").is_err());
        assert!(validate_repository_name("--unsafe").is_err());
        assert!(validate_repository_name(".").is_err());
        assert!(validate_repository_name("one/two/three").is_err());
    }

    #[test]
    fn publish_owner_response_requires_the_expected_viewer_and_preserves_permissions() {
        let response = r#"{
          "data": {
            "viewer": {
              "login": "octo",
              "name": "Octo Cat",
              "organizations": {
                "nodes": [
                  {"login": "allowed", "name": "Allowed Org", "viewerCanCreateRepositories": true},
                  {"login": "restricted", "name": null, "viewerCanCreateRepositories": false}
                ],
                "pageInfo": {"hasNextPage": false, "endCursor": null}
              }
            }
          }
        }"#;
        let page = parse_publish_owners_page(response, "OCTO").unwrap();
        assert_eq!(page.viewer_login, "octo");
        assert_eq!(page.organizations.len(), 2);
        assert!(page.organizations[0].viewer_can_create_repositories);
        assert!(!page.organizations[1].viewer_can_create_repositories);
        assert!(parse_publish_owners_page(response, "another-user").is_err());
    }

    #[test]
    fn publish_error_classifies_organization_failures_without_forwarding_diagnostics() {
        assert_eq!(
            github_publish_command_error(b"GraphQL: Name already exists on this account").code(),
            "github_repository_exists"
        );
        assert_eq!(
            github_publish_command_error(b"Resource protected by organization SAML SSO").code(),
            "github_sso_required"
        );
        assert_eq!(
            github_publish_command_error(b"Resource not accessible by integration").code(),
            "github_organization_publish_forbidden"
        );
    }

    #[test]
    fn pinned_publish_uses_selected_token_and_disables_repository_hooks() {
        let endpoint = CodeHostEndpoint {
            id: "github.com".into(),
            kind: CodeHostKind::GitHub,
            display_name: "GitHub.com".into(),
            web_url: "https://github.com".into(),
            api_url: "https://api.github.com".into(),
            git_host: "github.com".into(),
            is_builtin: true,
        };
        let identity = PinnedGhIdentity {
            hostname: "github.com".into(),
            token: Zeroizing::new("test-token".into()),
            config_directory: PinnedGhConfigDirectory::create().unwrap(),
        };
        let mut command = Command::new("gh");
        apply_pinned_gh_identity(&mut command, &endpoint, &identity).unwrap();
        isolate_pinned_publish_git_environment(&mut command);
        let environment: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(OsStr::to_os_string)))
            .collect();
        assert_eq!(
            environment.get(OsStr::new("GH_TOKEN")),
            Some(&Some(OsString::from("test-token")))
        );
        assert_eq!(
            environment.get(OsStr::new("GIT_CONFIG_VALUE_5")),
            Some(&Some(OsString::from(DISABLED_PUBLISH_GIT_PATH)))
        );
        assert_eq!(environment.get(OsStr::new("GIT_DIR")), Some(&None));
    }
}
