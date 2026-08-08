#[test]
fn builds_repository_command_arguments_without_a_shell() {
    let arguments = command_arguments(
        Some(Path::new("/workspace/repository with spaces")),
        &os_arguments(&["status", "--porcelain=v2", "-z"]),
    );
    assert_eq!(
        as_strings(&arguments),
        vec![
            "-C",
            "/workspace/repository with spaces",
            "status",
            "--porcelain=v2",
            "-z",
        ]
    );
}

#[test]
fn normalizes_external_editor_arguments_without_shell_parsing() {
    let executable = std::env::current_exe().expect("test executable path is available");
    let canonical_executable = fs::canonicalize(&executable).expect("test executable is canonical");
    let editor = normalize_external_editor_config(ExternalEditorConfig {
        executable: format!("  {}  ", executable.to_string_lossy()),
        arguments: vec![" --wait ".into(), "--reuse-window".into()],
    })
    .expect("editor configuration is valid");
    assert_eq!(
        editor.executable,
        canonical_executable.to_string_lossy().into_owned()
    );
    assert_eq!(editor.arguments, vec!["--wait", "--reuse-window"]);

    assert_eq!(
        normalize_external_editor_config(ExternalEditorConfig {
            executable: "code".into(),
            arguments: Vec::new(),
        })
        .expect_err("a mutable PATH lookup is not an editor identity")
        .code(),
        "external_editor_config_invalid"
    );

    let error = normalize_external_editor_config(ExternalEditorConfig {
        executable: "code\n--malicious".into(),
        arguments: Vec::new(),
    })
    .expect_err("control characters are not a command separator");
    assert_eq!(error.code(), "external_editor_config_invalid");
}

#[test]
fn recognizes_every_unmerged_porcelain_status() {
    for (index_status, worktree_status) in [
        ("D", "D"),
        ("A", "U"),
        ("U", "D"),
        ("U", "A"),
        ("D", "U"),
        ("A", "A"),
        ("U", "U"),
    ] {
        assert!(is_conflicted_status_entry(&GitStatusEntry {
            path: "conflicted.txt".into(),
            original_path: None,
            index_status: index_status.into(),
            worktree_status: worktree_status.into(),
        }));
    }
    assert!(!is_conflicted_status_entry(&GitStatusEntry {
        path: "changed.txt".into(),
        original_path: None,
        index_status: "M".into(),
        worktree_status: "M".into(),
    }));
}

#[test]
fn git_children_replace_inherited_credential_environment() {
    let environment = GitEnvironment::new().with_variable("BKM_GIT_ASKPASS_MODE", "trusted");
    let mut command = Command::new("git");
    command.env("GIT_ASKPASS", "/untrusted/askpass");
    command.env("GIT_CONFIG_COUNT", "1");
    command.env("GIT_TEMPLATE_DIR", "/untrusted/template");
    command.env("GIT_DIR", "/untrusted/git-dir");
    command.env("GIT_EXEC_PATH", "/untrusted/git-exec");
    command.env("GIT_SSL_NO_VERIFY", "1");
    command.env("GIT_SSL_CAINFO", "/untrusted/ca.pem");
    command.env("GIT_SSL_CAPATH", "/untrusted/ca-directory");
    apply_git_environment(&mut command, &environment);
    let values: std::collections::HashMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
        .collect();

    assert_eq!(values.get(OsStr::new("GIT_ASKPASS")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_CONFIG_COUNT")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_TEMPLATE_DIR")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_DIR")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_EXEC_PATH")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_SSL_NO_VERIFY")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_SSL_CAINFO")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_SSL_CAPATH")), Some(&None));
    assert_eq!(
        values.get(OsStr::new("GIT_TERMINAL_PROMPT")),
        Some(&Some(OsString::from("0")))
    );
    assert_eq!(
        values.get(OsStr::new("BKM_GIT_ASKPASS_MODE")),
        Some(&Some(OsString::from("trusted")))
    );
    assert_eq!(
        values.get(OsStr::new("LC_ALL")),
        Some(&Some(OsString::from("C")))
    );
    assert_eq!(
        values.get(OsStr::new("LANG")),
        Some(&Some(OsString::from("C")))
    );
}

