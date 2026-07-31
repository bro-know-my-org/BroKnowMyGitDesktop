#[test]
fn reads_blame_and_reflog_and_cleans_only_untracked_items() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-inspection-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "inspection@example.invalid"]);
    run_git(&["config", "user.name", "Inspection Test"]);
    fs::write(repository.join("tracked.txt"), "tracked line\n").unwrap();
    run_git(&["add", "tracked.txt"]);
    run_git(&["commit", "-qm", "initial tracked file"]);
    fs::write(repository.join("untracked.txt"), "remove me\n").unwrap();
    fs::create_dir_all(repository.join("generated")).unwrap();
    fs::write(repository.join("generated/output.txt"), "remove me too\n").unwrap();
    fs::create_dir_all(repository.join("preserved")).unwrap();
    fs::write(repository.join("preserved/output.txt"), "remove me too\n").unwrap();

    let git = GitCore::new();
    let blame = git.blame(&repository, "tracked.txt").unwrap();
    assert!(blame.contains("Inspection Test"));
    let reflog = git.reflog(&repository, 10).unwrap();
    assert!(reflog.contains("initial tracked file"));
    let previewed_paths = vec![
        "untracked.txt".into(),
        "generated/output.txt".into(),
        "preserved/output.txt".into(),
    ];
    let preview = git
        .preview_clean_untracked_paths(&repository, &previewed_paths)
        .unwrap();
    assert!(preview.contains("Would remove untracked.txt"));
    assert!(preview.contains("Would remove generated/output.txt"));
    assert!(preview.contains("Would remove preserved/output.txt"));
    let fingerprints = git
        .capture_clean_untracked_path_fingerprints(&repository, &previewed_paths)
        .expect("previewed paths are captured before later files appear");
    // A later file must not be captured by the previously reviewed set.
    fs::write(repository.join("created-after-preview.txt"), "keep me\n").unwrap();
    fs::write(
        repository.join("preserved/created-after-preview.txt"),
        "keep me\n",
    )
    .unwrap();

    git.clean_untracked_paths_with_fingerprints(&repository, &fingerprints)
        .unwrap();
    assert!(!repository.join("untracked.txt").exists());
    assert!(!repository.join("generated").exists());
    assert!(!repository.join("preserved/output.txt").exists());
    assert!(repository
        .join("preserved/created-after-preview.txt")
        .exists());
    assert!(repository.join("created-after-preview.txt").exists());
    assert!(repository.join("tracked.txt").exists());

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn clean_confirmation_rejects_a_replaced_untracked_path() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clean-fingerprint-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let target = repository.join("previewed.txt");
    fs::write(&target, "original\n").unwrap();

    let git = GitCore::new();
    let fingerprints = git
        .capture_clean_untracked_path_fingerprints(&repository, &["previewed.txt".into()])
        .expect("the preview captures the original file identity");
    fs::remove_file(&target).unwrap();
    fs::write(&target, "replacement\n").unwrap();

    assert_eq!(
        git.validate_clean_untracked_path_fingerprints(&repository, &fingerprints)
            .expect_err("a replacement at the same path requires a new preview")
            .code(),
        "clean_untracked_confirmation_changed"
    );
    assert!(target.exists());

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn opens_actual_conflict_files_with_the_configured_editor() {
    use std::os::unix::fs::PermissionsExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-editor-conflict-test-{}-{suffix}",
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
    run_git(&["config", "user.email", "editor@example.invalid"]);
    run_git(&["config", "user.name", "Editor Test"]);
    fs::write(repository.join("conflict.txt"), "base\n").unwrap();
    run_git(&["add", "conflict.txt"]);
    run_git(&["commit", "-qm", "base"]);
    let initial_branch = String::from_utf8(
        test_git_command()
            .args([
                "-C",
                repository.to_str().unwrap(),
                "symbolic-ref",
                "--short",
                "HEAD",
            ])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    run_git(&["checkout", "-qb", "other"]);
    fs::write(repository.join("conflict.txt"), "other\n").unwrap();
    run_git(&["commit", "-am", "other change"]);
    run_git(&["checkout", "-q", &initial_branch]);
    fs::write(repository.join("conflict.txt"), "main\n").unwrap();
    run_git(&["commit", "-am", "main change"]);

    let merge = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "merge",
            "--no-edit",
            "other",
        ])
        .status()
        .unwrap();
    assert!(!merge.success());
    let editor = repository.join("editor.sh");
    fs::write(&editor, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).unwrap();
    let opened = GitCore::new()
        .open_conflicted_files_in_external_editor(
            &repository,
            &ExternalEditorConfig {
                executable: editor.to_string_lossy().into_owned(),
                arguments: Vec::new(),
            },
        )
        .expect("configured editor receives each conflict file");
    assert_eq!(opened, vec!["conflict.txt"]);

    let _ = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "merge", "--abort"])
        .status();
    let _ = fs::remove_dir_all(repository);
}

