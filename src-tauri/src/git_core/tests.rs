use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn as_strings(arguments: &[OsString]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

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
    let editor = normalize_external_editor_config(ExternalEditorConfig {
        executable: "  code  ".into(),
        arguments: vec![" --wait ".into(), "--reuse-window".into()],
    })
    .expect("editor configuration is valid");
    assert_eq!(editor.executable, "code");
    assert_eq!(editor.arguments, vec!["--wait", "--reuse-window"]);

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
    command.env("GIT_DIR", "/untrusted/git-dir");
    apply_git_environment(&mut command, &environment);
    let values: std::collections::HashMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(|value| value.to_os_string())))
        .collect();

    assert_eq!(values.get(OsStr::new("GIT_ASKPASS")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_CONFIG_COUNT")), Some(&None));
    assert_eq!(values.get(OsStr::new("GIT_DIR")), Some(&None));
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
        Some(&Some(OsString::from("6")))
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
        Some(&Some(OsString::from("7")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_KEY_6")),
        Some(&Some(OsString::from("core.hooksPath")))
    );
    assert_eq!(
        values.get(OsStr::new("GIT_CONFIG_VALUE_6")),
        Some(&Some(OsString::from(DISABLED_GIT_HOOKS_PATH)))
    );
}

#[test]
fn builds_single_remote_fetch_arguments() {
    let arguments = fetch_arguments(&FetchOptions {
        remote: "origin".into(),
        refspecs: vec!["refs/heads/main:refs/remotes/origin/main".into()],
        prune: true,
    })
    .unwrap();
    assert_eq!(
        as_strings(&arguments),
        vec![
            "fetch",
            "--prune",
            "origin",
            "refs/heads/main:refs/remotes/origin/main",
        ]
    );
}

#[test]
fn rejects_credential_bearing_https_remote_urls() {
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
    assert!(validate_git_remote_url("git://token-value@gitee.com/team/repository.git").is_err());
    assert!(validate_git_remote_url("ext::unsafe-helper %S").is_err());
    assert!(validate_git_remote_url("--mirror=fetch").is_err());
}

#[test]
fn parses_porcelain_v2_status_with_rename_and_untracked_path() {
    let raw = concat!(
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

    let tags = parse_tags(b"v1.0.0\0abc123\nv0.9.0\0def456\n").unwrap();
    assert_eq!(tags[0].name, "v1.0.0");
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
    assert!(is_remote_branch_fetch_refspec(
        "+refs/heads/main:refs/remotes/origin/main"
    ));
    assert!(!is_remote_branch_fetch_refspec(
        "+refs/changes/*:refs/remotes/origin/changes/*"
    ));
}

#[test]
fn migrates_single_branch_fetch_scope_tracks_remote_branch_and_prunes_deletion() {
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
        let status = Command::new("git")
            .args(["-C", repository.to_str().unwrap()])
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    };

    let remote_initialized = Command::new("git")
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

    let cloned = Command::new("git")
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
            "+refs/changes/*:refs/remotes/origin/changes/*",
        ],
    );
    git.ensure_remote_fetches_all_branches(&clone, "origin")
        .unwrap();
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

    let configured_refspecs = Command::new("git")
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
    assert_eq!(
        String::from_utf8_lossy(&configured_refspecs.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "+refs/changes/*:refs/remotes/origin/changes/*",
            "+refs/heads/*:refs/remotes/origin/*",
        ]
    );

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
    )
    .as_bytes();
    let submodules = parse_submodules(raw).unwrap();
    assert_eq!(submodules.len(), 4);
    assert_eq!(submodules[0].path, "modules/ready");
    assert_eq!(submodules[0].status, SubmoduleStatus::Initialized);
    assert_eq!(submodules[1].status, SubmoduleStatus::Uninitialized);
    assert_eq!(submodules[2].status, SubmoduleStatus::NeedsUpdate);
    assert_eq!(submodules[3].status, SubmoduleStatus::Conflicted);
}

