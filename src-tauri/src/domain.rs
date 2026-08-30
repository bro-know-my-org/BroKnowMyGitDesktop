use serde::{Deserialize, Serialize};
use url::{Host, Url};

/// Code hosting products supported by the common adapter model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CodeHostKind {
    GitHub,
    GitLab,
    Gitee,
    Gitea,
    Forgejo,
    /// A Git host without a platform-specific API integration.  It still
    /// supports ordinary Git over SSH/HTTPS and explicit account bindings.
    GenericGit,
    Unknown,
}

impl CodeHostKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Gitee => "Gitee",
            Self::Gitea => "Gitea",
            Self::Forgejo => "Forgejo",
            Self::GenericGit => "通用 Git 服务",
            Self::Unknown => "未知平台",
        }
    }
}

/// A concrete platform installation, including a self-hosted installation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeHostEndpoint {
    pub id: String,
    pub kind: CodeHostKind,
    pub display_name: String,
    pub web_url: String,
    pub api_url: String,
    /// The Git remote authority: a lower-case hostname with an optional
    /// explicit port (for example `git.example.test:8443`).
    pub git_host: String,
    pub is_builtin: bool,
}

impl CodeHostEndpoint {
    pub fn builtin() -> Vec<Self> {
        vec![
            Self {
                id: "github.com".into(),
                kind: CodeHostKind::GitHub,
                display_name: "GitHub.com".into(),
                web_url: "https://github.com".into(),
                api_url: "https://api.github.com".into(),
                git_host: "github.com".into(),
                is_builtin: true,
            },
            Self {
                id: "gitlab.com".into(),
                kind: CodeHostKind::GitLab,
                display_name: "GitLab.com".into(),
                web_url: "https://gitlab.com".into(),
                api_url: "https://gitlab.com/api/v4".into(),
                git_host: "gitlab.com".into(),
                is_builtin: true,
            },
            Self {
                id: "gitee.com".into(),
                kind: CodeHostKind::Gitee,
                display_name: "Gitee.com".into(),
                web_url: "https://gitee.com".into(),
                api_url: "https://gitee.com/api/v5".into(),
                git_host: "gitee.com".into(),
                is_builtin: true,
            },
        ]
    }
}

/// Returns whether this GitHub Endpoint can safely use host-scoped `gh`
/// commands. `gh --hostname` ignores the configurable `api_url`, so exposing
/// platform actions is only sound when that URL is exactly the endpoint gh
/// will address for the configured Git host.
pub fn github_endpoint_has_gh_compatible_api(endpoint: &CodeHostEndpoint) -> bool {
    if endpoint.kind != CodeHostKind::GitHub || git_authority_has_explicit_port(&endpoint.git_host)
    {
        return false;
    }
    let Some(hostname) = hostname_from_git_authority(&endpoint.git_host) else {
        return false;
    };
    let Ok(api_url) = Url::parse(&endpoint.api_url) else {
        return false;
    };
    if api_url.scheme() != "https"
        || !api_url.username().is_empty()
        || api_url.password().is_some()
        || api_url.query().is_some()
        || api_url.fragment().is_some()
        || api_url.port_or_known_default() != Some(443)
    {
        return false;
    }

    let (expected_host, expected_paths): (&str, &[&str]) =
        if hostname.eq_ignore_ascii_case("github.com") {
            ("api.github.com", &["", "/"])
        } else {
            (hostname.as_str(), &["/api/v3", "/api/v3/"])
        };
    api_url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(expected_host))
        && expected_paths.contains(&api_url.path())
}

/// Each UI control must be backed by one of these explicitly advertised capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeHostCapabilities {
    pub repository_info: bool,
    pub repository_publish: bool,
    pub repository_fork: bool,
    pub pull_requests_read: bool,
    pub pull_requests_write: bool,
    pub account_import: bool,
    pub account_bindings: bool,
    pub pat_keychain: bool,
}

impl CodeHostCapabilities {
    pub fn unavailable() -> Self {
        Self {
            repository_info: false,
            repository_publish: false,
            repository_fork: false,
            pull_requests_read: false,
            pull_requests_write: false,
            account_import: false,
            account_bindings: true,
            pat_keychain: false,
        }
    }

