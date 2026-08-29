#[test]
fn local_credential_helper_configuration_is_rejected_before_network_git() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-unsafe-config-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let configured = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "credential.helper",
            "!echo should-not-run",
        ])
        .status()
        .unwrap();
    assert!(configured.success());

    let error = GitCore::new()
        .ensure_network_configuration_is_safe(&repository)
        .expect_err("repository-local helper must not run for a selected account");
    assert_eq!(error.code(), "unsafe_git_config");
    assert!(error.details().is_some());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn oversized_network_configuration_fails_closed() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-oversized-config-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    assert!(test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap()
        .success());
    let config = repository.join(".git").join("config");
    let mut file = fs::OpenOptions::new().append(true).open(&config).unwrap();
    writeln!(file, "[merge \"oversized\"]").unwrap();
    writeln!(
        file,
        "\tdriver = {}",
        "x".repeat(1024 * 1024 + 1024)
    )
    .unwrap();

    assert_eq!(
        GitCore::new()
            .network_configuration_assessment(&repository)
            .expect_err("oversized repository config must fail closed")
            .code(),
        "git_output_too_large"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn exact_dangerous_configuration_fingerprint_can_be_trusted_until_it_changes() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-test-{}-{suffix}",
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
    run_git(&[
        "config",
        "merge.translation.driver",
        "scripts/merge-translation.sh %O %A %B %P",
    ]);

    let assessment = GitCore::new()
        .network_configuration_assessment(&repository)
        .expect("dangerous configuration is classified");
    assert_eq!(assessment.unsafe_keys, vec!["merge.*.driver"]);
    let fingerprint = assessment.fingerprint.expect("dangerous config has a digest");
    let trusted = GitCore::new()
        .with_trusted_network_configuration(&repository, fingerprint)
        .expect("repository trust is scoped to a canonical root");
    assert!(trusted
        .ensure_network_configuration_is_safe(&repository)
        .is_ok());

    run_git(&[
        "config",
        "merge.translation.driver",
        "scripts/replaced-driver.sh %O %A %B %P",
    ]);
    assert_eq!(
        trusted
            .ensure_network_configuration_is_safe(&repository)
            .expect_err("a changed command invalidates the approval")
            .code(),
        "unsafe_git_config"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn safe_configuration_changes_do_not_invalidate_a_dangerous_config_approval() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-safe-change-test-{}-{suffix}",
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
    run_git(&["config", "merge.translation.driver", "trusted-driver %A"]);
    let fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();
    let trusted = GitCore::new()
        .with_trusted_network_configuration(&repository, fingerprint)
        .unwrap();

    run_git(&["config", "user.name", "Updated display name"]);
    assert!(trusted
        .ensure_network_configuration_is_safe(&repository)
        .is_ok());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn dangerous_multi_value_order_is_part_of_the_configuration_fingerprint() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-order-test-{}-{suffix}",
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
    run_git(&["config", "--add", "credential.helper", "first-helper"]);
    run_git(&["config", "--add", "credential.helper", "second-helper"]);
    let first_fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();

    run_git(&["config", "--unset-all", "credential.helper"]);
    run_git(&["config", "--add", "credential.helper", "second-helper"]);
    run_git(&["config", "--add", "credential.helper", "first-helper"]);
    let reordered_fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();

    assert_ne!(first_fingerprint, reordered_fingerprint);
    let trusted = GitCore::new()
        .with_trusted_network_configuration(&repository, first_fingerprint)
        .unwrap();
    assert_eq!(
        trusted
            .ensure_network_configuration_is_safe(&repository)
            .expect_err("reordering dangerous values invalidates approval")
            .code(),
        "unsafe_git_config"
    );
    let _ = fs::remove_dir_all(repository);
}

#[test]
fn subsection_case_is_part_of_the_configuration_fingerprint() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-subsection-case-test-{}-{suffix}",
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
    run_git(&["config", "merge.DriverA.driver", "trusted-driver %A"]);
    let first_fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();

    run_git(&["config", "--unset-all", "merge.DriverA.driver"]);
    run_git(&["config", "merge.drivera.driver", "trusted-driver %A"]);
    let changed_fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();

    assert_ne!(first_fingerprint, changed_fingerprint);
    let trusted = GitCore::new()
        .with_trusted_network_configuration(&repository, first_fingerprint)
        .unwrap();
    assert_eq!(
        trusted
            .ensure_network_configuration_is_safe(&repository)
            .expect_err("changing subsection case invalidates approval")
            .code(),
        "unsafe_git_config"
    );
    let _ = fs::remove_dir_all(repository);
}

#[test]
fn external_config_includes_cannot_be_authorized() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-include-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let included = repository.join("included.config");
    fs::write(&included, "[user]\n\tname = Included User\n").unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&[
        "config",
        "include.path",
        included.to_str().expect("temporary path is UTF-8"),
    ]);
    let assessment = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap();
    assert!(!assessment.authorization_allowed);
    let trusted = GitCore::new()
        .with_trusted_network_configuration(
            &repository,
            assessment.fingerprint.expect("include has a fingerprint"),
        )
        .unwrap();
    assert_eq!(
        trusted
            .ensure_network_configuration_is_safe(&repository)
            .expect_err("external includes remain unapprovable")
            .code(),
        "unsafe_git_config_unapprovable"
    );

    run_git(&["config", "--unset-all", "include.path"]);
    for conditional_key in [
        "includeIf.onbranch:master.path",
        "includeIf.gitdir:/work/example/**.path",
    ] {
        run_git(&[
            "config",
            conditional_key,
            included.to_str().expect("temporary path is UTF-8"),
        ]);
        let conditional_assessment = GitCore::new()
            .network_configuration_assessment(&repository)
            .unwrap();
        assert!(
            !conditional_assessment.authorization_allowed,
            "{conditional_key} must remain unapprovable"
        );
        let conditional_trusted = GitCore::new()
            .with_trusted_network_configuration(
                &repository,
                conditional_assessment
                    .fingerprint
                    .expect("conditional include has a fingerprint"),
            )
            .unwrap();
        assert_eq!(
            conditional_trusted
                .ensure_network_configuration_is_safe(&repository)
                .expect_err("conditional includes remain unapprovable")
                .code(),
            "unsafe_git_config_unapprovable"
        );
        run_git(&["config", "--unset-all", conditional_key]);
    }

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn one_git_core_can_hold_independent_trust_for_parent_and_submodule_roots() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let parent = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-parent-test-{}-{suffix}",
        std::process::id()
    ));
    let child = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-trust-child-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&parent).unwrap();
    fs::create_dir_all(&child).unwrap();
    let run_git = |repository: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    for repository in [&parent, &child] {
        run_git(repository, &["init", "-q"]);
        run_git(repository, &["config", "user.email", "trust@example.invalid"]);
        run_git(repository, &["config", "user.name", "Trust Test"]);
    }
    fs::write(child.join("tracked.txt"), "child\n").unwrap();
    run_git(&child, &["add", "tracked.txt"]);
    run_git(&child, &["commit", "-qm", "child"]);
    let added = test_git_command()
        .arg("-C")
        .arg(&parent)
        .args(["-c", "protocol.file.allow=always", "submodule", "add"])
        .arg(&child)
        .arg("modules/child")
        .status()
        .unwrap();
    assert!(added.success());
    run_git(&parent, &["commit", "-qam", "add child"]);

    let child_checkout = parent.join("modules/child");
    run_git(
        &parent,
        &["config", "merge.parent.driver", "trusted-parent-driver %A"],
    );
    run_git(
        &child_checkout,
        &["config", "merge.child.driver", "trusted-child-driver %A"],
    );
    let parent_fingerprint = GitCore::new()
        .network_configuration_assessment(&parent)
        .unwrap()
        .fingerprint
        .unwrap();
    let child_fingerprint = GitCore::new()
        .network_configuration_assessment(&child_checkout)
        .unwrap()
        .fingerprint
        .unwrap();
    let parent_only = GitCore::new()
        .with_trusted_network_configuration(&parent, parent_fingerprint.clone())
        .unwrap();
    assert_eq!(
        parent_only
            .update_submodules(&parent, false, true)
            .expect_err("the initialized child requires its own grant")
            .code(),
        "unsafe_git_config"
    );

    let trusted = GitCore::new()
        .with_trusted_network_configuration(&parent, parent_fingerprint)
        .unwrap()
        .with_trusted_network_configuration(&child_checkout, child_fingerprint)
        .unwrap();
    trusted
        .update_submodules(&parent, false, true)
        .expect("parent and initialized child grants are both applied");

    let _ = fs::remove_dir_all(parent);
    let _ = fs::remove_dir_all(child);
}

