use super::account_support::account_profile_identity_matches;
use super::credential_support::resolve_clone_credential_target_from_store;
use super::repository_support::pending_publish_matches_requested_identity;
use super::support::*;
use super::*;

struct TestDirectoryCleanup(std::path::PathBuf);

impl Drop for TestDirectoryCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn login_validation_rejects_whitespace() {
    assert!(required_login("work account").is_err());
    assert_eq!(required_login(" account ").unwrap(), "account");
}

#[test]
fn account_profile_identity_is_case_insensitive_for_logins() {
    let account = AccountProfile {
        id: "account-a".into(),
        endpoint_id: "github.com".into(),
        login: "Alice".into(),
        display_name: None,
        auth_method: AuthenticationMethod::GhCli,
        secret_ref: None,
        status: AuthenticationStatus::Valid,
        is_default_candidate: false,
    };
    let same_identity = AccountProfile {
        id: "account-b".into(),
        login: "alice".into(),
        ..account.clone()
    };
    let different_method = AccountProfile {
        auth_method: AuthenticationMethod::SshAgent,
        ..same_identity.clone()
    };

    assert!(account_profile_identity_matches(&account, &same_identity));
    assert!(!account_profile_identity_matches(
        &account,
        &different_method
    ));
}

#[test]
fn clone_destination_lease_key_normalizes_parent_aliases() {
    let root = std::env::temp_dir().join(format!("bkm-clone-key-{}", Uuid::new_v4()));
    let _cleanup = TestDirectoryCleanup(root.clone());
    std::fs::create_dir_all(&root).expect("temporary parent is created");
    std::fs::create_dir(root.join("child")).expect("alias component is created");

    let canonical = clone_destination_key(&root.join("project"))
        .expect("a normal destination produces a lease key");
    let lexical_alias = clone_destination_key(&root.join("child").join("..").join("project"))
        .expect("a lexical parent alias produces a lease key");
    assert_eq!(canonical, lexical_alias);
    assert_eq!(
        clone_destination_key(&root.join("project").join(".."))
            .expect_err("a destination cannot end in a parent component")
            .code(),
        "invalid_clone_destination"
    );
}

#[test]
fn pinned_clone_resolution_fails_instead_of_switching_accounts() {
    let endpoint = CodeHostEndpoint::builtin()
        .into_iter()
        .find(|endpoint| endpoint.id == "github.com")
        .expect("GitHub endpoint is built in");
    let first_account = AccountProfile {
        id: "clone-account-a".into(),
        endpoint_id: endpoint.id.clone(),
        login: "alice".into(),
        display_name: None,
        auth_method: AuthenticationMethod::PersonalAccessToken,
        secret_ref: Some(default_pat_keychain_reference("clone-account-a")),
        status: AuthenticationStatus::Valid,
        is_default_candidate: true,
    };
    let second_account = AccountProfile {
        id: "clone-account-b".into(),
        login: "bob".into(),
        is_default_candidate: false,
        ..first_account.clone()
    };
    let mut store = ProfileStore {
        accounts: vec![first_account, second_account],
        ..ProfileStore::default()
    };
    let destination = Path::new("/work/clone-target");
    let initial = resolve_clone_credential_target_from_store(
        &store,
        "https://github.com/example/project.git",
        destination,
        Some(&endpoint.id),
        None,
        None,
    )
    .expect("an automatic selection resolves")
    .expect("an endpoint selection uses an account");
    assert_eq!(initial.account_profile_id(), "clone-account-a");
    assert!(matches!(
        initial.binding.account_selection,
        AccountSelection::Auto
    ));

    store
        .accounts
        .retain(|account| account.id != "clone-account-a");
    let error = match resolve_clone_credential_target_from_store(
        &store,
        "https://github.com/example/project.git",
        destination,
        Some(&endpoint.id),
        None,
        Some("clone-account-a"),
    ) {
        Ok(_) => panic!("the recheck must not silently choose the remaining account"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "account_selection_required");
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

    let mut credential_bearing_endpoint = github.clone();
    credential_bearing_endpoint.web_url = "https://token@example.invalid".into();
    assert_eq!(
        validate_custom_endpoint(&credential_bearing_endpoint)
            .expect_err("Endpoint URLs must never persist userinfo")
            .code(),
        "invalid_endpoint"
    );
    credential_bearing_endpoint.web_url = "https://github.example.test/?access_token=hidden".into();
    assert_eq!(
        validate_custom_endpoint(&credential_bearing_endpoint)
            .expect_err("Endpoint URLs must not retain query parameters")
            .code(),
        "invalid_endpoint"
    );

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
fn repository_web_url_prefers_origin_and_preserves_nested_namespaces() {
    let endpoints = CodeHostEndpoint::builtin();
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
            fetch_url: "https://gitlab.com/team/platform/project.git".into(),
            push_url: "https://gitlab.com/team/platform/project.git".into(),
            parsed: parse_remote_url("https://gitlab.com/team/platform/project.git"),
            endpoint_id: Some("gitlab.com".into()),
            code_host_kind: CodeHostKind::GitLab,
        },
    ];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://gitlab.com/team/platform/project")
    );
}

