//! Resolve actual comparison sides; never run textconv, smudge filters or external diff tools.
use super::environment::apply_git_environment;
use super::error::output_text;
use super::models::{command_arguments, os_arguments};
use super::validation::{literal_pathspec, validate_worktree_relative_path};
use super::*;

pub(crate) const PREVIEW_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(crate) enum ResourceSource {
    Blob(String),
    Worktree {
        path: String,
        modified: Option<std::time::SystemTime>,
        digest: Option<[u8; 32]>,
    },
}

#[derive(Clone)]
pub(crate) struct ResourceSide {
    pub path: String,
    pub size: u64,
    pub source: Option<ResourceSource>,
    pub notice: Option<&'static str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceComparison {
    pub path: String,
    pub previous_path: Option<String>,
    pub staged: bool,
    pub revision: Option<String>,
}

fn resource_error() -> AppError {
    AppError::new(
        "resource_unavailable",
        "无法读取此版本的资源。请刷新后重试。",
    )
}

fn open_worktree(repository: &Path, path: &str) -> Result<cap_std::fs::File, std::io::Error> {
    let directory = cap_std::fs::Dir::open_ambient_dir(repository, cap_std::ambient_authority())?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    // cap-std resolves every component beneath the directory handle, including symlinks.
    directory.open_with(path, &options)
}

fn copy_and_hash(
    reader: impl Read,
    writer: &mut impl Write,
    size: u64,
) -> Result<[u8; 32], AppError> {
    let mut reader = reader.take(size.saturating_add(1));
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut copied = 0;
    let started = Instant::now();
    loop {
        let length = reader.read(&mut buffer).map_err(|_| resource_error())?;
        if length == 0 {
            break;
        }
        copied += length as u64;
        if copied > size || started.elapsed() > Duration::from_secs(120) {
            return Err(resource_error());
        }
        hash.update(&buffer[..length]);
        writer
            .write_all(&buffer[..length])
            .map_err(|_| resource_error())?;
    }
    if copied != size {
        return Err(resource_error());
    }
    Ok(hash.finalize().into())
}

impl GitCore {
    pub(super) fn is_binary_resource(
        &self,
        repository: &Path,
        path: &str,
    ) -> Result<bool, AppError> {
        let file = open_worktree(repository, path).map_err(|_| resource_error())?;
        let metadata = file.metadata().map_err(|_| resource_error())?;
        if !metadata.is_file() {
            return Err(resource_error());
        }
        // Files beyond the text diff budget still get metadata/export, even if
        // their binary signature is after this cheap prefix probe.
        if metadata.len() > 4 * 1024 * 1024 {
            return Ok(true);
        }
        let mut bytes = Vec::new();
        file.take(8192)
            .read_to_end(&mut bytes)
            .map_err(|_| resource_error())?;
        Ok(bytes.contains(&0)
            || std::str::from_utf8(&bytes).is_err_and(|error| error.error_len().is_some()))
    }

    pub(crate) fn resource_sides(
        &self,
        repository: &Path,
        comparison: &ResourceComparison,
    ) -> Result<[Option<ResourceSide>; 2], AppError> {
        validate_worktree_relative_path(&comparison.path)?;
        if let Some(previous) = &comparison.previous_path {
            validate_worktree_relative_path(previous)?;
        }
        let path = comparison.path.as_str();
        if let Some(revision) = &comparison.revision {
            let (commit, parent) = self.commit_and_first_parent(repository, revision)?;
            let previous = comparison.previous_path.as_deref().unwrap_or(path);
            return Ok([
                parent
                    .map(|parent| self.tree_resource(repository, &parent, previous))
                    .transpose()?
                    .flatten(),
                self.tree_resource(repository, &commit, path)?,
            ]);
        }
        let status = self.status(repository)?;
        let entry = status
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or_else(resource_error)?;
        if comparison.staged {
            let previous = entry.original_path.as_deref().unwrap_or(path);
            let head = self.execute(
                Some(repository),
                &os_arguments(&["rev-parse", "--verify", "HEAD"]),
                &GitEnvironment::default(),
            )?;
            let old = if head.status.success() {
                self.tree_resource(repository, output_text(&head.stdout).trim(), previous)?
            } else if self.has_head(repository)? {
                return Err(resource_error());
            } else {
                None
            };
            Ok([old, self.index_resource(repository, path)?])
        } else {
            let previous = if matches!(entry.worktree_status.as_str(), "R" | "C") {
                entry.original_path.as_deref().unwrap_or(path)
            } else {
                path
            };
            Ok([
                self.index_resource(repository, previous)?,
                self.worktree_resource(repository, path)?,
            ])
        }
    }