#[test]
fn repository_hooks_path_does_not_block_network_preflight() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-hooks-path-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let configured = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "core.hooksPath",
            ".husky/_",
        ])
        .status()
        .unwrap();
    assert!(configured.success());

    assert!(GitCore::new()
        .ensure_network_configuration_is_safe(&repository)
        .is_ok());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn local_git_config_includes_are_rejected_before_network_git() {
    assert!(is_unsafe_network_config_key("include.path"));
    assert!(is_unsafe_network_config_key(
        "includeIf.gitdir:/work/**.path"
    ));
    assert!(is_unsafe_network_config_key(
        "url.https://replacement.example/.insteadOf"
    ));
    assert!(is_unsafe_network_config_key(
        "url.ssh://replacement.example/.pushInsteadOf"
    ));
    assert!(is_unsafe_network_config_key("remote.origin.proxy"));
    assert!(is_unsafe_network_config_key(
        "remote.origin.proxyAuthMethod"
    ));
    assert!(is_unsafe_network_config_key("http.sslCAInfo"));
    assert!(is_unsafe_network_config_key(
        "http.https://code.example/.sslCAPath"
    ));
    assert!(is_unsafe_network_config_key("http.pinnedPubKey"));
    assert!(is_unsafe_network_config_key("http.schannelUseSSLCAInfo"));
    assert!(is_unsafe_network_config_key("submodule.third-party.update"));
}

#[test]
fn renders_untracked_file_content_as_an_addition() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-untracked-diff-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join("new file.txt"), "first line\nsecond line\n").unwrap();

    let git = GitCore::new();
    let diff = git
        .diff(
            &repository,
            &DiffOptions {
                staged: false,
                paths: vec!["new file.txt".into()],
                context_lines: Some(3),
            },
        )
        .expect("an untracked text file has a displayable diff");

    assert!(diff.contains("new file mode 100644"));
    assert!(diff.contains("+++ \"b/new file.txt\""));
    assert!(diff.contains("+first line"));
    assert!(diff.contains("+second line"));
    assert_eq!(
        git.diff(
            &repository,
            &DiffOptions {
                staged: false,
                paths: vec!["../outside.txt".into()],
                context_lines: Some(3),
            },
        )
        .expect_err("no-index fallback must not read outside the worktree")
        .code(),
        "invalid_pathspec"
    );

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn renders_untracked_executable_files_with_their_mode() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-untracked-executable-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let executable = repository.join("script.sh");
    fs::write(&executable, "#!/bin/sh\necho ready\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let diff = GitCore::new()
        .diff(
            &repository,
            &DiffOptions {
                staged: false,
                paths: vec!["script.sh".into()],
                context_lines: Some(3),
            },
        )
        .expect("an untracked executable file has a displayable diff");
    assert!(diff.contains("new file mode 100755"));

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn parses_commit_history_and_stash_output() {
    let raw = concat!(
        "0123456789abcdef\0",
        "0123456\0",
        "parent-one parent-two\0",
        "Ada Lovelace\0",
        "ada@example.test\0",
        "2026-07-19T10:20:30+08:00\0",
        "Implement \x1ehistory\0",
        "\n",
        "fedcba9876543210\0",
        "fedcba9\0",
        "\0",
        "Grace Hopper\0",
        "grace@example.test\0",
        "2026-07-18T09:00:00+08:00\0",
        "Initial commit\0"
    )
    .as_bytes();
    let commits = parse_commits(raw).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].parents, vec!["parent-one", "parent-two"]);
    assert_eq!(commits[0].summary, "Implement \u{1e}history");
    assert!(commits[0].tags.is_empty());
    assert!(!commits[0].tags_truncated);
    assert!(commits[1].parents.is_empty());
    assert_eq!(commits[1].summary, "Initial commit");
    assert!(commits[1].tags.is_empty());

    let stashes = parse_stashes(
        b"stash@{0}\0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0On main: first line\nsecond line\0\nstash@{1}\0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0On topic: draft\0",
    )
    .unwrap();
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].reference, "stash@{0}");
    assert_eq!(
        stashes[0].commit_id,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(stashes[0].message, "On main: first line\nsecond line");
    assert_eq!(
        stashes[1].commit_id,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(stashes[1].message, "On topic: draft");
}

#[test]
fn selects_the_exact_history_ref_from_prefix_matches() {
    let fields = parse_exact_history_ref(
        b"refs/heads/topic\0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0refs/remotes/origin/topic\nrefs/heads/topic/child\0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0\n",
        "refs/heads/topic",
        3,
    )
    .unwrap();
    assert_eq!(fields[0], "refs/heads/topic");
    assert_eq!(fields[1], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
}

#[test]
fn trims_an_incomplete_utf8_suffix_after_an_earlier_invalid_byte() {
    let mut bytes = vec![0xff, b' ', 0xe4, 0xb8];
    trim_incomplete_utf8_suffix(&mut bytes);
    assert_eq!(bytes, vec![0xff, b' ']);
    assert_eq!(
        String::from_utf8_lossy(&bytes).matches('\u{fffd}').count(),
        1
    );
}

#[test]
fn preview_pipe_stops_at_its_limit_without_waiting_for_eof() {
    struct OpenAfterSentinel {
        delivered: bool,
    }

    impl std::io::Read for OpenAfterSentinel {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            assert!(
                !self.delivered,
                "preview reader waited for EOF after the sentinel byte"
            );
            self.delivered = true;
            buffer[..5].copy_from_slice(b"12345");
            Ok(5)
        }
    }

    let within_limit = read_git_preview_pipe(std::io::Cursor::new(b"1234"), 5, 4).unwrap();
    assert_eq!(within_limit.stdout, b"1234");
    assert!(!within_limit.truncated);

    let beyond_requested_limit =
        read_git_preview_pipe(OpenAfterSentinel { delivered: false }, 5, 4).unwrap();
    assert_eq!(beyond_requested_limit.stdout, b"12345");
    assert!(beyond_requested_limit.truncated);

    let overflow = read_git_preview_pipe(std::io::Cursor::new(b"123456"), 5, 4).unwrap();
    assert_eq!(overflow.stdout, b"12345");
    assert!(overflow.truncated);
}

#[test]
fn lists_real_git_history_without_extra_format_records() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-history-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("history.txt"), "first\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "first commit"]);
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&["commit", "-am", "second commit"]);
    run_git(&["branch", "origin/main", "HEAD"]);

    let commits = GitCore::new().commits(&repository, 100).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].summary, "second commit");
    assert_eq!(commits[1].summary, "first commit");
    assert!(commits.iter().all(|commit| {
        (40..=64).contains(&commit.sha.len())
            && commit.sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    }));
    GitCore::new()
        .commit_diff(&repository, &commits[1].sha)
        .expect("every listed SHA can be used to read its commit diff");
    assert!(GitCore::new()
        .commit_diff(&repository, "HEAD^{tree}")
        .is_err());
    GitCore::new()
        .create_branch(&repository, "from-first-commit", Some("HEAD~1"))
        .expect("a revision expression can be a branch start point");
    GitCore::new()
        .create_tag(&repository, "first-commit", Some("HEAD~1"), None)
        .expect("a revision expression can be a tag target");

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn commit_history_tracks_resolved_upstream_and_detached_head() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-history-position-test-{}-{suffix}",
        std::process::id()
    )));
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?} succeeds");
    };
    run_git(&["init", "-q"]);
    run_git(&["checkout", "-qb", "main"]);
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("history.txt"), "first\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "first commit"]);
    run_git(&[
        "remote",
        "add",
        "origin",
        "https://example.invalid/history.git",
    ]);
    run_git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    run_git(&["config", "branch.main.remote", "origin"]);
    run_git(&["config", "branch.main.merge", "refs/heads/main"]);
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&["commit", "-am", "second commit"]);

    let git = GitCore::new();
    let status = git.status(&repository).unwrap();
    let commits = git.commits(&repository, 100).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(
        status.upstream_commit.as_deref(),
        Some(commits[1].sha.as_str())
    );
    assert_eq!(commits[0].pushed, Some(false));
    assert_eq!(commits[1].pushed, Some(true));

    run_git(&["checkout", "--detach", "-q", "HEAD"]);
    let detached_commits = git.commits(&repository, 100).unwrap();
    assert_eq!(detached_commits.len(), 2);
    assert!(detached_commits
        .iter()
        .all(|commit| commit.pushed.is_none()));
}