    pub fn for_endpoint(endpoint: &CodeHostEndpoint) -> Self {
        match endpoint.kind {
            CodeHostKind::GitHub if git_authority_has_explicit_port(&endpoint.git_host) => Self {
                // gh identifies an installation by hostname and cannot be
                // safely directed to a port-scoped API authority from this
                // adapter. Git itself still supports PAT/system/SSH bindings.
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: false,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::GitHub if !github_endpoint_has_gh_compatible_api(endpoint) => Self {
                // `gh --hostname` does not consume Endpoint.api_url. Keep
                // normal Git credential support, but never send its active
                // token to an implicitly derived API destination.
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: false,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::GitHub => Self {
                repository_info: true,
                repository_publish: true,
                repository_fork: true,
                pull_requests_read: true,
                pull_requests_write: true,
                account_import: true,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::GitLab if git_authority_has_explicit_port(&endpoint.git_host) => Self {
                // glab has the same hostname-scoped account model. Avoid
                // promising CLI account import for a different port.
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: false,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::GitLab => Self {
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: true,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::Gitee => Self {
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: false,
                account_bindings: true,
                pat_keychain: true,
            },
            // These hosts deliberately expose no forge-specific API surface,
            // but remain first-class Git endpoints.  A user can bind an SSH
            // identity, existing Git credential helper, or an app-managed
            // HTTPS PAT to them.
            CodeHostKind::Gitea | CodeHostKind::Forgejo | CodeHostKind::GenericGit => Self {
                repository_info: false,
                repository_publish: false,
                repository_fork: false,
                pull_requests_read: false,
                pull_requests_write: false,
                account_import: false,
                account_bindings: true,
                pat_keychain: true,
            },
            CodeHostKind::Unknown => Self::unavailable(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthenticationMethod {
    SshAgent,
    SystemGitCredentialHelper,
    GhCli,
    GlabCli,
    PersonalAccessToken,
}

impl AuthenticationMethod {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::SshAgent => "SSH agent",
            Self::SystemGitCredentialHelper => "系统 Git credential helper",
            Self::GhCli => "GitHub CLI (gh)",
            Self::GlabCli => "GitLab CLI (glab)",
            Self::PersonalAccessToken => "个人访问令牌 (PAT)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthenticationStatus {
    Unknown,
    Available,
    Valid,
    NeedsReauthentication,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeychainReference {
    pub service: String,
    pub account: String,
}

/// Non-secret metadata for one usable identity on one endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub id: String,
    pub endpoint_id: String,
    pub login: String,
    pub display_name: Option<String>,
    pub auth_method: AuthenticationMethod,
    pub secret_ref: Option<KeychainReference>,
    pub status: AuthenticationStatus,
    pub is_default_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoteTransport {
    Https,
    Http,
    Ssh,
    /// The unauthenticated, read-only Git protocol (`git://`).
    Git,
    /// A local bare/worktree path or `file://` remote.
    Local,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRemoteUrl {
    pub transport: RemoteTransport,
    pub host: Option<String>,
    /// A port written explicitly in the remote URL.  This deliberately keeps
    /// `:443`, `:80`, and `:22`: endpoint identity is port-scoped, so an
    /// explicitly configured `host:443` must not collapse into `host`.
    #[serde(default)]
    pub port: Option<u16>,
    pub namespace: Option<String>,
    pub repository: Option<String>,
}

impl ParsedRemoteUrl {
    pub fn repository_slug(&self) -> Option<String> {
        match (&self.namespace, &self.repository) {
            (Some(namespace), Some(repository)) => Some(format!("{namespace}/{repository}")),
            _ => None,
        }
    }

    /// Host plus an explicitly written port, suitable for matching a
    /// configured Git endpoint authority. The hostname field intentionally
    /// remains portless for SSH/askpass host validation.
    pub fn authority(&self) -> Option<String> {
        if !matches!(
            &self.transport,
            RemoteTransport::Https
                | RemoteTransport::Http
                | RemoteTransport::Ssh
                | RemoteTransport::Git
        ) {
            return None;
        }
        let host = self.host.as_deref()?;
        Some(format_git_authority(host, self.port))
    }
}

/// Safe-to-display representation of a Git remote. URLs are redacted before being returned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProfile {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
    pub parsed: ParsedRemoteUrl,
    pub endpoint_id: Option<String>,
    pub code_host_kind: CodeHostKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", content = "accountProfileId", rename_all = "camelCase")]
pub enum AccountSelection {
    Auto,
    Profile(String),
}

/// Explicit per-repository remote identity selection. A binding wins over defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBinding {
    pub repository_path: String,
    pub remote_name: String,
    pub endpoint_id: String,
    pub account_selection: AccountSelection,
}

/// A resolved runtime strategy. Deliberately contains no secret value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CredentialStrategy {
    KeychainPatAskPass {
        account_profile_id: String,
        keychain_ref: KeychainReference,
        /// Canonical HTTPS Git authority, including a non-default port when
        /// one is part of the selected Endpoint. Askpass compares this exact
        /// authority before releasing a PAT.
        authority: String,
        username: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialResolution {
    pub endpoint_id: String,
    pub account_profile_id: String,
    pub strategy: CredentialStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusEntry {
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

/// Application-local external editor configuration. Arguments are stored as
/// separate values rather than as a shell command string, so repository file
/// paths never become shell syntax when conflict files are opened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalEditorConfig {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// A named IDE/editor available from repository context menus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryEditorConfig {
    pub name: String,
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// Application-local preferred terminal configuration. Arguments remain
/// separate values so launching a repository never requires a shell command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalConfig {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub head_commit: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub upstream_commit: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<GitStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

/// A branch advertised by one configured remote and stored locally as a
/// remote-tracking ref. `name` deliberately omits the remote prefix so the
/// caller can create the conventional local tracking branch with the same
/// name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranchInfo {
    pub remote_name: String,
    pub name: String,
}

/// A concise, safe-to-display entry in the repository history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// True when tag enumeration exceeded its safety limit. In that case
    /// `tags` may be incomplete and the UI must not present it as exhaustive.
    #[serde(default)]
    pub tags_truncated: bool,
    /// `Some(true)` when the configured upstream contains this commit,
    /// `Some(false)` when it exists only on the local branch, and `None`
    /// when the current branch has no configured upstream.
    #[serde(default)]
    pub pushed: Option<bool>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub summary: String,
}

/// The kind of change made to one path by a commit. Merge commits are
/// compared with their first parent so this status matches the patch shown by
/// the history UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommitFileStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
    Unmerged,
    Unknown,
}

/// One file changed by a commit. `previous_path` is populated for copies and
/// renames so a path-filtered diff can retain Git's similarity detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitChangedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: CommitFileStatus,
    pub similarity: Option<u8>,
}

/// A bounded preview of one commit-file patch. Large patches can be requested
/// again with a larger explicit limit after the user asks to reveal more.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitFileDiff {
    pub text: String,
    pub truncated: bool,
}

/// A local stash entry. The display reference can be renumbered by another
/// Git client, so the immutable stash commit ID accompanies destructive pop
/// and drop requests as an identity check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub reference: String,
    #[serde(default)]
    pub commit_id: String,
    pub message: String,
}

/// The three non-interactive reset modes offered by system Git.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

/// A Git operation that can pause while the user resolves conflicts.  The
/// frontend uses this explicit state to offer only the matching continue and
/// abort controls instead of guessing from error text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GitOperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// One main or linked worktree reported by `git worktree list --porcelain`.
/// Paths are safe to display but are always passed back to Git as separate
/// process arguments rather than shell text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    pub is_main: bool,
    pub is_detached: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

/// The state prefix reported by the Git submodule status command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubmoduleStatus {
    Initialized,
    Uninitialized,
    NeedsUpdate,
    Conflicted,
}

/// Safe-to-display status for one submodule checkout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubmoduleInfo {
    pub path: String,
    pub commit: String,
    pub status: SubmoduleStatus,
}

/// Local Git LFS capability and the paths currently stored through LFS.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LfsStatus {
    pub available: bool,
    pub initialized: bool,
    pub version: Option<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRecord {
    pub path: String,
    pub name: String,
    /// A user-defined display alias. This is visually distinguished from the
    /// directory-derived repository name and never changes the filesystem.
    #[serde(default)]
    pub alias: Option<String>,
    /// A user-defined collection name. This is application metadata, never a
    /// Git setting, so moving it does not modify the repository.
    #[serde(default)]
    pub group: Option<String>,
    /// The safe host component of the preferred remote URL, captured when the
    /// repository is opened. It is used only for local sidebar grouping.
    #[serde(default)]
    pub host: Option<String>,
    /// The remote owner / organization / nested group path paired with `host`.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Unix time in milliseconds for the last explicit repository open.
    #[serde(default)]
    pub last_opened_at: Option<u64>,
}

/// Parse HTTP(S), ssh://, and SCP-style SSH Git remote URLs without retaining userinfo.
pub fn parse_remote_url(raw: &str) -> ParsedRemoteUrl {
    let value = raw.trim();
    // On Windows, `Url::parse` treats `C:\\work\\repo.git` as a URL with the
    // one-letter `c` scheme. Recognize drive and UNC paths before URL parsing
    // only on that platform: POSIX permits `c:group/repo.git` as an opaque Git
    // URL, so treating it as a Windows local path there is incorrect.
    if is_windows_local_path(value) {
        return parsed_parts(RemoteTransport::Local, None, None, value);
    }
    // SCP-style Git syntax intentionally has no URI scheme. Recognize it
    // before `Url::parse`, which otherwise treats `github.com:path` as an
    // opaque custom-scheme URL and loses its SSH authority.
    if let Some((host, path)) = parse_scp_style_ssh_remote(value) {
        return parsed_parts(RemoteTransport::Ssh, host, None, path);
    }
    if let Ok(url) = Url::parse(value) {
        let transport = match url.scheme() {
            "https" => RemoteTransport::Https,
            "http" => RemoteTransport::Http,
            "ssh" | "git+ssh" => RemoteTransport::Ssh,
            "git" => RemoteTransport::Git,
            "file" => RemoteTransport::Local,
            _ => RemoteTransport::Other,
        };
        // A file URL can syntactically contain an authority (`file://host/...`)
        // even though it is a local filesystem remote. Keeping that host would
        // let a local path accidentally match a code-host Endpoint.
        let (host, port) = if matches!(&transport, RemoteTransport::Local) {
            (None, None)
        } else {
            (
                url.host().and_then(normalized_url_host),
                // `Url::port()` normalizes an explicitly supplied scheme-default
                // port away. Preserve the raw authority's port so a remote such
                // as `https://host:443/org/repo.git` can select an Endpoint that
                // was intentionally configured as `host:443`.
                explicit_port_in_remote_url(value)
                    .flatten()
                    .or_else(|| url.port()),
            )
        };
        return parsed_parts(transport, host, port, url.path());
    }

    // A local bare repository is a valid Git remote and deliberately has no
    // endpoint or account identity to resolve.
    if value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        // A local path may legally contain `@`; only a colon introduces the
        // SCP-style `host:path` ambiguity handled below.
        || !value.contains(':')
    {
        return parsed_parts(RemoteTransport::Local, None, None, value);
    }

    ParsedRemoteUrl {
        transport: RemoteTransport::Other,
        host: None,
        port: None,
        namespace: None,
        repository: None,
    }
}

/// Parses Git's scheme-less `user@host:path` and `host:path` SSH forms.
/// Restrict bare hostnames without a user component to a dotted or bracketed
/// authority so opaque values such as `c:group/repository` retain their
/// existing non-remote interpretation on POSIX.
fn parse_scp_style_ssh_remote(value: &str) -> Option<(Option<String>, &str)> {
    if value.contains("://") {
        return None;
    }
    let (host_part, path) = value.rsplit_once(':')?;
    if host_part.contains('/') || path.is_empty() || path.starts_with('/') {
        return None;
    }
    let has_user = host_part.contains('@');
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part)
        .trim_matches(['[', ']']);
    if host.is_empty() || (!has_user && !host.contains('.') && !host_part.starts_with('[')) {
        return None;
    }
    Some((normalize_remote_hostname(host), path))
}

#[cfg(windows)]
fn is_windows_local_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || value.starts_with(r"\\")
}

#[cfg(not(windows))]
fn is_windows_local_path(_value: &str) -> bool {
    false
}

/// Extract an explicitly written port from a hierarchical remote URL without
/// relying on `Url::port()`, which treats a scheme-default port as absent.
/// `Url::parse` has already validated the URL before this helper is used, so
/// this only needs to isolate its raw authority safely.
fn explicit_port_in_remote_url(value: &str) -> Option<Option<u16>> {
    // A URL scheme is ASCII case-insensitive. `Url::parse` accepts and
    // normalizes e.g. `HTTPS://`, while a literal `"://"` search would miss
    // its authority and discard an explicitly written default port.
    let (_, after_scheme) = value.split_once(':')?;
    let raw_authority = after_scheme
        .strip_prefix("//")?
        .split(['/', '?', '#'])
        .next()?;
    let authority = raw_authority
        .rsplit_once('@')
        .map(|(_, authority)| authority)
        .unwrap_or(raw_authority);
    explicit_port_in_git_authority(authority)
}

fn parsed_parts(
    transport: RemoteTransport,
    host: Option<String>,
    port: Option<u16>,
    path: &str,
) -> ParsedRemoteUrl {
    let path = path.strip_prefix('/').unwrap_or(path);
    if path.is_empty() || path.split('/').any(str::is_empty) {
        return ParsedRemoteUrl {
            transport,
            host,
            port,
            namespace: None,
            repository: None,
        };
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    let repository = parts
        .pop()
        .map(|part| part.strip_suffix(".git").unwrap_or(part))
        .filter(|part| !part.is_empty())
        .map(str::to_string);
    let namespace = if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    };

    ParsedRemoteUrl {
        transport,
        host,
        port,
        namespace,
        repository,
    }
}

/// Returns whether a configured Endpoint's Git authority is exactly the
/// authority parsed from a remote. Host-only endpoints deliberately do not
/// match a non-default port on the same hostname.
pub fn endpoint_matches_authority(endpoint: &CodeHostEndpoint, authority: &str) -> bool {
    let Some(endpoint_authority) = normalize_git_authority(&endpoint.git_host) else {
        return false;
    };
    // Callers pass `ParsedRemoteUrl::authority()`, which preserves every port
    // written in the remote URL. Do not normalize it a second time: doing so
    // would collapse an explicit `:443` or make HTTPS on port 22 ambiguous.
    endpoint_authority.eq_ignore_ascii_case(authority)
}

/// Look up an Endpoint by a parsed Git authority. The historic function name
/// remains for IPC-adjacent callers, but its input may now be `host:port`.
pub fn endpoint_for_host<'a>(
    authority: &str,
    endpoints: &'a [CodeHostEndpoint],
) -> Option<&'a CodeHostEndpoint> {
    endpoints
        .iter()
        .find(|endpoint| endpoint_matches_authority(endpoint, authority))
}

/// Normalize a scheme-less Git authority for durable Endpoint identity.
///
/// Unlike a parsed remote URL, an Endpoint has no transport field, so its
/// explicit port must be preserved. Treating `:22` or `:80` as a default here
/// would make a valid HTTPS service on that port impossible to represent.
pub fn normalize_git_authority(value: &str) -> Option<String> {
    let (url, explicit_port) = parse_git_authority(value)?;
    let host = normalized_url_host(url.host()?)?;
    Some(format_git_authority(&host, explicit_port))
}

/// Extract the hostname from a configured Git authority. Credential helpers
/// and askpass validate prompt hostnames, not URL authorities, so they must
/// never receive a `host:port` value.
pub fn hostname_from_git_authority(value: &str) -> Option<String> {
    let (url, _) = parse_git_authority(value)?;
    match url.host()? {
        Host::Ipv6(address) => Some(address.to_string()),
        host => normalized_url_host(host),
    }
}

/// Whether the Endpoint carries an explicit port. Platform CLI integrations
/// use hostname-scoped configuration and therefore must not silently map a
/// port-scoped service to their default API port.
pub fn git_authority_has_explicit_port(value: &str) -> bool {
    parse_git_authority(value).is_some_and(|(_, explicit_port)| explicit_port.is_some())
}

fn parse_git_authority(value: &str) -> Option<(Url, Option<u16>)> {
    let value = value.trim();
    if value.is_empty() || value.contains(['/', '\\', '@']) {
        return None;
    }
    let url = Url::parse(&format!("https://{value}")).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some((url, explicit_port_in_git_authority(value)?))
}

/// `url::Url` canonicalizes the default port for its scheme away. Endpoint
/// authorities are scheme-less and must retain a port the user actually
/// supplied: `host:443` can describe an SSH service on port 443 even though
/// this parser uses a synthetic HTTPS scheme for validation.
fn explicit_port_in_git_authority(value: &str) -> Option<Option<u16>> {
    if let Some(bracketed) = value.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        return match suffix {
            "" => Some(None),
            _ => suffix.strip_prefix(':')?.parse::<u16>().ok().map(Some),
        };
    }

