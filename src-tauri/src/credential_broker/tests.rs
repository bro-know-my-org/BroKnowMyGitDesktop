#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> CodeHostEndpoint {
        CodeHostEndpoint {
            id: "gitee.com".into(),
            kind: CodeHostKind::Gitee,
            display_name: "Gitee.com".into(),
            web_url: "https://gitee.com".into(),
            api_url: "https://gitee.com/api/v5".into(),
            git_host: "gitee.com".into(),
            is_builtin: true,
        }
    }

    fn account(
        id: &str,
        auth_method: AuthenticationMethod,
        status: AuthenticationStatus,
        is_default_candidate: bool,
    ) -> AccountProfile {
        let secret_ref =
            matches!(&auth_method, AuthenticationMethod::PersonalAccessToken).then(|| {
                KeychainReference {
                    service: PAT_KEYCHAIN_SERVICE.into(),
                    account: id.into(),
                }
            });
        AccountProfile {
            id: id.into(),
            endpoint_id: "gitee.com".into(),
            login: format!("{id}-login"),
            display_name: None,
            auth_method,
            secret_ref,
            status,
            is_default_candidate,
        }
    }

    fn resolve(
        endpoints: &[CodeHostEndpoint],
        bindings: &[RemoteBinding],
        accounts: &[AccountProfile],
        remote_url: &str,
    ) -> Result<CredentialResolution, AppError> {
        CredentialBroker.resolve(CredentialResolutionRequest {
            repository_path: "/work/demo/",
            remote_name: "origin",
            remote_url,
            endpoint: None,
            endpoints,
            bindings,
            accounts,
        })
    }

    #[test]
    fn explicit_binding_wins_over_default_candidate() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::SystemGitCredentialHelper,
                AuthenticationStatus::Valid,
                true,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                false,
            ),
        ];
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "gitee.com".into(),
            account_selection: AccountSelection::Profile("work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("explicit binding should resolve");

        assert_eq!(resolved.account_profile_id, "work");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass {
                ref account_profile_id,
                ..
            } if account_profile_id == "work"
        ));
    }

    #[test]
    fn auto_uses_its_only_usable_candidate() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "expired",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::NeedsReauthentication,
                false,
            ),
            account(
                "usable",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Available,
                false,
            ),
        ];

        let resolved = resolve(
            &endpoints,
            &[],
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("only one usable account should resolve");

        assert_eq!(resolved.account_profile_id, "usable");
    }

    #[test]
    fn auto_uses_the_only_valid_default_when_multiple_accounts_exist() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::SystemGitCredentialHelper,
                AuthenticationStatus::Valid,
                false,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                true,
            ),
        ];
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "gitee.com".into(),
            account_selection: AccountSelection::Auto,
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect("one valid default candidate should resolve");

        assert_eq!(resolved.account_profile_id, "work");
    }

    #[test]
    fn auto_never_guesses_between_multiple_candidates() {
        let endpoints = vec![endpoint()];
        let accounts = vec![
            account(
                "personal",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Valid,
                false,
            ),
            account(
                "work",
                AuthenticationMethod::PersonalAccessToken,
                AuthenticationStatus::Available,
                false,
            ),
        ];

        let error = resolve(
            &endpoints,
            &[],
            &accounts,
            "https://gitee.com/team/project.git",
        )
        .expect_err("ambiguous auto selection must fail");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn ssh_agent_profiles_are_not_pinnable_to_remote_bindings() {
        let endpoints = vec![endpoint()];
        let accounts = vec![account(
            "ssh-account",
            AuthenticationMethod::SshAgent,
            AuthenticationStatus::Valid,
            false,
        )];

        let error = resolve(
            &endpoints,
            &[RemoteBinding {
                repository_path: "/work/demo".into(),
                remote_name: "origin".into(),
                endpoint_id: "gitee.com".into(),
                account_selection: AccountSelection::Profile("ssh-account".into()),
            }],
            &accounts,
            "git@gitee.com:team/project.git",
        )
        .expect_err("an SSH agent profile cannot prove a selected identity");
        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn credential_resolution_rejects_plain_http() {
        let endpoints = vec![endpoint()];
        let accounts = vec![account(
            "helper-account",
            AuthenticationMethod::SystemGitCredentialHelper,
            AuthenticationStatus::Valid,
            false,
        )];

        let error = resolve(
            &endpoints,
            &[],
            &accounts,
            "http://gitee.com/team/project.git",
        )
        .expect_err("credentials must never be sent over plain HTTP");
        assert_eq!(error.code(), "unsupported_remote_transport");
    }

    #[test]
    fn credential_url_validation_rejects_token_bearing_queries_and_fragments() {
        assert_eq!(
            validate_remote_url_for_credentials(
                "https://gitee.com/team/project.git?access_token=hidden",
                &RemoteTransport::Https,
            )
            .expect_err("HTTPS remote queries can retain tokens")
            .code(),
            "unsafe_remote_url"
        );
        assert_eq!(
            validate_remote_url_for_credentials(
                "ssh://git@gitee.com/team/project.git#token",
                &RemoteTransport::Ssh,
            )
            .expect_err("SSH URL fragments are not repository paths")
            .code(),
            "unsafe_remote_url"
        );
        assert_eq!(
            validate_remote_url_for_credentials(
                "git@gitee.com:team/project.git?access_token=hidden",
                &RemoteTransport::Ssh,
            )
            .expect_err("SCP-style SSH remotes cannot retain token-like query data")
            .code(),
            "unsafe_remote_url"
        );
    }

    #[test]
    fn generic_git_host_resolves_an_explicit_https_pat_binding() {
        let endpoints = vec![CodeHostEndpoint {
            id: "git.example.test".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test".into(),
            is_builtin: false,
        }];
        let mut profile = account(
            "example-work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile.endpoint_id = "git.example.test".into();
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "git.example.test".into(),
            account_selection: AccountSelection::Profile("example-work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &[profile],
            "https://git.example.test/team/project.git",
        )
        .expect("a generic Git endpoint should support HTTPS PAT authentication");

        assert_eq!(resolved.endpoint_id, "git.example.test");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass { .. }
        ));
    }

    #[test]
    fn non_default_endpoint_port_requires_an_exact_remote_and_keeps_its_askpass_authority() {
        let endpoints = vec![CodeHostEndpoint {
            id: "git.example.test-8443".into(),
            kind: CodeHostKind::GenericGit,
            display_name: "Example Git".into(),
            web_url: String::new(),
            api_url: String::new(),
            git_host: "git.example.test:8443".into(),
            is_builtin: false,
        }];
        let mut profile = account(
            "example-work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile.endpoint_id = "git.example.test-8443".into();
        let bindings = vec![RemoteBinding {
            repository_path: "/work/demo".into(),
            remote_name: "origin".into(),
            endpoint_id: "git.example.test-8443".into(),
            account_selection: AccountSelection::Profile("example-work".into()),
        }];

        let resolved = resolve(
            &endpoints,
            &bindings,
            &[profile.clone()],
            "https://git.example.test:8443/team/project.git",
        )
        .expect("matching non-default port should resolve");
        assert!(matches!(
            resolved.strategy,
            CredentialStrategy::KeychainPatAskPass { ref authority, .. }
                if authority == "git.example.test:8443"
        ));

        let error = resolve(
            &endpoints,
            &bindings,
            &[profile],
            "https://git.example.test/team/project.git",
        )
        .expect_err("a port-scoped binding must not apply to the default port");
        assert_eq!(error.code(), "endpoint_association_required");
    }

    #[test]
    fn askpass_environment_contains_only_a_one_shot_capability() {
        let strategy = CredentialStrategy::KeychainPatAskPass {
            account_profile_id: "work".into(),
            keychain_ref: KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            authority: "gitee.com:8443".into(),
            username: "work-login".into(),
        };

        let environment = CredentialEnvironment::from_strategy(&strategy).unwrap();
        let pairs = environment
            .environment_pairs(Some(Path::new("/trusted/bkm-askpass")))
            .unwrap();
        let rendered = pairs
            .iter()
            .map(|(_, value)| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join("|");

        assert!(!rendered.contains(PAT_KEYCHAIN_SERVICE));
        assert!(!rendered.contains("not-a-personal-access-token"));
        assert!(pairs.iter().all(|(key, _)| key != "GIT_PASSWORD"));
        assert!(pairs
            .iter()
            .any(|(key, value)| { key == ASKPASS_MODE_ENV && value == ASKPASS_MODE_KEYCHAIN }));
        let capability_id = pairs
            .iter()
            .find(|(key, _)| key == ASKPASS_CAPABILITY_ENV)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .expect("askpass capability is present");
        let capability = AskPassCapability {
            path: askpass_capability_directory().unwrap().join(&capability_id),
            id: capability_id,
        };
        let restored = capability
            .load_configuration(true)
            .expect("capability restores the selected askpass configuration");
        assert!(matches!(
            restored.source,
            AskPassSecretSource::Keychain { authority, .. } if authority == "gitee.com:8443"
        ));
    }

    #[test]
    fn cli_profiles_are_not_repurposed_as_git_https_credentials() {
        let endpoint = CodeHostEndpoint {
            id: "github.com".into(),
            kind: CodeHostKind::GitHub,
            display_name: "GitHub.com".into(),
            web_url: "https://github.com".into(),
            api_url: "https://api.github.com".into(),
            git_host: "github.com".into(),
            is_builtin: true,
        };
        let profile = AccountProfile {
            id: "octo".into(),
            endpoint_id: endpoint.id.clone(),
            login: "octo".into(),
            display_name: None,
            auth_method: AuthenticationMethod::GhCli,
            secret_ref: None,
            status: AuthenticationStatus::Valid,
            is_default_candidate: true,
        };

        assert!(strategy_for_account(&profile, &endpoint, &RemoteTransport::Https).is_none());
    }

    #[test]
    fn system_helper_profiles_are_not_explicit_git_credential_candidates() {
        let endpoint = endpoint();
        let profile = account(
            "work",
            AuthenticationMethod::SystemGitCredentialHelper,
            AuthenticationStatus::Valid,
            true,
        );

        assert!(strategy_for_account(&profile, &endpoint, &RemoteTransport::Https).is_none());
        assert!(!CredentialBroker::has_explicit_account_candidate(
            &[profile],
            &endpoint,
            &RemoteTransport::Https,
        ));
    }

    #[test]
    fn askpass_capability_rejects_a_rebound_keychain_account() {
        let configuration = AskPassCapabilityConfiguration {
            username: "work-login".into(),
            source: AskPassCapabilitySource::Keychain {
                account_profile_id: "work".into(),
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "personal".into(),
                authority: "gitee.com".into(),
            },
        };

        let error = match AskPassConfig::from_capability_configuration(configuration) {
            Ok(_) => panic!("a capability must not read another profile's PAT"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "invalid_askpass_configuration");
    }

    #[test]
    fn askpass_capability_is_consumed_by_one_password_prompt() {
        let configuration = AskPassConfig::for_keychain(
            "work".into(),
            KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            "gitee.com".into(),
            "work-login".into(),
        )
        .unwrap();
        let capability = AskPassCapability::create(&configuration).unwrap();

        assert!(capability.load_configuration(false).is_ok());
        assert!(capability.load_configuration(true).is_ok());
        let error = match capability.load_configuration(true) {
            Ok(_) => panic!("a consumed capability must not be replayed"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "askpass_capability_invalid");
    }

    #[test]
    fn platform_cli_pipe_permit_can_release_before_detached_workers_finish() {
        let before = ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire);
        let permit = PlatformCliPipeOperationPermit::acquire().expect("a test permit is available");
        let worker = Arc::clone(&permit);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );

        drop(permit);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before + 1
        );
        worker.release();
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before
        );
        drop(worker);
        assert_eq!(
            ACTIVE_PLATFORM_CLI_PIPE_OPERATIONS.load(Ordering::Acquire),
            before
        );
    }

    #[cfg(unix)]
    #[test]
    fn platform_cli_runner_drains_stdout_and_stderr_while_the_child_runs() {
        let output = run_platform_cli_program(
            OsStr::new("sh"),
            &[
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
            ],
        )
        .expect("large concurrent CLI output must not deadlock the runner");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131_072);
        assert_eq!(output.stderr.len(), 131_072);
    }

    #[test]
    fn tampered_profile_cannot_target_another_keychain_service() {
        let endpoints = vec![endpoint()];
        let mut profile = account(
            "work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile
            .secret_ref
            .as_mut()
            .expect("PAT profile has a reference")
            .service = "unrelated.system.service".into();

        let error = resolve(
            &endpoints,
            &[],
            &[profile],
            "https://gitee.com/team/project.git",
        )
        .expect_err("only application-owned keychain entries may be used");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn tampered_profile_cannot_target_another_profile_pat() {
        let endpoints = vec![endpoint()];
        let mut profile = account(
            "work",
            AuthenticationMethod::PersonalAccessToken,
            AuthenticationStatus::Valid,
            false,
        );
        profile
            .secret_ref
            .as_mut()
            .expect("PAT profile has a reference")
            .account = "personal".into();

        let error = resolve(
            &endpoints,
            &[],
            &[profile],
            "https://gitee.com/team/project.git",
        )
        .expect_err("a profile may only resolve its own keychain reference");

        assert_eq!(error.code(), "account_selection_required");
    }

    #[test]
    fn askpass_only_accepts_a_matching_https_prompt_authority() {
        let configuration = AskPassConfig::for_keychain(
            "work".into(),
            KeychainReference {
                service: PAT_KEYCHAIN_SERVICE.into(),
                account: "work".into(),
            },
            "gitee.com:8443".into(),
            "work-login".into(),
        )
        .unwrap();
        assert_eq!(
            askpass_prompt_authority("Password for 'https://gitee.com:8443':").unwrap(),
            "gitee.com:8443"
        );
        assert_eq!(
            askpass_username_for_prompt(&configuration, "gitee.com:8443").unwrap(),
            "work-login"
        );
        assert_eq!(
            askpass_username_for_prompt(&configuration, "gitee.com:9443")
                .unwrap_err()
                .code(),
            "askpass_prompt_host_mismatch"
        );
        assert_eq!(
            askpass_password_prompt_authority(
                "Password for 'https://work-login@gitee.com:8443/team/project.git':",
                "work-login",
            )
            .unwrap(),
            "gitee.com:8443"
        );
        assert!(askpass_password_prompt_authority(
            "Password for 'https://other-login@gitee.com:8443/team/project.git':",
            "work-login",
        )
        .is_err());
        assert!(askpass_prompt_authority(
            "Username for 'https://work-login@gitee.com:8443/team/project.git':"
        )
        .is_err());
        assert!(askpass_prompt_authority("Password for gitee.com:").is_err());
    }

    #[test]
    fn askpass_prompt_authority_keeps_bracketed_ipv6_ports() {
        let authority =
            askpass_prompt_authority("Password for 'https://[2001:db8::1]:8443/team/project.git':")
                .expect("IPv6 HTTPS prompt parses");

        assert_eq!(authority, "[2001:db8::1]:8443");
        assert_eq!(
            hostname_from_git_authority(&authority).as_deref(),
            Some("2001:db8::1")
        );
        assert!(is_safe_git_authority(&authority));
    }
}
