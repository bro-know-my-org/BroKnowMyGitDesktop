fn run_push_tags_fixture_git(directory: &Path, arguments: &[&str]) -> String {
    let output = test_git_command()
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success(), "Git {arguments:?}: {output:?}");
    output_text(&output.stdout).trim().to_owned()
}

fn push_tags_fixture() -> (TemporaryTestDirectory, PathBuf, PathBuf) {
    let root = TemporaryTestDirectory::create(std::env::temp_dir().join("bkmg-push-tags"));
    let repository = root.join("repository");
    let remote = root.join("remote.git");
    run_push_tags_fixture_git(
        &root,
        &["init", "-q", "-b", "main", repository.to_str().unwrap()],
    );
    run_push_tags_fixture_git(&root, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    run_push_tags_fixture_git(&repository, &["config", "user.name", "Tags Test"]);
    run_push_tags_fixture_git(
        &repository,
        &["config", "user.email", "tags@example.invalid"],
    );
    run_push_tags_fixture_git(&repository, &["commit", "--allow-empty", "-qm", "initial"]);
    run_push_tags_fixture_git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    (root, repository, remote)
}

fn push_tags_options(git: &GitCore, repository: &Path, include_tags: bool) -> PushOptions {
    let (refspecs, set_upstream) = git.push_plan_for_remote(repository, "origin").unwrap();
    PushOptions {
        remote: "origin".into(),
        refspecs,
        set_upstream,
        force_with_lease: false,
        follow_tags: false,
        include_tags,
    }
}

#[test]
fn push_all_tags_handles_first_push_existing_upstream_and_tag_only_changes() {
    let (_root, repository, remote) = push_tags_fixture();
    let git = GitCore::new();
    run_push_tags_fixture_git(&repository, &["tag", "lightweight"]);
    run_push_tags_fixture_git(&repository, &["tag", "-a", "annotated", "-m", "release"]);
    git.push(&repository, &push_tags_options(&git, &repository, true))
        .unwrap();
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["tag", "--list"]),
        "annotated\nlightweight"
    );
    assert_eq!(
        git.status(&repository).unwrap().upstream.as_deref(),
        Some("origin/main")
    );
    assert_eq!(git.status(&repository).unwrap().ahead, 0);

    run_push_tags_fixture_git(&repository, &["commit", "--allow-empty", "-qm", "second"]);
    run_push_tags_fixture_git(&repository, &["tag", "second"]);
    git.push(&repository, &push_tags_options(&git, &repository, true))
        .unwrap();
    assert_eq!(git.status(&repository).unwrap().ahead, 0);
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["rev-parse", "main"]),
        git.head_commit_id(&repository).unwrap()
    );

    // All tags includes tags on other branches, even when main has no new commits.
    run_push_tags_fixture_git(&repository, &["checkout", "-qb", "other"]);
    run_push_tags_fixture_git(&repository, &["commit", "--allow-empty", "-qm", "other"]);
    run_push_tags_fixture_git(&repository, &["tag", "other-branch"]);
    let other = git.head_commit_id(&repository).unwrap();
    run_push_tags_fixture_git(&repository, &["checkout", "-q", "main"]);
    git.push(&repository, &push_tags_options(&git, &repository, true))
        .unwrap();
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["rev-parse", "refs/tags/other-branch"]),
        other
    );
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["branch", "--list", "other"]),
        ""
    );
}

#[test]
fn push_all_tags_is_opt_in_and_conflicts_leave_branch_and_tracking_unchanged() {
    let (_root, repository, remote) = push_tags_fixture();
    let git = GitCore::new();
    run_push_tags_fixture_git(&repository, &["tag", "lightweight"]);
    run_push_tags_fixture_git(&repository, &["tag", "-a", "annotated", "-m", "release"]);
    run_push_tags_fixture_git(&repository, &["config", "push.followTags", "true"]);
    git.push(&repository, &push_tags_options(&git, &repository, false))
        .unwrap();
    assert_eq!(run_push_tags_fixture_git(&remote, &["tag", "--list"]), "");
    git.push(&repository, &push_tags_options(&git, &repository, true))
        .unwrap();
    let before = git.head_commit_id(&repository).unwrap();

    run_push_tags_fixture_git(&repository, &["commit", "--allow-empty", "-qm", "second"]);
    run_push_tags_fixture_git(&repository, &["tag", "-f", "lightweight"]);
    run_push_tags_fixture_git(&repository, &["tag", "new-tag"]);
    assert!(git
        .push(&repository, &push_tags_options(&git, &repository, true))
        .is_err());
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["rev-parse", "main"]),
        before
    );
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["rev-parse", "refs/tags/lightweight"]),
        before
    );
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["tag", "--list", "new-tag"]),
        ""
    );
    assert_eq!(
        run_push_tags_fixture_git(&repository, &["rev-parse", "origin/main"]),
        before
    );
    assert_eq!(git.status(&repository).unwrap().ahead, 1);

    let mut forced = push_tags_options(&git, &repository, true);
    forced.force_with_lease = true;
    assert_eq!(
        git.push(&repository, &forced).unwrap_err().code(),
        "push_tags_with_force_unsupported"
    );
}

#[test]
fn push_all_tags_requires_atomic_support_without_partial_updates() {
    let (_root, repository, remote) = push_tags_fixture();
    let git = GitCore::new();
    run_push_tags_fixture_git(&remote, &["config", "receive.advertiseAtomic", "false"]);
    run_push_tags_fixture_git(&repository, &["tag", "release"]);
    assert!(git
        .push(&repository, &push_tags_options(&git, &repository, true))
        .is_err());
    assert_eq!(
        run_push_tags_fixture_git(&remote, &["for-each-ref", "--format=%(refname)"]),
        ""
    );
    assert!(git.status(&repository).unwrap().upstream.is_none());
    // Ordinary push remains available on this server.
    git.push(&repository, &push_tags_options(&git, &repository, false))
        .unwrap();
    assert_eq!(git.status(&repository).unwrap().ahead, 0);
}
