use super::support::*;
use super::*;

#[test]
fn login_validation_rejects_whitespace() {
    assert!(required_login("work account").is_err());
    assert_eq!(required_login(" account ").unwrap(), "account");
}

#[test]
fn generic_endpoint_needs_only_a_safe_git_host() {
    let generic = CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GenericGit,
        display_name: "Example Git".into(),
        web_url: String::new(),
        api_url: String::new(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    };
    assert!(validate_custom_endpoint(&generic).is_ok());

    let mut port_scoped = generic.clone();
    port_scoped.id = "git.example.test-8443".into();
    port_scoped.git_host = "git.example.test:8443".into();
    assert!(validate_custom_endpoint(&port_scoped).is_ok());

    let mut platform = generic;
    platform.kind = CodeHostKind::GitLab;
    assert_eq!(
        validate_custom_endpoint(&platform).unwrap_err().code(),
        "invalid_endpoint"
    );

    let github = CodeHostEndpoint {
        id: "github.example.test".into(),
        kind: CodeHostKind::GitHub,
        display_name: "Example GitHub".into(),
        web_url: "https://github.example.test".into(),
        api_url: "https://github.example.test/api/v3".into(),
        git_host: "github.example.test".into(),
        is_builtin: false,
    };
    assert!(validate_custom_endpoint(&github).is_ok());

    let mut mismatched_github = github;
    mismatched_github.api_url = "https://api.other.test/api/v3".into();
    assert_eq!(
        validate_custom_endpoint(&mismatched_github)
            .unwrap_err()
            .code(),
        "invalid_endpoint"
    );
}

#[test]
fn endpoint_git_authorities_cannot_shadow_builtin_or_custom_routing() {
    let builtin_collision = CodeHostEndpoint {
        id: "private-github".into(),
        kind: CodeHostKind::GenericGit,
        display_name: "Private GitHub".into(),
        web_url: String::new(),
        api_url: String::new(),
        git_host: "github.com".into(),
        is_builtin: false,
    };
    assert!(endpoint_git_host_conflicts(
        &ProfileStore::default(),
        &builtin_collision
    ));

    let port_endpoint = CodeHostEndpoint {
        id: "git.example.test-8443".into(),
        kind: CodeHostKind::GenericGit,
        display_name: "Example Git".into(),
        web_url: String::new(),
        api_url: String::new(),
        git_host: "git.example.test:8443".into(),
        is_builtin: false,
    };
    let store = ProfileStore {
        custom_endpoints: vec![port_endpoint.clone()],
        ..ProfileStore::default()
    };
    let mut duplicate = port_endpoint.clone();
    duplicate.id = "another-id".into();
    assert!(endpoint_git_host_conflicts(&store, &duplicate));

    let mut default_port = duplicate;
    default_port.git_host = "git.example.test".into();
    assert!(!endpoint_git_host_conflicts(&store, &default_port));
}

#[test]
fn referenced_endpoint_cannot_change_routing_identity() {
    let endpoint = CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GenericGit,
        display_name: "Example Git".into(),
        web_url: String::new(),
        api_url: String::new(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    };
    let store = ProfileStore {
        custom_endpoints: vec![endpoint.clone()],
        accounts: vec![AccountProfile {
            id: "account-1".into(),
            endpoint_id: endpoint.id.clone(),
            login: "alice".into(),
            display_name: None,
            auth_method: AuthenticationMethod::SystemGitCredentialHelper,
            secret_ref: None,
            status: AuthenticationStatus::Available,
            is_default_candidate: false,
        }],
        ..ProfileStore::default()
    };

    let mut renamed = endpoint.clone();
    renamed.display_name = "Renamed only".into();
    assert!(!endpoint_routing_change_is_referenced(&store, &renamed));

    let mut moved = endpoint;
    moved.git_host = "git.other.test".into();
    assert!(endpoint_routing_change_is_referenced(&store, &moved));
}