#[test]
fn commit_history_is_empty_for_an_unborn_branch() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-unborn-history-test-{}-{suffix}",
        std::process::id()
    )));
    let status = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(GitCore::new().commits(&repository, 100).unwrap().is_empty());
}

#[test]
fn commit_history_is_empty_for_an_unborn_branch_with_a_prefix_child_ref() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-unborn-prefix-history-test-{}-{suffix}",
        std::process::id()
    )));
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?} succeeds");
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("history.txt"), "child\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "child commit"]);
    run_git(&["branch", "main/child", "HEAD"]);
    run_git(&["symbolic-ref", "HEAD", "refs/heads/main"]);

    assert!(GitCore::new().commits(&repository, 100).unwrap().is_empty());
}

#[test]
fn lists_commit_files_and_reads_one_file_patch() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-commit-files-test-{}-{suffix}",
        std::process::id()
    )));
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("old-name.txt"), "rename me\n").unwrap();
    fs::write(repository.join("deleted.txt"), "delete me\n").unwrap();
    fs::write(repository.join("modified.txt"), "before\n").unwrap();
    fs::write(repository.join("copy-source.txt"), "copy me\n").unwrap();
    fs::write(
        repository.join("changed-copy-source.txt"),
        "copy then modify me\n",
    )
    .unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-qm", "initial files"]);

    run_git(&["mv", "old-name.txt", "new-name.txt"]);
    fs::remove_file(repository.join("deleted.txt")).unwrap();
    fs::copy(
        repository.join("copy-source.txt"),
        repository.join("copied.txt"),
    )
    .unwrap();
    fs::write(repository.join("modified.txt"), "after 中文\n").unwrap();
    fs::write(repository.join("added.txt"), "new\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-qm", "change files"]);

    let git = GitCore::new();
    let root_files = git.commit_changed_files(&repository, "HEAD~1").unwrap();
    assert_eq!(root_files.len(), 5);
    assert!(root_files
        .iter()
        .all(|file| file.status == CommitFileStatus::Added));

    let files = git.commit_changed_files(&repository, "HEAD").unwrap();
    assert_eq!(files.len(), 5);
    assert!(files
        .iter()
        .any(|file| { file.path == "added.txt" && file.status == CommitFileStatus::Added }));
    assert!(files
        .iter()
        .any(|file| { file.path == "deleted.txt" && file.status == CommitFileStatus::Deleted }));
    assert!(files
        .iter()
        .any(|file| { file.path == "modified.txt" && file.status == CommitFileStatus::Modified }));
    let copied = files
        .iter()
        .find(|file| file.path == "copied.txt")
        .expect("copied path is listed");
    assert_eq!(copied.previous_path.as_deref(), Some("copy-source.txt"));
    assert_eq!(copied.status, CommitFileStatus::Copied);
    assert_eq!(copied.similarity, Some(100));
    let renamed = files
        .iter()
        .find(|file| file.path == "new-name.txt")
        .expect("renamed path is listed");
    assert_eq!(renamed.previous_path.as_deref(), Some("old-name.txt"));
    assert_eq!(renamed.status, CommitFileStatus::Renamed);
    assert_eq!(renamed.similarity, Some(100));

    let modified_diff = git
        .commit_file_diff(&repository, "HEAD", "modified.txt", None)
        .unwrap();
    assert!(modified_diff.contains("-before"));
    assert!(modified_diff.contains("+after"));
    assert!(!modified_diff.contains("added.txt"));
    let multibyte_boundary = modified_diff.find('中').unwrap() + 1;
    let modified_preview = git
        .commit_file_diff_preview(
            &repository,
            "HEAD",
            "modified.txt",
            None,
            multibyte_boundary,
        )
        .unwrap();
    assert!(modified_preview.truncated);
    assert!(modified_preview.text.len() <= multibyte_boundary);
    assert!(!modified_preview.text.contains('\u{fffd}'));
    assert_eq!(
        git.commit_file_diff_preview(&repository, "HEAD", "modified.txt", None, usize::MAX)
            .unwrap_err()
            .code(),
        "invalid_diff_limit"
    );
    let renamed_diff = git
        .commit_file_diff(
            &repository,
            "HEAD",
            &renamed.path,
            renamed.previous_path.as_deref(),
        )
        .unwrap();
    assert!(renamed_diff.contains("rename from old-name.txt"));
    assert!(renamed_diff.contains("rename to new-name.txt"));
    let copied_diff = git
        .commit_file_diff(
            &repository,
            "HEAD",
            &copied.path,
            copied.previous_path.as_deref(),
        )
        .unwrap();
    assert!(copied_diff.contains("copied.txt"));
    assert!(copied_diff.contains("copy from copy-source.txt"));
    assert!(copied_diff.contains("copy to copied.txt"));
    assert_eq!(
        git.commit_file_diff(&repository, "HEAD", &renamed.path, Some("modified.txt"),)
            .unwrap_err()
            .code(),
        "commit_file_not_found"
    );
    assert_eq!(
        git.commit_file_diff(&repository, "HEAD", "not-changed.txt", None)
            .unwrap_err()
            .code(),
        "commit_file_not_found"
    );
    assert_eq!(
        git.commit_file_diff(&repository, "HEAD", "modified.txt", Some("modified.txt"))
            .unwrap_err()
            .code(),
        "commit_file_not_found"
    );

    fs::copy(
        repository.join("changed-copy-source.txt"),
        repository.join("copied-from-changed.txt"),
    )
    .unwrap();
    fs::write(
        repository.join("changed-copy-source.txt"),
        "source changed after copy\n",
    )
    .unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-qm", "copy and modify source"]);
    let copied_from_changed = git
        .commit_changed_files(&repository, "HEAD")
        .unwrap()
        .into_iter()
        .find(|file| file.path == "copied-from-changed.txt")
        .expect("copy from a separately modified source is listed");
    assert_eq!(copied_from_changed.status, CommitFileStatus::Copied);
    assert_eq!(
        copied_from_changed.previous_path.as_deref(),
        Some("changed-copy-source.txt")
    );
    let copied_from_changed_diff = git
        .commit_file_diff(
            &repository,
            "HEAD",
            &copied_from_changed.path,
            copied_from_changed.previous_path.as_deref(),
        )
        .unwrap();
    assert!(copied_from_changed_diff.contains("copied-from-changed.txt"));
    assert!(!copied_from_changed_diff
        .contains("diff --git a/changed-copy-source.txt b/changed-copy-source.txt"));
}

