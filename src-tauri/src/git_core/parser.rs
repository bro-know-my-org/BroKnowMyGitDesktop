use super::error::*;
use super::*;

pub(super) fn parse_status_porcelain_v2(raw: &[u8]) -> Result<GitStatus, AppError> {
    let mut status = GitStatus {
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
    };
    let mut cursor = 0;

    while cursor < raw.len() {
        if raw[cursor] == 0 {
            cursor += 1;
            continue;
        }

        if raw[cursor..].starts_with(b"# ") {
            let end = raw[cursor..]
                .iter()
                .position(|byte| *byte == b'\n' || *byte == 0)
                .map(|offset| cursor + offset)
                .unwrap_or(raw.len());
            parse_status_header(&raw[cursor..end], &mut status)?;
            cursor = end.saturating_add(1);
            continue;
        }

        let end = raw[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| cursor + offset)
            .unwrap_or(raw.len());
        let record = &raw[cursor..end];
        cursor = end.saturating_add(1);
        if record.is_empty() {
            continue;
        }

        match record.first().copied() {
            Some(b'1') => status.entries.push(parse_status_ordinary(record)?),
            Some(b'2') => {
                if cursor >= raw.len() {
                    return Err(status_parse_error());
                }
                let original_end = raw[cursor..]
                    .iter()
                    .position(|byte| *byte == 0)
                    .map(|offset| cursor + offset)
                    .unwrap_or(raw.len());
                if original_end == cursor || (original_end == raw.len() && !raw.ends_with(&[0])) {
                    return Err(status_parse_error());
                }
                let original_path = parse_status_path(&raw[cursor..original_end])?;
                cursor = original_end.saturating_add(1);
                status
                    .entries
                    .push(parse_status_renamed(record, original_path)?);
            }
            Some(b'u') => status.entries.push(parse_status_unmerged(record)?),
            // A short untracked record has no index side. Keep the worktree
            // marker but normalize its absent index status to the same empty
            // value used by ordinary `.<status>` records.
            Some(b'?') => status.entries.push(parse_status_short(record, "", "?")?),
            Some(b'!') => status.entries.push(parse_status_short(record, "", "!")?),
            _ => return Err(status_parse_error()),
        }
    }

    Ok(status)
}

pub(super) fn parse_status_header(record: &[u8], status: &mut GitStatus) -> Result<(), AppError> {
    let value = std::str::from_utf8(record).map_err(|_| status_parse_error())?;
    let value = value.strip_prefix("# ").ok_or_else(status_parse_error)?;
    if let Some(branch) = value.strip_prefix("branch.head ") {
        status.branch = match branch {
            "(detached)" | "(unknown)" => None,
            "" => return Err(status_parse_error()),
            branch => Some(branch.to_owned()),
        };
    } else if let Some(upstream) = value.strip_prefix("branch.upstream ") {
        status.upstream = (!upstream.is_empty()).then(|| upstream.to_owned());
    } else if let Some(ahead_behind) = value.strip_prefix("branch.ab ") {
        let mut values = ahead_behind.split_whitespace();
        let ahead = values.next().and_then(|value| value.strip_prefix('+'));
        let behind = values.next().and_then(|value| value.strip_prefix('-'));
        if values.next().is_some() {
            return Err(status_parse_error());
        }
        status.ahead = ahead
            .and_then(|value| value.parse().ok())
            .ok_or_else(status_parse_error)?;
        status.behind = behind
            .and_then(|value| value.parse().ok())
            .ok_or_else(status_parse_error)?;
    }
    Ok(())
}

pub(super) fn parse_status_ordinary(record: &[u8]) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 9)?;
    status_entry_from_xy(fields[1], fields[8], None)
}

pub(super) fn parse_status_renamed(
    record: &[u8],
    original_path: String,
) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 10)?;
    status_entry_from_xy(fields[1], fields[9], Some(original_path))
}

pub(super) fn parse_status_unmerged(record: &[u8]) -> Result<GitStatusEntry, AppError> {
    let fields = split_fields(record, 11)?;
    status_entry_from_xy(fields[1], fields[10], None)
}

pub(super) fn parse_status_short(
    record: &[u8],
    index_status: &str,
    worktree_status: &str,
) -> Result<GitStatusEntry, AppError> {
    if record.len() < 3 || record[1] != b' ' {
        return Err(status_parse_error());
    }
    Ok(GitStatusEntry {
        path: parse_status_path(&record[2..])?,
        original_path: None,
        index_status: index_status.to_owned(),
        worktree_status: worktree_status.to_owned(),
    })
}

