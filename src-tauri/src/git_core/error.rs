use super::*;

pub(super) fn git_command_error(operation: &'static str, output: &Output) -> AppError {
    const MAX_CLASSIFICATION_BYTES: usize = 64 * 1024;
    let stderr = bounded_diagnostic_bytes(&output.stderr, MAX_CLASSIFICATION_BYTES);
    let stdout = bounded_diagnostic_bytes(&output.stdout, MAX_CLASSIFICATION_BYTES);
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stderr),
        String::from_utf8_lossy(&stdout)
    );
    git_error_from_diagnostic(operation, &diagnostic)
}

pub(super) fn git_command_error_with_metadata_directories(
    operation: &'static str,
    output: &Output,
    metadata_directories: &[PathBuf],
) -> AppError {
    const MAX_CLASSIFICATION_BYTES: usize = 64 * 1024;
    let stderr = bounded_diagnostic_bytes(&output.stderr, MAX_CLASSIFICATION_BYTES);
    let stdout = bounded_diagnostic_bytes(&output.stdout, MAX_CLASSIFICATION_BYTES);
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stderr),
        String::from_utf8_lossy(&stdout)
    );
    git_error_from_diagnostic_with_metadata_directories(
        operation,
        &diagnostic,
        metadata_directories,
    )
}

pub(super) fn git_output_may_report_lock_contention(output: &Output) -> bool {
    const MAX_CLASSIFICATION_BYTES: usize = 64 * 1024;
    let stderr = bounded_diagnostic_bytes(&output.stderr, MAX_CLASSIFICATION_BYTES);
    let stdout = bounded_diagnostic_bytes(&output.stdout, MAX_CLASSIFICATION_BYTES);
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stderr),
        String::from_utf8_lossy(&stdout)
    )
    .to_ascii_lowercase();
    contains_any(&diagnostic, &["file exists", "already exists"])
        && (diagnostic.contains(".lock")
            || diagnostic.contains("could not lock config file")
            || diagnostic.contains("/logs/")
            || diagnostic.contains("\\logs\\"))
}

/// Retain both ends of a bounded diagnostic. Remote helpers often print
/// progress first and their actionable failure last; inspecting only a prefix
/// would turn those failures into an unhelpful generic error.
fn bounded_diagnostic_bytes(bytes: &[u8], maximum: usize) -> Vec<u8> {
    if bytes.len() <= maximum {
        return bytes.to_vec();
    }

    let head_length = maximum / 2;
    let tail_length = maximum - head_length;
    let mut retained = Vec::with_capacity(maximum);
    retained.extend_from_slice(&bytes[..head_length]);
    retained.extend_from_slice(&bytes[bytes.len() - tail_length..]);
    retained
}

/// Classifies known Git failures but never returns original process output.
/// Git may echo a URL or a server response containing a token, so forwarding
/// raw stdout/stderr would violate the credential boundary.
pub(super) fn git_error_from_diagnostic(operation: &'static str, diagnostic: &str) -> AppError {
    classify_git_error_from_diagnostic(operation, diagnostic, &[], false)
}

pub(super) fn git_error_from_diagnostic_with_metadata_directories(
    operation: &'static str,
    diagnostic: &str,
    metadata_directories: &[PathBuf],
) -> AppError {
    classify_git_error_from_diagnostic(operation, diagnostic, metadata_directories, true)
}