#[test]
fn merge_commit_files_use_the_first_parent_comparison() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-merge-files-test-{}-{suffix}",
        std::process::id()
    )));
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?} succeeds");
    };
    run_git(&["init", "-q"]);
    run_git(&["checkout", "-qb", "main"]);
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("base.txt"), "base\n").unwrap();
    run_git(&["add", "base.txt"]);
    run_git(&["commit", "-qm", "base"]);
    run_git(&["checkout", "-qb", "feature"]);
    fs::write(repository.join("feature.txt"), "feature\n").unwrap();
    run_git(&["add", "feature.txt"]);
    run_git(&["commit", "-qm", "feature"]);
    run_git(&["checkout", "-q", "main"]);
    fs::write(repository.join("main.txt"), "main\n").unwrap();
    run_git(&["add", "main.txt"]);
    run_git(&["commit", "-qm", "main"]);
    run_git(&["merge", "-q", "--no-ff", "feature", "-m", "merge feature"]);

    let git = GitCore::new();
    let files = git.commit_changed_files(&repository, "HEAD").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "feature.txt");
    assert_eq!(files[0].status, CommitFileStatus::Added);
    let diff = git
        .commit_file_diff(&repository, "HEAD", "feature.txt", None)
        .unwrap();
    assert!(diff.contains("feature.txt"));
    assert!(!diff.contains("main.txt"));
}

#[test]
fn commit_history_omits_push_state_for_an_unresolvable_configured_upstream() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-broken-history-upstream-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "history@example.invalid"]);
    run_git(&["config", "user.name", "History Test"]);
    fs::write(repository.join("history.txt"), "first\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "first commit"]);
    run_git(&[
        "remote",
        "add",
        "origin",
        "https://example.invalid/missing.git",
    ]);

    let git = GitCore::new();
    let branch = git.current_branch_name(&repository).unwrap();
    run_git(&["config", &format!("branch.{branch}.remote"), "origin"]);
    run_git(&[
        "config",
        &format!("branch.{branch}.merge"),
        "refs/heads/missing",
    ]);

    let commits = git.commits(&repository, 100).unwrap();
    assert_eq!(commits.len(), 1);
    assert!(commits.iter().all(|commit| commit.pushed.is_none()));

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn squash_uses_an_immutable_head_and_preserves_soft_reset_semantics() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-squash-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "squash@example.invalid"]);
    run_git(&["config", "user.name", "Squash Test"]);
    for (content, message) in [
        ("base\n", "base"),
        ("base\none\n", "one"),
        ("base\none\ntwo\n", "two"),
    ] {
        fs::write(repository.join("history.txt"), content).unwrap();
        run_git(&["add", "history.txt"]);
        run_git(&["commit", "-qm", message]);
    }

    GitCore::new()
        .squash_recent_commits(&repository, 2, "squashed")
        .expect("two recent commits are replaced by one commit");
    let commits = GitCore::new().commits(&repository, 10).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].summary, "squashed");
    assert_eq!(commits[1].summary, "base");
    assert_eq!(
        fs::read_to_string(repository.join("history.txt")).unwrap(),
        "base\none\ntwo\n"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn push_with_url_preserves_push_default_upstream_behavior() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-push-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-push-upstream-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let git = GitCore::new();
    let branch = git.current_branch_name(&repository).unwrap();
    let branch_remote_config = format!("branch.{branch}.remote");
    let branch_merge_config = format!("branch.{branch}.merge");
    run_git(
        &repository,
        &["config", "--local", &branch_remote_config, "origin"],
    );
    run_git(
        &repository,
        &[
            "config",
            "--local",
            &branch_merge_config,
            "refs/heads/review-target",
        ],
    );
    run_git(
        &repository,
        &["config", "--local", "push.default", "upstream"],
    );

    git.push(
        &repository,
        &PushOptions {
            remote: "origin".into(),
            refspecs: Vec::new(),
            set_upstream: false,
            force_with_lease: false,
            follow_tags: false,
        },
    )
    .expect("URL push must retain the configured upstream destination");

    let expected = git.head_commit_id(&repository).unwrap();
    let output = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/review-target"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout).trim(), expected);

    run_git(
        &repository,
        &["config", "--local", "push.default", "matching"],
    );
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["commit", "-qam", "second"]);
    let error = git
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: Vec::new(),
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("implicit matching push cannot be synchronized safely");
    assert_eq!(error.code(), "push_explicit_refspec_required");
    let output = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/review-target"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout).trim(), expected);

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn push_updates_existing_remote_tracking_state() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-push-tracking-test"),
    );
    let remote = root.join("remote.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&repository, &["push", "-qu", "origin", "main"]);

    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "second"]);
    run_git(&repository, &["tag", "should-not-follow"]);
    run_git(&repository, &["config", "push.followTags", "true"]);

    let git = GitCore::new();
    assert_eq!(git.status(&repository).unwrap().ahead, 1);
    let main_head = git.head_commit_id(&repository).unwrap();
    let (refspecs, set_upstream) = git.push_plan_for_remote(&repository, "origin").unwrap();
    assert!(!set_upstream);
    run_git(&repository, &["checkout", "-qb", "feature"]);

    git.push(
        &repository,
        &PushOptions {
            remote: "origin".into(),
            refspecs,
            set_upstream,
            force_with_lease: false,
            follow_tags: false,
        },
    )
    .expect("push succeeds");

    run_git(&repository, &["checkout", "-q", "main"]);
    let head = git.head_commit_id(&repository).unwrap();
    assert_eq!(head, main_head);
    assert_eq!(git.status(&repository).unwrap().ahead, 0);
    let tracking = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/remotes/origin/main"])
        .output()
        .unwrap();
    assert!(tracking.status.success());
    assert_eq!(
        output_text(&tracking.stdout).trim(),
        head
    );
    let remote_tag = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "--verify", "refs/tags/should-not-follow"])
        .output()
        .unwrap();
    assert!(!remote_tag.status.success());

    let remote_before_reftable_rejection = git.head_commit_id(&repository).unwrap();
    fs::write(repository.join("history.txt"), "third\n").unwrap();
    run_git(&repository, &["commit", "-qam", "third"]);
    run_git(
        &repository,
        &["config", "extensions.refStorage", "reftable"],
    );
    let error = git
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec!["refs/heads/main:refs/heads/main".into()],
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("unsupported reference storage is rejected before push");
    assert_eq!(error.code(), "push_tracking_preflight_failed");
    let remote_main = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .unwrap();
    assert!(remote_main.status.success());
    assert_eq!(
        output_text(&remote_main.stdout).trim(),
        remote_before_reftable_rejection
    );

}

#[test]
fn short_push_destination_preserves_remote_ref_resolution() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-short-push-destination-test"),
    );
    let remote = root.join("remote.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&repository, &["push", "-qu", "origin", "main"]);
    let initial = GitCore::new().head_commit_id(&repository).unwrap();
    run_git(
        &remote,
        &["update-ref", "refs/tags/release", &initial],
    );
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["commit", "-qam", "second"]);
    let second = GitCore::new().head_commit_id(&repository).unwrap();

    GitCore::new()
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec!["+main:release".into()],
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect("Git resolves the short destination to the existing remote tag");
    let remote_tag = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/tags/release"])
        .output()
        .unwrap();
    assert!(remote_tag.status.success());
    assert_eq!(output_text(&remote_tag.stdout).trim(), second);
    let remote_branch = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "--verify", "refs/heads/release"])
        .output()
        .unwrap();
    assert!(!remote_branch.status.success());
    assert_eq!(GitCore::new().status(&repository).unwrap().ahead, 1);
}