#[test]
fn revision_validation_accepts_git_expressions_but_rejects_options() {
    assert!(validate_revision("HEAD~2", "引用").is_ok());
    assert!(validate_revision("stash@{0}", "引用").is_ok());
    assert!(validate_revision("-bad", "引用").is_err());
    assert!(validate_revision("HEAD\n--hard", "引用").is_err());
}

#[test]
fn ref_name_validation_rejects_revision_expressions() {
    assert!(validate_ref_name("feature/ready", "分支名").is_ok());
    for value in [
        "HEAD~1",
        "main^{commit}",
        "topic..other",
        "topic@{1}",
        "topic\tname",
        "topic\u{1b}name",
        "topic\u{7f}name",
    ] {
        assert!(
            validate_ref_name(value, "分支名").is_err(),
            "{value} must not reach a branch-only operation"
        );
    }
}

#[cfg(unix)]
#[test]
fn git_path_output_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path =
        super::runtime::git_path_from_output(b"/tmp/repository-\xff\n", "test path").unwrap();
    assert_eq!(path.as_os_str().as_bytes(), b"/tmp/repository-\xff");
    assert_eq!(
        repository_record_from_root(&path)
            .expect_err("the string-based UI must not receive a lossy path")
            .code(),
        "unsupported_repository_path_encoding"
    );
}

#[test]
fn commit_diff_lossily_decodes_non_utf8_blob_content() {
    let output = super::history::commit_diff_output_text(
        b"diff --git a/file b/file\n@@ -1 +1 @@\n-plain\n+\xff\n".to_vec(),
    )
    .expect("display-only blob content can be decoded lossily");
    assert!(output.contains("diff --git a/file b/file"));
    assert!(output.contains('\u{fffd}'));
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn no_replace_clone_rename_never_overwrites_a_destination() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-clone-no-replace-test-{}-{suffix}",
        std::process::id()
    ));
    let source = root.join("staging");
    let destination = root.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();

    assert!(super::repository::rename_directory_without_replacing(&source, &destination).is_err());
    assert!(source.is_dir());
    assert!(destination.is_dir());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn discard_path_validation_cannot_escape_the_worktree() {
    assert!(validate_worktree_relative_path("src/App.vue").is_ok());
    assert!(validate_worktree_relative_path("nested/file name.txt").is_ok());
    assert!(validate_worktree_relative_path("../outside.txt").is_err());
    assert!(validate_worktree_relative_path("./file.txt").is_err());
    assert!(validate_worktree_relative_path(".").is_err());
}

#[test]
fn discard_rejects_a_paused_git_operation() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-operation-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join(".git/MERGE_HEAD"), "deadbeef\n").unwrap();

    assert_eq!(
        GitCore::new()
            .discard_paths(&repository, &["changed.txt".into()])
            .expect_err("discard must not alter a paused Git operation")
            .code(),
        "git_operation_in_progress"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn rename_branch_rejects_a_paused_git_operation() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-paused-branch-rename-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(repository.join(".git/MERGE_HEAD"), "deadbeef\n").unwrap();

    assert_eq!(
        GitCore::new()
            .rename_branch(&repository, "main", "renamed")
            .expect_err("branch metadata must not change during a paused operation")
            .code(),
        "git_operation_in_progress"
    );

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn ui_paths_are_converted_to_literal_git_pathspecs() {
    assert_eq!(
        literal_pathspec(":(glob)**").to_string_lossy(),
        ":(literal):(glob)**"
    );
    assert_eq!(
        literal_pathspec("src/App.vue").to_string_lossy(),
        ":(literal)src/App.vue"
    );
    assert!(validate_paths(&[":(exclude)important.txt".into()]).is_ok());
}

#[cfg(unix)]
#[test]
fn untracked_diff_reader_rejects_symlinked_components() {
    use std::os::unix::fs::symlink;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-symlink-test-{}-{suffix}",
        std::process::id()
    ));
    let outside = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-symlink-outside-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "outside\n").unwrap();
    symlink(&outside, repository.join("linked")).unwrap();

    assert_eq!(
        read_untracked_file_safely(&repository, "linked/secret.txt")
            .unwrap_err()
            .code(),
        "untracked_diff_unavailable"
    );

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(outside);
}