fn classify_git_error_from_diagnostic(
    operation: &'static str,
    diagnostic: &str,
    metadata_directories: &[PathBuf],
    metadata_context_supplied: bool,
) -> AppError {
    let original_diagnostic = diagnostic;
    let diagnostic = diagnostic.to_ascii_lowercase();
    let lock_contention_clauses = diagnostic
        .lines()
        .zip(original_diagnostic.lines())
        .map(|(lowercase_line, original_line)| {
            (lowercase_line.trim_start(), original_line.trim_start())
        })
        .filter(|(lowercase_line, _)| {
            (lowercase_line.starts_with("fatal:") || lowercase_line.starts_with("error:"))
                && contains_any(lowercase_line, &["file exists", "already exists"])
        })
        .map(|(_, original_line)| original_line.trim_start())
        .collect::<Vec<_>>();
    let reference_lock = lock_contention_clauses.iter().any(|line| {
        [
            "packed-refs.lock",
            "HEAD.lock",
            "AUTO_MERGE.lock",
            "BISECT_HEAD.lock",
            "CHERRY_PICK_HEAD.lock",
            "FETCH_HEAD.lock",
            "MERGE_HEAD.lock",
            "ORIG_HEAD.lock",
            "REBASE_HEAD.lock",
            "REVERT_HEAD.lock",
        ]
        .iter()
        .any(|name| {
            contains_git_metadata_component(
                line,
                name,
                metadata_directories,
                metadata_context_supplied,
            )
        }) || contains_reference_lock_path(
            line,
            '/',
            metadata_directories,
            metadata_context_supplied,
        ) || contains_reference_lock_path(
            line,
            '\\',
            metadata_directories,
            metadata_context_supplied,
        ) || contains_reflog_contention_path(
            line,
            '/',
            metadata_directories,
            metadata_context_supplied,
        ) || contains_reflog_contention_path(
            line,
            '\\',
            metadata_directories,
            metadata_context_supplied,
        )
    });
    let configuration_lock = lock_contention_clauses.iter().any(|line| {
        contains_git_metadata_component(
            line,
            "config.lock",
            metadata_directories,
            metadata_context_supplied,
        ) || contains_git_metadata_component(
            line,
            "config.worktree.lock",
            metadata_directories,
            metadata_context_supplied,
        ) || contains_repository_config_path(
            line,
            '/',
            metadata_directories,
            metadata_context_supplied,
        ) || contains_repository_config_path(
            line,
            '\\',
            metadata_directories,
            metadata_context_supplied,
        ) || contains_resolved_repository_config_path(line, metadata_directories)
    });
    let index_lock = lock_contention_clauses.iter().any(|line| {
        contains_git_metadata_component(
            line,
            "index.lock",
            metadata_directories,
            metadata_context_supplied,
        )
    });
    let shallow_lock = lock_contention_clauses.iter().any(|line| {
        contains_git_metadata_component(
            line,
            "shallow.lock",
            metadata_directories,
            metadata_context_supplied,
        )
    });
    let (code, guidance) = if reference_lock {
        (
            "git_reference_locked",
            "请关闭可能正在操作此仓库的其他 Git 客户端，检查引用锁后重试。",
        )
    } else if configuration_lock {
        (
            "git_config_locked",
            "请关闭可能正在操作此仓库的其他 Git 客户端，检查锁文件后重试。",
        )
    } else if index_lock {
        (
            "git_index_locked",
            "请关闭可能正在操作此仓库的其他 Git 客户端，检查暂存区锁后重试。",
        )
    } else if shallow_lock {
        (
            "git_lock_detected",
            "请关闭可能正在操作此仓库的其他 Git 客户端，检查 Git 锁文件后重试。",
        )
    } else if contains_any(
        &diagnostic,
        &[
            "authentication failed",
            "could not read username",
            "terminal prompts disabled",
            "permission denied (publickey)",
            "http basic: access denied",
            "invalid username or password",
        ],
    ) {
        (
            "git_authentication_failed",
            "请检查 remote 绑定的账号、SSH agent 或系统 Git credential helper。",
        )
    } else if operation == "推送改动"
        && (contains_any(
            &diagnostic,
            &[
                "write access to repository not granted",
                "not allowed to push code to this project",
                "not allowed to push code to this repository",
            ],
        ) || (diagnostic.contains("permission to ") && diagnostic.contains(" denied")))
    {
        (
            "git_push_permission_denied",
            "当前远程仓库拒绝了写入；请 Fork 到自己的账号后再推送。",
        )
    } else if contains_any(
        &diagnostic,
        &[
            "could not resolve host",
            "failed to connect",
            "network is unreachable",
            "connection timed out",
        ],
    ) {
        ("git_network_error", "请检查网络连接、代理和远程服务状态。")
    } else if contains_any(
        &diagnostic,
        &[
            "repository not found",
            "does not appear to be a git repository",
            "remote: not found",
        ],
    ) {
        (
            "git_remote_not_found",
            "请检查远程地址、访问权限和账号绑定。",
        )
    } else if contains_any(
        &diagnostic,
        &[
            "couldn't find remote ref",
            "could not find remote ref",
            "remote ref does not exist",
        ],
    ) {
        (
            "git_remote_ref_not_found",
            "远端分支可能已删除；请获取远程更新并刷新分支列表。",
        )
    } else if diagnostic.contains("not a git repository") {
        (
            "not_a_git_repository",
            "请确认所选目录仍是有效的 Git 工作区。",
        )
    } else if diagnostic.contains("conflict") {
        ("git_merge_conflict", "Git 检测到冲突；请处理冲突后再继续。")
    } else {
        (
            "git_command_failed",
            "Git 未能完成操作；请检查仓库状态和 Git 配置。",
        )
    };
    AppError::with_safe_details(code, "Git 命令失败。", guidance)
}