#[test]
fn push_updates_a_custom_remote_tracking_reference() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-push-custom-tracking-test"),
    );
    let remote = root.join("remote.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
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
    run_git(&repository, &["push", "-qu", "origin", "main"]);
    run_git(
        &repository,
        &["tag", "-am", "tracking base", "tracking-base"],
    );
    let tracking_tag = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/tags/tracking-base"])
        .output()
        .unwrap();
    assert!(tracking_tag.status.success());
    run_git(
        &repository,
        &[
            "update-ref",
            "refs/remotes/origin-mirror/main",
            output_text(&tracking_tag.stdout).trim(),
        ],
    );

    run_git(&repository, &["checkout", "-qb", "other"]);
    run_git(&repository, &["push", "-qu", "origin", "other"]);
    fs::write(repository.join("history.txt"), "other second\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "other second"]);
    let other_head = GitCore::new().head_commit_id(&repository).unwrap();
    run_git(&repository, &["checkout", "-q", "main"]);
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "second"]);

    let git = GitCore::new();
    assert_eq!(git.status(&repository).unwrap().ahead, 1);
    git.push(
        &repository,
        &PushOptions {
            remote: "origin".into(),
            refspecs: vec!["HEAD".into()],
            set_upstream: false,
            force_with_lease: true,
            follow_tags: false,
        },
    )
    .expect("force-with-lease uses the peeled tracking commit");
    git.push(
        &repository,
        &PushOptions {
            remote: "origin".into(),
            refspecs: vec!["other:refs/heads/other".into()],
            set_upstream: false,
            force_with_lease: false,
            follow_tags: false,
        },
    )
    .expect("second tracked branch push succeeds");

    let head = git.head_commit_id(&repository).unwrap();
    assert_eq!(git.status(&repository).unwrap().ahead, 0);
    let tracking = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/remotes/origin-mirror/main"])
        .output()
        .unwrap();
    assert!(tracking.status.success());
    assert_eq!(output_text(&tracking.stdout).trim(), head);
    let other_tracking = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/remotes/origin-mirror/other"])
        .output()
        .unwrap();
    assert!(other_tracking.status.success());
    assert_eq!(output_text(&other_tracking.stdout).trim(), other_head);

}

#[test]
fn multi_ref_push_does_not_require_atomic_server_support() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-atomic-multi-push-test"),
    );
    let remote = root.join("remote.git");
    let repository = root.join("repository");
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };
    let remote_commit = |branch: &str| {
        let output = test_git_command()
            .args(["--git-dir", remote.to_str().unwrap()])
            .args(["rev-parse", &format!("refs/heads/{branch}")])
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };

    run_git(&root, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&repository, &["push", "-qu", "origin", "main"]);
    run_git(&repository, &["checkout", "-qb", "other"]);
    run_git(&repository, &["push", "-qu", "origin", "other"]);
    fs::write(repository.join("history.txt"), "other accepted\n").unwrap();
    run_git(&repository, &["commit", "-qam", "other accepted"]);
    run_git(&repository, &["push", "-q", "origin", "other"]);
    let remote_other = remote_commit("other");
    run_git(&repository, &["reset", "-q", "--hard", "HEAD^"]);
    fs::write(repository.join("history.txt"), "other rejected\n").unwrap();
    run_git(&repository, &["commit", "-qam", "other rejected"]);
    run_git(&repository, &["checkout", "-q", "main"]);
    let remote_main = remote_commit("main");
    fs::write(repository.join("history.txt"), "main accepted\n").unwrap();
    run_git(&repository, &["commit", "-qam", "main accepted"]);

    let error = GitCore::new()
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec![
                    "refs/heads/main:refs/heads/main".into(),
                    "refs/heads/other:refs/heads/other".into(),
                ],
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("the non-fast-forward branch rejects the atomic push");
    assert_ne!(error.code(), "push_partial_remote_success");
    assert_ne!(remote_commit("main"), remote_main);
    assert_eq!(remote_commit("other"), remote_other);
    assert_eq!(GitCore::new().status(&repository).unwrap().ahead, 1);
}

#[cfg(unix)]
#[test]
fn tracking_update_rejects_symlinked_reference_ancestry() {
    use std::os::unix::fs::symlink;

    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-tracking-symlink-test"),
    );
    let repository = root.join("repository");
    let outside = root.join("outside");
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "user.name", "Push Test"]);
    run_git(&["config", "user.email", "push@example.invalid"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    let commit = GitCore::new().head_commit_id(&repository).unwrap();
    let remote_parent = repository.join(".git/refs/remotes/origin");
    fs::create_dir_all(remote_parent.parent().unwrap()).unwrap();
    symlink(&outside, &remote_parent).unwrap();

    GitCore::new()
        .update_tracking_references_after_push_for_test(
            &repository,
            &[("refs/remotes/origin/main", &commit, None)],
        )
        .expect_err("tracking updates must not follow repository-controlled symlinks");
    assert!(!outside.join("main").exists());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn tracking_update_rejects_symlinked_reflog_ancestry() {
    use std::os::unix::fs::symlink;

    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-tracking-reflog-symlink-test"),
    );
    let repository = root.join("repository");
    let outside = root.join("outside");
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "user.name", "Push Test"]);
    run_git(&["config", "user.email", "push@example.invalid"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    let commit = GitCore::new().head_commit_id(&repository).unwrap();
    fs::create_dir_all(repository.join(".git/refs/remotes/origin")).unwrap();
    let reflog_parent = repository.join(".git/logs/refs/remotes");
    fs::create_dir_all(&reflog_parent).unwrap();
    symlink(&outside, reflog_parent.join("origin")).unwrap();

    GitCore::new()
        .update_tracking_references_after_push_for_test(
            &repository,
            &[("refs/remotes/origin/main", &commit, None)],
        )
        .expect_err("tracking updates must not follow repository-controlled reflog symlinks");
    assert!(!repository.join(".git/refs/remotes/origin/main").exists());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());

    fs::remove_file(reflog_parent.join("origin")).unwrap();
    fs::create_dir(reflog_parent.join("origin")).unwrap();
    let outside_log = outside.join("captured-log");
    symlink(&outside_log, reflog_parent.join("origin/main")).unwrap();
    GitCore::new()
        .update_tracking_references_after_push_for_test(
            &repository,
            &[("refs/remotes/origin/main", &commit, None)],
        )
        .expect_err("tracking updates must not follow a repository-controlled reflog leaf");
    assert!(!outside_log.exists());

    fs::remove_file(reflog_parent.join("origin/main")).unwrap();
    fs::write(&outside_log, "unchanged\n").unwrap();
    fs::hard_link(&outside_log, reflog_parent.join("origin/main")).unwrap();
    GitCore::new()
        .update_tracking_references_after_push_for_test(
            &repository,
            &[("refs/remotes/origin/main", &commit, None)],
        )
        .expect_err("tracking updates must reject a hard-linked reflog leaf");
    assert_eq!(fs::read_to_string(&outside_log).unwrap(), "unchanged\n");
}

#[cfg(unix)]
#[test]
fn tracking_guards_preserve_shared_repository_directory_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-shared-tracking-guard-test"),
    );
    let repository = root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };
    run_git(&["init", "-q", "--shared=group", "-b", "main"]);
    run_git(&["config", "user.name", "Push Test"]);
    run_git(&["config", "user.email", "push@example.invalid"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    let commit = GitCore::new().head_commit_id(&repository).unwrap();

    GitCore::new()
        .update_tracking_references_after_push_for_test(
            &repository,
            &[("refs/remotes/origin/feature/topic", &commit, None)],
        )
        .expect("nested tracking update succeeds");
    for path in [
        ".git/refs/remotes/origin/feature",
        ".git/logs/refs/remotes/origin/feature",
    ] {
        let mode = fs::metadata(repository.join(path))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o020, 0, "{path} remains group-writable");
        assert_ne!(mode & 0o2000, 0, "{path} retains setgid inheritance");
    }
}