#[cfg(unix)]
#[test]
fn untracked_diff_reader_rejects_hard_linked_content() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-hard-link-test-{}-{suffix}",
        std::process::id()
    ));
    let outside = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-hard-link-outside-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.txt");
    fs::write(&secret, "outside\n").unwrap();
    fs::hard_link(&secret, repository.join("linked.txt")).unwrap();

    assert_eq!(
        read_untracked_file_safely(&repository, "linked.txt")
            .expect_err("a worktree file must not expose a hard-linked external inode")
            .code(),
        "untracked_diff_unavailable"
    );

    let _ = fs::remove_dir_all(repository);
    let _ = fs::remove_dir_all(outside);
}

#[cfg(unix)]
#[test]
fn untracked_diff_reader_rejects_fifos_without_waiting_for_a_writer() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-untracked-fifo-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let fifo = repository.join("pending.fifo");
    let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo_path is NUL-terminated and names a fresh test artifact.
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);

    let started = Instant::now();
    assert_eq!(
        read_untracked_file_safely(&repository, "pending.fifo")
            .expect_err("a FIFO is not a readable untracked text file")
            .code(),
        "untracked_diff_unavailable"
    );
    assert!(started.elapsed() < Duration::from_secs(2));

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn repository_discovery_rejects_a_symlinked_root() {
    use std::os::unix::fs::symlink;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discovery-symlink-test-{}-{suffix}",
        std::process::id()
    ));
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, &link).unwrap();

    assert_eq!(
        GitCore::new()
            .discover_repositories(&link, 1)
            .expect_err("repository discovery must not follow the selected symlink")
            .code(),
        "invalid_repository_root"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repository_discovery_skips_live_clone_staging_directories() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discovery-staging-test-{}-{suffix}",
        std::process::id()
    ));
    let normal_repository = root.join("normal");
    let similarly_named_repository = root.join(".bkm-clone-staging-docs");
    let staging_repository = root
        .join(".bkm-clone-staging-123e4567-e89b-12d3-a456-426614174000")
        .join("repository");
    fs::create_dir_all(&normal_repository).unwrap();
    fs::create_dir_all(&similarly_named_repository).unwrap();
    fs::create_dir_all(&staging_repository).unwrap();
    for repository in [
        &normal_repository,
        &similarly_named_repository,
        &staging_repository,
    ] {
        let initialized = test_git_command()
            .args(["-C", repository.to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success());
    }

    let repositories = GitCore::new().discover_repositories(&root, 3).unwrap();
    assert_eq!(repositories.len(), 2);
    let paths: Vec<PathBuf> = repositories
        .into_iter()
        .map(|repository| PathBuf::from(repository.path))
        .collect();
    assert!(paths.contains(&fs::canonicalize(&normal_repository).unwrap()));
    assert!(paths.contains(&fs::canonicalize(&similarly_named_repository).unwrap()));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn recursive_submodule_update_rejects_unsafe_initialized_submodule_config() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-submodule-config-test-{}-{suffix}",
        std::process::id()
    ));
    let repository = root.join("repository");
    let module = root.join("module");
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&module).unwrap();
    let run_git = |path: &Path, arguments: &[&str]| {
        let status = test_git_command()
            .args(["-C", path.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };

    run_git(&module, &["init", "-q"]);
    run_git(
        &module,
        &["config", "user.email", "submodule@example.invalid"],
    );
    run_git(&module, &["config", "user.name", "Submodule Test"]);
    fs::write(module.join("module.txt"), "module\n").unwrap();
    run_git(&module, &["add", "module.txt"]);
    run_git(&module, &["commit", "-qm", "module"]);

    run_git(&repository, &["init", "-q"]);
    run_git(
        &repository,
        &["config", "user.email", "repository@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "Repository Test"]);
    fs::write(repository.join("README.md"), "repository\n").unwrap();
    run_git(&repository, &["add", "README.md"]);
    run_git(&repository, &["commit", "-qm", "initial"]);
    let added = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            module.to_str().unwrap(),
            "nested",
        ])
        .status()
        .unwrap();
    assert!(added.success());
    run_git(&repository, &["commit", "-am", "add submodule"]);
    run_git(
        &repository.join("nested"),
        &["config", "core.sshCommand", "ssh untrusted-host"],
    );

    assert_eq!(
        GitCore::new()
            .update_submodules(&repository, false, true)
            .expect_err("nested local network configuration must be rejected")
            .code(),
        "unsafe_git_config"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn submodule_update_reserves_configured_uninitialized_destinations() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-submodule-lease-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    fs::write(
        repository.join(".gitmodules"),
        "[submodule \"future\"]\n\tpath = dependencies/future\n\turl = https://example.invalid/future.git\n",
    )
    .unwrap();

    let paths = GitCore::new()
        .submodule_operation_repository_paths(&repository, false)
        .unwrap();
    let canonical_repository = fs::canonicalize(&repository)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(paths.contains(&canonical_repository));
    assert!(paths.contains(
        &repository
            .join("dependencies/future")
            .to_string_lossy()
            .into_owned()
    ));

    let _ = fs::remove_dir_all(repository);
}

#[test]
fn git_failure_never_echoes_token_like_diagnostic_content() {
    let error = git_error_from_diagnostic(
        "推送改动",
        "remote: authentication failed for https://secret-token@gitee.com/org/repo.git",
    );
    let returned = format!(
        "{} {}",
        error.message(),
        error.details().unwrap_or_default()
    );
    assert_eq!(error.code(), "git_authentication_failed");
    assert!(!returned.contains("secret-token"));
}

#[test]
fn generic_access_denied_is_not_misclassified_as_an_askpass_failure() {
    assert_ne!(
        git_error_from_diagnostic("获取远程更新", "remote: access denied").code(),
        "git_authentication_failed"
    );
}

#[cfg(unix)]
#[test]
fn network_runner_classifies_bounded_private_diagnostics() {
    let error = GitCore::with_executable("sh")
        .run_network(
            None,
            &os_arguments(&["-c", "printf 'authentication failed' >&2; exit 1"]),
            &GitEnvironment::default(),
            "测试网络命令",
        )
        .expect_err("the helper exits with an authentication diagnostic");
    assert_eq!(error.code(), "git_authentication_failed");
    assert!(!error.message().contains("authentication failed"));
}

#[cfg(unix)]
#[test]
fn bounded_local_runner_stops_an_oversized_ipc_response() {
    let error = GitCore::with_executable("sh")
        .run_with_output_limit(
            None,
            &os_arguments(&["-c", "head -c 4096 /dev/zero"]),
            &GitEnvironment::default(),
            128,
            "测试有界输出",
        )
        .expect_err("the stdout cap must stop the child before buffering its full output");
    assert_eq!(error.code(), "git_output_too_large");
}

#[test]
fn classifies_a_deleted_remote_branch_without_echoing_its_ref() {
    let error = git_error_from_diagnostic(
        "获取远程更新",
        "fatal: couldn't find remote ref refs/heads/released-feature",
    );
    assert_eq!(error.code(), "git_remote_ref_not_found");
    assert_eq!(
        error.details(),
        Some("远端分支可能已删除；请获取远程更新并刷新分支列表。")
    );
}

#[test]
fn returned_remote_urls_run_through_domain_sanitizer() {
    assert_eq!(
        safe_remote_url("https://token-value@github.com/org/repo.git?access_token=hidden"),
        "https://github.com/org/repo.git"
    );
}

#[test]
fn repository_configured_excludes_is_deferred_until_the_file_exists() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-excludes-path-test-{}-{suffix}",
        std::process::id()
    )));
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let configured_path = Path::new("config/ignore");
    let excludes_file = repository.join(configured_path);
    let configured = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "core.excludesFile",
            configured_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(configured.success());

    assert_eq!(
        GitCore::new()
            .worktree_excludes_file(&repository)
            .unwrap()
            .map(|exclude| (exclude.path, exclude.watch_externally, exclude.load_rules)),
        Some((excludes_file.clone(), false, false))
    );
    fs::create_dir_all(excludes_file.parent().unwrap()).unwrap();
    fs::write(&excludes_file, "generated/\n").unwrap();
    assert_eq!(
        GitCore::new()
            .worktree_excludes_file(&repository)
            .unwrap()
            .map(|exclude| (exclude.path, exclude.watch_externally, exclude.load_rules)),
        Some((fs::canonicalize(excludes_file).unwrap(), false, true))
    );
}

#[test]
fn repository_configured_excludes_cannot_expand_watcher_scope_outside_repository() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-excludes-scope-test-{}-{suffix}",
        std::process::id()
    )));
    let initialized = test_git_command()
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let external_directory = TemporaryTestDirectory::create(std::env::temp_dir().join(format!(
        "broknowmygitdesktop-external-excludes-test-{}-{suffix}",
        std::process::id()
    )));
    let external_excludes_file = external_directory.join("ignore");
    fs::write(&external_excludes_file, "ignored/\n").unwrap();
    let configured = test_git_command()
        .args([
            "-C",
            repository.to_str().unwrap(),
            "config",
            "core.excludesFile",
            external_excludes_file.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(configured.success());

    assert_eq!(
        GitCore::new()
            .worktree_excludes_file(&repository)
            .unwrap()
            .map(|exclude| exclude.path),
        None
    );
}
