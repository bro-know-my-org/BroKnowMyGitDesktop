use crate::domain::{
    BranchInfo, GitOperationKind, GitStatus, RemoteBinding, RemoteBranchInfo, RemoteProfile,
    RepositoryRecord, TagInfo,
};
use serde::Serialize;

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

impl NetworkOperationResult {
    pub fn completed(message: impl Into<String>) -> Self {
        Self {
            summary: message.into(),
        }
    }
}
