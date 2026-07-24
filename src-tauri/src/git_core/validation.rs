use super::error::*;
use super::*;

pub(super) fn validate_paths(paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Err(AppError::new(
            "invalid_pathspec",
            "请选择至少一个有效文件路径",
        ));
    }
    for path in paths {
        if path.is_empty() || path.contains('\0') {
            return Err(AppError::new(
                "invalid_pathspec",
                "请选择至少一个有效文件路径",
            ));
        }
        validate_worktree_relative_path(path)?;
    }
    Ok(())
}

/// UI paths identify one status entry, never a Git path pattern. Prefix them
/// with Git's literal magic after validation so names such as `:(glob)**` are
/// treated as filenames rather than a request to affect the whole worktree.
pub(super) fn literal_pathspec(path: &str) -> OsString {
    OsString::from(format!(":(literal){path}"))
}

/// Extra validation used before filesystem removal. Local deletion must prove
/// that a user-provided path cannot escape the worktree; Git-facing calls add
/// [`literal_pathspec`] separately to prevent pathspec magic expansion.
pub(super) fn validate_worktree_relative_path(value: &str) -> Result<(), AppError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\0')
        || value.contains(['\r', '\n'])
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AppError::new(
            "invalid_pathspec",
            "要丢弃的文件路径必须位于当前仓库内",
        ));
    }
    Ok(())
}

pub(super) fn validate_worktree_path(path: &Path) -> Result<(), AppError> {
    let value = path.to_string_lossy();
    if value.trim().is_empty() || !path.is_absolute() || value.starts_with('-') {
        return Err(AppError::new(
            "invalid_worktree_path",
            "工作树路径必须是有效的绝对路径",
        ));
    }
    Ok(())
}

pub(super) fn validate_lfs_pattern(value: &str) -> Result<(), AppError> {
    let pattern = value.trim();
    if pattern.is_empty()
        || pattern.starts_with('-')
        || pattern.contains('\0')
        || pattern.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_lfs_pattern", "Git LFS 跟踪规则无效"));
    }
    Ok(())
}

pub(super) fn validate_git_patch(patch: &str) -> Result<(), AppError> {
    const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
    if patch.is_empty()
        || patch.len() > MAX_PATCH_BYTES
        || patch.contains('\0')
        || !patch.starts_with("diff --git ")
    {
        return Err(AppError::new(
            "invalid_git_patch",
            "代码块补丁无效或超过大小限制",
        ));
    }
    Ok(())
}

pub(super) fn paths_match(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

const MAX_UNTRACKED_DIFF_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct UntrackedFileContent {
    pub(super) bytes: Vec<u8>,
    pub(super) is_executable: bool,
}

pub(super) fn render_untracked_file_diff(path: &str, content: &UntrackedFileContent) -> String {
    let mode = if content.is_executable {
        "100755"
    } else {
        "100644"
    };
    let header = format!(
        "diff --git a/{path} b/{path}\nnew file mode {mode}\n--- /dev/null\n+++ b/{path}\n"
    );
    let Ok(text) = std::str::from_utf8(&content.bytes) else {
        return format!("{header}Binary files /dev/null and b/{path} differ\n");
    };
    if text.is_empty() {
        return header;
    }

    let mut diff = format!("{header}@@ -0,0 +1,{} @@\n", text.lines().count());
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        diff.push('+');
        diff.push_str(body);
        diff.push('\n');
    }
    if !text.ends_with('\n') {
        diff.push_str("\\ No newline at end of file\n");
    }
    diff
}

