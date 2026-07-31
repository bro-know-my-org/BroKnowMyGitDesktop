#[test]
fn parses_porcelain_v2_status_with_rename_and_untracked_path() {
    let raw = concat!(
        "# branch.oid 0123456789abcdef0123456789abcdef01234567\n",
        "# branch.head main\n",
        "# branch.upstream origin/main\n",
        "# branch.ab +2 -3\n",
        "1 M. N... 100644 100644 100644 abcdef1 abcdef2 README.md\0",
        "2 R. N... 100644 100644 100644 abcdef1 abcdef2 R100 renamed file.txt\0",
        "original file.txt\0",
        "? untracked file.txt\0"
    )
    .as_bytes();

    let status = parse_status_porcelain_v2(raw).unwrap();
    assert_eq!(
        status.head_commit.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (2, 3));
    assert_eq!(status.entries.len(), 3);
    assert_eq!(status.entries[0].index_status, "M");
    assert_eq!(status.entries[0].worktree_status, "");
    assert_eq!(status.entries[1].path, "renamed file.txt");
    assert_eq!(
        status.entries[1].original_path.as_deref(),
        Some("original file.txt")
    );
    assert_eq!(status.entries[2].path, "untracked file.txt");
    assert_eq!(status.entries[2].index_status, "");
    assert_eq!(status.entries[2].worktree_status, "?");
}

#[test]
fn parses_nul_terminated_status_headers_too() {
    let raw = concat!(
        "# branch.head feature\0",
        "# branch.ab +0 -1\0",
        "1 A. N... 100644 100644 100644 a b src/lib.rs\0",
    )
    .as_bytes();
    let status = parse_status_porcelain_v2(raw).unwrap();
    assert_eq!(status.branch.as_deref(), Some("feature"));
    assert_eq!((status.ahead, status.behind), (0, 1));
    assert_eq!(status.entries[0].path, "src/lib.rs");
    assert_eq!(status.entries[0].index_status, "A");
    assert_eq!(status.entries[0].worktree_status, "");
}

#[test]
fn parses_branch_and_tag_output() {
    let branches = parse_branches(b"*\0main\0origin/main\n \0topic\0\n").unwrap();
    assert_eq!(branches.len(), 2);
    assert!(branches[0].is_current);
    assert_eq!(branches[1].upstream, None);

    let tags = parse_tags(b"v1.0.0\0peeled123\0tag123\nv0.9.0\0\0def456\n").unwrap();
    assert_eq!(tags[0].name, "v1.0.0");
    assert_eq!(tags[0].target, "peeled123");
    assert_eq!(tags[1].target, "def456");
}

#[test]
fn parses_remote_branches_without_symbolic_or_stale_refs() {
    let raw = concat!(
        "refs/remotes/origin/HEAD\0refs/remotes/origin/main\n",
        "refs/remotes/origin/feature/demo\0\n",
        "refs/remotes/team/origin/release\0\n",
        "refs/remotes/removed/legacy\0\n",
    )
    .as_bytes();
    let remote_names = vec!["origin".to_owned(), "team/origin".to_owned()];

    let branches = parse_remote_branches(raw, &remote_names).unwrap();

    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0].remote_name, "origin");
    assert_eq!(branches[0].name, "feature/demo");
    assert_eq!(branches[1].remote_name, "team/origin");
    assert_eq!(branches[1].name, "release");
    assert_eq!(
        remote_tracking_reference("origin", "feature/demo"),
        "refs/remotes/origin/feature/demo"
    );
    assert_eq!(
        remote_tracking_branch_name("origin", "feature/demo"),
        "origin/feature/demo"
    );
    assert_eq!(
        all_remote_branches_fetch_refspec("origin").unwrap(),
        "+refs/heads/*:refs/remotes/origin/*"
    );
}