#[test]
fn tracking_reference_batch_update_is_atomic_on_cas_conflict() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-push-tracking-cas-test"),
    );
    let repository = root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .expect("test Git command runs");
        assert!(status.success(), "test Git command succeeds: {arguments:?}");
    };
    let read_head = || {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };

    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "user.name", "Push Test"]);
    run_git(&["config", "user.email", "push@example.invalid"]);
    fs::write(repository.join("history.txt"), "base\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "base"]);
    let base = read_head();
    fs::write(repository.join("history.txt"), "target\n").unwrap();
    run_git(&["commit", "-qam", "target"]);
    let target = read_head();
    fs::write(repository.join("history.txt"), "conflict\n").unwrap();
    run_git(&["commit", "-qam", "conflict"]);
    let conflict = read_head();
    run_git(&["update-ref", "refs/remotes/origin/main", &base]);
    run_git(&["update-ref", "refs/remotes/origin/other", &conflict]);

    let result = GitCore::new().update_tracking_references_after_push_for_test(
        &repository,
        &[
            ("refs/remotes/origin/main", &target, Some(&base)),
            ("refs/remotes/origin/other", &target, Some(&base)),
        ],
    );
    assert!(result.is_err());

    let read_reference = |reference: &str| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(["rev-parse", reference])
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };
    assert_eq!(read_reference("refs/remotes/origin/main"), base);
    assert_eq!(read_reference("refs/remotes/origin/other"), conflict);
}

#[test]
fn push_reports_partial_success_when_a_later_push_url_fails() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-partial-push-test"),
    );
    let remote = root.join("remote.git");
    let missing_remote = root.join("missing.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&repository, &["push", "-qu", "origin", "main"]);
    run_git(
        &repository,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            remote.to_str().unwrap(),
        ],
    );
    run_git(
        &repository,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            missing_remote.to_str().unwrap(),
        ],
    );
    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["commit", "-qam", "second"]);

    let git = GitCore::new();
    let head = git.head_commit_id(&repository).unwrap();
    let (refspecs, set_upstream) = git.push_plan_for_remote(&repository, "origin").unwrap();
    let error = git
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs,
                set_upstream,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("the second push URL is unavailable");
    assert_eq!(error.code(), "push_partial_remote_success");

    let remote_head = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .unwrap();
    assert!(remote_head.status.success());
    assert_eq!(output_text(&remote_head.stdout).trim(), head);
    assert_eq!(git.status(&repository).unwrap().ahead, 1);

    run_git(
        &repository,
        &["config", "--unset-all", "remote.origin.pushurl"],
    );
    run_git(
        &repository,
        &[
            "config",
            "--add",
            "remote.origin.pushurl",
            remote.to_str().unwrap(),
        ],
    );
    run_git(
        &repository,
        &[
            "config",
            "--add",
            "remote.origin.pushurl",
            "https://gitee.com/%zz/team/repository.git",
        ],
    );
    fs::write(repository.join("history.txt"), "third\n").unwrap();
    run_git(&repository, &["commit", "-qam", "third"]);
    let (refspecs, set_upstream) = git.push_plan_for_remote(&repository, "origin").unwrap();
    let error = git
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs,
                set_upstream,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("all push URLs are validated before the first push");
    assert_eq!(error.code(), "invalid_remote_url");
    let remote_head_after_invalid_url = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .unwrap();
    assert!(remote_head_after_invalid_url.status.success());
    assert_eq!(remote_head_after_invalid_url.stdout, remote_head.stdout);
}

#[test]
fn detached_head_can_push_a_tag_without_tracking_finalization() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-detached-tag-push-test"),
    );
    let remote = root.join("remote.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&repository, &["tag", "release-test"]);
    run_git(&repository, &["tag", "release-test-2"]);
    run_git(&repository, &["checkout", "-q", "--detach"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    GitCore::new()
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec!["refs/tags/*:refs/tags/*".into()],
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect("tag push succeeds from detached HEAD");

    for tag in ["release-test", "release-test-2"] {
        let remote_tag = test_git_command()
            .args(["--git-dir", remote.to_str().unwrap()])
            .args(["rev-parse", &format!("refs/tags/{tag}")])
            .output()
            .unwrap();
        assert!(remote_tag.status.success());
    }
}

#[test]
fn push_does_not_update_a_tracking_reference_outside_refs_remotes() {
    let root = TemporaryTestDirectory::create(
        std::env::temp_dir().join("broknowmygitdesktop-unsafe-push-tracking-test"),
    );
    let remote = root.join("remote.git");
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
    run_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&repository, &["branch", "victim/main"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
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
            "+refs/heads/*:refs/heads/victim/*",
        ],
    );
    run_git(
        &repository,
        &["config", "branch.main.remote", "origin"],
    );
    run_git(
        &repository,
        &["config", "branch.main.merge", "refs/heads/main"],
    );
    let victim_before = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/victim/main"])
        .output()
        .unwrap();
    assert!(victim_before.status.success());

    fs::write(repository.join("history.txt"), "second\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "second"]);
    GitCore::new()
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec!["refs/heads/main:refs/heads/main".into()],
                set_upstream: false,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect("push succeeds without rewriting a local branch");

    let victim_after = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/victim/main"])
        .output()
        .unwrap();
    assert!(victim_after.status.success());
    assert_eq!(victim_after.stdout, victim_before.stdout);

    run_git(&repository, &["branch", "new-branch"]);
    let error = GitCore::new()
        .push(
            &repository,
            &PushOptions {
                remote: "origin".into(),
                refspecs: vec!["refs/heads/new-branch:refs/heads/new-branch".into()],
                set_upstream: true,
                force_with_lease: false,
                follow_tags: false,
            },
        )
        .expect_err("unsafe tracking mapping is rejected before a first push");
    assert_eq!(error.code(), "push_upstream_fetch_mapping_unavailable");
    let remote_branch = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "--verify", "refs/heads/new-branch"])
        .output()
        .unwrap();
    assert!(!remote_branch.status.success());
}

#[test]
fn push_with_url_sets_an_official_upstream_for_a_single_branch_remote() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-push-set-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-push-set-upstream-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "quoted\"branch"]);
    run_git(
        &repository,
        &["config", "user.email", "push@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Push Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);
    run_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
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
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );

    let git = GitCore::new();
    let branch = git.current_branch_name(&repository).unwrap();
    let remote_branch = "review-target";
    let refspec = format!("refs/heads/{branch}:refs/heads/{remote_branch}");
    git.push(
        &repository,
        &PushOptions {
            remote: "origin".into(),
            refspecs: vec![refspec],
            set_upstream: true,
            force_with_lease: false,
            follow_tags: false,
        },
    )
    .expect("the locked network push can configure its named upstream afterward");

    let branch_remote_config = format!("branch.{branch}.remote");
    let branch_merge_config = format!("branch.{branch}.merge");
    let read_config = |key: &str| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap(), "config", "--get", key])
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };
    assert_eq!(read_config(&branch_remote_config), "origin");
    assert_eq!(
        read_config(&branch_merge_config),
        format!("refs/heads/{remote_branch}")
    );
    let expected = git.head_commit_id(&repository).unwrap();
    let output = test_git_command()
        .args(["--git-dir", remote.to_str().unwrap()])
        .args(["rev-parse", "refs/heads/review-target"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout).trim(), expected);
    let tracking_reference = "refs/remotes/origin/review-target";
    let output = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["rev-parse", tracking_reference])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout).trim(), expected);
    let output = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args([
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout).trim(), tracking_reference);
    let fetch_refspecs = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["config", "--get-all", "remote.origin.fetch"])
        .output()
        .unwrap();
    assert!(fetch_refspecs.status.success());
    assert_eq!(
        output_text(&fetch_refspecs.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "+refs/heads/main:refs/remotes/origin/main",
            "+refs/heads/review-target:refs/remotes/origin/review-target",
        ]
    );

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

fn published_remote_state(remote_url: &str) -> PublishedRemoteState {
    PublishedRemoteState {
        verified_fetch_url: remote_url.to_owned(),
        fetch_urls: vec![remote_url.to_owned()],
        push_urls: vec![remote_url.to_owned()],
    }
}