#[test]
fn parses_lfs_file_names_and_validates_patterns() {
    assert_eq!(
        parse_lfs_files(b"assets/video.mov\nassets/model.bin\n\n"),
        vec!["assets/video.mov", "assets/model.bin"]
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
        let status = Command::new("git")
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
    git.apply_patch(&repository, &patch, false).unwrap();
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
    git.apply_patch(&repository, &cached_patch, true).unwrap();
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
    let initialized = Command::new("git")
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
fn discard_staged_rename_restores_both_original_and_destination_paths() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-discard-rename-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).unwrap();
    let run_git = |arguments: &[&str]| {
        let status = Command::new("git")
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

    GitCore::new()
        .discard_paths(&repository, &["new-name.txt".into()])
        .expect("discarding a staged rename restores HEAD");

    assert_eq!(
        fs::read_to_string(repository.join("old-name.txt")).unwrap(),
        "original\n"
    );
    assert!(!repository.join("new-name.txt").exists());

    let _ = fs::remove_dir_all(repository);
}

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
    let initialized = Command::new("git")
        .args(["-C", repository.to_str().unwrap(), "init", "-q"])
        .status()
        .unwrap();
    assert!(initialized.success());
    let configured = Command::new("git")
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

    assert_eq!(
        GitCore::new()
            .ensure_network_configuration_is_safe(&repository)
            .expect_err("repository-local helper must not run for a selected account")
            .code(),
        "unsafe_git_config"
    );

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
    let initialized = Command::new("git")
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
    assert!(diff.contains("+++ b/new file.txt"));
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
    let initialized = Command::new("git")
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
    assert!(commits[1].parents.is_empty());
    assert_eq!(commits[1].summary, "Initial commit");

    let stashes = parse_stashes(
        b"stash@{0}\0On main: first line\nsecond line\0\nstash@{1}\0On topic: draft\0",
    )
    .unwrap();
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].reference, "stash@{0}");
    assert_eq!(stashes[0].message, "On main: first line\nsecond line");
    assert_eq!(stashes[1].message, "On topic: draft");
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
        let status = Command::new("git")
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

    let _ = fs::remove_dir_all(repository);
}

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
        let status = Command::new("git")
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

    let git = GitCore::new();
    let blame = git.blame(&repository, "tracked.txt").unwrap();
    assert!(blame.contains("Inspection Test"));
    let reflog = git.reflog(&repository, 10).unwrap();
    assert!(reflog.contains("initial tracked file"));
    let preview = git.preview_clean_untracked(&repository).unwrap();
    assert!(preview.contains("Would remove untracked.txt"));
    assert!(preview.contains("Would remove generated/"));
    // A later file must not be captured by the previously reviewed set.
    fs::write(repository.join("created-after-preview.txt"), "keep me\n").unwrap();

    git.clean_untracked_paths(
        &repository,
        &["untracked.txt".into(), "generated/output.txt".into()],
    )
    .unwrap();
    assert!(!repository.join("untracked.txt").exists());
    assert!(!repository.join("generated").exists());
    assert!(repository.join("created-after-preview.txt").exists());
    assert!(repository.join("tracked.txt").exists());

    let _ = fs::remove_dir_all(repository);
}

#[cfg(unix)]
#[test]
fn opens_actual_conflict_files_with_the_configured_editor() {
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
        let status = Command::new("git")
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
        Command::new("git")
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

    let merge = Command::new("git")
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
    let opened = GitCore::new()
        .open_conflicted_files_in_external_editor(
            &repository,
            &ExternalEditorConfig {
                executable: "true".into(),
                arguments: Vec::new(),
            },
        )
        .expect("configured editor receives each conflict file");
    assert_eq!(opened, vec!["conflict.txt"]);

    let _ = Command::new("git")
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
fn discard_path_validation_cannot_escape_the_worktree() {
    assert!(validate_worktree_relative_path("src/App.vue").is_ok());
    assert!(validate_worktree_relative_path("nested/file name.txt").is_ok());
    assert!(validate_worktree_relative_path("../outside.txt").is_err());
    assert!(validate_worktree_relative_path("./file.txt").is_err());
    assert!(validate_worktree_relative_path(".").is_err());
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
