use crate::domain::{
    BranchInfo, GitOperationKind, GitStatus, RemoteBinding, RemoteBranchInfo, RemoteProfile,
    RepositoryRecord, TagInfo,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDiscoveryResult {
    pub repositories: Vec<RepositoryRecord>,
    pub import_limit: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryImportResult {
    pub repositories: Vec<RepositoryRecord>,
    pub rejected_count: usize,
    pub remaining_capacity: usize,
    pub persistence_warning: bool,
}

/// One round-trip model used by the repository overview page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryRecord,
    pub has_head: bool,
    pub status: GitStatus,
    pub branches: Vec<BranchInfo>,
    pub remote_branches: Vec<RemoteBranchInfo>,
    pub remotes: Vec<RemoteProfile>,
    pub tags: Vec<TagInfo>,
    pub bindings: Vec<RemoteBinding>,
    pub operation: Option<GitOperationState>,
}

/// The branch command reports whether its fresh status check actually created
/// a stash, rather than making callers infer that from the requested option.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchResult {
    pub snapshot: RepositorySnapshot,
    pub stashed_local_changes: bool,
}

/// State for an operation that Git has paused in the working tree.  Conflict
/// detection comes from the same status snapshot that is rendered in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationState {
    pub kind: GitOperationKind,
    pub has_conflicts: bool,
}

/// A deliberately small response for network Git commands. Git's raw stdout is not returned
/// because it can contain remote URLs and credential-helper diagnostics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkOperationResult {
    pub summary: String,
}

/// Safe, display-ready repository network trust state. Dangerous values are
/// deliberately omitted because they may contain credentials or commands.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryNetworkSecurityStatus {
    pub repository_path: String,
    pub unsafe_keys: Vec<String>,
    pub current_fingerprint: Option<String>,
    pub authorization_allowed: bool,
    pub trusted: bool,
    pub authorized_keys: Vec<String>,
    pub authorized_fingerprint: Option<String>,
    pub authorization_stale: bool,
}

/// A short-lived server-side confirmation for one exact set of untracked
/// files. The token prevents a later broad clean from deleting files that did
/// not appear in the preview the user reviewed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanUntrackedPreview {
    pub preview: String,
    pub confirmation: String,
}

/// A short-lived server-side confirmation for one exact discard operation.
/// Execution accepts only the token, so callers cannot substitute a different
/// repository path set after the user has reviewed the warning.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardChangesPreview {
    pub confirmation: String,
    pub path_count: usize,
    pub untracked_count: usize,
}

impl NetworkOperationResult {
    pub fn completed(message: impl Into<String>) -> Self {
        Self {
            summary: message.into(),
        }
    }
}
