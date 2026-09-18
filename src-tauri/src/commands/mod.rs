use crate::app_error::AppError;
use crate::codehost::{
    AdapterContext, CodeHostAdapter, CodeHostRepositoryInfo, CreatePullRequestInput,
    GitHubForkSummary, GitHubPublishOwner, PullRequestSummary,
};
use crate::command_models::{
    CleanUntrackedPreview, CreateBranchResult, DiscardChangesPreview, GitOperationState,
    NetworkOperationResult, RepositoryDiscoveryResult, RepositoryImportResult,
    RepositoryNetworkSecurityStatus, RepositorySnapshot,
};
use crate::credential_broker::{
    default_pat_keychain_reference, delete_personal_access_token, gh_auth_status, glab_auth_status,
    import_gh_current_account_new, import_glab_current_account_new,
    personal_access_token_reference, personal_access_token_status, save_personal_access_token,
    validate_remote_url_for_credentials, CredentialBroker, CredentialEnvironment,
    CredentialResolutionRequest,
};
use crate::domain::{
    endpoint_for_host, endpoint_matches_authority, git_authority_has_explicit_port,
    github_endpoint_has_gh_compatible_api, hostname_from_git_authority, normalize_git_authority,
    parse_remote_url, AccountProfile, AccountSelection, AuthenticationMethod, AuthenticationStatus,
    CodeHostCapabilities, CodeHostEndpoint, CodeHostKind, CommitSummary, ExternalEditorConfig,
    GitOperationKind, LfsStatus, RemoteBinding, RemoteProfile, RepositoryEditorConfig,
    RepositoryRecord, ResetMode, StashEntry, SubmoduleInfo, WorktreeInfo,
};
use crate::git_core::{
    normalize_external_editor_config, normalize_repository_editor_config,
    normalize_repository_editor_configs_structure, repository_editor_launch_config,
    validate_git_remote_url, validate_ref_name, CloneOptions, DiffOptions, FetchOptions, GitCore,
    GitEnvironment, NetworkConfigurationAssessment, PublishedRemoteState, PullOptions, PushOptions,
    RemoteUrlKind, RepositoryLocksResult,
};
use crate::github_adapter::GitHubAdapter;
use crate::persistence::{
    AppState, PendingClone, PendingPublish, ProfileStore, RepositoryNetworkTrust, MAX_REPOSITORIES,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;
use uuid::Uuid;

const DISCOVERY_MAX_DEPTH: usize = 4;
const SSH_AGENT_STATUS_TIMEOUT: Duration = Duration::from_secs(2);
const BRANCH_SWITCH_STASH_MESSAGE: &str = "切换分支前自动暂存";

mod account_support;
mod accounts;
mod branches;
mod changes;
mod codehost;
mod commits;
mod credential_support;
mod endpoint_support;
mod endpoints;
mod locks;
mod network;
mod remotes;
mod repositories;
mod repository_support;
mod repository_watcher;
mod resources;
mod security;
mod support;
mod system;
mod workspace;

#[cfg(test)]
mod tests;

pub use accounts::*;
pub use branches::*;
pub use changes::*;
pub use codehost::*;
pub use commits::*;
pub use endpoints::*;
pub use locks::*;
pub use network::*;
pub use remotes::*;
pub use repositories::*;
pub use repository_watcher::*;
pub use resources::*;
pub use security::*;
pub use system::*;
pub use workspace::*;
