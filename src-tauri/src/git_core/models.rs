use super::*;

/// Options for cloning a worktree repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloneOptions {
    pub branch: Option<String>,
    pub depth: Option<u32>,
    pub recursive: bool,
}

/// Selection for an unstaged or staged diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffOptions {
    pub staged: bool,
    pub paths: Vec<String>,
    pub context_lines: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub commit_id: String,
}

/// Fetch only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FetchOptions {
    pub remote: String,
    pub refspecs: Vec<String>,
    pub prune: bool,
}

/// Pull only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullOptions {
    pub remote: String,
    pub refspec: Option<String>,
}

/// Push only one named remote, after CredentialBroker has selected an
/// account/credential strategy for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushOptions {
    pub remote: String,
    pub refspecs: Vec<String>,
    pub set_upstream: bool,
    pub force_with_lease: bool,
    pub follow_tags: bool,
}

/// Selects which local remote URL setting is updated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemoteUrlKind {
    Fetch,
    Push,
}

/// The data needed by the repository overview page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryRecord,
    pub status: GitStatus,
    pub branches: Vec<BranchInfo>,
    pub remote_branches: Vec<RemoteBranchInfo>,
    pub remotes: Vec<RemoteProfile>,
    pub tags: Vec<TagInfo>,
}

/// A recognized standard `.lock` file currently present inside this
/// worktree's Git metadata. `id` includes a filesystem identity fingerprint and is deliberately opaque:
/// directory-opening commands must resolve it against a fresh inventory
/// instead of accepting an arbitrary filesystem path from IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryLockInfo {
    pub id: String,
    pub path: String,
    pub kind: RepositoryLockKind,
    pub modified_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryLocksResult {
    pub locks: Vec<RepositoryLockInfo>,
    pub incomplete: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryLockKind {
    Configuration,
    Index,
    Reference,
    Shallow,
    Other,
}

pub(super) fn command_arguments(
    repository: Option<&Path>,
    arguments: &[OsString],
) -> Vec<OsString> {
    let mut all_arguments =
        Vec::with_capacity(arguments.len() + usize::from(repository.is_some()) * 2);
    if let Some(repository) = repository {
        all_arguments.push(OsString::from("-C"));
        all_arguments.push(repository.as_os_str().to_os_string());
    }
    all_arguments.extend(arguments.iter().cloned());
    all_arguments
}

pub(super) fn os_arguments(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}