pub(super) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn contains_reference_lock_path(
    line: &str,
    separator: char,
    metadata_directories: &[PathBuf],
    metadata_context_supplied: bool,
) -> bool {
    let unresolved_absolute_path = metadata_context_supplied && line_contains_absolute_path(line);
    diagnostic_path_tokens(line).into_iter().any(|token| {
        if !path_has_lock_suffix(token) {
            return false;
        }
        if !diagnostic_token_is_absolute(token) {
            if unresolved_absolute_path {
                return false;
            }
            return token
                .split(separator)
                .find(|component| !component.is_empty() && *component != ".")
                .is_some_and(|component| path_component_eq(component, "refs"));
        }
        if metadata_context_supplied {
            return resolved_metadata_relative_path(token, metadata_directories).is_some_and(
                |relative| {
                    path_starts_with_components(&relative, &["refs"])
                        && path_has_lock_suffix(&relative)
                },
            );
        }
        git_metadata_relative_components(token, separator).is_some_and(|relative| {
            relative
                .first()
                .is_some_and(|component| path_component_eq(component, "refs"))
        })
    })
}

fn contains_git_metadata_component(
    line: &str,
    component: &str,
    metadata_directories: &[PathBuf],
    metadata_context_supplied: bool,
) -> bool {
    let unresolved_absolute_path = metadata_context_supplied && line_contains_absolute_path(line);
    diagnostic_path_tokens(line).into_iter().any(|token| {
        let resolved =
            token_has_resolved_metadata_relative_path(token, component, metadata_directories);
        if metadata_context_supplied
            && (diagnostic_token_is_absolute(token) || unresolved_absolute_path)
        {
            resolved
        } else {
            resolved
                || token_has_git_metadata_relative_path(token, '/', component)
                || token_has_git_metadata_relative_path(token, '\\', component)
        }
    })
}

fn diagnostic_path_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut token_start = None;
    let mut quote = None;
    for (index, character) in line.char_indices() {
        if let Some(expected_quote) = quote {
            if character == expected_quote {
                if let Some(start) = token_start.take() {
                    let token = line[start..index].trim_end_matches(':');
                    if !token.is_empty() {
                        tokens.push(token);
                    }
                }
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            if let Some(start) = token_start.take() {
                let token = line[start..index].trim_end_matches(':');
                if !token.is_empty() {
                    tokens.push(token);
                }
            }
            quote = Some(character);
            token_start = Some(index + character.len_utf8());
        } else if character.is_whitespace() {
            if let Some(start) = token_start.take() {
                let token = line[start..index].trim_end_matches(':');
                if !token.is_empty() {
                    tokens.push(token);
                }
            }
        } else if token_start.is_none() {
            token_start = Some(index);
        }
    }
    if let Some(start) = token_start {
        let token = line[start..].trim_end_matches([':', '\'', '"']);
        if !token.is_empty() {
            tokens.push(token);
        }
    }
    tokens
}