#[test]
fn custom_generic_endpoint_is_recognized_without_a_binding() {
    let endpoint = CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GenericGit,
        display_name: "Example Git".into(),
        web_url: String::new(),
        api_url: String::new(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    };
    let store = ProfileStore {
        custom_endpoints: vec![endpoint],
        ..ProfileStore::default()
    };
    let mut remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: "https://git.example.test/team/project.git".into(),
        push_url: "https://git.example.test/team/project.git".into(),
        parsed: parse_remote_url("https://git.example.test/team/project.git"),
        endpoint_id: None,
        code_host_kind: CodeHostKind::Unknown,
    }];

    enrich_remote_endpoints(&mut remotes, &store, "/work/project");

    assert_eq!(remotes[0].endpoint_id.as_deref(), Some("git.example.test"));
    assert_eq!(remotes[0].code_host_kind, CodeHostKind::GenericGit);
}

#[test]
fn repository_owner_grouping_prefers_origin() {
    let remotes = vec![
        RemoteProfile {
            name: "upstream".into(),
            fetch_url: "https://github.com/upstream/project.git".into(),
            push_url: "https://github.com/upstream/project.git".into(),
            parsed: parse_remote_url("https://github.com/upstream/project.git"),
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        },
        RemoteProfile {
            name: "origin".into(),
            fetch_url: "git@gitlab.com:team/platform/project.git".into(),
            push_url: "git@gitlab.com:team/platform/project.git".into(),
            parsed: parse_remote_url("git@gitlab.com:team/platform/project.git"),
            endpoint_id: Some("gitlab.com".into()),
            code_host_kind: CodeHostKind::GitLab,
        },
    ];

    let (host, namespace) = repository_owner_from_remotes(&remotes);

    assert_eq!(host.as_deref(), Some("gitlab.com"));
    assert_eq!(namespace.as_deref(), Some("team/platform"));
}

#[test]
fn repository_group_validation_trims_and_rejects_control_characters() {
    assert_eq!(
        normalize_repository_group(Some("  工作项目  ".into())).unwrap(),
        Some("工作项目".into())
    );
    assert_eq!(
        normalize_repository_group(Some("   ".into())).unwrap(),
        None
    );
    assert_eq!(
        normalize_repository_group(Some("工作\n项目".into()))
            .expect_err("line breaks are not valid group names")
            .code(),
        "repository_group_invalid"
    );
}

#[test]
fn unauthenticated_remote_url_cannot_hide_userinfo_or_tokens() {
    let error = validate_unauthenticated_remote_url(
        "git://secret-token@git.example.test/team/project.git?access_token=hidden",
    )
    .expect_err("a git protocol URL must not carry credential-like data");
    assert_eq!(error.code(), "unsafe_remote_url");
    assert!(validate_unauthenticated_remote_url("git://git.example.test/team/project.git").is_ok());
    assert!(validate_unauthenticated_remote_url("/srv/git/project.git").is_ok());
}

#[test]
fn remote_binding_requires_both_effective_operation_urls_to_match() {
    let repository_path = "/work/project";
    let store = ProfileStore {
        remote_bindings: vec![RemoteBinding {
            repository_path: repository_path.into(),
            remote_name: "origin".into(),
            endpoint_id: "github.com".into(),
            account_selection: AccountSelection::Auto,
        }],
        ..ProfileStore::default()
    };

    assert!(remote_binding_matches_urls(
        &store,
        repository_path,
        "origin",
        "https://github.com/example/project.git",
        "git@github.com:example/project.git",
    ));
    assert!(!remote_binding_matches_urls(
        &store,
        repository_path,
        "origin",
        "https://gitlab.com/example/project.git",
        "https://github.com/example/project.git",
    ));
    assert!(!remote_binding_matches_urls(
        &store,
        repository_path,
        "origin",
        "https://github.com/example/project.git",
        "https://gitlab.com/example/project.git",
    ));
}