#[test]
fn repository_web_url_falls_back_when_origin_has_no_web_route() {
    let endpoints = CodeHostEndpoint::builtin();
    let remotes = vec![
        RemoteProfile {
            name: "origin".into(),
            fetch_url: "/work/local.git".into(),
            push_url: "/work/local.git".into(),
            parsed: parse_remote_url("/work/local.git"),
            endpoint_id: None,
            code_host_kind: CodeHostKind::Unknown,
        },
        RemoteProfile {
            name: "upstream".into(),
            fetch_url: "https://github.com/upstream/project.git".into(),
            push_url: "https://github.com/upstream/project.git".into(),
            parsed: parse_remote_url("https://github.com/upstream/project.git"),
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        },
    ];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://github.com/upstream/project")
    );
}

#[test]
fn repository_web_url_falls_back_when_origin_endpoint_binding_is_stale() {
    let endpoints = CodeHostEndpoint::builtin();
    let remotes = vec![
        RemoteProfile {
            name: "origin".into(),
            fetch_url: "https://git.example.test/team/project.git".into(),
            push_url: "https://git.example.test/team/project.git".into(),
            parsed: parse_remote_url("https://git.example.test/team/project.git"),
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        },
        RemoteProfile {
            name: "upstream".into(),
            fetch_url: "https://gitlab.com/upstream/project.git".into(),
            push_url: "https://gitlab.com/upstream/project.git".into(),
            parsed: parse_remote_url("https://gitlab.com/upstream/project.git"),
            endpoint_id: Some("gitlab.com".into()),
            code_host_kind: CodeHostKind::GitLab,
        },
    ];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://gitlab.com/upstream/project")
    );
}

#[test]
fn repository_web_url_falls_back_when_origin_has_an_empty_repository_name() {
    let endpoints = CodeHostEndpoint::builtin();
    let remotes = vec![
        RemoteProfile {
            name: "origin".into(),
            fetch_url: "git@github.com:team/.git".into(),
            push_url: "git@github.com:team/.git".into(),
            parsed: parse_remote_url("git@github.com:team/.git"),
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        },
        RemoteProfile {
            name: "upstream".into(),
            fetch_url: "https://gitlab.com/upstream/project.git".into(),
            push_url: "https://gitlab.com/upstream/project.git".into(),
            parsed: parse_remote_url("https://gitlab.com/upstream/project.git"),
            endpoint_id: Some("gitlab.com".into()),
            code_host_kind: CodeHostKind::GitLab,
        },
    ];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://gitlab.com/upstream/project")
    );
}

#[test]
fn repository_web_url_falls_back_when_origin_has_an_empty_path_segment() {
    let endpoints = CodeHostEndpoint::builtin();
    let remotes = vec![
        RemoteProfile {
            name: "origin".into(),
            fetch_url: "git@github.com:team//project.git".into(),
            push_url: "git@github.com:team//project.git".into(),
            parsed: parse_remote_url("git@github.com:team//project.git"),
            endpoint_id: Some("github.com".into()),
            code_host_kind: CodeHostKind::GitHub,
        },
        RemoteProfile {
            name: "upstream".into(),
            fetch_url: "https://gitlab.com/upstream/project.git".into(),
            push_url: "https://gitlab.com/upstream/project.git".into(),
            parsed: parse_remote_url("https://gitlab.com/upstream/project.git"),
            endpoint_id: Some("gitlab.com".into()),
            code_host_kind: CodeHostKind::GitLab,
        },
    ];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://gitlab.com/upstream/project")
    );
}