#[test]
fn published_branch_finalization_sets_upstream_after_the_remote_is_added_late() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-upstream-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    run_git(&repository, &["branch", "work-in-progress"]);
    run_git(&repository, &["checkout", "-q", "work-in-progress"]);
    let read_upstream = || {
        test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "for-each-ref",
                "--format=%(upstream)",
                "refs/heads/main",
            ])
            .output()
            .unwrap()
    };
    assert!(output_text(&read_upstream().stdout).trim().is_empty());
    git.configure_published_branch_upstream(
        &repository,
        "origin",
        "main",
        &expected,
        &published_remote_state(remote_url),
    )
        .expect("publish finalization records the already-pushed branch upstream");

    assert_eq!(
        output_text(&read_upstream().stdout).trim(),
        "refs/remotes/origin/main"
    );
    let tracking_commit = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "rev-parse",
            "refs/remotes/origin/main",
        ])
        .output()
        .unwrap();
    assert!(tracking_commit.status.success());
    assert_eq!(output_text(&tracking_commit.stdout).trim(), expected);
    run_git(&repository, &["checkout", "-q", "main"]);

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_rejects_a_moved_local_branch_without_writing_tracking_state() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-moved-branch-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-moved-branch-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);

    let git = GitCore::new();
    let published = git.head_commit_id(&repository).unwrap();
    fs::write(repository.join("history.txt"), "moved\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "move main"]);

    let error = git
        .configure_published_branch_upstream(
            &repository,
            "origin",
            "main",
            &published,
            &published_remote_state(remote_url),
        )
        .expect_err("a moved published branch must not be finalized");
    assert_eq!(error.code(), "push_tracking_setup_incomplete");

    let upstream = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "for-each-ref",
            "--format=%(upstream)",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert!(output_text(&upstream.stdout).trim().is_empty());
    let tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main",
        ])
        .status()
        .unwrap();
    assert!(!tracking.success());

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_rejects_a_symbolic_local_branch() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-symbolic-branch-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-symbolic-branch-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/published"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    run_git(
        &repository,
        &["symbolic-ref", "refs/heads/published", "refs/heads/main"],
    );

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    let error = git
        .configure_published_branch_upstream(
            &repository,
            "origin",
            "published",
            &expected,
            &published_remote_state(remote_url),
        )
        .expect_err("symbolic local branches must not be finalized");
    assert_eq!(error.code(), "push_tracking_setup_incomplete");
    let tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/published",
        ])
        .status()
        .unwrap();
    assert!(!tracking.success());

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_preserves_a_symbolic_tracking_reference() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-symbolic-tracking-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-symbolic-tracking-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    run_git(
        &repository,
        &[
            "symbolic-ref",
            "refs/remotes/origin/main",
            "refs/heads/main",
        ],
    );

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    let error = git
        .configure_published_branch_upstream(
            &repository,
            "origin",
            "main",
            &expected,
            &published_remote_state(remote_url),
        )
        .expect_err("symbolic tracking refs must not be replaced");
    assert_eq!(error.code(), "push_tracking_setup_incomplete");
    let symbolic_target = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "symbolic-ref",
            "refs/remotes/origin/main",
        ])
        .output()
        .unwrap();
    assert!(symbolic_target.status.success());
    assert_eq!(
        output_text(&symbolic_target.stdout).trim(),
        "refs/heads/main"
    );

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_does_not_require_a_nested_config_lock() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-config-rollback-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-config-rollback-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    fs::create_dir(repository.join(".git/config.lock.lock")).unwrap();

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    git.configure_published_branch_upstream(
        &repository,
        "origin",
        "main",
        &expected,
        &published_remote_state(remote_url),
    )
    .expect("staging writes through the already-owned config lock handle");
    let tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main",
        ])
        .status()
        .unwrap();
    assert!(tracking.success());
    let upstream = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "for-each-ref",
            "--format=%(upstream)",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output_text(&upstream.stdout).trim(),
        "refs/remotes/origin/main"
    );

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_preserves_partial_worktree_upstream_configuration() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-existing-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-existing-upstream-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    run_git(
        &repository,
        &["config", "extensions.worktreeConfig", "true"],
    );
    run_git(
        &repository,
        &["config", "--worktree", "branch.main.remote", "backup"],
    );

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    let error = git
        .configure_published_branch_upstream(
            &repository,
            "origin",
            "main",
            &expected,
            &published_remote_state(remote_url),
        )
        .expect_err("publish finalization must preserve partial worktree branch configuration");
    assert_eq!(error.code(), "push_tracking_setup_incomplete");

    let configured_remote = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "--worktree",
            "--get",
            "branch.main.remote",
        ])
        .output()
        .unwrap();
    assert!(configured_remote.status.success());
    assert_eq!(output_text(&configured_remote.stdout).trim(), "backup");
    let configured_merge = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "--worktree",
            "--get",
            "branch.main.merge",
        ])
        .status()
        .unwrap();
    assert!(!configured_merge.success());
    let origin_tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main",
        ])
        .status()
        .unwrap();
    assert!(!origin_tracking.success());

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn published_branch_finalization_accepts_upstream_split_across_config_scopes() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-split-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-split-upstream-remote-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    run_git(
        &repository,
        &["config", "extensions.worktreeConfig", "true"],
    );
    run_git(
        &repository,
        &["config", "branch.main.remote", "origin"],
    );
    run_git(
        &repository,
        &[
            "config",
            "--worktree",
            "branch.main.merge",
            "refs/heads/main",
        ],
    );

    let git = GitCore::new();
    let expected = git.head_commit_id(&repository).unwrap();
    git.configure_published_branch_upstream(
        &repository,
        "origin",
        "main",
        &expected,
        &published_remote_state(remote_url),
    )
    .expect("split compatible branch configuration must be preserved");

    let upstream = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "for-each-ref",
            "--format=%(upstream)",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert!(upstream.status.success());
    assert_eq!(
        output_text(&upstream.stdout).trim(),
        "refs/remotes/origin/main"
    );
    let tracking = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "rev-parse",
            "refs/remotes/origin/main",
        ])
        .output()
        .unwrap();
    assert!(tracking.status.success());
    assert_eq!(output_text(&tracking.stdout).trim(), expected);

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[cfg(unix)]
#[test]
fn published_branch_finalization_rejects_symlinked_reference_directories() {
    use std::os::unix::fs::symlink;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-ref-symlink-test-{}-{suffix}",
        std::process::id()
    ));
    let remote = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-ref-symlink-remote-{}-{suffix}",
        std::process::id()
    ));
    let outside_heads = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-publish-ref-symlink-outside-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    let run_git = |directory: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", directory.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&repository, &["init", "-q"]);
    run_git(&repository, &["branch", "-m", "main"]);
    run_git(
        &repository,
        &["config", "user.email", "publish@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Publish Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&repository, &["add", "history.txt"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    run_git(&remote, &["init", "--bare", "-q"]);

    let remote_url = remote.to_str().unwrap();
    run_git(
        &repository,
        &["push", remote_url, "HEAD:refs/heads/main"],
    );
    run_git(&repository, &["remote", "add", "origin", remote_url]);
    let expected = GitCore::new().head_commit_id(&repository).unwrap();
    let heads = repository.join(".git/refs/heads");
    fs::rename(&heads, &outside_heads).unwrap();
    symlink(&outside_heads, &heads).unwrap();

    GitCore::new()
        .configure_published_branch_upstream(
            &repository,
            "origin",
            "main",
            &expected,
            &published_remote_state(remote_url),
        )
        .expect_err("reference locks must not follow repository-controlled symlinks");
    assert!(!outside_heads.join("main.lock").exists());

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
    let _ = fs::remove_dir_all(outside_heads);
}

#[test]
fn legacy_publish_branch_lookup_rejects_noncanonical_object_id_lengths() {
    let error = GitCore::new()
        .unique_local_branch_at_commit(Path::new("."), &"a".repeat(41))
        .expect_err("noncanonical object IDs must fail before Git is invoked");

    assert_eq!(error.code(), "git_parse_error");
}

#[test]
fn push_upstream_rejects_fetch_mappings_outside_the_selected_remote_namespace() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-push-fetch-mapping-safety-{}-{suffix}",
        std::process::id()
    )));
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());

    let git = GitCore::new();
    for destination in [
        "HEAD",
        "refs/heads/main",
        "refs/tags/pwned",
        "refs/remotes/other/review-target",
        "refs/remotes/origin2/review-target",
        "refs/remotes/origin/../heads/main",
    ] {
        let configured = test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "config",
                "--replace-all",
                "remote.origin.fetch",
                &format!("+refs/heads/review-target:{destination}"),
            ])
            .status()
            .unwrap();
        assert!(configured.success());

        let error = git
            .configured_tracking_reference(&repository, "origin", "review-target")
            .expect_err("unsafe fetch destinations must not become tracking references");
        assert_eq!(error.code(), "push_upstream_fetch_mapping_unavailable");
    }
}