    fn tree_resource(
        &self,
        repository: &Path,
        revision: &str,
        path: &str,
    ) -> Result<Option<ResourceSide>, AppError> {
        let output = self.run(
            Some(repository),
            &[
                "ls-tree".into(),
                "-z".into(),
                revision.into(),
                "--".into(),
                literal_pathspec(path),
            ],
            &GitEnvironment::new().with_variable("GIT_NO_LAZY_FETCH", "1"),
            "读取资源版本",
        )?;
        let Some(record) = output
            .stdout
            .split(|byte| *byte == 0)
            .find(|record| !record.is_empty())
        else {
            return Ok(None);
        };
        let header = record
            .split(|byte| *byte == b'\t')
            .next()
            .ok_or_else(resource_error)?;
        let fields = std::str::from_utf8(header)
            .map_err(|_| resource_error())?
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(resource_error());
        }
        self.blob_resource(repository, path, fields[0], fields[2])
            .map(Some)
    }

    fn index_resource(
        &self,
        repository: &Path,
        path: &str,
    ) -> Result<Option<ResourceSide>, AppError> {
        let output = self.run(
            Some(repository),
            &[
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                literal_pathspec(path),
            ],
            &GitEnvironment::default(),
            "读取暂存资源",
        )?;
        let Some(record) = output
            .stdout
            .split(|byte| *byte == 0)
            .find(|record| !record.is_empty())
        else {
            return Ok(None);
        };
        let header = record
            .split(|byte| *byte == b'\t')
            .next()
            .ok_or_else(resource_error)?;
        let fields = std::str::from_utf8(header)
            .map_err(|_| resource_error())?
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(resource_error());
        }
        if fields[2] != "0" {
            return Ok(Some(ResourceSide {
                path: path.into(),
                size: 0,
                source: None,
                notice: Some("conflict"),
            }));
        }
        self.blob_resource(repository, path, fields[0], fields[1])
            .map(Some)
    }

    fn blob_resource(
        &self,
        repository: &Path,
        path: &str,
        mode: &str,
        oid: &str,
    ) -> Result<ResourceSide, AppError> {
        if !matches!(mode, "100644" | "100755") {
            return Ok(ResourceSide {
                path: path.into(),
                size: 0,
                source: None,
                notice: Some("special"),
            });
        }
        if !oid.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(oid.len(), 40 | 64) {
            return Err(resource_error());
        }
        let output = self.run(
            Some(repository),
            &os_arguments(&["--no-replace-objects", "cat-file", "-s", oid]),
            &GitEnvironment::new().with_variable("GIT_NO_LAZY_FETCH", "1"),
            "读取资源大小",
        )?;
        let size = output_text(&output.stdout)
            .trim()
            .parse()
            .map_err(|_| resource_error())?;
        Ok(ResourceSide {
            path: path.into(),
            size,
            source: Some(ResourceSource::Blob(oid.into())),
            notice: None,
        })
    }

    fn worktree_resource(
        &self,
        repository: &Path,
        path: &str,
    ) -> Result<Option<ResourceSide>, AppError> {
        let directory =
            cap_std::fs::Dir::open_ambient_dir(repository, cap_std::ambient_authority())
                .map_err(|_| resource_error())?;
        match directory.symlink_metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                return Ok(Some(ResourceSide {
                    path: path.into(),
                    size: metadata.len(),
                    source: None,
                    notice: Some("special"),
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(resource_error()),
            _ => {}
        }
        let file = match open_worktree(repository, path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(resource_error()),
        };
        let metadata = file.metadata().map_err(|_| resource_error())?;
        let regular = metadata.is_file();
        // Oversized files have no cached preview bytes. Pin their content identity
        // now, so a later export cannot silently read a replacement at this path.
        let digest = if regular && metadata.len() > PREVIEW_LIMIT {
            Some(copy_and_hash(&file, &mut std::io::sink(), metadata.len())?)
        } else {
            None
        };
        if file
            .metadata()
            .map_err(|_| resource_error())?
            .modified()
            .ok()
            != metadata.modified().ok()
        {
            return Err(resource_error());
        }
        Ok(Some(ResourceSide {
            path: path.into(),
            size: metadata.len(),
            source: regular.then(|| ResourceSource::Worktree {
                path: path.into(),
                modified: metadata.modified().ok().map(|time| time.into_std()),
                digest,
            }),
            notice: (!regular).then_some("special"),
        }))
    }

    /// Copy to a private snapshot/export file without buffering a whole media file in IPC/RAM.
    pub(crate) fn copy_resource(
        &self,
        repository: &Path,
        side: &ResourceSide,
        output: &mut fs::File,
    ) -> Result<(), AppError> {
        match side.source.as_ref().ok_or_else(resource_error)? {
            ResourceSource::Worktree {
                path,
                modified,
                digest,
            } => {
                let file = open_worktree(repository, path).map_err(|_| resource_error())?;
                let before = file.metadata().map_err(|_| resource_error())?;
                if !before.is_file()
                    || before.len() != side.size
                    || before.modified().ok().map(|time| time.into_std()) != *modified
                {
                    return Err(resource_error());
                }
                let actual = copy_and_hash(&file, output, side.size)?;
                let after = file.metadata().map_err(|_| resource_error())?;
                if digest.is_some_and(|expected| actual != expected)
                    || before.modified().ok() != after.modified().ok()
                {
                    return Err(resource_error());
                }
            }
            ResourceSource::Blob(oid) => {
                // cat-file writes only the requested object, with no filters or child helpers.
                let mut command = Command::new(&self.executable);
                command
                    .args(command_arguments(
                        Some(repository),
                        &os_arguments(&["--no-replace-objects", "cat-file", "blob", oid]),
                    ))
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .stdout(output.try_clone().map_err(|_| resource_error())?);
                apply_git_environment(&mut command, &GitEnvironment::default());
                command.env("GIT_NO_LAZY_FETCH", "1");
                let mut child = command.spawn().map_err(|_| resource_error())?;
                let started = Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                return Err(resource_error());
                            }
                            break;
                        }
                        Ok(None) if started.elapsed() < Duration::from_secs(120) => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(resource_error());
                        }
                    }
                }
                if output.metadata().map_err(|_| resource_error())?.len() != side.size {
                    return Err(resource_error());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