pub(super) fn split_fields(record: &[u8], count: usize) -> Result<Vec<&[u8]>, AppError> {
    let mut fields = Vec::with_capacity(count);
    let mut remaining = record;
    for _ in 0..count.saturating_sub(1) {
        let Some(separator) = remaining.iter().position(|byte| *byte == b' ') else {
            return Err(status_parse_error());
        };
        fields.push(&remaining[..separator]);
        remaining = &remaining[separator + 1..];
    }
    if remaining.is_empty() {
        return Err(status_parse_error());
    }
    fields.push(remaining);
    Ok(fields)
}

pub(super) fn status_entry_from_xy(
    xy: &[u8],
    path: &[u8],
    original_path: Option<String>,
) -> Result<GitStatusEntry, AppError> {
    if xy.len() != 2 {
        return Err(status_parse_error());
    }
    Ok(GitStatusEntry {
        path: parse_status_path(path)?,
        original_path,
        index_status: status_side_from_porcelain(xy[0])?,
        worktree_status: status_side_from_porcelain(xy[1])?,
    })
}

/// Status paths flow back through IPC into mutating Git commands. The domain
/// model is UTF-8 text, so reject non-UTF-8 filenames instead of replacing
/// bytes with U+FFFD and potentially operating on a different file later.
fn parse_status_path(raw: &[u8]) -> Result<String, AppError> {
    let path = std::str::from_utf8(raw).map_err(|_| status_parse_error())?;
    if path.is_empty() {
        return Err(status_parse_error());
    }
    Ok(path.to_owned())
}

/// Git porcelain v2 uses `.` for a clean side of an ordinary status record.
/// The application model represents an absent index/worktree change as an
/// empty string, which lets all UI consumers use one consistent check rather
/// than treating `.` as a real modification.
pub(super) fn status_side_from_porcelain(status: u8) -> Result<String, AppError> {
    match status {
        b'.' => Ok(String::new()),
        b'M' | b'T' | b'A' | b'D' | b'R' | b'C' | b'U' => Ok((status as char).to_string()),
        _ => Err(status_parse_error()),
    }
}

pub(super) fn parse_branches(raw: &[u8]) -> Result<Vec<BranchInfo>, AppError> {
    let mut branches = Vec::new();
    let raw = std::str::from_utf8(raw)
        .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 分支信息"))?;
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 3 || fields[1].is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 分支信息"));
        }
        branches.push(BranchInfo {
            name: fields[1].to_owned(),
            is_current: fields[0].trim() == "*",
            upstream: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
        });
    }
    Ok(branches)
}

pub(super) fn parse_remote_branches(
    raw: &[u8],
    configured_remote_names: &[String],
) -> Result<Vec<RemoteBranchInfo>, AppError> {
    let mut remote_names = configured_remote_names
        .iter()
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    // A remote name may itself contain `/`; use the longest configured name
    // first so `team/origin` is not accidentally parsed as `team`.
    remote_names.sort_by_key(|name| std::cmp::Reverse(name.len()));

    let raw = std::str::from_utf8(raw)
        .map_err(|_| AppError::new("git_parse_error", "无法解析远程分支信息"))?;
    let mut branches = Vec::new();
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\0').collect::<Vec<_>>();
        if fields.len() != 2 || fields[0].is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析远程分支信息"));
        }
        // `refs/remotes/<remote>/HEAD` is a symbolic ref pointing at the
        // remote's default branch, not a branch users can check out.
        if !fields[1].is_empty() {
            continue;
        }
        let Some((remote_name, branch_name)) = remote_names.iter().find_map(|remote_name| {
            let prefix = format!("refs/remotes/{remote_name}/");
            fields[0]
                .strip_prefix(&prefix)
                .filter(|name| !name.is_empty() && *name != "HEAD")
                .map(|name| ((*remote_name).clone(), name.to_owned()))
        }) else {
            // Git keeps remote-tracking refs after removing a remote. They
            // have no usable fetch configuration, so omit them from the UI.
            continue;
        };
        branches.push(RemoteBranchInfo {
            remote_name,
            name: branch_name,
        });
    }
    branches.sort_by(|left, right| {
        left.remote_name
            .cmp(&right.remote_name)
            .then(left.name.cmp(&right.name))
    });
    Ok(branches)
}

pub(super) fn parse_worktrees(raw: &[u8]) -> Result<Vec<WorktreeInfo>, AppError> {
    let mut worktrees = Vec::new();
    let mut fields = Vec::new();
    for field in raw.split(|byte| *byte == 0) {
        if field.is_empty() {
            if !fields.is_empty() {
                worktrees.push(parse_worktree_fields(&fields, worktrees.is_empty())?);
                fields.clear();
            }
        } else {
            fields.push(field);
        }
    }
    if !fields.is_empty() {
        worktrees.push(parse_worktree_fields(&fields, worktrees.is_empty())?);
    }
    Ok(worktrees)
}

