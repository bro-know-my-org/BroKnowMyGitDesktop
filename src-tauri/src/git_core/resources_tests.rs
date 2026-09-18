use super::*;

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", DISABLED_GIT_HOOKS_PATH)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(directory.path(), &["config", "user.name", "Resource Test"]);
    directory
}
fn comparison(path: &str, staged: bool, revision: Option<String>) -> ResourceComparison {
    ResourceComparison {
        path: path.into(),
        previous_path: None,
        staged,
        revision,
    }
}
fn content(core: &GitCore, directory: &Path, side: &Option<ResourceSide>) -> Option<Vec<u8>> {
    side.as_ref().map(|side| {
        let mut file = tempfile::tempfile().unwrap();
        core.copy_resource(directory, side, &mut file).unwrap();
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    })
}

#[test]
fn resource_comparison_keeps_head_index_and_worktree_distinct() {
    let directory = repository();
    let root = directory.path();
    let core = GitCore::new();
    fs::write(root.join("icon.png"), b"head\0").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
    fs::write(root.join("icon.png"), b"index\0").unwrap();
    git(root, &["add", "."]);
    fs::write(root.join("icon.png"), b"worktree\0").unwrap();
    let staged = core
        .resource_sides(root, &comparison("icon.png", true, None))
        .unwrap();
    assert_eq!(content(&core, root, &staged[0]).unwrap(), b"head\0");
    assert_eq!(content(&core, root, &staged[1]).unwrap(), b"index\0");
    let working = core
        .resource_sides(root, &comparison("icon.png", false, None))
        .unwrap();
    assert_eq!(content(&core, root, &working[0]).unwrap(), b"index\0");
    assert_eq!(content(&core, root, &working[1]).unwrap(), b"worktree\0");
    fs::write(root.join("icon.png"), b"changed again\0").unwrap();
    assert!(core
        .copy_resource(
            root,
            working[1].as_ref().unwrap(),
            &mut tempfile::tempfile().unwrap()
        )
        .is_err());
}
