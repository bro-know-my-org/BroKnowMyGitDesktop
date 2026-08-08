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

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
}

#[test]
fn push_with_url_sets_the_named_upstream_after_the_network_push() {
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

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(remote);
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