    match value.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            port.parse::<u16>().ok().map(Some)
        }
        Some(_) => None,
        None => Some(None),
    }
}

fn normalized_url_host(host: Host<&str>) -> Option<String> {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.');
            (!domain.is_empty()
                && domain.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
                }))
            .then(|| domain.to_ascii_lowercase())
        }
        Host::Ipv4(address) => Some(address.to_string()),
        Host::Ipv6(address) => Some(format!("[{address}]")),
    }
}

fn normalize_remote_hostname(host: &str) -> Option<String> {
    let host = host.trim_end_matches('.');
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn format_git_authority(host: &str, port: Option<u16>) -> String {
    let host = if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn has_invalid_percent_escape(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return true;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    false
}

pub fn sanitize_remote_url(raw: &str) -> String {
    let raw = raw.trim();
    // `url::Url` normalizes a few malformed path escapes instead of rejecting
    // them. Do not turn such an input into a seemingly safe display value: a
    // malformed URL can still carry userinfo before the host component.
    if raw.contains("://") && has_invalid_percent_escape(raw) {
        return "[无法解析的远程地址]".to_string();
    }

    if let Ok(mut url) = Url::parse(raw) {
        let scheme = url.scheme().to_ascii_lowercase();
        let supported_hierarchical_scheme = matches!(
            scheme.as_str(),
            "https" | "http" | "ssh" | "git+ssh" | "git" | "file"
        );
        // `Url` also accepts opaque schemes such as `custom:payload`. They
        // are not Git remotes we understand, and their opaque payload can
        // contain a token without being exposed as URL userinfo.
        if !supported_hierarchical_scheme || (scheme != "file" && url.host_str().is_none()) {
            return "[无法解析的远程地址]".to_string();
        }
        if !url.username().is_empty() || url.password().is_some() {
            let _ = url.set_username("");
            let _ = url.set_password(None);
        }
        // Query and fragment components are never needed to identify a Git
        // remote and can carry access tokens on any parseable URL scheme.
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }

    // SCP syntax and local paths are not parseable as generic URLs but are
    // normal Git remotes. Every other parse failure is hidden rather than
    // returned verbatim because malformed URL userinfo can still contain a
    // token (for example an invalid percent escape after `token@host`).
    match parse_remote_url(raw).transport {
        RemoteTransport::Ssh => {
            sanitize_scp_ssh_remote(raw).unwrap_or_else(|| "[无法解析的远程地址]".to_string())
        }
        RemoteTransport::Local => raw.to_string(),
        RemoteTransport::Https
        | RemoteTransport::Http
        | RemoteTransport::Git
        | RemoteTransport::Other => "[无法解析的远程地址]".to_string(),
    }
}

/// SCP-style remotes are not generic URLs, but the optional user component is
/// still userinfo and can be a credential. Keep the host and repository path
/// useful for display while never returning the `user@` prefix verbatim.
fn sanitize_scp_ssh_remote(raw: &str) -> Option<String> {
    let (host_part, path) = raw.rsplit_once(':')?;
    if host_part.contains('/') || path.is_empty() || path.starts_with('/') {
        return None;
    }
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part)
        .trim_matches(['[', ']']);
    (!host.is_empty()).then(|| format!("{host}:{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_builtin_https_remote() {
        let parsed = parse_remote_url("https://github.com/broknowmyorg/example.git");
        assert_eq!(parsed.transport, RemoteTransport::Https);
        assert_eq!(parsed.host.as_deref(), Some("github.com"));
        assert_eq!(parsed.namespace.as_deref(), Some("broknowmyorg"));
        assert_eq!(parsed.repository.as_deref(), Some("example"));
        assert_eq!(
            parsed.repository_slug().as_deref(),
            Some("broknowmyorg/example")
        );
    }

    #[test]
    fn recognizes_scp_style_ssh_remote() {
        let parsed = parse_remote_url("git@gitee.com:team/project.git");
        assert_eq!(parsed.transport, RemoteTransport::Ssh);
        assert_eq!(parsed.host.as_deref(), Some("gitee.com"));
        assert_eq!(parsed.repository_slug().as_deref(), Some("team/project"));

        let username_less = parse_remote_url("gitee.com:team/project.git");
        assert_eq!(username_less.transport, RemoteTransport::Ssh);
        assert_eq!(username_less.host.as_deref(), Some("gitee.com"));
        assert_eq!(
            username_less.repository_slug().as_deref(),
            Some("team/project")
        );
    }

    #[test]
    fn recognizes_unauthenticated_and_local_git_remotes() {
        let git = parse_remote_url("git://git.example.test/team/project.git");
        assert_eq!(git.transport, RemoteTransport::Git);
        assert_eq!(git.host.as_deref(), Some("git.example.test"));

        let local = parse_remote_url("/srv/git/project.git");
        assert_eq!(local.transport, RemoteTransport::Local);
        assert_eq!(local.host, None);

        let relative = parse_remote_url("shared/project.git");
        assert_eq!(relative.transport, RemoteTransport::Local);
        assert_eq!(relative.host, None);

        let local_with_at = parse_remote_url("team@work/project.git");
        assert_eq!(local_with_at.transport, RemoteTransport::Local);
        assert_eq!(local_with_at.host, None);

        #[cfg(windows)]
        {
            let windows = parse_remote_url(r"C:\\work\\project.git");
            assert_eq!(windows.transport, RemoteTransport::Local);
            let unc = parse_remote_url(r"\\server\\share\\project.git");
            assert_eq!(unc.transport, RemoteTransport::Local);
        }
    }

    #[test]
    fn local_file_urls_never_expose_a_code_host_authority() {
        let local = parse_remote_url("file://github.com/org/project.git");

        assert_eq!(local.transport, RemoteTransport::Local);
        assert_eq!(local.host, None);
        assert_eq!(local.port, None);
        assert_eq!(local.authority(), None);
        assert_eq!(local.repository_slug().as_deref(), Some("org/project"));
    }

    #[test]
    fn removes_embedded_http_credentials_from_display_url() {
        assert_eq!(
            sanitize_remote_url("https://token-value@github.com/org/repo.git"),
            "https://github.com/org/repo.git"
        );
    }

    #[test]
    fn removes_query_and_fragment_from_every_parseable_remote_url() {
        assert_eq!(
            sanitize_remote_url("ssh://git@example.test/team/repo.git?access_token=secret#part"),
            "ssh://example.test/team/repo.git"
        );
    }

    #[test]
    fn hides_unparseable_remote_values_that_can_contain_userinfo() {
        let sanitized = sanitize_remote_url("https://token-value@github.com/%zz");
        assert_eq!(sanitized, "[无法解析的远程地址]");
        assert!(!sanitized.contains("token-value"));
    }

    #[test]
    fn removes_scp_style_ssh_userinfo_from_display_url() {
        let sanitized = sanitize_remote_url("token-value@git.example.test:team/project.git");
        assert_eq!(sanitized, "git.example.test:team/project.git");
        assert!(!sanitized.contains("token-value"));
    }

    #[test]
    fn hides_opaque_unknown_url_payloads() {
        assert_eq!(
            sanitize_remote_url("custom:token-value"),
            "[无法解析的远程地址]"
        );
    }

    #[cfg(windows)]
    #[test]
    fn recognizes_drive_relative_windows_paths_as_local_remotes() {
        let parsed = parse_remote_url(r"C:work\\project.git");
        assert_eq!(parsed.transport, RemoteTransport::Local);
        assert_eq!(parsed.host, None);
    }

    #[cfg(not(windows))]
    #[test]
    fn does_not_treat_drive_relative_urls_as_windows_paths() {
        let parsed = parse_remote_url("c:group/project.git");
        assert_eq!(parsed.transport, RemoteTransport::Other);
        assert_eq!(parsed.host, None);
    }

    #[test]
    fn parses_non_default_ports_and_matches_the_full_endpoint_authority() {
        let parsed = parse_remote_url("https://git.example.test:8443/org/repo.git");
        assert_eq!(parsed.transport, RemoteTransport::Https);
        assert_eq!(parsed.host.as_deref(), Some("git.example.test"));
        assert_eq!(parsed.port, Some(8443));
        assert_eq!(parsed.authority().as_deref(), Some("git.example.test:8443"));

        let hostname_only = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Default port".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        let port_endpoint = CodeHostEndpoint {
            id: "git.example.test-8443".into(),
            display_name: "Non-default port".into(),
            git_host: "git.example.test:8443".into(),
            ..hostname_only.clone()
        };
        assert!(!endpoint_matches_authority(
            &hostname_only,
            parsed.authority().as_deref().expect("parsed authority")
        ));
        assert!(endpoint_matches_authority(
            &port_endpoint,
            parsed.authority().as_deref().expect("parsed authority")
        ));
        assert_eq!(
            hostname_from_git_authority(&port_endpoint.git_host).as_deref(),
            Some("git.example.test")
        );
    }

    #[test]
    fn preserves_explicit_default_ports_for_exact_endpoint_matching() {
        let https_default = parse_remote_url("https://git.example.test:443/org/repo.git");
        let uppercase_https_default = parse_remote_url("HTTPS://git.example.test:443/org/repo.git");
        let ssh_default = parse_remote_url("ssh://git@git.example.test:22/org/repo.git");
        let https_on_ssh_port = parse_remote_url("https://git.example.test:22/org/repo.git");
        let ssh_on_https_port = parse_remote_url("ssh://git@git.example.test:443/org/repo.git");
        assert_eq!(
            https_default.authority().as_deref(),
            Some("git.example.test:443")
        );
        assert_eq!(
            uppercase_https_default.authority().as_deref(),
            Some("git.example.test:443")
        );
        assert_eq!(
            ssh_default.authority().as_deref(),
            Some("git.example.test:22")
        );
        assert_eq!(
            https_on_ssh_port.authority().as_deref(),
            Some("git.example.test:22")
        );
        let endpoint = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        assert!(!endpoint_matches_authority(
            &endpoint,
            https_default
                .authority()
                .as_deref()
                .expect("parsed authority")
        ));
        assert!(!endpoint_matches_authority(
            &endpoint,
            https_on_ssh_port
                .authority()
                .as_deref()
                .expect("parsed authority")
        ));
        assert_eq!(
            normalize_git_authority("GIT.EXAMPLE.TEST:443").as_deref(),
            Some("git.example.test:443")
        );
        assert!(git_authority_has_explicit_port("GIT.EXAMPLE.TEST:443"));
        assert_eq!(
            normalize_git_authority("git.example.test:8443").as_deref(),
            Some("git.example.test:8443")
        );

        let explicit_443_endpoint = CodeHostEndpoint {
            id: "git.example.test-443".into(),
            git_host: "git.example.test:443".into(),
            ..endpoint
        };
        assert!(endpoint_matches_authority(
            &explicit_443_endpoint,
            https_default
                .authority()
                .as_deref()
                .expect("parsed authority")
        ));
        assert!(endpoint_matches_authority(
            &explicit_443_endpoint,
            ssh_on_https_port
                .authority()
                .as_deref()
                .expect("parsed authority")
        ));

        let explicit_22_endpoint = CodeHostEndpoint {
            id: "git.example.test-22".into(),
            git_host: "git.example.test:22".into(),
            ..explicit_443_endpoint
        };
        assert!(endpoint_matches_authority(
            &explicit_22_endpoint,
            ssh_default
                .authority()
                .as_deref()
                .expect("parsed authority")
        ));
    }

    #[test]
    fn normalizes_ipv6_authorities_with_explicit_ports() {
        let endpoint = CodeHostEndpoint {
            id: "ipv6-git".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "IPv6 Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "[2001:db8::1]:443".into(),
            is_builtin: false,
        };
        let parsed = parse_remote_url("ssh://git@[2001:db8::1]:443/org/repo.git");

        assert_eq!(
            normalize_git_authority(&endpoint.git_host).as_deref(),
            Some("[2001:db8::1]:443")
        );
        assert_eq!(
            hostname_from_git_authority(&endpoint.git_host).as_deref(),
            Some("2001:db8::1")
        );
        assert!(git_authority_has_explicit_port(&endpoint.git_host));
        assert!(endpoint_matches_authority(
            &endpoint,
            parsed.authority().as_deref().expect("parsed authority")
        ));
    }

    #[test]
    fn github_platform_capabilities_require_gh_compatible_api_routing() {
        let enterprise = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GitHub,
            display_name: "Example GitHub".into(),
            web_url: "https://git.example.test".into(),
            api_url: "https://git.example.test/api/v3".into(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };
        assert!(github_endpoint_has_gh_compatible_api(&enterprise));
        assert!(CodeHostCapabilities::for_endpoint(&enterprise).repository_info);

        let mut wrong_host = enterprise.clone();
        wrong_host.api_url = "https://api.other.test/api/v3".into();
        assert!(!github_endpoint_has_gh_compatible_api(&wrong_host));
        assert!(!CodeHostCapabilities::for_endpoint(&wrong_host).repository_info);

        let mut wrong_path = enterprise;
        wrong_path.api_url = "https://git.example.test/custom-api".into();
        assert!(!github_endpoint_has_gh_compatible_api(&wrong_path));
    }

    #[test]
    fn generic_git_endpoint_keeps_safe_git_account_capabilities() {
        let endpoint = CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        };

        let capabilities = CodeHostCapabilities::for_endpoint(&endpoint);
        assert!(capabilities.account_bindings);
        assert!(capabilities.pat_keychain);
        assert!(!capabilities.repository_info);
        assert!(!capabilities.pull_requests_read);
    }
}
