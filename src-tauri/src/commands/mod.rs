use crate::app_error::AppError;
use crate::codehost::{
    AdapterContext, CodeHostAdapter, CodeHostRepositoryInfo, CreatePullRequestInput,
    PullRequestSummary,
};
use crate::command_models::{
    CleanUntrackedPreview, GitOperationState, NetworkOperationResult, RepositorySnapshot,
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
    GitOperationKind, LfsStatus, RemoteBinding, RemoteProfile, RepositoryRecord, ResetMode,
    StashEntry, SubmoduleInfo, WorktreeInfo,
};
use crate::git_core::{
    normalize_external_editor_config, validate_git_remote_url, CloneOptions, DiffOptions,
    FetchOptions, GitCore, GitEnvironment, PullOptions, PushOptions, RemoteUrlKind,
};
use crate::github_adapter::GitHubAdapter;
use crate::persistence::{AppState, PendingClone, PendingPublish, ProfileStore};
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
mod network;
mod remotes;
mod repositories;
mod repository_support;
mod support;
mod workspace;

#[cfg(test)]
mod tests;

pub use accounts::*;
pub use branches::*;
pub use changes::*;
pub use codehost::*;
pub use commits::*;
pub use endpoints::*;
pub use network::*;
pub use remotes::*;
pub use repositories::*;
pub use workspace::*;