fn token_has_git_metadata_relative_path(token: &str, separator: char, expected: &str) -> bool {
    git_metadata_relative_components(token, separator).is_some_and(|relative| {
        (relative.len() == 1 && path_component_eq(relative[0], expected))
            || (relative.len() == 3
                && path_component_eq(relative[0], "worktrees")
                && !relative[1].is_empty()
                && path_component_eq(relative[2], expected))
    })
}

fn token_has_resolved_metadata_relative_path(
    token: &str,
    expected: &str,
    metadata_directories: &[PathBuf],
) -> bool {
    resolved_metadata_relative_path(token, metadata_directories)
        .is_some_and(|relative| path_eq_components(&relative, &[expected]))
}

fn git_metadata_relative_components(token: &str, separator: char) -> Option<Vec<&str>> {
    let components = token.split(separator).collect::<Vec<_>>();
    let metadata_index = components
        .iter()
        .rposition(|component| is_git_metadata_root_component(component))?;
    Some(components[metadata_index + 1..].to_vec())
}

fn contains_reflog_contention_path(
    line: &str,
    separator: char,
    metadata_directories: &[PathBuf],
    metadata_context_supplied: bool,
) -> bool {
    let unresolved_absolute_path = metadata_context_supplied && line_contains_absolute_path(line);
    diagnostic_path_tokens(line).into_iter().any(|token| {
        let resolved =
            resolved_metadata_relative_path(token, metadata_directories).is_some_and(|relative| {
                (path_eq_components(&relative, &["logs", "HEAD.lock"])
                    || path_starts_with_components(&relative, &["logs", "refs", "heads"]))
                    && path_has_lock_suffix(&relative)
            });
        if metadata_context_supplied
            && (diagnostic_token_is_absolute(token) || unresolved_absolute_path)
        {
            return resolved;
        }
        resolved
            || git_metadata_relative_components(token, separator).is_some_and(|relative| {
                (relative.len() == 2
                    && path_component_eq(relative[0], "logs")
                    && path_component_eq(relative[1], "HEAD.lock"))
                    || (relative.len() > 3
                        && path_component_eq(relative[0], "logs")
                        && path_component_eq(relative[1], "refs")
                        && path_component_eq(relative[2], "heads"))
            }) && path_has_lock_suffix(token)
    })
}

fn resolved_metadata_relative_path(
    token: &str,
    metadata_directories: &[PathBuf],
) -> Option<String> {
    let normalized_token = normalize_diagnostic_path(token);
    let lexical_match = metadata_directories.iter().find_map(|directory| {
        let normalized_directory = normalize_diagnostic_path(&directory.to_string_lossy());
        let normalized_directory = normalized_directory.trim_end_matches('/');
        normalized_token
            .strip_prefix(&format!("{normalized_directory}/"))
            .map(str::to_owned)
    });
    if lexical_match.is_some() {
        return lexical_match;
    }
    let canonical_token = Path::new(token).canonicalize().ok()?;
    let normalized_token = normalize_diagnostic_path(&canonical_token.to_string_lossy());
    metadata_directories.iter().find_map(|directory| {
        let normalized_directory = normalize_diagnostic_path(&directory.to_string_lossy());
        let normalized_directory = normalized_directory.trim_end_matches('/');
        normalized_token
            .strip_prefix(&format!("{normalized_directory}/"))
            .map(str::to_owned)
    })
}