pub(super) fn parse_worktree_fields(
    fields: &[&[u8]],
    is_main: bool,
) -> Result<WorktreeInfo, AppError> {
    let mut path = None;
    let mut head = String::new();
    let mut branch = None;
    let mut is_detached = false;
    let mut is_locked = false;
    let mut is_prunable = false;

    for field in fields {
        let field = std::str::from_utf8(field)
            .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 工作树信息"))?;
        if let Some(value) = field.strip_prefix("worktree ") {
            if value.is_empty() || path.replace(value.to_owned()).is_some() {
                return Err(AppError::new("git_parse_error", "无法解析 Git 工作树信息"));
            }
        } else if let Some(value) = field.strip_prefix("HEAD ") {
            head = value.to_owned();
        } else if let Some(value) = field.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_owned(),
            );
        } else if field == "detached" {
            is_detached = true;
        } else if field == "locked" || field.starts_with("locked ") {
            is_locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            is_prunable = true;
        }
    }

    Ok(WorktreeInfo {
        path: path.ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 工作树信息"))?,
        head,
        branch,
        is_main,
        is_detached,
        is_locked,
        is_prunable,
    })
}

pub(super) fn parse_submodules(raw: &[u8]) -> Result<Vec<SubmoduleInfo>, AppError> {
    let mut submodules = Vec::new();
    let raw = std::str::from_utf8(raw)
        .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Some((&marker, remainder)) = line.as_bytes().split_first() else {
            return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"));
        };
        let status = match marker {
            b' ' => SubmoduleStatus::Initialized,
            b'-' => SubmoduleStatus::Uninitialized,
            b'+' => SubmoduleStatus::NeedsUpdate,
            b'U' => SubmoduleStatus::Conflicted,
            _ => return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态")),
        };
        let remainder = std::str::from_utf8(remainder)
            .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
        let mut fields = remainder.trim_start().splitn(2, char::is_whitespace);
        let commit = fields
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
        let path_with_description = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
        let path = parse_submodule_status_path(path_with_description)?;
        submodules.push(SubmoduleInfo {
            path,
            commit: commit.to_owned(),
            status,
        });
    }
    Ok(submodules)
}

/// `git submodule status` quotes paths that contain whitespace or special
/// bytes. Parse that C-style field before skipping its optional parenthesized
/// status description, so a path such as `modules/foo (old)` is not shortened
/// to a different filesystem location.
fn parse_submodule_status_path(value: &str) -> Result<String, AppError> {
    let value = value.trim_start();
    let path = if value.starts_with('"') {
        let (path, remainder) = parse_git_c_quoted_path(value)?;
        let remainder = remainder.trim();
        if !(remainder.is_empty() || (remainder.starts_with('(') && remainder.ends_with(')'))) {
            return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"));
        }
        path
    } else {
        // Plain paths do not contain spaces in Git's unquoted output. Treat a
        // final parenthesized field as Git's status description only when it
        // is syntactically complete; quoted paths above retain literal ` (`.
        value
            .rsplit_once(" (")
            .filter(|(_, description)| description.ends_with(')'))
            .map(|(path, _)| path)
            .unwrap_or(value)
            .trim_end()
            .to_owned()
    };
    if path.is_empty() {
        return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"));
    }
    Ok(path)
}

fn parse_git_c_quoted_path(value: &str) -> Result<(String, &str), AppError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let path = String::from_utf8(decoded)
                    .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 子模块状态"))?;
                return Ok((path, &value[index + 1..]));
            }
            b'\\' => {
                index += 1;
                let Some(escape) = bytes.get(index).copied() else {
                    break;
                };
                match escape {
                    b'\\' | b'"' => decoded.push(escape),
                    b'a' => decoded.push(0x07),
                    b'b' => decoded.push(0x08),
                    b't' => decoded.push(b'\t'),
                    b'n' => decoded.push(b'\n'),
                    b'v' => decoded.push(0x0b),
                    b'f' => decoded.push(0x0c),
                    b'r' => decoded.push(b'\r'),
                    b'0'..=b'7' => {
                        let mut octal = 0_u16;
                        for _ in 0..3 {
                            let Some(digit) = bytes.get(index).copied() else {
                                return Err(AppError::new(
                                    "git_parse_error",
                                    "无法解析 Git 子模块状态",
                                ));
                            };
                            if !(b'0'..=b'7').contains(&digit) {
                                return Err(AppError::new(
                                    "git_parse_error",
                                    "无法解析 Git 子模块状态",
                                ));
                            }
                            octal = octal * 8 + u16::from(digit - b'0');
                            index += 1;
                        }
                        if octal > u16::from(u8::MAX) {
                            return Err(AppError::new(
                                "git_parse_error",
                                "无法解析 Git 子模块状态",
                            ));
                        }
                        decoded.push(octal as u8);
                        continue;
                    }
                    _ => {
                        return Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"));
                    }
                }
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    Err(AppError::new("git_parse_error", "无法解析 Git 子模块状态"))
}

