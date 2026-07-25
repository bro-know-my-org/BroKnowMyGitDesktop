use super::editor::*;
use super::environment::*;
use super::error::*;
use super::models::*;
use super::parser::*;
use super::validation::*;

use super::*;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn as_strings(arguments: &[OsString]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn test_git_template_dir() -> &'static Path {
    static TEMPLATE_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    TEMPLATE_DIRECTORY
        .get_or_init(|| {
            // A fixed template path can retain files from an earlier failed
            // test run (or be pre-created), which would make every `git init`
            // fixture inherit untrusted config. Keep one fresh, randomized
            // empty directory alive for this process instead.
            let directory = std::env::temp_dir().join(format!(
                "broknowmygitdesktop-empty-git-template-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&directory).expect("the test Git template directory is available");
            directory
        })
        .as_path()
}

/// Builds a Git command for integration-test setup. Tests must not inherit a
/// developer's repository selection, configuration, hooks, or template
/// directory: those can make an otherwise local fixture operate on the wrong
/// repository or execute a user-installed hook.
fn test_git_command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH)
        .env("GIT_TEMPLATE_DIR", test_git_template_dir())
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-c")
        .arg(format!("core.hooksPath={DISABLED_GIT_HOOKS_PATH}"));
    command
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
            refspec: Some("refs/heads/feature".into()),
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
            "refs/heads/feature",
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
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is available")
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "broknowmygitdesktop-pull-upstream-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&repository).expect("test repository directory is created");
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
        git.pull_refspec_for_remote(&repository, "origin")
            .expect("the tracked branch produces its configured merge ref"),
        "refs/heads/feature"
    );
    assert_eq!(
        git.pull_refspec_for_remote(&repository, "upstream")
            .expect_err("a selected remote must match the current branch upstream")
            .code(),
        "pull_upstream_remote_mismatch"
    );

    let _ = fs::remove_dir_all(repository);
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
        "core.hooksPath",
        "merge.payload.driver",
    ] {
        assert!(
            is_unsafe_network_config_key(key),
            "{key} must not run beside an app-selected askpass capability"
        );
    }
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
