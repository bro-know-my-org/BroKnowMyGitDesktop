use super::*;

pub(super) fn git_command_error(operation: &'static str, output: &Output) -> AppError {
    const MAX_CLASSIFICATION_BYTES: usize = 64 * 1024;
    let stderr = bounded_diagnostic_bytes(&output.stderr, MAX_CLASSIFICATION_BYTES);
    let stdout = bounded_diagnostic_bytes(&output.stdout, MAX_CLASSIFICATION_BYTES);
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stderr),
        String::from_utf8_lossy(&stdout)
    )
    .to_ascii_lowercase();
    git_error_from_diagnostic(operation, &diagnostic)
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
    let diagnostic = diagnostic.to_ascii_lowercase();
    let (code, guidance) = if contains_any(
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