#[cfg(unix)]
pub(super) fn read_untracked_file_safely(
    repository: &Path,
    relative_path: &str,
) -> Result<UntrackedFileContent, AppError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    validate_worktree_relative_path(relative_path)?;
    let repository_path = CString::new(repository.as_os_str().as_bytes())
        .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
    // SAFETY: `repository_path` is NUL-terminated and remains live during the
    // call. O_NOFOLLOW pins the repository root to a real directory instead
    // of resolving a symlink supplied by another local process.
    let root_fd = unsafe {
        libc::open(
            repository_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "无法安全读取未跟踪文件的改动内容。",
        ));
    }
    // SAFETY: open returned a new owned descriptor on success.
    let mut parent = unsafe { fs::File::from_raw_fd(root_fd) };
    let components: Vec<_> = Path::new(relative_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => Some(component),
            _ => None,
        })
        .collect();
    let Some((file_name, parent_components)) = components.split_last() else {
        return Err(AppError::new("invalid_pathspec", "未跟踪文件路径无效"));
    };

    for component in parent_components {
        let component = CString::new(component.as_bytes())
            .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
        // SAFETY: parent is an open directory descriptor and component is a
        // NUL-terminated single path component. O_NOFOLLOW prevents an
        // intermediate replacement from escaping the worktree.
        let directory_fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_fd < 0 {
            return Err(AppError::new(
                "untracked_diff_unavailable",
                "无法安全读取未跟踪文件的改动内容。",
            ));
        }
        // SAFETY: openat returned a new owned descriptor on success.
        parent = unsafe { fs::File::from_raw_fd(directory_fd) };
    }

    let file_name = CString::new(file_name.as_bytes())
        .map_err(|_| AppError::new("invalid_pathspec", "未跟踪文件路径无效"))?;
    // SAFETY: parent is pinned above and file_name is a NUL-terminated final
    // component. O_NOFOLLOW ensures this final lookup cannot follow a swapped
    // symlink outside the opened worktree hierarchy.
    let file_fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if file_fd < 0 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "无法安全读取未跟踪文件的改动内容。",
        ));
    }
    // SAFETY: openat returned a new owned descriptor on success.
    let file = unsafe { fs::File::from_raw_fd(file_fd) };
    let metadata = file
        .metadata()
        .map_err(|_| AppError::new("untracked_diff_unavailable", "无法读取未跟踪文件。"))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_UNTRACKED_DIFF_BYTES as u64 {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "未跟踪文件不是可安全显示的常规文本文件，或内容过大。",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    // Metadata is only a point-in-time size check. Bound the read itself so a
    // concurrent writer cannot grow the file into an unbounded allocation.
    let mut limited = file.take(MAX_UNTRACKED_DIFF_BYTES as u64 + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::new("untracked_diff_unavailable", "无法读取未跟踪文件。"))?;
    if bytes.len() > MAX_UNTRACKED_DIFF_BYTES {
        return Err(AppError::new(
            "untracked_diff_unavailable",
            "未跟踪文件内容过大，无法安全显示。",
        ));
    }
    Ok(UntrackedFileContent {
        bytes,
        is_executable: metadata.permissions().mode() & 0o111 != 0,
    })
}

#[cfg(not(unix))]
pub(super) fn read_untracked_file_safely(
    _repository: &Path,
    _relative_path: &str,
) -> Result<UntrackedFileContent, AppError> {
    // The standard library does not provide descriptor-relative no-follow
    // traversal on this platform. Do not fall back to a path check followed by
    // a second path open, which could expose data outside the worktree.
    Err(AppError::new(
        "untracked_diff_unavailable",
        "当前平台无法安全读取未跟踪文件的改动内容。",
    ))
}

pub(super) fn validate_remote_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty()
        || name.starts_with('-')
        || name.contains('\0')
        || name.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_remote_name", "远程名称无效"));
    }
    Ok(())
}

pub(super) fn all_remote_branches_fetch_refspec(remote_name: &str) -> Result<String, AppError> {
    validate_remote_name(remote_name)?;
    Ok(format!("+refs/heads/*:refs/remotes/{remote_name}/*"))
}

pub(super) fn is_remote_branch_fetch_refspec(refspec: &str) -> bool {
    refspec
        .strip_prefix('+')
        .unwrap_or(refspec)
        .starts_with("refs/heads/")
}

pub(super) fn remote_fetch_refspec_config_key(remote_name: &str) -> String {
    format!("remote.{remote_name}.fetch")
}

pub(super) fn remote_tracking_reference(remote_name: &str, branch: &str) -> String {
    format!("refs/remotes/{remote_name}/{branch}")
}

pub(super) fn remote_tracking_branch_name(remote_name: &str, branch: &str) -> String {
    format!("{remote_name}/{branch}")
}

pub(super) fn validate_ref_name(value: &str, _label: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_git_reference", "Git 引用无效。"));
    }
    Ok(())
}