#[test]
fn pushed_tracking_update_never_dereferences_a_symbolic_remote_tracking_ref() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-symbolic-tracking-ref-safety-{}-{suffix}",
        std::process::id()
    )));
    let run_git = |arguments: &[&str]| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success());
        output
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "push@example.invalid"]);
    run_git(&["config", "user.name", "Push Test"]);
    fs::write(repository.join("history.txt"), "initial\n").unwrap();
    run_git(&["add", "history.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    let branch = output_text(&run_git(&["branch", "--show-current"]).stdout)
        .trim()
        .to_owned();
    let head_before = output_text(&run_git(&["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_owned();
    let branch_reference = format!("refs/heads/{branch}");
    run_git(&[
        "symbolic-ref",
        "refs/remotes/origin/review-target",
        &branch_reference,
    ]);

    GitCore::new()
        .update_tracking_reference_after_push(
            &repository,
            "refs/remotes/origin/review-target",
            &head_before,
            Some(&head_before),
        )
        .expect("the symbolic tracking ref itself can be replaced without dereferencing its target");
    let head_after = output_text(&run_git(&["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_owned();
    assert_eq!(head_after, head_before);
    let symbolic_check = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "symbolic-ref",
            "--quiet",
            "refs/remotes/origin/review-target",
        ])
        .output()
        .unwrap();
    assert!(!symbolic_check.status.success());
    let tracking_commit = output_text(
        &run_git(&["rev-parse", "refs/remotes/origin/review-target"]).stdout,
    )
    .trim()
    .to_owned();
    assert_eq!(tracking_commit, head_before);
}

#[test]
fn network_configuration_lock_blocks_a_concurrent_git_config_write() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-network-config-lock-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());

    let git = GitCore::new();
    let lock = git
        .lock_network_configuration(&repository)
        .expect("the repository configuration can be locked");
    let blocked = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "--local",
            "core.sshCommand",
            "unsafe-command",
        ])
        .status()
        .unwrap();
    assert!(!blocked.success());
    drop(lock);

    let updated = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "--local",
            "core.sshCommand",
            "safe-command",
        ])
        .status()
        .unwrap();
    assert!(updated.success());

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn approved_network_operation_holds_config_lock_through_git_process_start() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-approved-config-lock-test-{}-{suffix}",
        std::process::id()
    ));
    let repository = root.join("repository");
    let remote = root.join("remote.git");
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&remote).unwrap();
    assert!(test_git_command()
        .arg("-C")
        .arg(&repository)
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success());
    assert!(test_git_command()
        .arg("-C")
        .arg(&remote)
        .args(["init", "--bare", "-q"])
        .status()
        .unwrap()
        .success());
    assert!(test_git_command()
        .arg("-C")
        .arg(&repository)
        .args(["remote", "add", "origin"])
        .arg(&remote)
        .status()
        .unwrap()
        .success());
    assert!(test_git_command()
        .arg("-C")
        .arg(&repository)
        .args([
            "config",
            "merge.translation.driver",
            "approved-driver %A",
        ])
        .status()
        .unwrap()
        .success());

    let fingerprint = GitCore::new()
        .network_configuration_assessment(&repository)
        .unwrap()
        .fingerprint
        .unwrap();
    let marker = root.join("config-write-result");
    let wrapper = root.join("git-wrapper.sh");
    fs::write(
        &wrapper,
        "#!/bin/sh\nrepository=''\nprevious=''\nis_fetch=0\nmarker=$(dirname \"$0\")/config-write-result\nfor argument in \"$@\"; do\n  if [ \"$previous\" = '-C' ]; then repository=$argument; fi\n  if [ \"$argument\" = 'fetch' ]; then is_fetch=1; fi\n  previous=$argument\ndone\nif [ \"$is_fetch\" = 1 ]; then\n  if git -C \"$repository\" config merge.translation.driver 'replacement-driver %A'; then\n    printf changed > \"$marker\"\n  else\n    printf blocked > \"$marker\"\n  fi\nfi\nexec git \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();

    let git = GitCore::with_executable(&wrapper)
        .with_trusted_network_configuration(&repository, fingerprint)
        .unwrap();
    git.fetch(
        &repository,
        &FetchOptions {
            remote: "origin".into(),
            refspecs: Vec::new(),
            prune: false,
        },
    )
    .expect("the approved fetch completes with the configuration lock held");

    assert_eq!(fs::read_to_string(&marker).unwrap(), "blocked");
    let driver = test_git_command()
        .arg("-C")
        .arg(&repository)
        .args(["config", "--get", "merge.translation.driver"])
        .output()
        .unwrap();
    assert!(driver.status.success());
    assert_eq!(output_text(&driver.stdout).trim(), "approved-driver %A");

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn squash_honors_rejecting_commit_hooks_and_restores_the_original_head() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-squash-hook-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "squash-hook@example.invalid"]);
    run_git(&["config", "user.name", "Squash Hook Test"]);
    for (content, message) in [
        ("base\n", "base"),
        ("base\none\n", "one"),
        ("base\none\ntwo\n", "two"),
    ] {
        fs::write(repository.join("history.txt"), content).unwrap();
        run_git(&["add", "history.txt"]);
        run_git(&["commit", "-qm", message]);
    }
    let hooks = repository.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let pre_commit = hooks.join("pre-commit");
    fs::write(&pre_commit, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&pre_commit, fs::Permissions::from_mode(0o700)).unwrap();
    let git = GitCore::new();
    let original_head = git.head_commit_id(&repository).unwrap();
    assert!(git
        .squash_recent_commits(&repository, 2, "must fail")
        .is_err());
    assert_eq!(git.head_commit_id(&repository).unwrap(), original_head);
    assert_eq!(git.commits(&repository, 10).unwrap().len(), 3);

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn squash_commits_the_tree_produced_by_commit_hooks() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-squash-format-hook-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "squash-format@example.invalid"]);
    run_git(&["config", "user.name", "Squash Format Hook Test"]);
    for (content, message) in [
        ("base\n", "base"),
        ("base\none\n", "one"),
        ("base\none\ntwo\n", "two"),
    ] {
        fs::write(repository.join("history.txt"), content).unwrap();
        run_git(&["add", "history.txt"]);
        run_git(&["commit", "-qm", message]);
    }
    let hooks = repository.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let pre_commit = hooks.join("pre-commit");
    fs::write(
        &pre_commit,
        "#!/bin/sh\nprintf 'formatted\\n' >> history.txt\ngit add history.txt\n",
    )
    .unwrap();
    fs::set_permissions(&pre_commit, fs::Permissions::from_mode(0o700)).unwrap();
    GitCore::new()
        .squash_recent_commits(&repository, 2, "squashed")
        .expect("a formatting hook may stage its normal commit changes");
    let output = test_git_command()
        .args(["-C", repository.to_str().unwrap()])
        .args(["show", "HEAD:history.txt"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output_text(&output.stdout), "base\none\ntwo\nformatted\n");

    let _ = fs::remove_dir_all(repository);
}
