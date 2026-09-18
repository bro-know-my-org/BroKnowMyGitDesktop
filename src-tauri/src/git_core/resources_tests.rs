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

#[test]
fn resource_root_commit_rename_and_delete_use_the_correct_paths() {
    let directory = repository();
    let root = directory.path();
    let core = GitCore::new();
    fs::write(root.join("old.png"), b"image\0").unwrap();
    git(root, &["add", "."]);
    let unborn = core
        .resource_sides(root, &comparison("old.png", true, None))
        .unwrap();
    assert!(unborn[0].is_none());
    git(root, &["commit", "-qm", "initial"]);
    let initial = git(root, &["rev-parse", "HEAD"]);
    let first = core
        .resource_sides(root, &comparison("old.png", false, Some(initial)))
        .unwrap();
    assert!(first[0].is_none());
    assert_eq!(content(&core, root, &first[1]).unwrap(), b"image\0");
    git(root, &["mv", "old.png", "new.png"]);
    let staged = core
        .resource_sides(root, &comparison("new.png", true, None))
        .unwrap();
    assert_eq!(content(&core, root, &staged[0]).unwrap(), b"image\0");
    git(root, &["commit", "-qm", "rename"]);
    let mut request = comparison("new.png", false, Some("HEAD".into()));
    request.previous_path = Some("old.png".into());
    let renamed = core.resource_sides(root, &request).unwrap();
    assert_eq!(
        content(&core, root, &renamed[0]),
        content(&core, root, &renamed[1])
    );
    git(root, &["rm", "new.png"]);
    git(root, &["commit", "-qm", "delete"]);
    let deleted = core
        .resource_sides(root, &comparison("new.png", false, Some("HEAD".into())))
        .unwrap();
    assert!(deleted[0].is_some());
    assert!(deleted[1].is_none());
}

#[test]
fn resource_paths_are_literal_and_cannot_escape_the_repository() {
    let directory = repository();
    let root = directory.path();
    let core = GitCore::new();
    let name = "image[1].png";
    fs::write(root.join(name), b"literal\0").unwrap();
    let sides = core
        .resource_sides(root, &comparison(name, false, None))
        .unwrap();
    assert!(sides[0].is_none());
    assert_eq!(content(&core, root, &sides[1]).unwrap(), b"literal\0");
    assert!(core
        .resource_sides(root, &comparison("../outside.png", false, None))
        .is_err());
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
    let committed = core
        .resource_sides(root, &comparison(name, false, Some("HEAD".into())))
        .unwrap();
    assert_eq!(content(&core, root, &committed[1]).unwrap(), b"literal\0");
}

#[cfg(unix)]
#[test]
fn resource_symlink_cannot_read_outside_worktree() {
    use std::os::unix::fs::symlink;
    let directory = repository();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.png"), b"secret").unwrap();
    symlink(
        outside.path().join("secret.png"),
        directory.path().join("link.png"),
    )
    .unwrap();
    let sides = GitCore::new()
        .resource_sides(directory.path(), &comparison("link.png", false, None))
        .unwrap();
    assert!(sides[1].as_ref().unwrap().source.is_none());
}

#[test]
fn resource_export_digest_rejects_same_size_timestamp_preserving_replacement() {
    let directory = repository();
    let root = directory.path();
    let path = root.join("large.png");
    fs::write(&path, b"old\0").unwrap();
    let core = GitCore::new();
    let mut sides = core
        .resource_sides(root, &comparison("large.png", false, None))
        .unwrap();
    let side = sides[1].as_mut().unwrap();
    if let Some(ResourceSource::Worktree { digest, .. }) = &mut side.source {
        *digest =
            Some(copy_and_hash(fs::File::open(&path).unwrap(), &mut std::io::sink(), 4).unwrap());
    }
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, b"new\0").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(core
        .copy_resource(root, side, &mut tempfile::tempfile().unwrap())
        .is_err());
}

#[test]
fn resource_large_untracked_binary_bypasses_text_diff_limit() {
    let directory = repository();
    let path = directory.path().join("unknown.dat");
    let mut file = fs::File::create(path).unwrap();
    file.write_all(&vec![b'A'; 9000]).unwrap();
    file.set_len(5 * 1024 * 1024).unwrap();
    let diff = GitCore::new()
        .diff(
            directory.path(),
            &super::super::DiffOptions {
                staged: false,
                paths: vec!["unknown.dat".into()],
                context_lines: Some(3),
            },
        )
        .unwrap();
    assert!(diff.starts_with("Binary files "));
}

#[test]
fn oversized_worktree_resource_automatically_pins_content_digest() {
    let directory = repository();
    let path = directory.path().join("large.dat");
    let file = fs::File::create(&path).unwrap();
    file.set_len(PREVIEW_LIMIT + 1).unwrap();
    let sides = GitCore::new()
        .resource_sides(directory.path(), &comparison("large.dat", false, None))
        .unwrap();
    assert!(matches!(
        sides[1].as_ref().unwrap().source,
        Some(ResourceSource::Worktree {
            digest: Some(_),
            ..
        })
    ));
}