/// `git config --null --file .gitmodules --get-regexp` writes one
/// `key\nvalue\0` record per configured submodule path. This is used for
/// safety preflight because the value is machine-readable and never needs to
/// be inferred from `git submodule status`'s presentation text.
pub(super) fn parse_submodule_config_paths(raw: &[u8]) -> Result<Vec<String>, AppError> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for record in raw
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err(AppError::new("git_parse_error", "无法解析 Git 子模块配置"));
        };
        let path = &record[separator + 1..];
        let path = std::str::from_utf8(path)
            .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 子模块配置"))?;
        if path.is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 子模块配置"));
        }
        if seen.insert(path.to_owned()) {
            paths.push(path.to_owned());
        }
    }
    Ok(paths)
}

pub(super) fn parse_lfs_files(raw: &[u8]) -> Vec<String> {
    raw.split(|byte| *byte == b'\n')
        .map(|path| path.strip_suffix(b"\r").unwrap_or(path))
        .filter(|path| !path.is_empty())
        .map(lossy)
        .collect()
}

pub(super) fn parse_commits(raw: &[u8]) -> Result<Vec<CommitSummary>, AppError> {
    let mut fields: Vec<_> = raw.split(|byte| *byte == 0).collect();
    if matches!(fields.last(), Some(field) if field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 7 != 0 {
        return Err(AppError::new("git_parse_error", "无法解析 Git 提交历史"));
    }

    let mut commits = Vec::with_capacity(fields.len() / 7);
    for (record_index, fields) in fields.chunks_exact(7).enumerate() {
        // `git log --format=format:...` inserts one LF between adjacent
        // records. It precedes the next SHA, outside the NUL-delimited data.
        let sha = if record_index == 0 {
            fields[0]
        } else {
            fields[0].strip_prefix(b"\n").unwrap_or(fields[0])
        };
        if sha.is_empty() || [1, 3, 4, 5].iter().any(|index| fields[*index].is_empty()) {
            return Err(AppError::new("git_parse_error", "无法解析 Git 提交历史"));
        }
        commits.push(CommitSummary {
            sha: lossy(sha),
            short_sha: lossy(fields[1]),
            parents: lossy(fields[2])
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect(),
            tags: Vec::new(),
            tags_truncated: false,
            pushed: None,
            author_name: lossy(fields[3]),
            author_email: lossy(fields[4]),
            authored_at: lossy(fields[5]),
            summary: lossy(fields[6]),
        });
    }
    Ok(commits)
}

pub(super) fn parse_stashes(raw: &[u8]) -> Result<Vec<StashEntry>, AppError> {
    let mut fields: Vec<_> = raw.split(|byte| *byte == 0).collect();
    if matches!(fields.last(), Some(field) if field.is_empty()) {
        fields.pop();
    }
    if matches!(fields.last(), Some(field) if *field == b"\n") {
        fields.pop();
    }
    if fields.len() % 3 != 0 {
        return Err(AppError::new("git_parse_error", "无法解析 Git 暂存栈"));
    }

    let mut stashes = Vec::with_capacity(fields.len() / 3);
    for (index, fields) in fields.chunks_exact(3).enumerate() {
        let reference = if index == 0 {
            fields[0]
        } else {
            fields[0].strip_prefix(b"\n").unwrap_or(fields[0])
        };
        if reference.is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 暂存栈"));
        }
        let reference = std::str::from_utf8(reference)
            .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 暂存栈"))?;
        let commit_id = std::str::from_utf8(fields[1])
            .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 暂存栈"))?;
        if !(40..=64).contains(&commit_id.len())
            || !commit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AppError::new("git_parse_error", "无法解析 Git 暂存栈"));
        }
        stashes.push(StashEntry {
            reference: reference.to_owned(),
            commit_id: commit_id.to_owned(),
            message: lossy(fields[2]),
        });
    }
    Ok(stashes)
}

pub(super) fn parse_tags(raw: &[u8]) -> Result<Vec<TagInfo>, AppError> {
    let mut tags = Vec::new();
    let raw = std::str::from_utf8(raw)
        .map_err(|_| AppError::new("git_parse_error", "无法解析 Git 标签信息"))?;
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 3 || fields[0].is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 标签信息"));
        }
        let target = if fields[1].is_empty() {
            fields[2]
        } else {
            fields[1]
        };
        if target.is_empty() {
            return Err(AppError::new("git_parse_error", "无法解析 Git 标签信息"));
        }
        tags.push(TagInfo {
            name: fields[0].to_owned(),
            target: target.to_owned(),
        });
    }
    Ok(tags)
}