fn normalize_diagnostic_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    #[cfg(windows)]
    {
        path.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn contains_repository_config_path(
    line: &str,
    separator: char,
    metadata_directories: &[PathBuf],
    metadata_context_supplied: bool,
) -> bool {
    let unresolved_absolute_path = metadata_context_supplied && line_contains_absolute_path(line);
    diagnostic_path_tokens(line).into_iter().any(|token| {
        let resolved =
            token_has_resolved_metadata_relative_path(token, "config", metadata_directories)
                || token_has_resolved_metadata_relative_path(
                    token,
                    "config.worktree",
                    metadata_directories,
                );
        if metadata_context_supplied
            && (diagnostic_token_is_absolute(token) || unresolved_absolute_path)
        {
            resolved
        } else {
            resolved
                || token_has_git_metadata_relative_path(token, separator, "config")
                || token_has_git_metadata_relative_path(token, separator, "config.worktree")
        }
    })
}

fn contains_resolved_repository_config_path(line: &str, metadata_directories: &[PathBuf]) -> bool {
    let normalized_line = normalize_diagnostic_path(line);
    metadata_directories.iter().any(|directory| {
        let normalized_directory = normalize_diagnostic_path(&directory.to_string_lossy());
        let normalized_directory = normalized_directory.trim_end_matches('/');
        ["config", "config.worktree"].iter().any(|name| {
            let candidate = format!("{normalized_directory}/{name}");
            normalized_line
                .match_indices(&candidate)
                .any(|(index, matched)| {
                    let before_is_boundary = normalized_line[..index]
                        .chars()
                        .next_back()
                        .is_none_or(is_path_token_boundary);
                    before_is_boundary
                        && normalized_line[index + matched.len()..]
                            .chars()
                            .next()
                            .is_none_or(is_diagnostic_path_delimiter)
                })
        })
    })
}

fn diagnostic_token_is_absolute(token: &str) -> bool {
    token.starts_with('/') || token.starts_with('\\') || token.as_bytes().get(1) == Some(&b':')
}

fn line_contains_absolute_path(line: &str) -> bool {
    diagnostic_path_tokens(line)
        .into_iter()
        .any(diagnostic_token_is_absolute)
}

fn path_has_lock_suffix(path: &str) -> bool {
    #[cfg(windows)]
    {
        path.to_ascii_lowercase().ends_with(".lock")
    }
    #[cfg(not(windows))]
    {
        path.ends_with(".lock")
    }
}

fn is_path_token_boundary(character: char) -> bool {
    !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-' | '.')
}

fn is_diagnostic_path_delimiter(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\'' | '"' | ':')
}

fn is_git_metadata_root_component(component: &str) -> bool {
    path_component_eq(component, ".git")
        || component
            .strip_suffix(".git")
            .is_some_and(|prefix| !prefix.is_empty())
        || cfg!(windows)
            && component
                .to_ascii_lowercase()
                .strip_suffix(".git")
                .is_some_and(|prefix| !prefix.is_empty())
}

fn path_eq_components(path: &str, expected: &[&str]) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    components.len() == expected.len()
        && components
            .iter()
            .zip(expected)
            .all(|(left, right)| path_component_eq(left, right))
}

fn path_starts_with_components(path: &str, expected: &[&str]) -> bool {
    let mut components = path.split('/');
    expected.iter().all(|expected| {
        components
            .next()
            .is_some_and(|actual| path_component_eq(actual, expected))
    })
}

fn path_component_eq(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub(super) fn safe_remote_url(raw: &str) -> String {
    // The common domain sanitizer is always the first pass, as required for
    // every remote URL sent over IPC.  The second pass removes HTTP query and
    // fragment values from pre-existing user config, which could otherwise
    // carry an access token.
    let sanitized = sanitize_remote_url(raw);
    if let Ok(mut url) = Url::parse(&sanitized) {
        if matches!(url.scheme(), "http" | "https") {
            url.set_query(None);
            url.set_fragment(None);
        }
        return url.to_string();
    }
    sanitized
}

pub(super) fn repository_record_from_root(root: &Path) -> Result<RepositoryRecord, AppError> {
    let root = normalize_path(root);
    let path = root.to_str().ok_or_else(|| {
        AppError::new(
            "unsupported_repository_path_encoding",
            "仓库路径包含当前界面不支持的字符编码。",
        )
    })?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.to_owned());
    Ok(RepositoryRecord {
        path: path.to_owned(),
        name,
        alias: None,
        group: None,
        host: None,
        namespace: None,
        last_opened_at: None,
    })
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(super) fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(super) fn status_parse_error() -> AppError {
    AppError::new("git_parse_error", "无法解析 Git 仓库状态")
}