#[test]
fn fetches_all_remote_branches_without_rewriting_refspecs_and_prunes_deletion() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-remote-branch-test-{}-{suffix}",
        std::process::id()
    ));
    let source = root.join("source");
    let remote = root.join("remote.git");
    let clone = root.join("clone");
    fs::create_dir_all(&source).unwrap();
    let run_git = |repository: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };

    let remote_initialized = test_git_command()
        .args(["init", "--bare", "-q", remote.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(remote_initialized.success());
    run_git(&source, &["init", "-q"]);
    run_git(&source, &["switch", "-c", "main"]);
    run_git(
        &source,
        &["config", "user.email", "remote-test@example.invalid"],
    );
    run_git(&source, &["config", "user.name", "Remote Branch Test"]);
    fs::write(source.join("file.txt"), "main\n").unwrap();
    run_git(&source, &["add", "file.txt"]);
    run_git(&source, &["commit", "-qm", "main"]);
    run_git(
        &source,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&source, &["push", "-u", "origin", "main"]);
    run_git(&source, &["switch", "-c", "release"]);
    fs::write(source.join("file.txt"), "release\n").unwrap();
    run_git(&source, &["commit", "-am", "release"]);
    run_git(&source, &["push", "-u", "origin", "release"]);

    let cloned = test_git_command()
        .args([
            "clone",
            "--no-local",
            "--single-branch",
            "--branch",
            "main",
            remote.to_str().unwrap(),
            clone.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(cloned.success());

    let git = GitCore::new();
    let before_fetch = git.remote_branches(&clone).unwrap();
    assert!(before_fetch.iter().all(|branch| branch.name != "release"));

    // This is the shape written by the earlier implementation: the
    // original exact single-branch refspec remained beside the wildcard.
    // Once that exact branch is deleted remotely, `git fetch` fails.
    run_git(
        &clone,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    run_git(
        &clone,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );
    run_git(
        &clone,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/changes/*:refs/remotes/origin/changes/*",
        ],
    );
    let configured_refspecs_before = test_git_command()
        .args([
            "-C",
            clone.to_str().unwrap(),
            "config",
            "--get-all",
            "remote.origin.fetch",
        ])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(
        git.fetch_refspecs_for_remote(&clone, "origin")
            .expect("all branch and non-branch mappings are captured before fetch"),
        vec![
            "+refs/heads/*:refs/remotes/origin/*",
            "+refs/changes/*:refs/remotes/origin/changes/*",
        ]
    );
    git.fetch(
        &clone,
        &FetchOptions {
            remote: "origin".into(),
            refspecs: Vec::new(),
            prune: true,
        },
    )
    .unwrap();
    let remote_branches = git.remote_branches(&clone).unwrap();
    assert!(remote_branches
        .iter()
        .any(|branch| { branch.remote_name == "origin" && branch.name == "release" }));

    git.switch_remote_branch(&clone, "origin", "release")
        .unwrap();
    let release = git
        .branches(&clone)
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "release")
        .expect("a local tracking branch is created for the selected remote branch");
    assert!(release.is_current);
    assert_eq!(release.upstream.as_deref(), Some("origin/release"));

    let configured_refspecs = test_git_command()
        .args([
            "-C",
            clone.to_str().unwrap(),
            "config",
            "--get-all",
            "remote.origin.fetch",
        ])
        .output()
        .unwrap();
    assert!(configured_refspecs.status.success());
    assert_eq!(configured_refspecs.stdout, configured_refspecs_before);

    run_git(&source, &["push", "origin", "--delete", "release"]);
    git.fetch(
        &clone,
        &FetchOptions {
            remote: "origin".into(),
            refspecs: Vec::new(),
            prune: true,
        },
    )
    .expect("fetching succeeds after a previously tracked remote branch is deleted");
    assert!(git
        .remote_branches(&clone)
        .unwrap()
        .iter()
        .all(|branch| branch.name != "release"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn switching_existing_tracking_branch_does_not_expand_fetch_refspec() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-existing-tracking-branch-test-{}-{suffix}",
        std::process::id()
    ));
    let repository = root.join("repository");
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
    run_git(&["config", "user.email", "tracking@example.invalid"]);
    run_git(&["config", "user.name", "Tracking Branch Test"]);
    run_git(&["checkout", "-qb", "main"]);
    fs::write(repository.join("file.txt"), "main\n").unwrap();
    run_git(&["add", "file.txt"]);
    run_git(&["commit", "-qm", "main"]);
    run_git(&[
        "remote",
        "add",
        "origin",
        root.join("origin.git").to_str().unwrap(),
    ]);
    run_git(&["update-ref", "refs/remotes/origin/release", "HEAD"]);
    run_git(&["checkout", "-qb", "release"]);
    run_git(&["branch", "--set-upstream-to=origin/release", "release"]);
    run_git(&["checkout", "-q", "main"]);
    run_git(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/main:refs/remotes/origin/main",
    ]);

    let read_refspec = || {
        let output = test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "config",
                "--get-all",
                "remote.origin.fetch",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        output.stdout
    };
    let before = read_refspec();
    let release_before_switch = GitCore::new()
        .branches(&repository)
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "release")
        .expect("release branch remains available");
    assert_eq!(release_before_switch.upstream, None);

    GitCore::new()
        .switch_remote_branch(&repository, "origin", "release")
        .expect("an existing matching tracking branch switches directly");

    assert_eq!(read_refspec(), before);
    let current = GitCore::new()
        .branches(&repository)
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "release")
        .expect("release branch remains available");
    assert!(current.is_current);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn parses_worktree_porcelain_with_linked_state() {
    let raw = concat!(
        "worktree /workspace/main\0",
        "HEAD abcdef123\0",
        "branch refs/heads/main\0\0",
        "worktree /workspace/topic\0",
        "HEAD fedcba987\0",
        "detached\0",
        "locked maintenance\0",
        "prunable stale metadata\0\0",
    )
    .as_bytes();
    let worktrees = parse_worktrees(raw).unwrap();
    assert_eq!(worktrees.len(), 2);
    assert!(worktrees[0].is_main);
    assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    assert!(!worktrees[1].is_main);
    assert!(worktrees[1].is_detached);
    assert!(worktrees[1].is_locked);
    assert!(worktrees[1].is_prunable);
}

#[test]
fn parses_submodule_status_markers() {
    let raw = concat!(
        " abcdef0123456789 modules/ready (heads/main)\n",
        "-fedcba9876543210 modules/missing\n",
        "+0011223344556677 modules/stale (new commits)\n",
        "U8899aabbccddeeff modules/conflicted\n",
        " 0011223344556677 \"modules/ready (archived)\" (heads/main)\n",
    )
    .as_bytes();
    let submodules = parse_submodules(raw).unwrap();
    assert_eq!(submodules.len(), 5);
    assert_eq!(submodules[0].path, "modules/ready");
    assert_eq!(submodules[0].status, SubmoduleStatus::Initialized);
    assert_eq!(submodules[1].status, SubmoduleStatus::Uninitialized);
    assert_eq!(submodules[2].status, SubmoduleStatus::NeedsUpdate);
    assert_eq!(submodules[3].status, SubmoduleStatus::Conflicted);
    assert_eq!(submodules[4].path, "modules/ready (archived)");
}

#[test]
fn rejects_non_marker_utf8_at_the_start_of_submodule_status() {
    assert!(parse_submodules("é unexpected submodule output\n".as_bytes()).is_err());
}

#[test]
fn rejects_invalid_porcelain_status_codes() {
    assert!(status_entry_from_xy(b"Z.", b"file.txt", None).is_err());
    assert!(status_entry_from_xy(b".Z", b"file.txt", None).is_err());
}

#[test]
fn parses_machine_readable_submodule_paths() {
    assert_eq!(
        parse_submodule_config_paths(
            b"submodule.ready.path\nmodules/ready\0submodule.archived.path\nmodules/ready (archived)\0"
        )
        .unwrap(),
        vec!["modules/ready", "modules/ready (archived)"]
    );
}

#[test]
fn parses_lfs_file_names_and_validates_patterns() {
    assert_eq!(
        parse_lfs_files(b"assets/video.mov\nassets/line-break.bin\r\n"),
        vec!["assets/video.mov", "assets/line-break.bin"]
    );
    assert!(validate_lfs_pattern("*.psd").is_ok());
    assert!(validate_lfs_pattern("-unsafe").is_err());
    assert!(validate_lfs_pattern("*.zip\n--help").is_err());
}

#[test]
fn patch_validation_accepts_unified_git_diff_only() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    assert!(validate_git_patch(patch).is_ok());
    assert!(validate_git_patch("@@ -1 +1 @@\n-old\n+new\n").is_err());
    assert!(validate_git_patch("diff --git a/a b/a\0").is_err());
    assert!(validate_commit_message("valid message").is_ok());
    assert!(validate_commit_message("invalid\0message").is_err());
    assert!(validate_tag_message("release notes").is_ok());
    assert!(validate_tag_message("invalid\0message").is_err());
    assert!(validate_stash_message("draft").is_ok());
    assert!(validate_stash_message("invalid\0message").is_err());
}

#[test]
fn rejects_truncated_or_non_utf8_status_paths() {
    assert!(parse_status_porcelain_v2(b"2 incomplete\0").is_err());
    assert!(parse_status_porcelain_v2(
        b"2 R. N... 100644 100644 100644 abcdef1 abcdef2 R100 renamed.txt\0\0"
    )
    .is_err());
    assert!(parse_status_porcelain_v2(b"? invalid\xff\0").is_err());
}

#[test]
fn quotes_special_untracked_paths_in_generated_diffs() {
    let diff = render_untracked_file_diff(
        "tab\tname.txt",
        &UntrackedFileContent {
            bytes: b"content\n".to_vec(),
            is_executable: false,
        },
    );
    assert!(diff.starts_with(
        "diff --git \"a/tab\\tname.txt\" \"b/tab\\tname.txt\"\nnew file mode 100644\n"
    ));
    assert!(diff.contains("+++ \"b/tab\\tname.txt\"\n"));
}

#[test]
fn renders_nul_containing_untracked_content_as_binary() {
    let diff = render_untracked_file_diff(
        "binary.bin",
        &UntrackedFileContent {
            bytes: b"text\0payload".to_vec(),
            is_executable: false,
        },
    );
    assert!(diff.contains("Binary files /dev/null and b/binary.bin differ"));
    assert!(!diff.contains('\0'));
}

#[test]
fn applies_and_reverses_a_generated_untracked_empty_file_patch() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-untracked-patch-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(repository.join("nested")).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    let path = "nested/tab\tname.txt";
    fs::write(repository.join(path), b"").unwrap();

    let git = GitCore::new();
    let patch = git
        .diff(
            &repository,
            &DiffOptions {
                staged: false,
                paths: vec![path.into()],
                context_lines: Some(3),
            },
        )
        .unwrap();
    assert_eq!(
        git.apply_patch(&repository, "other.txt", &patch, false)
            .expect_err("a hunk must not stage a different path")
            .code(),
        "invalid_git_patch"
    );
    git.apply_patch(&repository, path, &patch, false).unwrap();
    let staged = git.status(&repository).unwrap();
    assert_eq!(staged.entries[0].path, path);
    assert_eq!(staged.entries[0].index_status, "A");

    let cached_patch = git
        .diff(
            &repository,
            &DiffOptions {
                staged: true,
                paths: vec![path.into()],
                context_lines: Some(3),
            },
        )
        .unwrap();
    git.apply_patch(&repository, path, &cached_patch, true)
        .unwrap();
    let unstaged = git.status(&repository).unwrap();
    assert_eq!(unstaged.entries[0].path, path);
    assert_eq!(unstaged.entries[0].index_status, "");
    assert_eq!(unstaged.entries[0].worktree_status, "?");

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn applies_and_reverses_a_generated_index_patch() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-patch-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "test@example.invalid"]);
    run_git(&["config", "user.name", "Patch Test"]);
    fs::write(repository.join("file.txt"), "before\n").unwrap();
    run_git(&["add", "file.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::write(repository.join("file.txt"), "after\n").unwrap();

    let git = GitCore::new();
    let head = git.head_commit_id(&repository).unwrap();
    assert!((40..=64).contains(&head.len()));
    assert!(head.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let patch = git
        .diff(
            &repository,
            &DiffOptions {
                staged: false,
                paths: vec!["file.txt".into()],
                context_lines: Some(3),
            },
        )
        .unwrap();
    git.apply_patch(&repository, "file.txt", &patch, false)
        .unwrap();
    let staged = git.status(&repository).unwrap();
    assert_eq!(staged.entries[0].index_status, "M");
    assert_eq!(staged.entries[0].worktree_status, "");

    let cached_patch = git
        .diff(
            &repository,
            &DiffOptions {
                staged: true,
                paths: vec!["file.txt".into()],
                context_lines: Some(3),
            },
        )
        .unwrap();
    git.apply_patch(&repository, "file.txt", &cached_patch, true)
        .unwrap();
    let unstaged = git.status(&repository).unwrap();
    assert_eq!(unstaged.entries[0].index_status, "");
    assert_eq!(unstaged.entries[0].worktree_status, "M");

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn unstages_files_without_head_in_an_unborn_repository() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-unstage-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join("first.txt"), "first\n").unwrap();

    let git = GitCore::new();
    git.stage_paths(&repository, &["first.txt".into()]).unwrap();
    let staged = git.status(&repository).unwrap();
    assert_eq!(staged.entries[0].index_status, "A");
    assert_eq!(staged.entries[0].worktree_status, "");

    git.unstage_paths(&repository, &["first.txt".into()])
        .unwrap();
    let unstaged = git.status(&repository).unwrap();
    assert_eq!(unstaged.entries[0].index_status, "");
    assert_eq!(unstaged.entries[0].worktree_status, "?");
    assert!(repository.join("first.txt").is_file());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_of_staged_rename_restores_both_paths() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-rename-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("old-name.txt"), "original\n").unwrap();
    run_git(&["add", "old-name.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    run_git(&["mv", "old-name.txt", "new-name.txt"]);

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["new-name.txt".into()])
        .expect("rename discard state is captured");
    git.discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("discarding a staged rename restores HEAD");

    assert_eq!(
        fs::read_to_string(repository.join("old-name.txt")).unwrap(),
        "original\n"
    );
    assert!(!repository.join("new-name.txt").exists());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_restores_a_deleted_file_and_parent_directory() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-deleted-parent-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::create_dir_all(repository.join("nested")).unwrap();
    fs::write(repository.join("nested/file.txt"), "original\n").unwrap();
    run_git(&["add", "nested/file.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::remove_file(repository.join("nested/file.txt")).unwrap();
    fs::remove_dir(repository.join("nested")).unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["nested/file.txt".into()])
        .expect("deleted path state is captured");
    git.discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the deleted file is restored without overwriting a concurrent path");

    assert_eq!(
        fs::read_to_string(repository.join("nested/file.txt")).unwrap(),
        "original\n"
    );
    assert!(git.status(&repository).unwrap().entries.is_empty());

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn failed_confirmed_discard_removes_parent_directories_created_by_the_transaction() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-parent-rollback-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::create_dir_all(repository.join("nested")).unwrap();
    fs::write(repository.join("nested/file.txt"), "original\n").unwrap();
    run_git(&["add", "nested/file.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::remove_file(repository.join("nested/file.txt")).unwrap();
    fs::remove_dir(repository.join("nested")).unwrap();

    let normal_git = GitCore::new();
    let fingerprint = normal_git
        .capture_discard_changes_fingerprint(&repository, &["nested/file.txt".into()])
        .expect("deleted path state is captured");
    let fake_git = repository.join("index-failure-git.sh");
    fs::write(
        &fake_git,
        "#!/bin/sh\nroot=$(dirname \"$0\")\nis_checkout=0\nis_ls_files=0\nfor argument in \"$@\"; do\n  if [ \"$argument\" = checkout-index ]; then is_checkout=1; fi\n  if [ \"$argument\" = ls-files ]; then is_ls_files=1; fi\ndone\nif [ \"$is_ls_files\" -eq 1 ] && [ -f \"$root/fail-next-index-read\" ]; then exit 1; fi\ngit \"$@\"\nstatus=$?\nif [ \"$status\" -eq 0 ] && [ \"$is_checkout\" -eq 1 ]; then\n  : > \"$root/fail-next-index-read\"\nfi\nexit \"$status\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    GitCore::with_executable(&fake_git)
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("the injected final index read failure aborts the discard");
    assert!(!repository.join("nested").exists());
    assert!(!repository.join("nested/file.txt").exists());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_rejects_a_changed_worktree_file() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-confirmation-worktree-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("changed.txt"), "original\n").unwrap();
    run_git(&["add", "changed.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::write(repository.join("changed.txt"), "reviewed\n").unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["changed.txt".into()])
        .expect("discard state is captured");
    fs::write(repository.join("changed.txt"), "newer\n").unwrap();

    let error = git
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("a newer edit must not be discarded by an older confirmation");
    assert_eq!(error.code(), "discard_confirmation_changed");
    assert_eq!(
        fs::read_to_string(repository.join("changed.txt")).unwrap(),
        "newer\n"
    );
    assert!(fs::read_dir(&repository).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bkm-discard-transaction-")));

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_discard_does_not_overwrite_a_file_changed_during_execution() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-execution-race-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("changed.txt"), "original\n").unwrap();
    run_git(&["add", "changed.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::write(repository.join("changed.txt"), "reviewed\n").unwrap();

    let normal_git = GitCore::new();
    let fingerprint = normal_git
        .capture_discard_changes_fingerprint(&repository, &["changed.txt".into()])
        .expect("discard state is captured");
    let fake_git = repository.join("race-git.sh");
    fs::write(
        &fake_git,
        "#!/bin/sh\nrace=0\nfor argument in \"$@\"; do\n  if [ \"$argument\" = checkout-index ]; then race=1; fi\ndone\ngit \"$@\"\nstatus=$?\nif [ \"$status\" -eq 0 ] && [ \"$race\" -eq 1 ]; then\n  target=$(dirname \"$0\")/changed.txt\n  rm -f \"$target\"\n  printf 'newer\\n' > \"$target\"\nfi\nexit \"$status\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    let error = GitCore::with_executable(&fake_git)
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("an edit racing the final mutation must not be overwritten");
    assert_eq!(error.code(), "discard_confirmation_changed");
    assert_eq!(
        fs::read_to_string(repository.join("changed.txt")).unwrap(),
        "newer\n"
    );

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_discard_holds_the_current_branch_ref_during_index_commit() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-ref-lock-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("changed.txt"), "original\n").unwrap();
    run_git(&["add", "changed.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    let reviewed_head = run_git(&["rev-parse", "HEAD"]);
    fs::write(repository.join("changed.txt"), "future\n").unwrap();
    run_git(&["commit", "-qam", "future"]);
    let future_head = run_git(&["rev-parse", "HEAD"]);
    run_git(&["reset", "--hard", &reviewed_head]);
    fs::write(repository.join("changed.txt"), "reviewed\n").unwrap();

    let normal_git = GitCore::new();
    let fingerprint = normal_git
        .capture_discard_changes_fingerprint(&repository, &["changed.txt".into()])
        .expect("discard state is captured");
    let fake_git = repository.join("ref-race-git.sh");
    fs::write(
        &fake_git,
        format!(
            "#!/bin/sh\nrace=0\nfor argument in \"$@\"; do\n  if [ \"$argument\" = checkout-index ]; then race=1; fi\ndone\ngit \"$@\"\nstatus=$?\nif [ \"$status\" -eq 0 ] && [ \"$race\" -eq 1 ]; then\n  root=$(dirname \"$0\")\n  if git -C \"$root\" update-ref refs/heads/main {future_head}; then\n    printf changed > \"$root/ref-race-result\"\n  else\n    printf blocked > \"$root/ref-race-result\"\n  fi\nfi\nexit \"$status\"\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    GitCore::with_executable(&fake_git)
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the guarded discard succeeds against the reviewed commit");
    assert_eq!(run_git(&["rev-parse", "HEAD"]), reviewed_head);
    assert_eq!(
        fs::read_to_string(repository.join("ref-race-result")).unwrap(),
        "blocked"
    );
    assert_eq!(
        fs::read_to_string(repository.join("changed.txt")).unwrap(),
        "original\n"
    );

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_discard_supports_a_split_index() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-split-index-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("changed.txt"), "original\n").unwrap();
    run_git(&["add", "changed.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    run_git(&["update-index", "--split-index"]);
    fs::write(repository.join("changed.txt"), "reviewed\n").unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["changed.txt".into()])
        .expect("split-index discard state is captured");
    git.discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the prepared index resolves its adjacent shared index");

    assert_eq!(
        fs::read_to_string(repository.join("changed.txt")).unwrap(),
        "original\n"
    );
    assert!(git.status(&repository).unwrap().entries.is_empty());
    let git_directory = repository.join(".git");
    assert!(fs::read_dir(git_directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bkm-discard-index-")));

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_discard_rejects_gitlinks_before_mutation() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-gitlink-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("seed.txt"), "one\n").unwrap();
    run_git(&["add", "seed.txt"]);
    run_git(&["commit", "-qm", "one"]);
    let first = run_git(&["rev-parse", "HEAD"]);
    fs::write(repository.join("seed.txt"), "two\n").unwrap();
    run_git(&["commit", "-qam", "two"]);
    let second = run_git(&["rev-parse", "HEAD"]);
    run_git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{first},submodule"),
    ]);
    run_git(&["commit", "-qm", "add gitlink"]);
    run_git(&[
        "update-index",
        "--cacheinfo",
        &format!("160000,{second},submodule"),
    ]);

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["submodule".into()])
        .expect("gitlink discard state is captured");
    let error = git
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("gitlink worktree changes need a dedicated protected transaction");
    assert_eq!(error.code(), "discard_protected_unsupported");
    let staged = run_git(&["ls-files", "--stage", "submodule"]);
    assert!(staged.contains(&second));

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_rejects_an_index_change() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-confirmation-index-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("changed.txt"), "original\n").unwrap();
    run_git(&["add", "changed.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::write(repository.join("changed.txt"), "reviewed\n").unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["changed.txt".into()])
        .expect("discard state is captured");
    run_git(&["add", "changed.txt"]);

    let error = git
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("a changed index must invalidate the confirmation");
    assert_eq!(error.code(), "discard_confirmation_changed");
    assert_eq!(
        git.status(&repository).unwrap().entries[0].index_status,
        "M"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_rejects_a_replaced_untracked_file() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-confirmation-untracked-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join("untracked.txt"), "reviewed\n").unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
        .expect("discard state is captured");
    fs::remove_file(repository.join("untracked.txt")).unwrap();
    fs::write(repository.join("untracked.txt"), "replacement\n").unwrap();

    let error = git
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect_err("a replacement must not inherit the old confirmation");
    assert_eq!(error.code(), "discard_confirmation_changed");
    assert_eq!(
        fs::read_to_string(repository.join("untracked.txt")).unwrap(),
        "replacement\n"
    );

    let _ = fs::remove_dir_all(repository);
}

#[cfg(target_os = "linux")]
#[test]
fn confirmed_untracked_discard_blocks_promotion_into_the_index() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-untracked-index-race-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let output = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success());
        output_text(&output.stdout).trim().to_owned()
    };
    run_git(&["init", "-q"]);
    fs::write(repository.join("untracked.txt"), "reviewed\n").unwrap();

    let normal_git = GitCore::new();
    let fingerprint = normal_git
        .capture_discard_changes_fingerprint(&repository, &["untracked.txt".into()])
        .expect("untracked discard state is captured");
    let fake_git = repository.join("index-promotion-git.sh");
    fs::write(
        &fake_git,
        "#!/bin/sh\nroot=$(dirname \"$0\")\nis_ls_files=0\nfor argument in \"$@\"; do\n  if [ \"$argument\" = ls-files ]; then is_ls_files=1; fi\ndone\nif [ \"$is_ls_files\" -eq 1 ] && [ -f \"$root/.git/index.lock\" ]; then\n  if git -C \"$root\" add untracked.txt; then\n    printf promoted > \"$root/index-promotion-result\"\n  else\n    printf blocked > \"$root/index-promotion-result\"\n  fi\nfi\ngit \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();

    GitCore::with_executable(&fake_git)
        .discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the untracked path is claimed while index writers are blocked");
    assert_eq!(
        fs::read_to_string(repository.join("index-promotion-result")).unwrap(),
        "blocked"
    );
    assert!(!repository.join("untracked.txt").exists());
    assert!(run_git(&["ls-files", "untracked.txt"]).is_empty());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_restores_tracked_and_removes_untracked_paths() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-confirmed-discard-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let run_git = |arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "discard@example.invalid"]);
    run_git(&["config", "user.name", "Discard Test"]);
    fs::write(repository.join("tracked.txt"), "original\n").unwrap();
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-qm", "initial"]);
    fs::write(repository.join("tracked.txt"), "changed\n").unwrap();
    fs::write(repository.join("untracked.txt"), "temporary\n").unwrap();

    let git = GitCore::new();
    let fingerprint = git
        .capture_discard_changes_fingerprint(
            &repository,
            &["tracked.txt".into(), "untracked.txt".into()],
        )
        .expect("discard state is captured");
    git.discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the exact reviewed state is discarded");

    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "original\n"
    );
    assert!(!repository.join("untracked.txt").exists());
    assert!(git.status(&repository).unwrap().entries.is_empty());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn confirmed_discard_removes_a_staged_file_from_an_unborn_repository() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-confirmed-discard-unborn-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let _cleanup = TemporaryTestDirectory(repository.clone());
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join("first.txt"), "first\n").unwrap();
    let git = GitCore::new();
    git.stage_paths(&repository, &["first.txt".into()]).unwrap();

    let fingerprint = git
        .capture_discard_changes_fingerprint(&repository, &["first.txt".into()])
        .expect("discard state is captured");
    git.discard_paths_with_fingerprint(&repository, &fingerprint)
        .expect("the staged initial file is discarded");

    assert!(!repository.join("first.txt").exists());
    assert!(git.status(&repository).unwrap().entries.is_empty());

    let _ = fs::remove_dir_all(repository);
}