/// Revision expressions such as `HEAD~2` and `stash@{0}` are valid Git
/// inputs but still must never be interpreted as an option or span lines.
pub(super) fn validate_revision(value: &str, _label: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_git_revision", "Git 修订版本无效。"));
    }
    Ok(())
}

pub(super) fn validate_refspec(value: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains('\0')
        || value.contains(['\r', '\n'])
    {
        return Err(AppError::new("invalid_refspec", "引用规格无效"));
    }
    Ok(())
}

/// Validate every URL form accepted by `git clone` and remote configuration.
/// This intentionally permits public HTTP, `git://`, file, and local-path
/// remotes while rejecting values that could inject options or credentials.
pub(crate) fn validate_git_remote_url(raw: &str) -> Result<(), AppError> {
    let value = raw.trim();
    if value.is_empty() || value.starts_with('-') || value.contains(['\r', '\n', '\0']) {
        return Err(AppError::new("invalid_remote_url", "远程地址无效"));
    }
    if raw_http_url_contains_unsafe_metadata(value) {
        return Err(AppError::new(
            "credential_in_remote_url",
            "远程地址不能包含认证信息或令牌",
        ));
    }

    if has_supported_url_scheme(value)
        && (has_invalid_percent_escape(value) || Url::parse(value).is_err())
    {
        return Err(AppError::new("invalid_remote_url", "远程地址无效"));
    }
    if let Ok(url) = Url::parse(value) {
        let scheme = url.scheme().to_ascii_lowercase();
        let http = matches!(scheme.as_str(), "http" | "https");
        let unauthenticated = matches!(scheme.as_str(), "git" | "file");
        if !matches!(
            scheme.as_str(),
            "https" | "http" | "ssh" | "git+ssh" | "git" | "file"
        ) {
            return Err(AppError::new(
                "invalid_remote_url",
                "远程地址使用了不受支持的协议",
            ));
        }
        if !matches!(scheme.as_str(), "file") && url.host_str().is_none() {
            return Err(AppError::new("invalid_remote_url", "远程地址无效"));
        }
        if url.password().is_some()
            || (http && !url.username().is_empty())
            || (http && (url.query().is_some() || url.fragment().is_some()))
            || (matches!(scheme.as_str(), "ssh" | "git+ssh")
                && (url.query().is_some() || url.fragment().is_some()))
            || (unauthenticated
                && (!url.username().is_empty()
                    || url.query().is_some()
                    || url.fragment().is_some()))
        {
            return Err(AppError::new(
                "credential_in_remote_url",
                "远程地址不能包含认证信息或令牌",
            ));
        }
    }
    Ok(())
}

pub(super) fn has_supported_url_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "https" | "http" | "ssh" | "git+ssh" | "git" | "file"
    )
}

pub(super) fn has_invalid_percent_escape(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return true;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    false
}

/// Inspect the raw HTTP authority before generic URL parsing. A malformed URL
/// such as `https://token@%zz/repo` can fail parsing but would otherwise be
/// passed through to Git configuration with its userinfo intact.
pub(super) fn raw_http_url_contains_unsafe_metadata(value: &str) -> bool {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@') || remainder.contains(['?', '#'])
}

pub(super) fn is_unsafe_network_config_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    let unsafe_http_key = key.starts_with("http.")
        && (matches!(
            key.as_str(),
            "http.extraheader"
                | "http.proxy"
                | "http.sslverify"
                | "http.cookiefile"
                | "http.savecookies"
        ) || key.ends_with(".extraheader")
            || key.ends_with(".proxy")
            || key.ends_with(".sslverify")
            || key.ends_with(".cookiefile")
            || key.ends_with(".savecookies"));
    let unsafe_remote_key = key.starts_with("remote.")
        && (key.ends_with(".sshcommand")
            || key.ends_with(".uploadpack")
            || key.ends_with(".receivepack"));
    let unsafe_url_rewrite_key =
        key.starts_with("url.") && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof"));
    key == "core.sshcommand"
        || key == "core.gitproxy"
        || key == "include.path"
        || key.starts_with("includeif.") && key.ends_with(".path")
        || key == "credential.helper"
        || key.starts_with("credential.")
        || unsafe_http_key
        || unsafe_remote_key
        || unsafe_url_rewrite_key
}