#[test]
fn pending_publish_identity_requires_the_expected_github_slug() {
    assert_eq!(
        github_publish_expected_slug("project", "octo").unwrap(),
        "octo/project"
    );
    assert_eq!(
        github_publish_expected_slug("team/project", "octo").unwrap(),
        "team/project"
    );
    assert!(github_publish_expected_slug("../project", "octo").is_err());
    assert!(pending_publish_remote_matches(
        "https://github.com/octo/project.git",
        "octo/project"
    ));
    assert!(pending_publish_remote_matches(
        "git@github.com:octo/project.git",
        "octo/project"
    ));
    assert!(!pending_publish_remote_matches(
        "https://github.com/octo/other.git",
        "octo/project"
    ));
}

#[test]
fn pending_clone_identity_requires_the_expected_repository_on_both_urls() {
    assert_eq!(
        clone_expected_remote_identity("git@git.example.test:team/project.git").unwrap(),
        ("git.example.test".into(), "team/project".into())
    );
    assert!(pending_clone_remote_matches(
        "https://git.example.test/team/project.git",
        "git.example.test",
        "team/project"
    ));
    assert!(pending_clone_remote_matches(
        "git@git.example.test:team/project.git",
        "git.example.test",
        "team/project"
    ));
    assert!(!pending_clone_remote_matches(
        "https://git.example.test/team/other.git",
        "git.example.test",
        "team/project"
    ));
    assert!(!pending_clone_remote_matches(
        "https://git.example.test:8443/team/project.git",
        "git.example.test",
        "team/project"
    ));
    assert!(clone_expected_remote_identity("/srv/git/project.git").is_err());
}

#[test]
fn pending_clone_binding_requires_its_selected_profile_to_still_exist() {
    let binding = RemoteBinding {
        repository_path: "/work/project".into(),
        remote_name: "origin".into(),
        endpoint_id: "github.com".into(),
        account_selection: AccountSelection::Profile("account-1".into()),
    };
    let mut store = ProfileStore::default();

    assert!(!binding_selected_account_is_current(&store, &binding));

    store.accounts.push(AccountProfile {
        id: "account-1".into(),
        endpoint_id: "gitlab.com".into(),
        login: "alice".into(),
        display_name: None,
        auth_method: AuthenticationMethod::GhCli,
        secret_ref: None,
        status: AuthenticationStatus::Valid,
        is_default_candidate: false,
    });
    assert!(!binding_selected_account_is_current(&store, &binding));

    store.accounts[0].endpoint_id = "github.com".into();
    assert!(binding_selected_account_is_current(&store, &binding));
}

#[test]
fn system_git_helper_is_not_limited_to_global_configuration() {
    assert_eq!(system_git_helper_status(), AuthenticationStatus::Available);
}

#[cfg(unix)]
#[test]
fn stalled_status_probe_is_killed_before_it_can_block_account_refresh() {
    let child = Command::new("sh")
        .args(["-c", "exec sleep 1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("test shell starts");
    let started = Instant::now();

    assert!(wait_for_child_status(child, Duration::from_millis(20)).is_none());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn background_status_merge_does_not_restore_removed_accounts() {
    let cached = AccountProfile {
        id: "kept".into(),
        endpoint_id: "github.com".into(),
        login: "octo".into(),
        display_name: None,
        auth_method: AuthenticationMethod::GhCli,
        secret_ref: None,
        status: AuthenticationStatus::Unknown,
        is_default_candidate: false,
    };
    let mut refreshed = cached.clone();
    refreshed.status = AuthenticationStatus::Valid;
    let removed_while_refreshing = AccountProfile {
        id: "removed".into(),
        ..refreshed.clone()
    };
    let mut store = ProfileStore {
        accounts: vec![cached],
        ..ProfileStore::default()
    };

    merge_refreshed_account_statuses(&mut store, &[refreshed, removed_while_refreshing]);

    assert_eq!(store.accounts.len(), 1);
    assert_eq!(store.accounts[0].id, "kept");
    assert_eq!(store.accounts[0].status, AuthenticationStatus::Valid);
}