#[cfg(unix)]
#[test]
fn status_disables_optional_index_locks() {
    use std::os::unix::fs::PermissionsExt;

    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-status-optional-lock-test"),
    );
    let fake_git = root.join("fake-git.sh");
    fs::write(
        &fake_git,
        "#!/bin/sh\n[ \"$GIT_OPTIONAL_LOCKS\" = \"0\" ] || exit 9\nprintf '%s\\n' \"$@\" | grep -qx -- status || exit 10\nprintf '%s\\n' \"$@\" | grep -qx -- --porcelain=v2 || exit 11\nprintf '%s\\n' \"$@\" | grep -qx -- -z || exit 12\nprintf '%s\\n' \"$@\" | grep -qx -- --branch || exit 13\nprintf '# branch.oid 0123456789abcdef0123456789abcdef01234567\\n# branch.head main\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    let status = GitCore::with_executable(fake_git).status(&root).unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
}

#[test]
fn explicit_credentials_clear_repository_helper_configuration() {
    let mut environment = GitEnvironment::new();
    environment.isolate_network_credentials();
    let mut command = Command::new("git");
    apply_git_environment(&mut command, &environment);
    let values: std::collections::HashMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
        .collect();

    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_COUNT")),
        Some(&Some(OsString::from("19")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_KEY_0")),
        Some(&Some(OsString::from("credential.helper")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_VALUE_0")),
        Some(&Some(OsString::new()))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_NOSYSTEM")),
        Some(&Some(OsString::from("1")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_GLOBAL")),
        Some(&Some(OsString::from(DISABLED_GIT_HOOKS_PATH)))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_KEY_6")),
        Some(&Some(OsString::from("core.hooksPath")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_KEY_18")),
        Some(&Some(OsString::from("protocol.ext.allow")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_VALUE_18")),
        Some(&Some(OsString::from("never")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_ALLOW_PROTOCOL")),
        Some(&Some(OsString::from("http:https:ssh:git:git+ssh:file")))
    );
}

#[test]
fn askpass_credentials_disable_hooks_before_git_spawns() {
    let mut environment = GitEnvironment::new();
    environment.isolate_network_credentials_and_disable_hooks();
    let mut command = Command::new("git");
    apply_git_environment(&mut command, &environment);
    let values: std::collections::HashMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
        .collect();

    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_COUNT")),
        Some(&Some(OsString::from("19")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_KEY_6")),
        Some(&Some(OsString::from("core.hooksPath")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_VALUE_6")),
        Some(&Some(OsString::from(DISABLED_GIT_HOOKS_PATH)))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_VALUE_18")),
        Some(&Some(OsString::from("never")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_SSH_COMMAND")),
        Some(&Some(OsString::from("ssh")))
    );
}

#[test]
fn git_error_classification_inspects_a_bounded_diagnostic_tail() {
    let status = Command::new("git")
        .arg("--version")
        .output()
        .expect("Git is available for GitCore tests")
        .status;
    let mut stderr = vec![b'x'; 64 * 1024];
    stderr.extend_from_slice(b" authentication failed");
    let error = git_command_error(
        "测试错误分类",
        &Output {
            status,
            stdout: Vec::new(),
            stderr,
        },
    );
    assert_eq!(error.code(), "git_authentication_failed");
}

#[test]
fn noninteractive_editor_uses_the_packaged_binary_not_a_path_utility() {
    let environment = noninteractive_git_editor_environment().unwrap();
    let mut command = Command::new("git");
    apply_git_environment(&mut command, &environment);
    let editor = command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new("GIT_EDITOR"))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
        .expect("the fixed Git editor is present");
    assert!(editor.starts_with('\''));
    assert!(editor.ends_with(GIT_NOOP_EDITOR_ARGUMENT));
    assert!(!editor.contains(" true"));
}

#[test]
fn builds_positionally_pinned_fetch_arguments() {
    let arguments = super::network::fetch_arguments_for_url(
        &FetchOptions {
            remote: "origin".into(),
            refspecs: vec!["refs/heads/main:refs/remotes/origin/main".into()],
            prune: true,
        },
        "https://github.com/example/project.git",
    )
    .unwrap();
    assert_eq!(
        as_strings(&arguments),
        vec![
            "fetch",
            "--no-recurse-submodules",
            "--prune",
            "https://github.com/example/project.git",
            "refs/heads/main:refs/remotes/origin/main",
        ]
    );
}

#[test]
fn repository_config_values_include_worktree_scope_but_not_inherited_scopes() {
    assert_eq!(
        super::network::parse_repository_config_values(
            b"system\0system-fetch\0global\0global-fetch\0local\0local-fetch\0worktree\0worktree-fetch\0",
        )
        .unwrap(),
        vec!["local-fetch", "worktree-fetch"]
    );
}

#[test]
fn pinned_fetch_requires_a_captured_refspec() {
    assert_eq!(
        super::network::fetch_arguments_for_url(
            &FetchOptions {
                remote: "origin".into(),
                refspecs: Vec::new(),
                prune: true,
            },
            "https://github.com/example/project.git",
        )
        .expect_err("a raw URL must not fall back to the remote default branch")
        .code(),
        "fetch_refspec_required"
    );
}

#[test]
fn builds_positionally_pinned_pull_arguments_for_the_configured_upstream() {
    let arguments = super::network::pull_arguments_for_url(
        &PullOptions {
            remote: "origin".into(),
            refspec: Some(
                "+refs/heads/feature:refs/remotes/origin/feature".into(),
            ),
        },
        "https://github.com/example/project.git",
    )
    .expect("an explicit upstream refspec is valid");
    assert_eq!(
        as_strings(&arguments),
        vec![
            "pull",
            "--no-edit",
            "--no-recurse-submodules",
            "https://github.com/example/project.git",
            "+refs/heads/feature:refs/remotes/origin/feature",
        ]
    );
    assert_eq!(
        super::network::pull_arguments_for_url(
            &PullOptions {
                remote: "origin".into(),
                refspec: None,
            },
            "https://github.com/example/project.git",
        )
        .expect_err("a pinned URL must never fall back to its default branch")
        .code(),
        "pull_upstream_not_configured"
    );
}

#[test]
fn resolves_the_current_branch_upstream_before_pinning_a_pull_url() {
    let repository = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-pull-upstream-test"),
    );
    let repository_path = repository
        .to_str()
        .expect("test repository path is valid UTF-8");
    for arguments in [
        vec!["-C", repository_path, "init", "-q"],
        vec!["-C", repository_path, "config", "user.name", "Test User"],
        vec![
            "-C",
            repository_path,
            "config",
            "user.email",
            "test@example.invalid",
        ],
        vec![
            "-C",
            repository_path,
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
        vec!["-C", repository_path, "branch", "-M", "feature"],
        vec![
            "-C",
            repository_path,
            "remote",
            "add",
            "origin",
            "https://example.invalid/project.git",
        ],
        vec![
            "-C",
            repository_path,
            "remote",
            "add",
            "upstream",
            "https://example.invalid/upstream.git",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "extensions.worktreeConfig",
            "true",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "--unset-all",
            "remote.origin.fetch",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "--worktree",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin-worktree/*",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "--worktree",
            "--add",
            "remote.origin.fetch",
            "^refs/heads/private",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "branch.feature.remote",
            "origin",
        ],
        vec![
            "-C",
            repository_path,
            "config",
            "branch.feature.merge",
            "refs/heads/feature",
        ],
    ] {
        assert!(
            test_git_command()
                .args(arguments)
                .status()
                .expect("test Git setup command runs")
                .success(),
            "test Git setup succeeds"
        );
    }

    let git = GitCore::new();
    assert_eq!(
        git.fetch_refspecs_for_remote(&repository, "origin")
            .expect("worktree-scoped exclusions remain part of the pinned fetch"),
        vec![
            "+refs/heads/*:refs/remotes/origin/*",
            "^refs/heads/private",
        ]
    );
    assert_eq!(
        git.pull_refspec_for_remote(&repository, "origin")
            .expect("the tracked branch produces its configured merge ref"),
        "+refs/heads/feature:refs/remotes/origin-worktree/feature"
    );
    assert_eq!(
        git.pull_refspec_for_remote(&repository, "upstream")
            .expect_err("a selected remote must match the current branch upstream")
            .code(),
        "pull_upstream_remote_mismatch"
    );
}

#[test]
fn url_pinned_pull_updates_a_custom_remote_tracking_branch_and_push_state() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-pull-tracking-test"),
    );
    let remote = root.join("remote.git");
    let source = root.join("source");
    let repository = root.join("repository");
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };

    run_git(&root, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    run_git(&root, &["init", "-q", "-b", "main", source.to_str().unwrap()]);
    run_git(&source, &["config", "user.name", "Pull Test"]);
    run_git(
        &source,
        &["config", "user.email", "pull@example.invalid"],
    );
    run_git(&source, &["commit", "--allow-empty", "-qm", "initial"]);
    run_git(
        &source,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&source, &["push", "-qu", "origin", "main"]);
    run_git(
        &root,
        &[
            "--git-dir",
            remote.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
    run_git(
        &root,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            repository.to_str().unwrap(),
        ],
    );
    run_git(
        &repository,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    run_git(
        &repository,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin-mirror/*",
        ],
    );
    run_git(
        &repository,
        &[
            "update-ref",
            "refs/remotes/origin-mirror/main",
            "refs/remotes/origin/main",
        ],
    );
    run_git(
        &repository,
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    run_git(&source, &["commit", "--allow-empty", "-qm", "upstream"]);
    run_git(&source, &["push", "-q", "origin", "main"]);

    let git = GitCore::new();
    git.pull(
        &repository,
        &PullOptions {
            remote: "origin".into(),
            refspec: None,
        },
    )
    .expect("URL-pinned pull succeeds");

    let head = git.head_commit_id(&repository).unwrap();
    let tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "rev-parse",
            "refs/remotes/origin-mirror/main",
        ])
        .output()
        .unwrap();
    assert!(tracking.status.success());
    assert_eq!(output_text(&tracking.stdout).trim(), head);
    let status = git.status(&repository).unwrap();
    assert_eq!((status.ahead, status.behind), (0, 0));
    let commits = git.commits(&repository, 10).unwrap();
    assert!(!commits.is_empty());
    assert!(commits
        .iter()
        .all(|commit| commit.pushed == Some(true)));
}

#[test]
fn rejects_credential_bearing_https_remote_urls() {
    assert_eq!(
        validate_git_remote_url(" https://gitee.com/team/repository.git ")
            .expect("surrounding whitespace is normalized"),
        "https://gitee.com/team/repository.git"
    );
    assert_eq!(
        validate_git_remote_url("https://gitee.com/team/repository.git\n")
            .expect_err("control characters must not reach Git")
            .code(),
        "invalid_remote_url"
    );
    let error =
        validate_git_remote_url("https://token-value@gitee.com/team/repository.git").unwrap_err();
    assert_eq!(error.code(), "credential_in_remote_url");
    assert!(!error.message().contains("token-value"));
    assert_eq!(
        validate_git_remote_url("https://token-value@%zz/team/repository.git")
            .expect_err("malformed HTTP userinfo must never reach Git config")
            .code(),
        "credential_in_remote_url"
    );
    assert_eq!(
        validate_git_remote_url("https://gitee.com/%zz/team/repository.git")
            .expect_err("malformed HTTP URLs must be rejected")
            .code(),
        "invalid_remote_url"
    );
    assert!(validate_git_remote_url("git@gitee.com:team/repository.git").is_ok());
    assert!(validate_git_remote_url("gitee.com:team/repository.git").is_ok());
    assert!(validate_git_remote_url("git@gitee.com:team/release#archive.git").is_ok());
    assert!(validate_git_remote_url("git@gitee.com:team/release?archive.git").is_ok());
    assert!(validate_git_remote_url("git://token-value@gitee.com/team/repository.git").is_err());
    assert!(validate_git_remote_url("ext::unsafe-helper %S").is_err());
    assert!(validate_git_remote_url("helper://%zz/repository").is_err());
    assert!(validate_git_remote_url("--mirror=fetch").is_err());
}

#[cfg(windows)]
#[test]
fn accepts_windows_absolute_local_remote_paths() {
    assert_eq!(
        validate_git_remote_url(r"C:\repositories\source").unwrap(),
        r"C:\repositories\source"
    );
    assert_eq!(
        validate_git_remote_url(r"\\server\share\source").unwrap(),
        r"\\server\share\source"
    );
}

#[test]
fn network_operations_require_a_configured_remote_name() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-remote-name-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());

    assert_eq!(
        GitCore::new()
            .fetch(
                &repository,
                &FetchOptions {
                    remote: "https://attacker.example/repository.git".into(),
                    refspecs: Vec::new(),
                    prune: false,
                },
            )
            .expect_err("a positional fetch URL is not an API remote name")
            .code(),
        "remote_not_found"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn network_configuration_rejects_executable_extensions() {
    for key in [
        "protocol.ext.allow",
        "filter.payload.process",
        "filter.payload.smudge",
        "filter.payload.clean",
        "core.editor",
        "core.askPass",
        "sequence.editor",
        "core.fsmonitor",
        "merge.payload.driver",
    ] {
        assert!(
            is_unsafe_network_config_key(key),
            "{key} must not run beside an app-selected askpass capability"
        );
    }
}

#[test]
fn repository_hooks_path_is_allowed_for_network_preflight() {
    assert!(!is_unsafe_network_config_key("core.hooksPath"));
}

#[test]
fn stash_operations_only_accept_canonical_stash_references() {
    assert!(validate_stash_reference("stash@{0}").is_ok());
    assert!(validate_stash_reference("stash@{123}").is_ok());
    assert!(validate_stash_reference("HEAD~2").is_err());
    assert!(validate_stash_reference("stash@{-1}").is_err());
    assert!(validate_stash_reference("stash@{name}").is_err());
}

#[test]
fn stash_actions_reject_a_reused_reflog_selector() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-stash-identity-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "stash@example.invalid"]);
    run_git(&["config", "user.name", "Stash Test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-qm", "initial"]);

    let git = GitCore::new();
    fs::write(repository.join("tracked.txt"), "first\n").unwrap();
    git.stash_push(&repository, Some("first"), false).unwrap();
    let first = git.stashes(&repository).unwrap().remove(0);
    fs::write(repository.join("tracked.txt"), "second\n").unwrap();
    git.stash_push(&repository, Some("second"), false).unwrap();

    assert_eq!(
        git.stash_drop(&repository, &first.reference, &first.commit_id)
            .expect_err("a shifted stash selector must not delete the newer stash")
            .code(),
        "stash_changed"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn clone_uses_a_staging_directory_and_rejects_an_existing_destination() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clone-staging-test-{}-{suffix}",
        std::process::id()
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(not(unix))]
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir_all(&source).unwrap();
    let run_git = |repository: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&source, &["init", "-q"]);
    run_git(&source, &["config", "user.email", "clone@example.invalid"]);
    run_git(&source, &["config", "user.name", "Clone Test"]);
    fs::write(source.join("tracked.txt"), "content\n").unwrap();
    run_git(&source, &["add", "tracked.txt"]);
    run_git(&source, &["commit", "-qm", "initial"]);
    fs::create_dir_all(&destination).unwrap();

    let error = GitCore::new()
        .clone_repository(
            source.to_str().unwrap(),
            &destination,
            &CloneOptions::default(),
        )
        .expect_err("an existing destination is never removed during clone");
    assert_eq!(error.code(), "invalid_clone_destination");
    assert!(destination.is_dir());
    fs::remove_dir(&destination).unwrap();

    let cloned = GitCore::new()
        .clone_repository(
            source.to_str().unwrap(),
            &destination,
            &CloneOptions::default(),
        )
        .expect("a missing destination is published from staging atomically");

    assert_eq!(
        PathBuf::from(cloned.path),
        fs::canonicalize(&destination).unwrap()
    );
    assert!(destination.join(".git").exists());
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bkm-clone-")));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn clone_rejects_a_symlinked_destination() {
    use std::os::unix::fs::symlink;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clone-symlink-test-{}-{suffix}",
        std::process::id()
    ));
    let target = root.join("target");
    let destination = root.join("destination");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, &destination).unwrap();

    assert_eq!(
        GitCore::new()
            .clone_repository(
                "https://example.invalid/repository.git",
                &destination,
                &CloneOptions::default(),
            )
            .expect_err("a symlink must never be used as the clone destination")
            .code(),
        "invalid_clone_destination"
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn clone_rejects_a_nonsticky_shared_parent_directory() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clone-shared-parent-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();

    assert_eq!(
        GitCore::new()
            .clone_repository(
                "https://example.invalid/repository.git",
                &root.join("destination"),
                &CloneOptions::default(),
            )
            .expect_err("a non-sticky shared parent cannot retain clone staging safely")
            .code(),
        "invalid_clone_destination"
    );

    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn failed_clone_recursively_removes_its_owned_staging_directory() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clone-failure-cleanup-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_git = root.join("fake-git.sh");
    fs::write(
        &fake_git,
        "#!/bin/sh\nfor argument in \"$@\"; do\n  destination=\"$argument\"\ndone\nmkdir -p \"$destination/.git/objects\"\nprintf failed > \"$destination/.git/objects/partial\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(GitCore::with_executable(&fake_git)
        .clone_repository(
            "https://example.invalid/repository.git",
            &root.join("destination"),
            &CloneOptions::default(),
        )
        .is_err());
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bkm-clone-staging-")));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn init_rejects_a_symlinked_parent_for_a_new_destination() {
    use std::os::unix::fs::symlink;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-init-symlink-test-{}-{suffix}",
        std::process::id()
    ));
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, &link).unwrap();

    assert_eq!(
        GitCore::new()
            .init_repository(&link.join("new-repository"), None)
            .expect_err("a new repository must not traverse a selected symlink")
            .code(),
        "invalid_init_destination"
    );
    assert!(!target.join("new-repository/.git").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn init_publishes_new_and_existing_directories_without_leaving_staging_entries() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-init-staging-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let git = GitCore::new();
    let new_repository = root.join("new-repository");
    let initialized = git
        .init_repository(&new_repository, Some("main"))
        .expect("a new destination is initialized through private staging");
    assert_eq!(initialized.path, new_repository.to_string_lossy());
    assert!(new_repository.join(".git").is_dir());

    let existing_repository = root.join("existing-repository");
    fs::create_dir(&existing_repository).unwrap();
    fs::write(existing_repository.join("README.md"), "preserve me\n").unwrap();
    git.init_repository(&existing_repository, None)
        .expect("an existing non-repository directory is restored after initialization");
    assert!(existing_repository.join(".git").is_dir());
    assert_eq!(
        fs::read_to_string(existing_repository.join("README.md")).unwrap(),
        "preserve me\n"
    );
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".bkm-init-staging-")
    }));

    let _ = fs::remove_dir_all(root);
}