#[test]
fn repository_web_url_preserves_encoded_segments_and_endpoint_base_path() {
    let endpoints = vec![CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GitLab,
        display_name: "Example GitLab".into(),
        web_url: "https://git.example.test/code/".into(),
        api_url: "https://git.example.test/code/api/v4".into(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    }];
    let remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: "ssh://git@git.example.test/team/platform%20tools/project%20name.git".into(),
        push_url: "ssh://git@git.example.test/team/platform%20tools/project%20name.git".into(),
        parsed: parse_remote_url(
            "ssh://git@git.example.test/team/platform%20tools/project%20name.git",
        ),
        endpoint_id: Some("git.example.test".into()),
        code_host_kind: CodeHostKind::GitLab,
    }];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://git.example.test/code/team/platform%20tools/project%20name")
    );
}

#[test]
fn repository_web_url_encodes_raw_path_segments() {
    let endpoints = vec![CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GitLab,
        display_name: "Example GitLab".into(),
        web_url: "https://git.example.test".into(),
        api_url: "https://git.example.test/api/v4".into(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    }];
    let remote_url = "git@git.example.test:team/platform tools/project name.git";
    let remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: remote_url.into(),
        push_url: remote_url.into(),
        parsed: parse_remote_url(remote_url),
        endpoint_id: Some("git.example.test".into()),
        code_host_kind: CodeHostKind::GitLab,
    }];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://git.example.test/team/platform%20tools/project%20name")
    );
}

#[test]
fn repository_web_url_preserves_percent_encoded_non_utf8_bytes() {
    let endpoints = CodeHostEndpoint::builtin();
    let remote_url = "ssh://git@github.com/team/project%FF.git";
    let remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: remote_url.into(),
        push_url: remote_url.into(),
        parsed: parse_remote_url(remote_url),
        endpoint_id: Some("github.com".into()),
        code_host_kind: CodeHostKind::GitHub,
    }];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://github.com/team/project%FF")
    );
}

#[test]
fn repository_web_url_rejects_encoded_dot_segments() {
    let endpoints = vec![CodeHostEndpoint {
        id: "git.example.test".into(),
        kind: CodeHostKind::GitLab,
        display_name: "Example GitLab".into(),
        web_url: "https://git.example.test/code/".into(),
        api_url: "https://git.example.test/code/api/v4".into(),
        git_host: "git.example.test".into(),
        is_builtin: false,
    }];
    let remote_url = "ssh://git@git.example.test/team/%2E%2E/project.git";
    let remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: remote_url.into(),
        push_url: remote_url.into(),
        parsed: parse_remote_url(remote_url),
        endpoint_id: Some("git.example.test".into()),
        code_host_kind: CodeHostKind::GitLab,
    }];

    assert_eq!(repository_web_url_from_remotes(&remotes, &endpoints), None);
}

