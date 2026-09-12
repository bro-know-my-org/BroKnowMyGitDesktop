#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::io::Cursor;

    #[test]
    fn clone_repository_list_preserves_private_and_empty_repository_metadata() {
        let repositories = parse_clone_repositories(
            r#"[
            {"full_name":"team/private","description":null,"default_branch":null,
             "html_url":"https://github.com/team/private","private":true},
            {"full_name":"me/public","description":"Example","default_branch":"main",
             "html_url":"https://github.com/me/public","private":false}
        ]"#,
        )
        .unwrap();
        assert_eq!(repositories[0].full_name, "team/private");
        assert_eq!(repositories[0].is_private, Some(true));
        assert_eq!(repositories[0].default_branch, None);
        assert_eq!(repositories[1].is_private, Some(false));
        assert_eq!(repositories[1].description.as_deref(), Some("Example"));
        assert!(parse_clone_repositories("[]").unwrap().is_empty());
        assert_eq!(
            parse_clone_repositories(r#"{"message":"Bad credentials"}"#)
                .unwrap_err()
                .code(),
            "github_api_invalid_response"
        );
    }

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
    fn github_api_error_code_classifies_missing_and_existing_resources() {
        assert_eq!(
            github_api_error_code(b"HTTP 404: Not Found"),
            "github_api_not_found"
        );
        assert_eq!(github_api_error_code(b"resource not found"), "github_api_failed");
        assert_eq!(
            github_api_error_code(b"name already exists on this account"),
            "github_repository_exists"
        );
        assert_eq!(
            github_api_error_code(b"authentication token expired"),
            "github_api_failed"
        );
        assert_eq!(
            github_api_error_code(b"rate limit exceeded"),
            "github_api_failed"
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
            environment.get(OsStr::new("GIT_CONFIG_VALUE_7")),
            Some(&Some(OsString::from(DISABLED_PUBLISH_GIT_PATH)))
        );
        assert_eq!(
            environment.get(OsStr::new("GIT_CONFIG_KEY_0")),
            Some(&Some(OsString::from("credential.helper")))
        );
        assert_eq!(
            environment.get(OsStr::new("GIT_CONFIG_VALUE_0")),
            Some(&Some(OsString::new()))
        );
        assert_eq!(
            environment.get(OsStr::new("GIT_CONFIG_KEY_1")),
            Some(&Some(OsString::from("credential.helper")))
        );
        assert_eq!(
            environment.get(OsStr::new("GIT_CONFIG_VALUE_1")),
            Some(&Some(OsString::from("!gh auth git-credential")))
        );
        assert_eq!(environment.get(OsStr::new("GIT_DIR")), Some(&None));
    }
}
