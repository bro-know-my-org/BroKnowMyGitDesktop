use crate::app_error::AppError;
use crate::domain::{
    endpoint_matches_authority, AccountProfile, CodeHostCapabilities, CodeHostEndpoint,
    CodeHostKind, RemoteProfile,
};
use serde::{Deserialize, Serialize};

/// Context passed to an adapter after Git remote and account resolution.
#[derive(Debug, Clone)]
pub struct AdapterContext {
    endpoint: CodeHostEndpoint,
    remote: RemoteProfile,
    account: AccountProfile,
}

impl AdapterContext {
    pub fn new(
        endpoint: CodeHostEndpoint,
        remote: RemoteProfile,
        account: AccountProfile,
    ) -> Result<Self, AppError> {
        let remote_matches_endpoint = remote
            .parsed
            .authority()
            .as_deref()
            .is_some_and(|authority| endpoint_matches_authority(&endpoint, authority));
        let resolved_endpoint_matches = remote.endpoint_id.as_deref() == Some(endpoint.id.as_str());
        let resolved_kind_matches = remote.code_host_kind == endpoint.kind;
        if !remote_matches_endpoint
            || !resolved_endpoint_matches
            || !resolved_kind_matches
            || account.endpoint_id != endpoint.id
        {
            return Err(AppError::new(
                "adapter_context_mismatch",
                "远程地址、Endpoint 和账号必须属于同一代码托管服务。",
            ));
        }
        Ok(Self {
            endpoint,
            remote,
            account,
        })
    }

    pub fn endpoint(&self) -> &CodeHostEndpoint {
        &self.endpoint
    }

    pub fn remote(&self) -> &RemoteProfile {
        &self.remote
    }

    pub fn account(&self) -> &AccountProfile {
        &self.account
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeHostRepositoryInfo {
    pub full_name: String,
    pub description: Option<String>,
    pub default_branch: Option<String>,
    pub web_url: String,
    pub is_private: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub web_url: String,
    pub updated_at: Option<String>,
}

/// User-authored fields for a new pull request. The backend validates these
/// before passing them as JSON on stdin to the GitHub CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreatePullRequestInput {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
}

/// Platform APIs are intentionally independent from GitCore and CredentialBroker.
/// Implementations must only expose methods advertised by `capabilities`.
pub trait CodeHostAdapter: Send + Sync {
    fn kind(&self) -> CodeHostKind;
    fn capabilities(&self, endpoint: &CodeHostEndpoint) -> CodeHostCapabilities;
    fn repository_info(&self, context: &AdapterContext)
        -> Result<CodeHostRepositoryInfo, AppError>;
    fn list_pull_requests(
        &self,
        context: &AdapterContext,
    ) -> Result<Vec<PullRequestSummary>, AppError>;
    fn create_pull_request(
        &self,
        context: &AdapterContext,
        input: &CreatePullRequestInput,
    ) -> Result<PullRequestSummary, AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AuthenticationMethod, AuthenticationStatus, ParsedRemoteUrl, RemoteTransport,
    };

    fn endpoint() -> CodeHostEndpoint {
        CodeHostEndpoint {
            id: "github.com".into(),
            kind: CodeHostKind::GitHub,
            display_name: "GitHub.com".into(),
            web_url: "https://github.com".into(),
            api_url: "https://api.github.com".into(),
            git_host: "github.com".into(),
            is_builtin: true,
        }
    }

    fn remote(host: &str) -> RemoteProfile {
        RemoteProfile {
            name: "origin".into(),
            fetch_url: format!("https://{host}/org/repository.git"),
            push_url: format!("https://{host}/org/repository.git"),
            parsed: ParsedRemoteUrl {
                transport: RemoteTransport::Https,
                host: Some(host.into()),
                port: None,
                namespace: Some("org".into()),
                repository: Some("repository".into()),
            },
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        }
    }

    fn account(endpoint_id: &str) -> AccountProfile {
        AccountProfile {
            id: "account-1".into(),
            endpoint_id: endpoint_id.into(),
            login: "octocat".into(),
            display_name: None,
            auth_method: AuthenticationMethod::GhCli,
            secret_ref: None,
            status: AuthenticationStatus::Valid,
            is_default_candidate: true,
        }
    }

    #[test]
    fn adapter_context_requires_matching_remote_and_account_endpoint() {
        let valid = AdapterContext::new(endpoint(), remote("github.com"), account("github.com"));
        assert!(valid.is_ok());

        let wrong_host = AdapterContext::new(
            endpoint(),
            remote("github.example.test"),
            account("github.com"),
        );
        assert_eq!(wrong_host.unwrap_err().code(), "adapter_context_mismatch");

        let wrong_account = AdapterContext::new(endpoint(), remote("github.com"), account("other"));
        assert_eq!(
            wrong_account.unwrap_err().code(),
            "adapter_context_mismatch"
        );
    }

    #[test]
    fn adapter_context_rejects_same_host_but_different_resolved_endpoint() {
        let mut alternate = endpoint();
        alternate.id = "github-alternate".into();
        let context = AdapterContext::new(
            alternate.clone(),
            remote("github.com"),
            account(&alternate.id),
        );

        assert_eq!(context.unwrap_err().code(), "adapter_context_mismatch");
    }
}