#[test]
fn repository_web_url_encodes_remote_backslashes() {
    let endpoints = CodeHostEndpoint::builtin();
    let remote_url = r"git@github.com:team\other/project.git";
    let remotes = vec![RemoteProfile {
        name: "origin".into(),
        fetch_url: remote_url.into(),
        push_url: remote_url.into(),
        parsed: parse_remote_url(remote_url),
        endpoint_id: Some("github.com".into()),
        code_host_kind: CodeHostKind::GitHub,
    }];

    assert_eq!(
        repository_web_url_from_remotes(&remotes, &endpoints).as_deref(),
        Some("https://github.com/team%5Cother/project")
    );
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
fn repository_alias_validation_trims_and_rejects_control_characters() {
    assert_eq!(
        normalize_repository_alias(Some("  客户端主仓  ".into())).unwrap(),
        Some("客户端主仓".into())
    );
    assert_eq!(
        normalize_repository_alias(Some("   ".into())).unwrap(),
        None
    );
    assert_eq!(
        normalize_repository_alias(Some("客户端\n主仓".into()))
            .expect_err("line breaks are not valid repository aliases")
            .code(),
        "repository_alias_invalid"
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
fn scp_style_binding_url_keeps_path_characters_out_of_url_semantics() {
    let endpoint = CodeHostEndpoint::builtin()
        .into_iter()
        .find(|endpoint| endpoint.id == "github.com")
        .expect("GitHub endpoint is built in");
    for remote_url in [
        "git@github.com:example/project.git?release=archive",
        "git@github.com:example/project.git#archive",
    ] {
        assert!(
            validate_codehost_binding_url(remote_url, &endpoint).is_ok(),
            "SCP-style SSH paths do not have URI query or fragment semantics"
        );
    }
}

#[test]
fn remote_binding_requires_all_effective_operation_urls_to_match() {
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

    assert!(remote_binding_matches_all_urls(
        &store,
        repository_path,
        "origin",
        &["https://github.com/example/project.git".into()],
        &["git@github.com:example/project.git".into()],
    ));
    assert!(!remote_binding_matches_all_urls(
        &store,
        repository_path,
        "origin",
        &["https://gitlab.com/example/project.git".into()],
        &["https://github.com/example/project.git".into()],
    ));
    assert!(!remote_binding_matches_all_urls(
        &store,
        repository_path,
        "origin",
        &["https://github.com/example/project.git".into()],
        &[
            "https://github.com/example/project.git".into(),
            "https://gitlab.com/example/project.git".into(),
        ],
    ));
}

#[test]
fn codehost_adapter_requires_all_operation_urls_to_match_endpoint() {
    let endpoint = CodeHostEndpoint::builtin()
        .into_iter()
        .find(|endpoint| endpoint.id == "github.com")
        .expect("GitHub endpoint is built in");

    assert!(codehost_adapter_urls_match_endpoint(
        &["https://github.com/example/project.git".into()],
        &["git@github.com:example/project.git".into()],
        &endpoint,
    ));
    assert!(!codehost_adapter_urls_match_endpoint(
        &[
            "https://github.com/example/project.git".into(),
            "https://gitlab.com/example/project.git".into(),
        ],
        &["git@github.com:example/project.git".into()],
        &endpoint,
    ));
    assert!(!codehost_adapter_urls_match_endpoint(
        &["https://github.com/example/project.git".into()],
        &[
            "git@github.com:example/project.git".into(),
            "git@gitlab.com:example/project.git".into(),
        ],
        &endpoint,
    ));
}

#[test]
fn pending_publish_identity_requires_the_expected_github_slug() {
    assert_eq!(
        github_publish_expected_slug("octo", "project").unwrap(),
        "octo/project"
    );
    assert_eq!(
        github_publish_expected_slug("team", "project").unwrap(),
        "team/project"
    );
    assert!(github_publish_expected_slug("..", "project").is_err());
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
fn pending_clone_binding_rejects_additional_cross_endpoint_operation_urls() {
    let endpoint = CodeHostEndpoint::builtin()
        .into_iter()
        .find(|endpoint| endpoint.id == "github.com")
        .expect("GitHub endpoint is built in");
    let fetch_urls = vec!["https://github.com/octo/project.git".into()];
    let push_urls = vec![
        "git@github.com:octo/project.git".into(),
        "git@gitlab.com:octo/project.git".into(),
    ];

    assert!(!pending_clone_operation_urls_match(
        &fetch_urls,
        &push_urls,
        &endpoint,
        "github.com",
        "octo/project",
    ));
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
fn pending_publish_reconciliation_requires_the_original_identity() {
    let pending = PendingPublish {
        repository_path: "/work/project".into(),
        endpoint_id: "github.com".into(),
        account_profile_id: "account-a".into(),
        remote_name: "origin".into(),
        expected_head: "a".repeat(40),
        expected_repository_slug: "owner/project".into(),
    };

    assert!(pending_publish_matches_requested_identity(
        &pending,
        "github.com",
        "account-a",
        "owner/project"
    ));
    assert!(!pending_publish_matches_requested_identity(
        &pending,
        "github.com",
        "account-b",
        "owner/project"
    ));
    assert!(!pending_publish_matches_requested_identity(
        &pending,
        "github.example.test",
        "account-a",
        "owner/project"
    ));
    assert!(!pending_publish_matches_requested_identity(
        &pending,
        "github.com",
        "account-a",
        "another/project"
    ));
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
